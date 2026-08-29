//! `OpenCode` messages out of a generated `opencode.db` (the 1.18 schema subset), read-only.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_hosts::{HostError, Role, opencode};
use rusqlite::{Connection, params};

const SCHEMA: &str = "
CREATE TABLE session (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT, slug TEXT NOT NULL,
    directory TEXT NOT NULL, title TEXT NOT NULL, version TEXT NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_archived INTEGER);
CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);
CREATE INDEX message_session_time_created_id_idx ON message (session_id, time_created, id);
CREATE INDEX part_message_id_id_idx ON part (message_id, id);
";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-opencode-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn session(c: &Connection, id: &str, dir: &str, parent: Option<&str>, updated: i64, archived: Option<i64>) {
    c.execute(
        "INSERT INTO session (id, project_id, parent_id, slug, directory, title, version, time_created, time_updated, time_archived) \
         VALUES (?1, 'p', ?2, ?1, ?3, ?1, '1.18.23', ?4, ?4, ?5)",
        params![id, parent, dir, updated, archived],
    )
    .expect("session");
}

fn message(c: &Connection, id: &str, session: &str, created: i64, role: &str) {
    let data =
        format!(r#"{{"id":"{id}","sessionID":"{session}","role":"{role}","time":{{"created":{created}}}}}"#);
    c.execute(
        "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?3, ?4)",
        params![id, session, created, data],
    )
    .expect("message");
}

fn part(c: &Connection, id: &str, message: &str, created: i64, data: &str) {
    c.execute(
        "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, 's', ?3, ?3, ?4)",
        params![id, message, created, data],
    )
    .expect("part");
}

/// One project with a main session, a newer child (subagent) session and a newer archived
/// session; another project elsewhere. The main session's messages exercise every skip rule.
fn populate(db: &Path) -> Connection {
    let c = Connection::open(db).expect("create");
    c.pragma_update(None, "journal_mode", "WAL").expect("wal");
    c.execute_batch(SCHEMA).expect("schema");
    session(&c, "ses_main", "/work/app", None, 1000, None);
    session(&c, "ses_older", "/work/app", None, 900, None);
    session(&c, "ses_child", "/work/app", Some("ses_main"), 2000, None);
    session(&c, "ses_archived", "/work/app", None, 3000, Some(3000));
    session(&c, "ses_other", "/work/other", None, 5000, None);
    session(&c, "ses_root", "/work", None, 100, None);

    message(&c, "msg_1", "ses_main", 10, "user");
    part(&c, "prt_1a", "msg_1", 10, r#"{"type":"text","text":"first question"}"#);
    part(
        &c,
        "prt_1b",
        "msg_1",
        11,
        r#"{"type":"text","text":"<system-reminder>injected</system-reminder>","synthetic":true}"#,
    );
    message(&c, "msg_2", "ses_main", 20, "assistant");
    part(&c, "prt_2a", "msg_2", 20, r#"{"type":"step-start"}"#);
    part(&c, "prt_2b", "msg_2", 21, r#"{"type":"reasoning","text":"**thinking**"}"#);
    part(&c, "prt_2c", "msg_2", 22, r#"{"type":"text","text":"oldest reply, part one"}"#);
    part(&c, "prt_2d", "msg_2", 23, r#"{"type":"tool","tool":"read","state":{"status":"completed"}}"#);
    part(&c, "prt_2e", "msg_2", 24, r#"{"type":"text","text":"oldest reply, part two"}"#);
    part(&c, "prt_2f", "msg_2", 25, r#"{"type":"step-finish"}"#);
    message(&c, "msg_3", "ses_main", 30, "assistant"); // aborted: no text at all
    part(&c, "prt_3a", "msg_3", 30, r#"{"type":"step-start"}"#);
    message(&c, "msg_4", "ses_main", 40, "user");
    part(&c, "prt_4a", "msg_4", 40, r#"{"type":"text","text":"ignored draft","ignored":true}"#);
    part(&c, "prt_4b", "msg_4", 41, r#"{"type":"text","text":"second question"}"#);
    message(&c, "msg_5", "ses_main", 50, "assistant");
    part(&c, "prt_5a", "msg_5", 50, r#"{"type":"text","text":"   "}"#);
    part(&c, "prt_5b", "msg_5", 51, r#"{"type":"text","text":"newest reply"}"#);
    message(&c, "msg_x", "ses_other", 60, "assistant");
    part(&c, "prt_xa", "msg_x", 60, r#"{"type":"text","text":"other project"}"#);
    c
}

#[test]
fn newest_session_for_the_directory_skips_children_and_archived_ones() {
    let dir = temp_dir("find");
    let db = dir.join("opencode.db");
    let writer = populate(&db);
    assert_eq!(opencode::find_session(&db, Path::new("/work/app")).expect("exact"), "ses_main");
    assert_eq!(opencode::find_session(&db, Path::new("/work/app/")).expect("trailing slash"), "ses_main");
    // Started in an ancestor: the newest session there, never a sibling project.
    assert_eq!(opencode::find_session(&db, Path::new("/work/app/src/deep")).expect("nested"), "ses_main");
    assert_eq!(opencode::find_session(&db, Path::new("/work/tools")).expect("root"), "ses_root");
    assert_eq!(opencode::find_session(&db, Path::new("/work/other")).expect("other"), "ses_other");
    match opencode::find_session(&db, Path::new("/elsewhere")) {
        Err(HostError::NoTranscript(msg)) => assert!(msg.contains("/elsewhere"), "{msg}"),
        other => panic!("expected NoTranscript, got {other:?}"),
    }
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn text_parts_join_in_order_and_injected_or_empty_ones_are_skipped() {
    let dir = temp_dir("read");
    let db = dir.join("opencode.db");
    let writer = populate(&db);
    let messages = opencode::messages_for_session(&db, "ses_main", 25).expect("messages");
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "newest reply",
            "second question",
            "oldest reply, part one\n\noldest reply, part two",
            "first question"
        ]
    );
    assert_eq!(messages[0].role, Role::Assistant);
    assert_eq!(messages[0].id, "msg_5");
    assert_eq!(messages[0].at.as_deref(), Some("1970-01-01T00:00:00.050Z"));
    assert_eq!(messages[1].role, Role::Human);
    // n counts messages with text; the aborted assistant turn does not consume a slot.
    let two = opencode::messages_for_session(&db, "ses_main", 2).expect("two");
    assert_eq!(two.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg_5", "msg_4"]);
    match opencode::messages_for_session(&db, "ses_empty", 25) {
        Err(HostError::NoMessages(msg)) => assert!(msg.contains("ses_empty"), "{msg}"),
        other => panic!("expected NoMessages, got {other:?}"),
    }
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn a_missing_database_is_reported_not_created() {
    let dir = temp_dir("missing");
    let db = dir.join("opencode.db");
    match opencode::find_session(&db, Path::new("/work/app")) {
        Err(HostError::NoTranscript(msg)) => assert!(msg.contains("no OpenCode database"), "{msg}"),
        other => panic!("expected NoTranscript, got {other:?}"),
    }
    assert!(!db.exists(), "a read must not create the database");
    std::fs::remove_dir_all(&dir).expect("cleanup");
}
