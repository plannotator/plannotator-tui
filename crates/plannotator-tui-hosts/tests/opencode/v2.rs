//! `OpenCode` 2 (`session_message` payload only, as before 2.4): the payload's text blocks are
//! the reply, and a database without the part table keeps the old tolerance.

use plannotator_tui_hosts::opencode::Schema;
use plannotator_tui_hosts::{Role, opencode};

use super::fixture::{Db, message_v2, populate_v2};

#[test]
fn payload_text_blocks_join_in_order_and_reasoning_is_not_reply_text() {
    let db = Db::new("v2-payload");
    let _writer = populate_v2(db.file());
    let messages = opencode::messages_for_session(db.file(), "ses2_new", Schema::V2, 25).expect("messages");
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        ["OPENCODE2_V2_SESSION_SELECTED\n\nDone.", "reply with exactly OPENCODE2_V2_SESSION_SELECTED"]
    );
    assert_eq!(messages[0].role, Role::Assistant);
    assert_eq!(messages[0].id, "msg2_3");
    assert_eq!(messages[0].at.as_deref(), Some("1970-01-01T00:00:50.002Z"));
    assert_eq!(messages[1].role, Role::Human);
}

#[test]
fn the_v2_reply_limit_counts_replies_with_payload_text() {
    let db = Db::new("v2-payload-limit");
    let _writer = populate_v2(db.file());
    // A limit of one stops at the newest reply: the reasoning-only turn and the switch rows
    // take no slot, so the next reply is not reached.
    let one = opencode::messages_for_session(db.file(), "ses2_new", Schema::V2, 1).expect("one");
    assert_eq!(one.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg2_3"]);
    let two = opencode::messages_for_session(db.file(), "ses2_new", Schema::V2, 2).expect("two");
    assert_eq!(two.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg2_3", "msg2_1"]);
}

#[test]
fn a_malformed_v2_message_payload_is_skipped_rather_than_failing_the_read() {
    let db = Db::new("v2-bad-payload");
    let writer = populate_v2(db.file());
    message_v2(&writer, "msg2_bad", "ses2_new", 6, "assistant", "{not json");
    // No part table here: the payload is the only source, and one unreadable row does not
    // hide the rest of the session.
    let messages = opencode::messages_for_session(db.file(), "ses2_new", Schema::V2, 25).expect("messages");
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["msg2_3", "msg2_1"],
        "the malformed row is skipped, the readable ones stay"
    );
}
