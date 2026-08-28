//! Claude Code transcripts: what renders, in what order, on which branch.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use plannotator_tui_hosts::Role;
use plannotator_tui_hosts::claude::parse_messages;

const TRANSCRIPT: &str = include_str!("fixtures/claude-code.jsonl");
const COMPACTED: &str = include_str!("fixtures/claude-code-compacted.jsonl");
const DANGLING: &str = include_str!("fixtures/claude-code-dangling.jsonl");

fn texts(role: Role, n: usize) -> Vec<String> {
    parse_messages(TRANSCRIPT, n).into_iter().filter(|m| m.role == role).map(|m| m.text).collect()
}

#[test]
fn the_newest_entry_with_a_uuid_wins_over_trailing_bookkeeping() {
    // last-prompt, ai-title and a non-JSON line come after the newest real entry.
    let newest = parse_messages(TRANSCRIPT, 1);
    assert_eq!(newest.len(), 1);
    assert_eq!(newest[0].text, "assistant text 3");
    assert_eq!(newest[0].at.as_deref(), Some("2026-08-28T10:05:00.000Z"));
}

#[test]
fn the_active_branch_follows_the_later_rewind_and_drops_the_abandoned_one() {
    let all = texts(Role::Assistant, 25);
    assert!(all.iter().any(|t| t == "assistant text 3"));
    assert!(!all.iter().any(|t| t.contains("abandoned")), "{all:?}");
    let humans = texts(Role::Human, 25);
    assert_eq!(humans, vec!["user prompt 3", "user prompt 1"]);
}

#[test]
fn streamed_chunks_concatenate_in_order_and_skip_thinking_and_tool_blocks() {
    let all = texts(Role::Assistant, 25);
    let streamed = all.iter().find(|t| t.starts_with("assistant text 1a")).expect("message 1");
    assert_eq!(*streamed, "assistant text 1a\n\nassistant text 1b\n\nassistant text 1c");
    assert!(!streamed.contains("thinking") && !streamed.contains("tool input"));
}

#[test]
fn machinery_meta_and_sidechain_entries_are_not_human_prompts() {
    let humans = texts(Role::Human, 25);
    assert!(
        !humans.iter().any(|t| t.contains("meta text")
            || t.contains("sidechain")
            || t.starts_with("<system-reminder>")),
        "{humans:?}"
    );
}

#[test]
fn newest_first_and_n_is_honoured() {
    let two = parse_messages(TRANSCRIPT, 2);
    assert_eq!(two.len(), 2);
    assert_eq!(two[0].text, "assistant text 3");
    assert_eq!((two[1].role, two[1].text.as_str()), (Role::Human, "user prompt 3"));
}

#[test]
fn a_compacted_transcript_falls_back_to_file_order_rather_than_nothing() {
    // The active branch ends at the compact summary and holds no assistant text.
    let messages = parse_messages(COMPACTED, 5);
    let assistants: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Assistant).map(|m| m.text.as_str()).collect();
    assert_eq!(assistants, vec!["assistant text before compact"]);
    assert!(
        !messages.iter().any(|m| m.text.starts_with("This session is being continued")),
        "the summary is not a prompt"
    );
}

#[test]
fn a_dangling_parent_makes_the_chain_untrusted_and_file_order_applies() {
    let messages = parse_messages(DANGLING, 5);
    let assistants: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Assistant).map(|m| m.text.as_str()).collect();
    assert_eq!(assistants, vec!["assistant text 2", "assistant text 1"]);
}

#[test]
fn garbage_input_yields_no_messages_rather_than_an_error() {
    assert!(parse_messages("not json\n{\"type\":\"progress\"}\n", 3).is_empty());
}
