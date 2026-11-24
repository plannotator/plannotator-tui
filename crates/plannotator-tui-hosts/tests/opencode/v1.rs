//! `OpenCode` 1 (`message` + `part` rows): text parts join in creation order, and only real
//! text is reply text.

use plannotator_tui_hosts::opencode::Schema;
use plannotator_tui_hosts::{HostError, Role, opencode};

use super::fixture::{Db, populate};

#[test]
fn text_parts_join_in_order_and_injected_or_empty_ones_are_skipped() {
    let db = Db::new("v1-read");
    let _writer = populate(db.file());
    let messages = opencode::messages_for_session(db.file(), "ses_main", Schema::V1, 25).expect("messages");
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
}

#[test]
fn the_v1_reply_limit_counts_replies_with_text() {
    let db = Db::new("v1-limit");
    let _writer = populate(db.file());
    // n counts replies with text; the prompt between them and the aborted assistant turn
    // do not consume a slot.
    let two = opencode::messages_for_session(db.file(), "ses_main", Schema::V1, 2).expect("two");
    assert_eq!(two.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg_5", "msg_4", "msg_2"]);
}

#[test]
fn a_v1_session_without_text_is_reported_by_id() {
    let db = Db::new("v1-empty");
    let _writer = populate(db.file());
    match opencode::messages_for_session(db.file(), "ses_empty", Schema::V1, 25) {
        Err(HostError::NoMessages(msg)) => assert!(msg.contains("ses_empty"), "{msg}"),
        other => panic!("expected NoMessages, got {other:?}"),
    }
}
