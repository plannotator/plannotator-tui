//! Which host launched us.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use plannotator_tui_hosts::{Host, HostError, detect_host};

fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| vars.iter().find(|(name, _)| *name == k).map(|(_, v)| (*v).to_owned())
}

#[test]
fn claude_code_is_the_default() {
    assert_eq!(detect_host(env(&[])).expect("host"), Host::ClaudeCode);
}

#[test]
fn the_override_wins_when_it_names_a_known_host_and_is_ignored_otherwise() {
    assert_eq!(
        detect_host(env(&[("PLANNOTATOR_TUI_HOST", "Codex"), ("CODEX_THREAD_ID", "")])).expect("host"),
        Host::Codex
    );
    assert_eq!(
        detect_host(env(&[("PLANNOTATOR_TUI_HOST", "claude-code"), ("CODEX_THREAD_ID", "t")])).expect("host"),
        Host::ClaudeCode
    );
    assert_eq!(
        detect_host(env(&[("PLANNOTATOR_TUI_HOST", "mystery"), ("CODEX_THREAD_ID", "t")])).expect("host"),
        Host::Codex
    );
}

#[test]
fn codex_marker_beats_unsupported_markers_which_are_reported_not_hidden() {
    assert_eq!(detect_host(env(&[("CODEX_THREAD_ID", "t"), ("OMPCODE", "1")])).expect("host"), Host::Codex);
    assert!(
        matches!(detect_host(env(&[("OMPCODE", "1")])), Err(HostError::Unsupported(name)) if name == "OMP")
    );
    assert!(matches!(detect_host(env(&[("GEMINI_CLI", "1")])), Err(HostError::Unsupported(_))));
}

#[test]
fn labels_are_the_short_names_herdr_uses() {
    assert_eq!(Host::ClaudeCode.label(), "claude");
    assert_eq!(Host::Codex.label(), "codex");
}
