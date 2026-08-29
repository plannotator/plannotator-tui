//! Hermes CLI messages out of a generated `state.db`, read-only.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::type_complexity,
    reason = "tests assert by panicking"
)]

use std::path::{Path, PathBuf};

use plannotator_tui_hosts::{HostError, Role, hermes};
use rusqlite::{Connection, params};

const SCHEMA: &str = "
CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, display_name TEXT, model TEXT,
    parent_session_id TEXT, started_at REAL NOT NULL, ended_at REAL, message_count INTEGER DEFAULT 0);
CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id),
    role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT,
    timestamp REAL NOT NULL, reasoning TEXT, observed INTEGER DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1, compacted INTEGER NOT NULL DEFAULT 0);
CREATE INDEX idx_messages_session_active ON messages(session_id, active, timestamp);
";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-hermes-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Three sessions; `s1` holds mixed roles, an inactive row, a compacted row and out-of-order
/// timestamps so ordering by time (not insertion) is what the reader must do.
fn populate(db: &Path, wal: bool) -> Connection {
    let connection = Connection::open(db).expect("create");
    if wal {
        connection.pragma_update(None, "journal_mode", "WAL").expect("wal");
    }
    connection.execute_batch(SCHEMA).expect("schema");
    for id in ["s1", "s2", "s3"] {
        connection
            .execute("INSERT INTO sessions (id, source, started_at) VALUES (?1, 'cli', 1.0)", params![id])
            .expect("session");
    }
    let rows: &[(&str, &str, Option<&str>, f64, i64, i64)] = &[
        ("s1", "user", Some("first question"), 100.0, 1, 0),
        ("s1", "assistant", Some("oldest reply"), 101.0, 1, 1),
        ("s1", "tool", Some("tool output"), 102.0, 1, 0),
        ("s1", "assistant", Some("newest reply"), 300.0, 1, 0),
        ("s1", "assistant", Some("middle reply"), 200.0, 1, 0),
        ("s1", "assistant", Some("retracted reply"), 400.0, 0, 0),
        ("s1", "assistant", None, 500.0, 1, 0),
        ("s2", "assistant", Some("other session"), 900.0, 1, 0),
    ];
    for (session, role, content, at, active, compacted) in rows {
        connection
            .execute(
                "INSERT INTO messages (session_id, role, content, timestamp, active, compacted) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![session, role, content, at, active, compacted],
            )
            .expect("message");
    }
    connection
}

#[test]
fn newest_active_assistant_reply_comes_first_and_n_is_honoured() {
    let dir = temp_dir("order");
    let db = dir.join("state.db");
    let writer = populate(&db, false);
    let messages = hermes::messages_for_session(&db, "s1", 25).expect("messages");
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["newest reply", "middle reply", "oldest reply", "first question"]);
    assert_eq!(messages[0].role, Role::Assistant);
    assert_eq!(messages[3].role, Role::Human);
    assert_eq!(messages[0].at.as_deref(), Some("1970-01-01T00:05:00.000Z"));
    assert_eq!(hermes::messages_for_session(&db, "s1", 2).expect("two").len(), 2);
    // Windows cannot delete a database another handle still holds open.
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn unknown_sessions_and_missing_databases_are_distinct_errors() {
    let dir = temp_dir("errors");
    let db = dir.join("state.db");
    let writer = populate(&db, false);
    assert!(matches!(hermes::messages_for_session(&db, "s3", 5), Err(HostError::NoMessages(_))));
    assert!(matches!(hermes::messages_for_session(&db, "nope", 5), Err(HostError::NoMessages(_))));
    assert!(matches!(
        hermes::messages_for_session(&dir.join("absent.db"), "s1", 5),
        Err(HostError::NoTranscript(_))
    ));
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn a_live_wal_database_is_read_without_being_touched() {
    let dir = temp_dir("wal");
    let db = dir.join("state.db");
    // The writer stays open with rows still in its WAL, as a running Hermes would.
    let writer = populate(&db, true);
    let before = listing(&dir);
    assert!(before.iter().any(|n| n.ends_with("-wal")), "writer holds a WAL: {before:?}");
    let messages = hermes::messages_for_session(&db, "s1", 1).expect("reads the live database");
    assert_eq!(messages[0].text, "newest reply", "rows still in the WAL are visible");
    assert_eq!(listing(&dir), before, "the reader created no journal or lock files");
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
