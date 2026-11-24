//! `OpenCode` messages out of a generated `opencode.db` (the 1.18 schema subset), read-only.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_hosts::opencode::{Found, Schema};
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

/// The `OpenCode` 2 tables (`packages/core/src/session/sql.ts` on the `beta` branch), the columns
/// this reader touches.
const SCHEMA_V2: &str = "
CREATE TABLE session_v2 (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, workspace_id TEXT, parent_id TEXT,
    slug TEXT NOT NULL, directory TEXT NOT NULL, title TEXT, version TEXT NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_archived INTEGER);
CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);
CREATE UNIQUE INDEX session_message_session_seq_idx ON session_message (session_id, seq);
";

/// `OpenCode` 2.4 split a message's payload: one `session_part` row per part, ordered by
/// `ordinal`. `packages/core/src/session/sql.ts`, migration `20261029002625_session_part`.
const SCHEMA_V2_PARTS: &str = "
CREATE TABLE session_part (message_id TEXT NOT NULL, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
    type TEXT NOT NULL, call_id TEXT, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
    data TEXT NOT NULL, CONSTRAINT session_part_pk PRIMARY KEY(message_id, ordinal));
CREATE INDEX session_part_session_message_idx ON session_part (session_id, message_id, ordinal);
";

/// Current `OpenCode` writes this table alongside the legacy message/part tables while the
/// session row remains in `session`.
const CURRENT_SESSION_MESSAGE: &str = "
CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
    seq INTEGER NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
    data TEXT NOT NULL);
";

fn session_v2(
    c: &Connection,
    id: &str,
    dir: &str,
    parent: Option<&str>,
    updated: i64,
    archived: Option<i64>,
) {
    c.execute(
        "INSERT INTO session_v2 (id, project_id, parent_id, slug, directory, version, time_created, time_updated, time_archived) \
         VALUES (?1, 'p', ?2, ?1, ?3, '0.0.0-beta', ?4, ?4, ?5)",
        params![id, parent, dir, updated, archived],
    )
    .expect("session_v2");
}

fn message_v2(c: &Connection, id: &str, session: &str, seq: i64, kind: &str, data: &str) {
    c.execute(
        "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
        params![id, session, kind, seq, seq * 10, data],
    )
    .expect("session_message");
}

