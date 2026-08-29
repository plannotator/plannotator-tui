//! `plannotator-tui last` through the real binary against the hosts crate's fixtures.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::process::Command;

use plannotator_tui_hosts::{Role, claude, codex, copilot, droid, omp, pi};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plannotator-tui-hosts/tests/fixtures")
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
