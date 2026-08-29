//! Hermes CLI: conversations live in `SQLite` (`~/.hermes/state.db`, WAL mode), one row per
//! message, addressed by the session id Herdr reports through `agent_session`. There is no
//! transcript file. Schema (`sessions`, `messages`) as documented in plannotator-tui#25; the
//! newest reply is one indexed query over `idx_messages_session_active`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};

use crate::{HostError, Message, Role};

/// The database relative to `$HERMES_HOME` (default `~/.hermes`).
pub const DB_FILE: &str = "state.db";

/// The newest `n` user and assistant messages of `session_id`, newest first.
///
/// The database is opened read-only and never written. A plain read-only open sees the
/// live WAL; when the shared-memory index cannot be opened, the read falls back to
/// `immutable=1`, which reads the main file only and may lag an active writer.
pub fn messages_for_session(db: &Path, session_id: &str, n: usize) -> Result<Vec<Message>, HostError> {
    if !db.is_file() {
        return Err(HostError::NoTranscript(format!("no Hermes database at {}", db.display())));
    }
    let connection = open_read_only(db)?;
    let mut statement = connection
        .prepare(
            "SELECT id, role, content, timestamp FROM messages \
             WHERE session_id = ?1 AND role IN ('assistant', 'user') AND active = 1 \
             AND content IS NOT NULL AND trim(content) <> '' \
             ORDER BY timestamp DESC, id DESC LIMIT ?2",
        )
        .map_err(|e| sql_error(&e))?;
    let rows = statement
        .query_map(params![session_id, n as i64], |row| {
            let id: i64 = row.get(0)?;
            let role: String = row.get(1)?;
            let content: Option<String> = row.get(2)?;
            let timestamp: Option<f64> = row.get(3)?;
            Ok((id, role, content, timestamp))
        })
        .map_err(|e| sql_error(&e))?;
    let mut messages = Vec::new();
    for row in rows {
        let (id, role, content, timestamp) = row.map_err(|e| sql_error(&e))?;
        let Some(text) = content.filter(|c| !c.trim().is_empty()) else { continue };
        let role = match role.as_str() {
            "assistant" => Role::Assistant,
            "user" => Role::Human,
            _ => continue,
        };
        messages.push(Message { id: id.to_string(), role, text, at: timestamp.map(iso) });
    }
    if messages.is_empty() {
        return Err(HostError::NoMessages(format!(
            "no messages for Hermes session {session_id} in {}",
            db.display()
        )));
    }
    Ok(messages)
}

fn open_read_only(db: &Path) -> Result<Connection, HostError> {
    let flags =
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let uri = |query: &str| format!("file:{}?{query}", db.display());
    match Connection::open_with_flags(uri("mode=ro"), flags) {
        Ok(connection) if probe(&connection) => Ok(connection),
        _ => Connection::open_with_flags(uri("mode=ro&immutable=1"), flags).map_err(|e| sql_error(&e)),
    }
}

/// A read-only WAL open can succeed and still fail on first read when the `-shm` cannot be
/// mapped; probe once so the fallback happens before any query.
fn probe(connection: &Connection) -> bool {
    connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0)).is_ok()
}

/// Hermes stamps rows in Unix seconds (REAL).
fn iso(seconds: f64) -> String {
    let ms = if seconds.is_finite() && seconds >= 0.0 { (seconds * 1000.0) as u64 } else { 0 };
    crate::time::iso_from_unix_ms(ms)
}

fn sql_error(err: &rusqlite::Error) -> HostError {
    HostError::NoTranscript(format!("reading the Hermes database: {err}"))
}
