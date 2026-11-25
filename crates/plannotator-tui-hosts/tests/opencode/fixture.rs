//! The generated databases the tests read: the `OpenCode` 1 schema subset, the `OpenCode` 2
//! tables, and the 2.4 `session_part` table, with one mixed session each.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

pub(crate) const SCHEMA: &str = "
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
pub(crate) const SCHEMA_V2: &str = "
CREATE TABLE session_v2 (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, workspace_id TEXT, parent_id TEXT,
    slug TEXT NOT NULL, directory TEXT NOT NULL, title TEXT, version TEXT NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_archived INTEGER);
CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL);
CREATE UNIQUE INDEX session_message_session_seq_idx ON session_message (session_id, seq);
";

/// `OpenCode` 2.4 split a message's payload: one `session_part` row per part, ordered by
/// `ordinal`. `packages/core/src/session/sql.ts`, migration `20261029002625_session_part`.
pub(crate) const SCHEMA_V2_PARTS: &str = "
CREATE TABLE session_part (message_id TEXT NOT NULL, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
    type TEXT NOT NULL, call_id TEXT, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
    data TEXT NOT NULL, CONSTRAINT session_part_pk PRIMARY KEY(message_id, ordinal));
CREATE INDEX session_part_session_message_idx ON session_part (session_id, message_id, ordinal);
";

/// Current `OpenCode` writes this table alongside the legacy message/part tables while the
/// session row remains in `session`.
pub(crate) const CURRENT_SESSION_MESSAGE: &str = "
CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
    seq INTEGER NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
    data TEXT NOT NULL);
";

/// A generated database in its own temp directory, removed when the test ends. Declare it
/// before the writer connection so the connection closes first.
pub(crate) struct Db {
    dir: PathBuf,
    pub(crate) path: PathBuf,
}

impl Db {
    /// A fresh directory named for `tag`; paths are unique per test, not just per process.
    pub(crate) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-opencode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        Self { path: dir.join("opencode.db"), dir }
    }

    /// The database path, for messages that must name the file.
    pub(crate) fn file(&self) -> &Path {
        &self.path
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Create the 1.x database at `db` and fill it with the shared `OpenCode` 1 fixture.
pub(crate) fn populate(db: &Path) -> Connection {
    let c = open(db);
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

/// Add the 2.x tables and a session whose replies live in the message payload: one reply with
/// reasoning and two text blocks, and a reasoning-only turn that has no visible text.
pub(crate) fn populate_v2(db: &Path) -> Connection {
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

/// A `OpenCode` 2.4 session whose replies are split between the message payload and
/// `session_part` rows, as after the part-table migration: one reply in each format, one in
/// both, a reasoning-only turn, and a prompt between them.
pub(crate) fn populate_v2_parts(db: &Path) -> Connection {
    let c = open(db);
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
    // Written out of order on purpose: only `ordinal` decides how the reply reads. Tool calls,
    // reasoning, blank, injected and ignored parts are not reply text.
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
    part_v2(&c, "msg2_parts", "ses2_mixed", 5, "text", r#"{"type":"text","text":"draft","ignored":true}"#);
    part_v2(&c, "msg2_parts", "ses2_mixed", 6, "reasoning", r#"{"type":"reasoning","text":"thinking"}"#);
    part_v2(&c, "msg2_both", "ses2_mixed", 0, "text", r#"{"type":"text","text":"part duplicate"}"#);
    part_v2(&c, "msg2_reasoning", "ses2_mixed", 0, "tool", r#"{"type":"tool","name":"read"}"#);
    c
}

fn open(db: &Path) -> Connection {
    let c = Connection::open(db).expect("create");
    c.pragma_update(None, "journal_mode", "WAL").expect("wal");
    c
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

pub(crate) fn message_v2(c: &Connection, id: &str, session: &str, seq: i64, kind: &str, data: &str) {
    c.execute(
        "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
        params![id, session, kind, seq, seq * 10, data],
    )
    .expect("session_message");
}

pub(crate) fn part_v2(c: &Connection, message: &str, session: &str, ordinal: i64, kind: &str, data: &str) {
    c.execute(
        "INSERT INTO session_part (message_id, session_id, ordinal, type, time_created, time_updated, data) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
        params![message, session, ordinal, kind, ordinal + 1, data],
    )
    .expect("session_part");
}
