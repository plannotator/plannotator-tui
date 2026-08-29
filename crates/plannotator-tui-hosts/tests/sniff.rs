//! A transcript's format from its first lines, for paths that arrive without a host name.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::PathBuf;

use plannotator_tui_hosts::{Host, sniff};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn head(relative: &str) -> String {
    let text = std::fs::read_to_string(fixtures().join(relative)).expect("fixture");
    text.chars().take(64 * 1024).collect()
}

#[test]
fn each_fixture_format_is_recognised() {
    assert_eq!(sniff(&head("claude-code.jsonl")), Some(Host::ClaudeCode));
    assert_eq!(sniff(&head("pi.jsonl")), Some(Host::Pi));
    assert_eq!(
        sniff(&head(
            "pi-sessions/--work-project--/2026-08-28T10-30-00-000Z_01a00000-0000-7000-8000-000000000002.jsonl"
        )),
        Some(Host::Pi)
    );
    assert_eq!(
        sniff(&head("copilot/session-state/aaaa1111-0000-4000-8000-000000000001/events.jsonl")),
        Some(Host::Copilot)
    );
    assert_eq!(
        sniff(&head("droid/sessions/-Users-me-repo/be4202cc-4266-4e3b-b0f1-9324af19e4be.jsonl")),
        Some(Host::Droid),
        "droid logs: Claude's message shape keyed by id/parentId, read in file order"
    );
    let codex = std::fs::read_dir(fixtures().join("codex/sessions/2026/08/28"))
        .expect("codex day")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .expect("a codex rollout");
    let text = std::fs::read_to_string(codex).expect("rollout");
    assert_eq!(sniff(&text), Some(Host::Codex));
}

#[test]
fn spacing_after_colons_does_not_matter() {
    assert_eq!(sniff(r#"{"type": "session_meta", "payload": {}}"#), Some(Host::Codex));
    assert_eq!(sniff(r#"{"uuid": "a", "parentUuid": null, "type": "user"}"#), Some(Host::ClaudeCode));
}

#[test]
fn unknown_text_is_not_guessed() {
    assert_eq!(sniff("# just markdown\n\nhello\n"), None);
    assert_eq!(sniff(""), None);
}
