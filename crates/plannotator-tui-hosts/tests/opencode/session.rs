//! Picking a session: the newest top-level unarchived one for a directory, across schemas.

use std::path::Path;

use plannotator_tui_hosts::opencode::{Found, Schema};
use plannotator_tui_hosts::{HostError, opencode};

use super::fixture::{CURRENT_SESSION_MESSAGE, Db, populate, populate_v2};

#[test]
fn newest_session_for_the_directory_skips_children_and_archived_ones() {
    let db = Db::new("find");
    let _writer = populate(db.file());
    assert_eq!(opencode::find_session(db.file(), Path::new("/work/app")).expect("exact").id, "ses_main");
    assert_eq!(
        opencode::find_session(db.file(), Path::new("/work/app/")).expect("trailing slash").id,
        "ses_main"
    );
    // Started in an ancestor: the newest session there, never a sibling project.
    assert_eq!(
        opencode::find_session(db.file(), Path::new("/work/app/src/deep")).expect("nested").id,
        "ses_main"
    );
    assert_eq!(opencode::find_session(db.file(), Path::new("/work/tools")).expect("root").id, "ses_root");
    assert_eq!(opencode::find_session(db.file(), Path::new("/work/other")).expect("other").id, "ses_other");
    match opencode::find_session(db.file(), Path::new("/elsewhere")) {
        Err(HostError::NoTranscript(msg)) => assert!(msg.contains("/elsewhere"), "{msg}"),
        other => panic!("expected NoTranscript, got {other:?}"),
    }
}

#[test]
fn an_opencode2_session_in_the_same_database_wins_when_it_is_newer() {
    let db = Db::new("v2-wins");
    let _writer = populate_v2(db.file());
    let found = opencode::find_session(db.file(), Path::new("/work/app")).expect("found");
    assert_eq!(found, Found { id: "ses2_new".to_owned(), schema: Schema::V2, updated: 5000 });
    // The v1 session is still the answer for a directory only v1 knows.
    assert_eq!(
        opencode::find_session(db.file(), Path::new("/work/other")).expect("v1 only").schema,
        Schema::V1
    );
    assert_eq!(opencode::schema_of(db.file(), "ses2_new").expect("v2"), Schema::V2);
    assert_eq!(opencode::schema_of(db.file(), "ses_main").expect("v1"), Schema::V1);
    assert!(matches!(opencode::schema_of(db.file(), "ses_nope"), Err(HostError::NoMessages(_))));
}

#[test]
fn a_database_without_the_v2_tables_still_reads_v1() {
    let db = Db::new("v1only");
    let _writer = populate(db.file());
    assert_eq!(opencode::find_session(db.file(), Path::new("/work/app")).expect("v1").schema, Schema::V1);
}

#[test]
fn current_schema_keeps_exact_ids_on_the_session_message_compatibility_path() {
    let db = Db::new("current");
    let writer = populate(db.file());
    writer.execute_batch(CURRENT_SESSION_MESSAGE).expect("current session_message table");
    writer
        .execute(
            "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data) VALUES ('current-only', 'ses_main', 'assistant', 1, 60, 60, '{\"content\":[{\"type\":\"text\",\"text\":\"duplicate current row\"}]}')",
            [],
        )
        .expect("current row");
    assert_eq!(opencode::schema_of(db.file(), "ses_main").expect("schema"), Schema::V1);
    let messages = opencode::messages_for_session(db.file(), "ses_main", Schema::V1, 1).expect("messages");
    assert_eq!(messages[0].text, "newest reply");
}

#[test]
fn a_missing_database_is_reported_not_created() {
    let db = Db::new("missing");
    let path = db.file().to_path_buf();
    match opencode::find_session(&path, Path::new("/work/app")) {
        Err(HostError::NoTranscript(msg)) => assert!(msg.contains("no OpenCode database"), "{msg}"),
        other => panic!("expected NoTranscript, got {other:?}"),
    }
    assert!(!path.exists(), "a read must not create the database");
}