/// A database `OpenCode` 2 has written to: the v1 tables with an older session for `/work/app`
/// (as after the v1 migration) plus the v2 tables with a newer session for the same directory.
fn populate_v2(db: &Path) -> Connection {
    let c = populate(db);
    c.execute_batch(SCHEMA_V2).expect("schema v2");
    session_v2(&c, "ses2_new", "/work/app", None, 5000, None);
    session_v2(&c, "ses2_child", "/work/app", Some("ses2_new"), 9000, None);
    session_v2(&c, "ses2_gone", "/work/app", None, 9500, Some(9500));
    message_v2(
        &c,
        "msg2_1",
        "ses2_new",
        1,
        "user",
        r#"{"text":"reply with exactly OPENCODE2_V2_SESSION_SELECTED","files":[],"agents":[],"skills":[],"time":{"created":50000}}"#,
    );
    message_v2(
        &c,
        "msg2_2",
        "ses2_new",
        2,
        "synthetic",
        r#"{"text":"<system-reminder>injected</system-reminder>","time":{"created":50001}}"#,
    );
    message_v2(
        &c,
        "msg2_3",
        "ses2_new",
        3,
        "assistant",
        r#"{"agent":"build","model":{"providerID":"x","id":"y"},"content":[{"type":"reasoning","text":"thinking"},{"type":"text","text":"OPENCODE2_V2_SESSION_SELECTED"},{"type":"tool","id":"t","name":"read","state":{"status":"completed","input":{},"content":[]},"time":{"created":50002}},{"type":"text","text":"Done."}],"time":{"created":50002,"completed":50003}}"#,
    );
    message_v2(
        &c,
        "msg2_4",
        "ses2_new",
        4,
        "assistant",
        r#"{"agent":"build","model":{"providerID":"x","id":"y"},"content":[{"type":"reasoning","text":"aborted"}],"time":{"created":50004}}"#,
    );
    message_v2(&c, "msg2_5", "ses2_new", 5, "agent-switched", r#"{"agent":"plan","time":{"created":50005}}"#);
    c
}

#[test]
fn an_opencode2_session_in_the_same_database_wins_when_it_is_newer() {
    let dir = temp_dir("v2");
    let db = dir.join("opencode.db");
    let writer = populate_v2(&db);
    let found = opencode::find_session(&db, Path::new("/work/app")).expect("found");
    assert_eq!(found, Found { id: "ses2_new".to_owned(), schema: Schema::V2, updated: 5000 });
    // The v1 session is still the answer for a directory only v1 knows.
    assert_eq!(opencode::find_session(&db, Path::new("/work/other")).expect("v1 only").schema, Schema::V1);
    assert_eq!(opencode::schema_of(&db, "ses2_new").expect("v2"), Schema::V2);
    assert_eq!(opencode::schema_of(&db, "ses_main").expect("v1"), Schema::V1);
    assert!(matches!(opencode::schema_of(&db, "ses_nope"), Err(HostError::NoMessages(_))));

    let messages = opencode::messages_for_session(&db, "ses2_new", Schema::V2, 25).expect("messages");
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        ["OPENCODE2_V2_SESSION_SELECTED\n\nDone.", "reply with exactly OPENCODE2_V2_SESSION_SELECTED"]
    );
    assert_eq!(messages[0].role, Role::Assistant);
    assert_eq!(messages[0].id, "msg2_3");
    assert_eq!(messages[0].at.as_deref(), Some("1970-01-01T00:00:50.002Z"));
    assert_eq!(messages[1].role, Role::Human);
    // n counts messages with text: the reasoning-only turn and the switch rows take no slot.
    assert_eq!(opencode::messages_for_session(&db, "ses2_new", Schema::V2, 1).expect("one").len(), 1);
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

fn part_v2(c: &Connection, message: &str, session: &str, ordinal: i64, kind: &str, data: &str) {
    c.execute(
        "INSERT INTO session_part (message_id, session_id, ordinal, type, time_created, time_updated, data) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
        params![message, session, ordinal, kind, ordinal + 1, data],
    )
    .expect("session_part");
}

/// A `OpenCode` 2.4 session whose replies are split between the message payload and
/// `session_part` rows, as after the part-table migration: one reply in each format, one in
/// both, a reasoning-only turn, and a prompt between them.
fn populate_v2_parts(db: &Path) -> Connection {
    let c = Connection::open(db).expect("create");
    c.pragma_update(None, "journal_mode", "WAL").expect("wal");
    c.execute_batch(SCHEMA_V2).expect("schema v2");
    c.execute_batch(SCHEMA_V2_PARTS).expect("schema v2 parts");
    session_v2(&c, "ses2_mixed", "/work/app", None, 7000, None);
    message_v2(
        &c,
        "msg2_q1",
        "ses2_mixed",
        1,
        "user",
        r#"{"text":"mixed question","files":[],"time":{"created":70001}}"#,
    );
    message_v2(
        &c,
        "msg2_legacy",
        "ses2_mixed",
        2,
        "assistant",
        r#"{"content":[{"type":"text","text":"legacy reply"}],"time":{"created":70002}}"#,
    );
    message_v2(&c, "msg2_parts", "ses2_mixed", 3, "assistant", r#"{"content":[],"time":{"created":70003}}"#);
    message_v2(
        &c,
        "msg2_both",
        "ses2_mixed",
        4,
        "assistant",
        r#"{"content":[{"type":"text","text":"both reply"}],"time":{"created":70004}}"#,
    );
    message_v2(
        &c,
        "msg2_q2",
        "ses2_mixed",
        5,
        "user",
        r#"{"text":"follow-up question","files":[],"time":{"created":70005}}"#,
    );
    message_v2(
        &c,
        "msg2_reasoning",
        "ses2_mixed",
        6,
        "assistant",
        r#"{"content":[{"type":"reasoning","text":"thinking"}],"time":{"created":70006}}"#,
    );
    // Written out of order on purpose: only `ordinal` decides how the reply reads.
    part_v2(&c, "msg2_parts", "ses2_mixed", 3, "text", r#"{"type":"text","text":"part two"}"#);
    part_v2(&c, "msg2_parts", "ses2_mixed", 1, "tool", r#"{"type":"tool","name":"read"}"#);
    part_v2(&c, "msg2_parts", "ses2_mixed", 0, "text", r#"{"type":"text","text":"part one"}"#);
    part_v2(&c, "msg2_parts", "ses2_mixed", 2, "text", r#"{"type":"text","text":"   "}"#);
    part_v2(
        &c,
        "msg2_parts",
        "ses2_mixed",
        4,
        "text",
        r#"{"type":"text","text":"injected","synthetic":true}"#,
    );
    part_v2(&c, "msg2_both", "ses2_mixed", 0, "text", r#"{"type":"text","text":"part duplicate"}"#);
    part_v2(&c, "msg2_reasoning", "ses2_mixed", 0, "tool", r#"{"type":"tool","name":"read"}"#);
    c
}

#[test]
fn replies_survive_the_part_table_once_each_in_order_and_under_the_limit() {
    let dir = temp_dir("v2parts");
    let db = dir.join("opencode.db");
    let writer = populate_v2_parts(&db);
    let messages = opencode::messages_for_session(&db, "ses2_mixed", Schema::V2, 25).expect("messages");
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        ["follow-up question", "both reply", "part one\n\npart two", "legacy reply", "mixed question"]
    );
    // A reply stored in both formats is one entry: the payload wins, its parts are not repeated.
    assert_eq!(messages.iter().filter(|m| m.id == "msg2_both").count(), 1);
    assert!(messages.iter().all(|m| !m.text.contains("part duplicate")));
    let parts = messages.iter().find(|m| m.id == "msg2_parts").expect("parts reply");
    assert_eq!(parts.role, Role::Assistant);
    assert_eq!(parts.at.as_deref(), Some("1970-01-01T00:01:10.003Z"));
    assert_eq!(messages[1].id, "msg2_both");
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].at.as_deref(), Some("1970-01-01T00:01:10.004Z"));
    assert_eq!(messages[0].role, Role::Human);
    assert_eq!(messages[3].at.as_deref(), Some("1970-01-01T00:01:10.002Z"));
    // Only the reasoning turn is invisible, so a limit of two replies still reaches the parts
    // reply; a limit of one stops at the newest reply and keeps the prompt after it.
    let one = opencode::messages_for_session(&db, "ses2_mixed", Schema::V2, 1).expect("one");
    assert_eq!(one.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg2_q2", "msg2_both"]);
    let two = opencode::messages_for_session(&db, "ses2_mixed", Schema::V2, 2).expect("two");
    assert_eq!(two.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg2_q2", "msg2_both", "msg2_parts"]);
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn a_database_without_the_v2_tables_still_reads_v1() {
    let dir = temp_dir("v1only");
    let db = dir.join("opencode.db");
    let writer = populate(&db);
    assert_eq!(opencode::find_session(&db, Path::new("/work/app")).expect("v1").schema, Schema::V1);
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn current_schema_keeps_exact_ids_on_the_session_message_compatibility_path() {
    let dir = temp_dir("current");
    let db = dir.join("opencode.db");
    let writer = populate(&db);
    writer.execute_batch(CURRENT_SESSION_MESSAGE).expect("current session_message table");
    writer
        .execute(
            "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data) VALUES ('current-only', 'ses_main', 'assistant', 1, 60, 60, '{\"content\":[{\"type\":\"text\",\"text\":\"duplicate current row\"}]}')",
            [],
        )
        .expect("current row");
    assert_eq!(opencode::schema_of(&db, "ses_main").expect("schema"), Schema::V1);
    let messages = opencode::messages_for_session(&db, "ses_main", Schema::V1, 1).expect("messages");
    assert_eq!(messages[0].text, "newest reply");
    drop(writer);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

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
    assert_eq!(opencode::find_session(&db, Path::new("/work/app")).expect("exact").id, "ses_main");
    assert_eq!(opencode::find_session(&db, Path::new("/work/app/")).expect("trailing slash").id, "ses_main");
    // Started in an ancestor: the newest session there, never a sibling project.
    assert_eq!(opencode::find_session(&db, Path::new("/work/app/src/deep")).expect("nested").id, "ses_main");
    assert_eq!(opencode::find_session(&db, Path::new("/work/tools")).expect("root").id, "ses_root");
    assert_eq!(opencode::find_session(&db, Path::new("/work/other")).expect("other").id, "ses_other");
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
    let messages = opencode::messages_for_session(&db, "ses_main", Schema::V1, 25).expect("messages");
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
    // n counts replies with text; the prompt between them and the aborted assistant turn
    // do not consume a slot.
    let two = opencode::messages_for_session(&db, "ses_main", Schema::V1, 2).expect("two");
    assert_eq!(two.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg_5", "msg_4", "msg_2"]);
    match opencode::messages_for_session(&db, "ses_empty", Schema::V1, 25) {
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
