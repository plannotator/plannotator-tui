//! Read-only `SQLite` access shared by the store-backed hosts (Hermes, `OpenCode`).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::HostError;

/// Open `db` read-only, never writing. A plain read-only open sees the live WAL; when the
/// shared-memory index cannot be opened, fall back to `immutable=1`, which reads the main
/// file only and may lag an active writer.
pub(crate) fn open_read_only(db: &Path, what: &str) -> Result<Connection, HostError> {
    if !db.is_file() {
        return Err(HostError::NoTranscript(format!("no {what} database at {}", db.display())));
    }
    let flags =
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let uri = |query: &str| format!("file:{}?{query}", db.display());
    match Connection::open_with_flags(uri("mode=ro"), flags) {
        Ok(connection) if probe(&connection) => Ok(connection),
        _ => Connection::open_with_flags(uri("mode=ro&immutable=1"), flags).map_err(|e| sql_error(what, &e)),
    }
}

/// A read-only WAL open can succeed and still fail on first read when the `-shm` cannot be
/// mapped; probe once so the fallback happens before any query.
fn probe(connection: &Connection) -> bool {
    connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0)).is_ok()
}

pub(crate) fn sql_error(what: &str, err: &rusqlite::Error) -> HostError {
    HostError::NoTranscript(format!("reading the {what} database: {err}"))
}
