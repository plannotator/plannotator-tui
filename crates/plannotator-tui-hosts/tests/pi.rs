//! Pi sessions: the active branch, what counts as a message, and finding a session by cwd.
//! Semantics match Plannotator's `apps/pi-extension/assistant-message.ts`.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::Path;

use plannotator_tui_hosts::pi::{encoded_dir, find_transcript, parse_messages};
use plannotator_tui_hosts::{Host, Role, detect_host};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")).join(name))
        .expect("fixture")
}

fn sessions() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pi-sessions"))
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-pi-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A writable copy of the session fixtures whose modification times rank in filename
/// order, one minute apart. A checkout leaves every fixture the same age in whatever
/// order git wrote them, which says nothing about which session was last used.
fn staged_sessions(name: &str) -> std::path::PathBuf {
    let root = temp(name);
    let mut copied = Vec::new();
    for entry in walk(sessions()) {
        let relative = entry.strip_prefix(sessions()).expect("under the fixtures");
        let target = root.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("bucket");
        }
        std::fs::copy(&entry, &target).expect("copy");
        copied.push(target);
    }
    copied.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    for (index, path) in copied.iter().enumerate() {
        touch(path, 1_000_000 + index as u64 * 60);
    }
    root
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("fixtures").flatten() {
        let path = entry.path();
        if path.is_dir() { out.extend(walk(&path)) } else { out.push(path) }
    }
    out
}

/// Set `path`'s modification time to `seconds` after the Unix epoch.
fn touch(path: &Path, seconds: u64) {
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    std::fs::File::options().write(true).open(path).expect("open").set_modified(time).expect("mtime");
}

#[test]
fn the_newest_assistant_message_comes_first_and_the_later_rewind_branch_wins() {
    let messages = parse_messages(&fixture("pi.jsonl"), 25);
    let assistant: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Assistant).map(|m| m.text.as_str()).collect();
    assert_eq!(
        assistant,
        vec!["assistant text 3", "assistant text 1"],
        "the abandoned branch is not on the chain"
    );
    assert_eq!(messages[0].id, "0c0c0c0c");
    assert_eq!(messages[0].at.as_deref(), Some("2026-08-28T17:27:20.000Z"));
}

#[test]
fn tool_calls_tool_results_and_thinking_are_not_messages() {
    let messages = parse_messages(&fixture("pi.jsonl"), 25);
    assert!(messages.iter().all(|m| !m.text.contains("tool result") && !m.text.contains("thinking")));
    let humans: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Human).map(|m| m.text.as_str()).collect();
    assert_eq!(humans, vec!["user prompt 3", "user prompt 1"]);
}

#[test]
fn an_unreconstructable_chain_yields_nothing_rather_than_the_wrong_messages() {
    assert!(parse_messages(&fixture("pi-dangling.jsonl"), 25).is_empty());
}

#[test]
fn n_caps_the_total_and_numeric_timestamps_become_iso() {
    let messages = parse_messages(&fixture("pi.jsonl"), 2);
    assert_eq!(messages.len(), 2);
    let line = r#"{"type":"message","id":"a1","parentId":null,"timestamp":1787938040000,"message":{"role":"assistant","content":[{"type":"text","text":"x"}]}}"#;
    assert_eq!(parse_messages(line, 1)[0].at.as_deref(), Some("2026-08-28T17:27:20.000Z"));
    let string_content = r#"{"type":"message","id":"a2","parentId":null,"timestamp":"t","message":{"role":"assistant","content":"plain string"}}"#;
    assert!(parse_messages(string_content, 1).is_empty(), "string content is not an array; skipped");
}

#[test]
fn the_encoded_dir_matches_pis_session_manager() {
    assert_eq!(
        encoded_dir(Path::new("/Users/ramos/oss/plannotator-tui")),
        "--Users-ramos-oss-plannotator-tui--"
    );
    assert_eq!(
        encoded_dir(Path::new(r"C:\work\x")),
        "--C--work-x--",
        "colon and backslash each become a dash"
    );
}

