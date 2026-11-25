//! `OpenCode`: sessions, messages and parts live in `SQLite` (`opencode.db` under the XDG data
//! dir), one row per message and one per part, with the payload as JSON in `data`.
//!
//! Verified against opencode 1.18 source (`packages/core/src/database/database.ts`,
//! `packages/core/src/global.ts`, `packages/opencode/src/session/message-v2.ts`):
//!
//! - the database is `$OPENCODE_DB`, else `<xdg data>/opencode/opencode.db`, where the xdg
//!   data dir is `$XDG_DATA_HOME` or `~/.local/share` on every platform;
//! - `session.directory` is where `OpenCode` was started; child sessions (subagents) carry a
//!   `parent_id`; archived sessions carry `time_archived`;
//! - a message's text is its `part` rows of type `text`, in id order; parts flagged
//!   `synthetic` or `ignored` are injected context, not something the user or model wrote.
//!
//! `OpenCode` 2 (the `opencode2` binary, `anomalyco/opencode` branch `beta`) shares the data
//! directory and, on the standard channels, the same `opencode.db`, but writes to new tables:
//! `session_v2` (same columns this reader needs) and `session_message` (`type` column is the
//! role, `seq` is the order, `data` holds the payload: `content[].type == "text"` for an
//! assistant, `text` for a user). Other channels use `opencode-<channel>.db` next to it
//! (`packages/cli/src/server-process.ts`). A v1 session and a v2 session can therefore both
//! match one directory; the newest wins, whichever schema and file it lives in.
//!
//! `OpenCode` 2.4 (migration `20261029002625_session_part`) writes each part as its own
//! `session_part` row (`message_id`, `session_id`, `ordinal`, `type`, `data`) and leaves the
//! assistant payload's `content` empty. A reply's text is its payload text when it has any,
//! else its `text` parts in `ordinal` order; one source per reply, so none is listed twice
//! (`docs/decisions.md`, decision 14).

mod messages;
mod parts;
mod payload;

use std::path::Path;

use rusqlite::params;

use crate::sqlite::{open_read_only, sql_error};
use crate::{HostError, Role};
pub use messages::messages_for_session;

/// The database relative to `OpenCode`'s data directory.
pub const DB_FILE: &str = "opencode.db";
/// The data directory relative to the XDG data home.
pub const DATA_DIR: &str = "opencode";

const WHAT: &str = "OpenCode";

/// Which table family a session lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schema {
    /// `OpenCode` 1: `session`, `message`, `part`.
    V1,
    /// `OpenCode` 2: `session_v2`, `session_message`.
    V2,
}

/// A session picked for a directory, with what is needed to compare picks across files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub schema: Schema,
    /// `time_updated`, Unix milliseconds.
    pub updated: i64,
}

/// The newest top-level, unarchived session started in `cwd`, across both schemas. When none
/// was started exactly there, the newest one started in an ancestor of `cwd` (`OpenCode`
/// records the directory it was launched from; the pane may have moved since).
pub fn find_session(db: &Path, cwd: &Path) -> Result<Found, HostError> {
    let connection = open_read_only(db, WHAT)?;
    let cwd = normalize(&cwd.to_string_lossy());
    let mut exact: Option<Found> = None;
    let mut ancestor: Option<Found> = None;
    for (schema, table) in [(Schema::V2, "session_v2"), (Schema::V1, "session")] {
        if !has_table(&connection, table)? {
            continue;
        }
        let mut statement = connection
            .prepare(&format!(
                "SELECT id, directory, time_updated FROM {table} \
                 WHERE parent_id IS NULL AND time_archived IS NULL"
            ))
            .map_err(|e| sql_error(WHAT, &e))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            })
            .map_err(|e| sql_error(WHAT, &e))?;
        for row in rows {
            let (id, directory, updated) = row.map_err(|e| sql_error(WHAT, &e))?;
            let directory = normalize(&directory);
            let found = Found { id, schema, updated };
            if directory == cwd {
                newer(&mut exact, found);
            } else if is_ancestor(&directory, &cwd) {
                newer(&mut ancestor, found);
            }
        }
    }
    exact
        .or(ancestor)
        .ok_or_else(|| HostError::NoTranscript(format!("no OpenCode session for {cwd} in {}", db.display())))
}

/// Which schema holds `session_id`, for sessions addressed by id rather than found by cwd.
pub fn schema_of(db: &Path, session_id: &str) -> Result<Schema, HostError> {
    crate::validate_session_id(session_id)?;
    let connection = open_read_only(db, WHAT)?;
    for (schema, table) in [(Schema::V2, "session_v2"), (Schema::V1, "session")] {
        if !has_table(&connection, table)? {
            continue;
        }
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table} WHERE id = ?1"), params![session_id], |r| {
                r.get(0)
            })
            .map_err(|e| sql_error(WHAT, &e))?;
        if count > 0 {
            return Ok(schema);
        }
    }
    Err(HostError::NoMessages(format!("no OpenCode session {session_id} in {}", db.display())))
}

fn newer(slot: &mut Option<Found>, candidate: Found) {
    if slot.as_ref().is_none_or(|current| candidate.updated > current.updated) {
        *slot = Some(candidate);
    }
}

fn has_table(connection: &rusqlite::Connection, name: &str) -> Result<bool, HostError> {
    connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(|e| sql_error(WHAT, &e))
}

/// How many of `messages` are assistant replies: `n` counts those, not prompts.
fn replies(messages: &[crate::Message]) -> usize {
    messages.iter().filter(|message| message.role == Role::Assistant).count()
}

/// Paths compare without a trailing separator; Windows drive letters case-insensitively.
fn normalize(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if cfg!(windows) { trimmed.replace('\\', "/").to_ascii_lowercase() } else { trimmed.to_owned() }
}

fn is_ancestor(directory: &str, cwd: &str) -> bool {
    !directory.is_empty() && cwd.starts_with(directory) && cwd.as_bytes().get(directory.len()) == Some(&b'/')
}
