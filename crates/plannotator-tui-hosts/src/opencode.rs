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

use std::path::Path;

use rusqlite::params;

use crate::sqlite::{open_read_only, sql_error};
use crate::{HostError, Message, Role};

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

/// The newest `n` user and assistant messages of `session_id` that carry text, newest first.
pub fn messages_for_session(
    db: &Path,
    session_id: &str,
    schema: Schema,
    n: usize,
) -> Result<Vec<Message>, HostError> {
    crate::validate_session_id(session_id)?;
    match schema {
        Schema::V1 => messages_v1(db, session_id, n),
        Schema::V2 => messages_v2(db, session_id, n),
    }
}

fn messages_v1(db: &Path, session_id: &str, n: usize) -> Result<Vec<Message>, HostError> {
    let connection = open_read_only(db, WHAT)?;
    // One indexed pass: every text part of the session, grouped by message, newest message
    // first and parts in creation order within it.
    let mut statement = connection
        .prepare(
            "SELECT m.id, m.data, p.data FROM message m \
             JOIN part p ON p.message_id = m.id \
             WHERE m.session_id = ?1 AND json_extract(p.data, '$.type') = 'text' \
             ORDER BY m.time_created DESC, m.id DESC, p.time_created ASC, p.id ASC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| sql_error(WHAT, &e))?;

    let mut messages: Vec<Message> = Vec::new();
    let mut current: Option<(String, Role, Option<String>, Vec<String>)> = None;
    let flush = |current: &mut Option<(String, Role, Option<String>, Vec<String>)>,
                 out: &mut Vec<Message>| {
        if let Some((id, role, at, parts)) = current.take()
            && !parts.is_empty()
        {
            out.push(Message { id, role, text: parts.join("\n\n"), at });
        }
    };
    for row in rows {
        let (id, message_data, part_data) = row.map_err(|e| sql_error(WHAT, &e))?;
        if current.as_ref().is_none_or(|c| c.0 != id) {
            flush(&mut current, &mut messages);
            if messages.len() >= n {
                break;
            }
            let Some((role, at)) = message_meta(&message_data) else { continue };
            current = Some((id, role, at, Vec::new()));
        }
        if let Some((_, _, _, parts)) = current.as_mut()
            && let Some(text) = part_text(&part_data)
        {
            parts.push(text);
        }
    }
    if messages.len() < n {
        flush(&mut current, &mut messages);
    }
    if messages.is_empty() {
        return Err(HostError::NoMessages(format!(
            "no messages for OpenCode session {session_id} in {}",
            db.display()
        )));
    }
    Ok(messages)
}

/// `OpenCode` 2: one `session_message` row per message; the payload carries the text directly.
fn messages_v2(db: &Path, session_id: &str, n: usize) -> Result<Vec<Message>, HostError> {
    let connection = open_read_only(db, WHAT)?;
    let mut statement = connection
        .prepare(
            "SELECT id, type, data, time_created FROM session_message \
             WHERE session_id = ?1 AND type IN ('user', 'assistant') \
             ORDER BY seq DESC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| sql_error(WHAT, &e))?;
    let mut messages = Vec::new();
    for row in rows {
        if messages.len() >= n {
            break;
        }
        let (id, kind, data, created) = row.map_err(|e| sql_error(WHAT, &e))?;
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) else { continue };
        let (role, text) = match kind.as_str() {
            "assistant" => {
                let parts: Vec<&str> = json
                    .get("content")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("text"))
                            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                            .filter(|t| !t.trim().is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                (Role::Assistant, parts.join("\n\n"))
            }
            "user" => (Role::Human, json.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_owned()),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        let at = json
            .pointer("/time/created")
            .and_then(serde_json::Value::as_u64)
            .or_else(|| u64::try_from(created).ok())
            .map(crate::time::iso_from_unix_ms);
        messages.push(Message { id, role, text, at });
    }
    if messages.is_empty() {
        return Err(HostError::NoMessages(format!(
            "no messages for OpenCode session {session_id} in {}",
            db.display()
        )));
    }
    Ok(messages)
}

/// Role and creation time from a `message.data` payload; `None` for roles we do not render.
fn message_meta(data: &str) -> Option<(Role, Option<String>)> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;
    let role = match json.get("role").and_then(|r| r.as_str())? {
        "assistant" => Role::Assistant,
        "user" => Role::Human,
        _ => return None,
    };
    let at =
        json.pointer("/time/created").and_then(serde_json::Value::as_u64).map(crate::time::iso_from_unix_ms);
    Some((role, at))
}

/// The text of a `part.data` payload when it is real text: type `text`, not `synthetic`
/// (injected by `OpenCode`), not `ignored`, not empty.
fn part_text(data: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;
    if json.get("type").and_then(|t| t.as_str()) != Some("text") {
        return None;
    }
    let flagged = |key: &str| json.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false);
    if flagged("synthetic") || flagged("ignored") {
        return None;
    }
    let text = json.get("text").and_then(|t| t.as_str())?;
    (!text.trim().is_empty()).then(|| text.to_owned())
}

/// Paths compare without a trailing separator; Windows drive letters case-insensitively.
fn normalize(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if cfg!(windows) { trimmed.replace('\\', "/").to_ascii_lowercase() } else { trimmed.to_owned() }
}

fn is_ancestor(directory: &str, cwd: &str) -> bool {
    !directory.is_empty() && cwd.starts_with(directory) && cwd.as_bytes().get(directory.len()) == Some(&b'/')
}