#[test]
fn the_newest_session_for_the_cwd_wins_over_a_newer_one_elsewhere_and_over_empty_ones() {
    let root = staged_sessions("cwd");
    let found = find_transcript(&root, Path::new("/work/project")).expect("found");
    assert!(
        found.ends_with(
            "--work-project--/2026-08-28T10-00-00-000Z_01a00000-0000-7000-8000-000000000001.jsonl"
        ),
        "{found:?}"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

/// A resumed session is written to; a session created after it and left alone is not. Pi
/// names a file once, at creation, so only the modification time says which one the pane
/// is actually in.
#[test]
fn a_session_written_to_more_recently_wins_over_one_with_a_newer_name() {
    let root = temp("mtime");
    let bucket = root.join(encoded_dir(Path::new("/work/project")));
    std::fs::create_dir_all(&bucket).expect("bucket");
    let session = |id: &str, text: &str| {
        format!(
            concat!(
                r#"{{"type":"session","version":3,"id":"{0}","cwd":"/work/project"}}"#,
                "\n",
                r#"{{"type":"message","id":"m-{0}","parentId":null,"timestamp":"2026-08-28T10:00:00.000Z","#,
                r#""message":{{"role":"assistant","content":[{{"type":"text","text":"{1}"}}]}}}}"#,
                "\n",
            ),
            id, text
        )
    };
    let resumed = bucket.join("2026-08-28T10-00-00-000Z_01a00000-0000-7000-8000-00000000000a.jsonl");
    let idle = bucket.join("2026-08-29T10-00-00-000Z_01a00000-0000-7000-8000-00000000000b.jsonl");
    std::fs::write(&resumed, session("a", "resumed")).expect("write");
    std::fs::write(&idle, session("b", "idle")).expect("write");
    touch(&idle, 1_000_000);
    touch(&resumed, 1_000_060);

    let found = find_transcript(&root, Path::new("/work/project")).expect("found");

    assert_eq!(found, resumed, "the last-written session is the one the pane is in");
    let text = std::fs::read_to_string(&found).expect("session");
    assert_eq!(parse_messages(&text, 1).first().map(|m| m.text.as_str()), Some("resumed"));
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn a_legacy_flat_file_counts_for_its_cwd_when_the_encoded_dir_has_nothing() {
    let root = temp("flat");
    let flat = sessions().join("2026-08-27T09-00-00-000Z_01a00000-0000-7000-8000-000000000003.jsonl");
    std::fs::copy(&flat, root.join(flat.file_name().expect("name"))).expect("copy");
    let found = find_transcript(&root, Path::new("/work/project")).expect("found");
    assert!(found.to_string_lossy().ends_with("000000000003.jsonl"));
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn an_unknown_cwd_falls_back_to_the_newest_session_anywhere() {
    let root = staged_sessions("anywhere");
    let found = find_transcript(&root, Path::new("/nowhere")).expect("found");
    assert!(
        found.ends_with("--work-other--/2026-08-28T11-00-00-000Z_01a00000-0000-7000-8000-000000000004.jsonl"),
        "{found:?}"
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn pi_markers_select_the_pi_host_after_codex() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |k: &str| vars.iter().find(|(name, _)| *name == k).map(|(_, v)| (*v).to_owned())
    };
    assert_eq!(detect_host(env(&[("PI_CODING_AGENT", "true")])).expect("host"), Host::Pi);
    assert_eq!(detect_host(env(&[("AI_AGENT", "pi")])).expect("host"), Host::Pi);
    assert_eq!(detect_host(env(&[("PLANNOTATOR_TUI_HOST", "pi")])).expect("host"), Host::Pi);
    assert_eq!(
        detect_host(env(&[("CODEX_THREAD_ID", "t"), ("PI_CODING_AGENT", "true")])).expect("host"),
        Host::Codex
    );
    assert_eq!(Host::Pi.label(), "pi");
}
