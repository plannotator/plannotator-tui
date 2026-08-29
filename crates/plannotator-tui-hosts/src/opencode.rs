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

use std::path::Path;

use rusqlite::params;

use crate::sqlite::{open_read_only, sql_error};
use crate::{HostError, Message, Role};

/// The database relative to `OpenCode`'s data directory.
pub const DB_FILE: &str = "opencode.db";
/// The data directory relative to the XDG data home.
pub const DATA_DIR: &str = "opencode";

const WHAT: &str = "OpenCode";

/// The newest top-level, unarchived session started in `cwd`. When none was started exactly
/// there, the newest one started in an ancestor of `cwd` (`OpenCode` records the directory it
/// was launched from; the pane may have moved since).
pub fn find_session(db: &Path, cwd: &Path) -> Result<String, HostError> {
    let connection = open_read_only(db, WHAT)?;
    let mut statement = connection
        .prepare(
            "SELECT id, directory FROM session \
             WHERE parent_id IS NULL AND time_archived IS NULL \
             ORDER BY time_updated DESC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| sql_error(WHAT, &e))?;
    let cwd = normalize(&cwd.to_string_lossy());
    let mut ancestor: Option<String> = None;
    for row in rows {
        let (id, directory) = row.map_err(|e| sql_error(WHAT, &e))?;
        let directory = normalize(&directory);
        if directory == cwd {
            return Ok(id);
        }
        if ancestor.is_none() && is_ancestor(&directory, &cwd) {
            ancestor = Some(id);
        }
    }
    ancestor
        .ok_or_else(|| HostError::NoTranscript(format!("no OpenCode session for {cwd} in {}", db.display())))
}

/// The newest `n` user and assistant messages of `session_id` that carry text, newest first.
pub fn messages_for_session(db: &Path, session_id: &str, n: usize) -> Result<Vec<Message>, HostError> {
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
