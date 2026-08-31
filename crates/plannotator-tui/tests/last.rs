//! `plannotator-tui last` through the real binary against the hosts crate's fixtures.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::process::Command;

use plannotator_tui_hosts::{Role, claude, codex, copilot, droid, omp, pi};

const NAMED_CODEX_ID: &str = "11111111-1111-4111-8111-111111111111";
const NEWER_CODEX_ID: &str = "22222222-2222-4222-8222-222222222222";

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plannotator-tui-hosts/tests/fixtures")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator last {tag} ü-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn codex_rollout(home: &std::path::Path, day: &str, time: &str, id: &str, text: &str) {
    let file =
        home.join("sessions/2026/08").join(day).join(format!("rollout-2026-08-{day}T{time}-{id}.jsonl"));
    std::fs::create_dir_all(file.parent().expect("parent")).expect("sessions");
    std::fs::write(
        file,
        format!(
            "{{\"timestamp\":\"2026-08-{day}T{time}Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}\n{{\"timestamp\":\"2026-08-{day}T{time}Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"id\":\"m-{id}\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{text}\"}}]}}}}\n"
        ),
    )
    .expect("rollout");
}

#[test]
fn print_writes_the_newest_assistant_message_of_a_claude_transcript() {
    let transcript = fixtures().join("claude-code.jsonl");
    let text = std::fs::read_to_string(&transcript).expect("fixture");
    let expected = claude::parse_messages(&text, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("fixture has an assistant message")
        .text;
    let out = bin().args(["last", "--session"]).arg(&transcript).arg("--print").output().expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn print_reads_a_codex_thread_from_a_sessions_root() {
    let root = fixtures().join("codex");
    let files = codex::find_transcripts(&root, None);
    let contents: Vec<String> = files.iter().map(|p| std::fs::read_to_string(p).expect("file")).collect();
    let expected = codex::parse_messages(&contents, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("fixture has an assistant message")
        .text;
    let out = bin()
        .args(["last", "--host", "codex", "--session"])
        .arg(&root)
        .arg("--print")
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn herdr_reported_codex_id_beats_a_newer_unrelated_rollout() {
    let root = temp_dir("codex id");
    let home = root.join("codex home");
    codex_rollout(&home, "30", "10-00-00", NAMED_CODEX_ID, "NAMED SESSION");
    codex_rollout(&home, "31", "10-00-00", NEWER_CODEX_ID, "NEWER UNRELATED SESSION");
    let out = bin()
        .env("CODEX_HOME", &home)
        .env_remove("CODEX_THREAD_ID")
        .args(["last", "--host", "codex", "--session-id", NAMED_CODEX_ID, "--print"])
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "NAMED SESSION");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));

    let miss = bin()
        .env("CODEX_HOME", &home)
        .env_remove("CODEX_THREAD_ID")
        .args(["last", "--host", "codex", "--session-id", "missing", "--print"])
        .output()
        .expect("runs");
    assert!(miss.status.success(), "--print keeps its exit-zero contract");
    assert!(miss.stdout.is_empty(), "an exact miss must not print the newer session");
    let stderr = String::from_utf8_lossy(&miss.stderr);
    assert!(stderr.contains("no Codex session missing"), "{stderr}");
    assert!(stderr.contains(&home.join("sessions").display().to_string()), "{stderr}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn claude_config_override_is_used_for_an_exact_id() {
    let root = temp_dir("claude override");
    let config = root.join("Claude Config ü");
    let cwd = root.join("work tree");
    let transcript = config.join("projects").join(claude::project_slug(&cwd)).join("claude-exact.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("parent")).expect("project");
    std::fs::copy(fixtures().join("claude-code.jsonl"), &transcript).expect("fixture copy");
    let text = std::fs::read_to_string(&transcript).expect("fixture");
    let expected = claude::parse_messages(&text, 25)
        .into_iter()
        .find(|message| message.role == Role::Assistant)
        .expect("assistant")
        .text;
    std::fs::create_dir_all(&cwd).expect("cwd");
    let out = bin()
        .current_dir(&cwd)
        .env("CLAUDE_CONFIG_DIR", &config)
        .env("PLANNOTATOR_TUI_CWD", &cwd)
        .args(["last", "--host", "claude", "--session-id", "claude-exact", "--print"])
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn print_reads_a_copilot_session_directory() {
    let dir = fixtures().join("copilot/session-state/aaaa1111-0000-4000-8000-000000000001");
    let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("fixture");
    let expected = copilot::parse_messages(&events, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("fixture has an assistant message")
        .text;
    let out = bin()
        .args(["last", "--host", "copilot", "--session"])
        .arg(&dir)
        .arg("--print")
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn print_reads_a_droid_log() {
    let log = fixtures().join("droid/sessions/-Users-me-repo/be4202cc-4266-4e3b-b0f1-9324af19e4be.jsonl");
    let text = std::fs::read_to_string(&log).expect("fixture");
    let expected = droid::parse_messages(&text, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("fixture has an assistant message")
        .text;
    let out =
        bin().args(["last", "--host", "droid", "--session"]).arg(&log).arg("--print").output().expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn print_never_fails_the_caller_when_nothing_is_found() {
    let missing = fixtures().join("does-not-exist.jsonl");
    let out = bin().args(["last", "--session"]).arg(&missing).arg("--print").output().expect("runs");
    assert!(out.status.success(), "exit 0 is the contract");
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does-not-exist.jsonl"));
}

#[test]
fn stdin_is_printed_back_verbatim() {
    use std::io::Write as _;
    let mut child = bin()
        .args(["last", "--stdin", "--print"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawns");
    child.stdin.take().expect("stdin").write_all(b"# hi\n\nfrom stdin\n").expect("write");
    let out = child.wait_with_output().expect("runs");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "# hi\n\nfrom stdin\n");
}

#[test]
fn print_writes_the_newest_assistant_message_of_a_pi_session() {
    let transcript = fixtures().join("pi.jsonl");
    let text = std::fs::read_to_string(&transcript).expect("fixture");
    let expected = pi::parse_messages(&text, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("fixture has an assistant message")
        .text;
    let out = bin()
        .args(["last", "--host", "pi", "--session"])
        .arg(&transcript)
        .arg("--print")
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

fn newest_pi_reply() -> (PathBuf, String) {
    let transcript = fixtures().join("pi.jsonl");
    let text = std::fs::read_to_string(&transcript).expect("fixture");
    let expected = omp::parse_messages(&text, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("fixture has an assistant message")
        .text;
    (transcript, expected)
}

#[test]
fn omp_reads_pi_format_sessions() {
    let (transcript, expected) = newest_pi_reply();
    let out = bin()
        .args(["last", "--host", "omp", "--session"])
        .arg(&transcript)
        .arg("--print")
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn a_session_path_without_a_host_is_sniffed() {
    let (transcript, expected) = newest_pi_reply();
    let out = bin()
        .env_remove("PLANNOTATOR_TUI_HOST")
        .args(["last", "--session"])
        .arg(&transcript)
        .arg("--print")
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        expected.trim_end(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn hermes_reads_the_session_named_by_id_from_hermes_home() {
    let home = std::env::temp_dir().join(format!("plannotator-tui-hermes-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let db = rusqlite::Connection::open(home.join("state.db")).expect("db");
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, started_at REAL NOT NULL);
         CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, role TEXT NOT NULL,
             content TEXT, timestamp REAL NOT NULL, active INTEGER NOT NULL DEFAULT 1, compacted INTEGER NOT NULL DEFAULT 0);
         INSERT INTO sessions VALUES ('abc', 'cli', 1.0);
         INSERT INTO messages (session_id, role, content, timestamp) VALUES ('abc', 'user', 'hi', 1.0);
         INSERT INTO messages (session_id, role, content, timestamp) VALUES ('abc', 'assistant', 'older', 2.0);
         INSERT INTO messages (session_id, role, content, timestamp) VALUES ('abc', 'assistant', 'the newest reply', 3.0);",
    )
    .expect("rows");
    drop(db);
    let out = bin()
        .env("HERMES_HOME", &home)
        .args(["last", "--host", "hermes", "--session-id", "abc", "--print"])
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        "the newest reply",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let missing =
        bin().env("HERMES_HOME", &home).args(["last", "--host", "hermes", "--print"]).output().expect("runs");
    assert!(missing.status.success(), "exit 0 is the contract");
    assert!(String::from_utf8_lossy(&missing.stderr).contains("needs a session id"));
    std::fs::remove_dir_all(&home).expect("cleanup");
}
