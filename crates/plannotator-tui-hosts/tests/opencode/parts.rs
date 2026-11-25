//! `OpenCode` 2.4 (`session_part` rows): a reply without payload text reads its visible parts
//! in `ordinal` order, and a part that cannot be read is an error naming what was being read.

use plannotator_tui_hosts::opencode::Schema;
use plannotator_tui_hosts::{HostError, Role, opencode};

use super::fixture::{Db, message_v2, part_v2, populate_v2_parts};

#[test]
fn a_reply_read_from_its_parts_joins_the_visible_ones_in_ordinal_order() {
    let db = Db::new("parts-order");
    let _writer = populate_v2_parts(db.file());
    let messages = opencode::messages_for_session(db.file(), "ses2_mixed", Schema::V2, 25).expect("messages");
    let parts = messages.iter().find(|m| m.id == "msg2_parts").expect("parts reply");
    assert_eq!(parts.text, "part one\n\npart two");
    assert_eq!(parts.role, Role::Assistant);
    assert_eq!(parts.at.as_deref(), Some("1970-01-01T00:01:10.003Z"));
}

#[test]
fn injected_ignored_and_reasoning_parts_are_not_reply_text() {
    let db = Db::new("parts-skipped");
    let writer = populate_v2_parts(db.file());
    message_v2(
        &writer,
        "msg2_only_noise",
        "ses2_mixed",
        7,
        "assistant",
        r#"{"content":[],"time":{"created":70007}}"#,
    );
    part_v2(
        &writer,
        "msg2_only_noise",
        "ses2_mixed",
        0,
        "reasoning",
        r#"{"type":"reasoning","text":"thinking"}"#,
    );
    part_v2(
        &writer,
        "msg2_only_noise",
        "ses2_mixed",
        1,
        "text",
        r#"{"type":"text","text":"injected","synthetic":true}"#,
    );
    part_v2(
        &writer,
        "msg2_only_noise",
        "ses2_mixed",
        2,
        "text",
        r#"{"type":"text","text":"draft","ignored":true}"#,
    );
    let messages = opencode::messages_for_session(db.file(), "ses2_mixed", Schema::V2, 25).expect("messages");
    assert!(
        messages.iter().all(|m| m.id != "msg2_only_noise"),
        "a reply with only reasoning, injected and ignored parts has no text"
    );
}

#[test]
fn a_reply_stored_in_both_formats_is_listed_once_from_its_payload() {
    let db = Db::new("parts-both");
    let _writer = populate_v2_parts(db.file());
    let messages = opencode::messages_for_session(db.file(), "ses2_mixed", Schema::V2, 25).expect("messages");
    let both: Vec<&str> = messages.iter().filter(|m| m.id == "msg2_both").map(|m| m.text.as_str()).collect();
    assert_eq!(both, ["both reply"]);
    assert!(messages.iter().all(|m| !m.text.contains("part duplicate")));
}

#[test]
fn a_mixed_session_lists_prompts_and_replies_newest_first() {
    let db = Db::new("parts-order-all");
    let _writer = populate_v2_parts(db.file());
    let messages = opencode::messages_for_session(db.file(), "ses2_mixed", Schema::V2, 25).expect("messages");
    let ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    let roles: Vec<Role> = messages.iter().map(|m| m.role).collect();
    let times: Vec<Option<&str>> = messages.iter().map(|m| m.at.as_deref()).collect();
    assert_eq!(ids, ["msg2_q2", "msg2_both", "msg2_parts", "msg2_legacy", "msg2_q1"]);
    assert_eq!(
        texts,
        ["follow-up question", "both reply", "part one\n\npart two", "legacy reply", "mixed question"]
    );
    assert_eq!(roles, [Role::Human, Role::Assistant, Role::Assistant, Role::Assistant, Role::Human]);
    assert_eq!(
        times,
        [
            Some("1970-01-01T00:01:10.005Z"),
            Some("1970-01-01T00:01:10.004Z"),
            Some("1970-01-01T00:01:10.003Z"),
            Some("1970-01-01T00:01:10.002Z"),
            Some("1970-01-01T00:01:10.001Z"),
        ]
    );
}

#[test]
fn the_reply_limit_counts_replies_that_have_visible_text() {
    let db = Db::new("parts-limit");
    let _writer = populate_v2_parts(db.file());
    // Only the reasoning turn is invisible, so a limit of two replies still reaches the parts
    // reply; a limit of one stops at the newest reply and keeps the prompt after it.
    let one = opencode::messages_for_session(db.file(), "ses2_mixed", Schema::V2, 1).expect("one");
    assert_eq!(one.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg2_q2", "msg2_both"]);
    let two = opencode::messages_for_session(db.file(), "ses2_mixed", Schema::V2, 2).expect("two");
    assert_eq!(two.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["msg2_q2", "msg2_both", "msg2_parts"]);
}

#[test]
fn malformed_part_json_is_an_error_naming_the_message_session_and_database() {
    let db = Db::new("parts-bad");
    let writer = populate_v2_parts(db.file());
    message_v2(
        &writer,
        "msg2_broken",
        "ses2_mixed",
        7,
        "assistant",
        r#"{"content":[],"time":{"created":70007}}"#,
    );
    part_v2(&writer, "msg2_broken", "ses2_mixed", 0, "text", "{not json");
    let path = db.file().display().to_string();
    match opencode::messages_for_session(db.file(), "ses2_mixed", Schema::V2, 25) {
        Err(HostError::NoTranscript(msg)) => {
            assert!(msg.contains("msg2_broken"), "{msg}");
            assert!(msg.contains("ses2_mixed"), "{msg}");
            assert!(msg.contains(&path), "{msg}");
            assert!(msg.contains("text parts"), "{msg}");
        }
        other => panic!("expected a part read error, got {other:?}"),
    }
}

#[test]
fn replies_with_payload_text_read_even_when_the_part_table_has_other_columns() {
    let db = Db::new("parts-other-columns");
    let writer = populate_v2_parts(db.file());
    // A later `OpenCode` could reshape the part table; a session that never needs it still reads.
    writer
        .execute_batch(
            "DROP TABLE session_part;
             CREATE TABLE session_part (message_id TEXT NOT NULL, position INTEGER NOT NULL,
                 payload TEXT NOT NULL, PRIMARY KEY (message_id, position));",
        )
        .expect("reshaped part table");
    message_v2(&writer, "msg2_i1", "ses2_inline", 1, "user", r#"{"text":"inline question"}"#);
    message_v2(
        &writer,
        "msg2_i2",
        "ses2_inline",
        2,
        "assistant",
        r#"{"content":[{"type":"text","text":"inline reply"}],"time":{"created":70010}}"#,
    );
    let messages =
        opencode::messages_for_session(db.file(), "ses2_inline", Schema::V2, 25).expect("messages");
    assert_eq!(
        messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["inline reply", "inline question"]
    );
}
