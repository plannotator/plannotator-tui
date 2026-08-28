//! Codex rollouts: thread files, turn boundaries, subagents.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::Path;

use plannotator_tui_hosts::codex::{find_transcripts, parse_messages};

const THREAD: &str = "01a04583-a848-7b21-a890-f3ed0c9fef05";

fn home() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/codex"))
}

fn contents(paths: &[std::path::PathBuf]) -> Vec<String> {
    paths.iter().map(|p| std::fs::read_to_string(p).expect("fixture")).collect()
}

#[test]
fn a_thread_spanning_two_files_is_returned_oldest_first() {
    let files = find_transcripts(home(), Some(THREAD));
    let names: Vec<String> =
        files.iter().map(|p| p.file_name().expect("name").to_string_lossy().into_owned()).collect();
    assert_eq!(
        names,
        vec![
            format!("rollout-2026-08-27T16-17-33-{THREAD}.jsonl"),
            format!("rollout-2026-08-28T09-00-00-{THREAD}.jsonl")
        ]
    );
}

#[test]
fn without_a_thread_id_the_newest_non_subagent_rollout_picks_the_thread() {
    // The newest file on disk is the guardian subagent's; it must not be chosen.
    let files = find_transcripts(home(), None);
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|p| p.to_string_lossy().contains(THREAD)));
}

#[test]
fn the_active_turn_is_the_one_started_after_the_last_completion() {
    let files = contents(&find_transcripts(home(), Some(THREAD)));
    let messages = parse_messages(&files, 10);
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, vec!["assistant text 5 (in progress)"]);
    assert_eq!(messages[0].id, "m-a5");
}

#[test]
fn when_every_turn_has_completed_the_whole_thread_counts_newest_first() {
    let files = contents(&find_transcripts(home(), Some(THREAD)));
    let only_first: Vec<String> = files.iter().take(1).cloned().collect();
    let texts: Vec<String> = parse_messages(&only_first, 10).into_iter().map(|m| m.text).collect();
    assert_eq!(texts, vec!["assistant text 2 (final)", "assistant text 1 (commentary)"]);
    assert_eq!(parse_messages(&only_first, 1).len(), 1);
}

#[test]
fn an_unknown_thread_has_no_files() {
    assert!(find_transcripts(home(), Some("nope")).is_empty());
}
