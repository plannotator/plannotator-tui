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
    assert!(matches!(detect_host(env(&[("GEMINI_CLI", "1")])), Err(HostError::Unsupported(_))));
    assert_eq!(detect_host(env(&[("OPENCODE", "1")])).expect("opencode"), Host::OpenCode);
}

#[test]
fn omp_is_selected_by_its_name_first_and_by_ompcode_last() {
    // OMP reuses pi's flag, so its name must win over pi's marker.
    assert_eq!(
        detect_host(env(&[("AI_AGENT", "omp"), ("PI_CODING_AGENT", "true")])).expect("host"),
        Host::Omp
    );
    assert_eq!(detect_host(env(&[("OMPCODE", "1")])).expect("host"), Host::Omp);
    // ...but OMPCODE alone is checked after every other marker (it leaks into every shell).
    assert_eq!(detect_host(env(&[("OMPCODE", "1"), ("PI_CODING_AGENT", "true")])).expect("host"), Host::Pi);
    assert!(matches!(
        detect_host(env(&[("OMPCODE", "1"), ("GEMINI_CLI", "1")])),
        Err(HostError::Unsupported(_))
    ));
    assert_eq!(detect_host(env(&[("PLANNOTATOR_TUI_HOST", "oh-my-pi")])).expect("host"), Host::Omp);
    assert_eq!(detect_host(env(&[("PLANNOTATOR_TUI_HOST", "hermes")])).expect("host"), Host::Hermes);
    assert_eq!(detect_host(env(&[("PLANNOTATOR_TUI_HOST", "opencode")])).expect("host"), Host::OpenCode);
}

#[test]
fn the_copilot_marker_selects_copilot_and_droid_needs_the_override() {
    assert_eq!(detect_host(env(&[("COPILOT_CLI", "1")])).expect("host"), Host::Copilot);
    assert_eq!(
        detect_host(env(&[("CODEX_THREAD_ID", "t"), ("COPILOT_CLI", "1")])).expect("host"),
        Host::Codex
    );
    assert_eq!(detect_host(env(&[("PLANNOTATOR_TUI_HOST", "droid")])).expect("host"), Host::Droid);
    assert_eq!(detect_host(env(&[("PLANNOTATOR_TUI_HOST", "copilot-cli")])).expect("host"), Host::Copilot);
}

#[test]
fn labels_are_the_short_names_herdr_uses() {
    assert_eq!(Host::ClaudeCode.label(), "claude");
    assert_eq!(Host::Codex.label(), "codex");
    assert_eq!(Host::Copilot.label(), "copilot");
    assert_eq!(Host::Droid.label(), "droid");
    assert_eq!(Host::Pi.label(), "pi");
    assert_eq!(Host::Omp.label(), "omp");
    assert_eq!(Host::Hermes.label(), "hermes");
    assert_eq!(Host::OpenCode.label(), "opencode");
    assert_eq!(Host::ALL.len(), 8);
}
