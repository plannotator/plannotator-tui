//! `plannotator-tui last` through the real binary against the hosts crate's fixtures.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::process::Command;

use plannotator_tui_hosts::{Role, claude, codex, copilot, droid, omp, pi};

const NAMED_CODEX_ID: &str = "11111111-1111-4111-8111-111111111111";
const NEWER_CODEX_ID: &str = "22222222-2222-4222-8222-222222222222";
const PI_SESSION_ID: &str = "01a04968-90e4-745a-a8d3-6903d935f391";

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

/// A session kept in `profile`, looked up by id with no `CLAUDE_CONFIG_DIR` and `config` as
/// the config file. Returns the output and the reply the session ends with.
fn exact_claude_lookup_in_profile(tag: &str, profile: &str, config: &str) -> (std::process::Output, String) {
    let root = temp_dir(tag);
    let home = root.join("home");
    let cwd = root.join("work tree");
    let id = "d2fcdbb5-0814-432c-8427-f1fcc75f603d";
    std::fs::create_dir_all(home.join(".claude/projects")).expect("default config");
    let transcript = home.join(profile).join("projects/-elsewhere").join(format!("{id}.jsonl"));
    std::fs::create_dir_all(transcript.parent().expect("parent")).expect("project");
    std::fs::copy(fixtures().join("claude-code.jsonl"), &transcript).expect("fixture copy");
    let text = std::fs::read_to_string(&transcript).expect("fixture");
    let expected = claude::parse_messages(&text, 25)
        .into_iter()
        .find(|message| message.role == Role::Assistant)
        .expect("assistant")
        .text;
    std::fs::create_dir_all(&cwd).expect("cwd");
    let config_file = root.join("config.toml");
    std::fs::write(&config_file, config).expect("config");
    let out = bin()
        .current_dir(&cwd)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("PLANNOTATOR_TUI_CONFIG", &config_file)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("PLANNOTATOR_TUI_CWD", &cwd)
        .args(["last", "--host", "claude", "--session-id", id, "--print"])
        .output()
        .expect("runs");
    std::fs::remove_dir_all(root).expect("cleanup");
    (out, expected)
}

#[test]
fn an_exact_claude_id_is_found_in_a_listed_profile_dir() {
    let (out, expected) = exact_claude_lookup_in_profile(
        "claude listed profile",
        "work-profile",
        "[last]\nclaude_profile_dirs = [\"~/work-profile\"]\n",
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn an_exact_claude_id_survives_a_config_file_with_an_error() {
    let (out, expected) =
        exact_claude_lookup_in_profile("claude broken config", ".claude", "[herdr]\nplacment = \"popup\"\n");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn a_session_not_found_names_the_listed_profile_dir_that_does_not_exist() {
    let (out, _) = exact_claude_lookup_in_profile(
        "claude missing profile",
        ".claude-max",
        "[last]\nclaude_profile_dirs = [\"~/.claude-gone\"]\n",
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let missing = stderr.split_once("(missing: ").map(|(_, rest)| rest.trim_end()).expect("named");
    assert!(missing.ends_with(".claude-gone)") && !missing.contains(", "), "{stderr}");
}

#[test]
fn a_listed_profile_dir_that_does_not_exist_is_no_error_when_the_session_is_found() {
    let (out, expected) = exact_claude_lookup_in_profile(
        "claude found despite missing",
        ".claude-max",
        "[last]\nclaude_profile_dirs = [\"~/.claude-gone\", \"~/.claude-max\"]\n",
    );
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.trim_end());
}

#[test]
fn an_exact_claude_id_in_an_unlisted_profile_dir_is_not_found() {
    let (out, _) = exact_claude_lookup_in_profile("claude unlisted profile", ".claude-max", "");
    assert!(out.stdout.is_empty(), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no Claude Code session"));
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
    assert_eq!(out.stdout, [] as [u8; 0]);
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
fn branched_pi_transcript(tag: &str) -> (PathBuf, PathBuf) {
    let root = temp_dir(tag);
    let transcript = root.join("snapshot.jsonl");
    let mut jsonl = std::fs::read_to_string(fixtures().join("pi.jsonl")).expect("fixture");
    jsonl.push('\n');
    jsonl.push_str(
        r#"{"type":"message","id":"beadfeed","parentId":"dbcc01c1","timestamp":"2026-08-28T17:28:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"assistant text appended on abandoned branch"}]}}"#,
    );
    std::fs::write(&transcript, jsonl).expect("branched session");
    (root, transcript)
}

#[test]
fn pi_leaf_id_selects_the_requested_branch_and_absence_keeps_legacy_selection() {
    let (root, transcript) = branched_pi_transcript("pi branch selection");
    let selected = bin()
        .args(["last", "--host", "pi", "--session"])
        .arg(&transcript)
        .args(["--session-id", PI_SESSION_ID, "--pi-leaf-id", "0c0c0c0c", "--print"])
        .output()
        .expect("runs");
    assert!(selected.status.success(), "{}", String::from_utf8_lossy(&selected.stderr));
    assert_eq!(String::from_utf8_lossy(&selected.stdout).trim(), "assistant text 3");

    let legacy = bin()
        .args(["last", "--host", "pi", "--session"])
        .arg(&transcript)
        .args(["--session-id", PI_SESSION_ID, "--print"])
        .output()
        .expect("runs");
    assert!(legacy.status.success(), "{}", String::from_utf8_lossy(&legacy.stderr));
    assert_eq!(String::from_utf8_lossy(&legacy.stdout).trim(), "assistant text appended on abandoned branch");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_pi_leaf_id_must_match_the_session_header_id() {
    let transcript = fixtures().join("pi.jsonl");
    let out = bin()
        .args(["last", "--host", "pi", "--session"])
        .arg(&transcript)
        .args(["--session-id", "22222222-2222-4222-8222-222222222222", "--pi-leaf-id", "0c0c0c0c", "--print"])
        .output()
        .expect("runs");
    assert!(out.status.success(), "--print keeps its exit-zero contract");
    assert!(out.stdout.is_empty(), "a mismatched file is never read as the requested session");
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not match supplied id"));
}

#[test]
fn null_or_unknown_pi_leaf_never_falls_back_to_another_branch() {
    let (root, transcript) = branched_pi_transcript("pi null leaf");
    for leaf in ["", "missing-leaf"] {
        let out = bin()
            .args(["last", "--host", "pi", "--session"])
            .arg(&transcript)
            .args(["--session-id", PI_SESSION_ID, "--pi-leaf-id"])
            .arg(leaf)
            .arg("--print")
            .output()
            .expect("runs");
        assert!(out.status.success(), "--print keeps its exit-zero contract");
        assert!(out.stdout.is_empty(), "leaf {leaf:?} must not print another branch");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("no assistant messages"),
            "leaf {leaf:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_pi_snapshot_requires_a_valid_session_header() {
    let root = temp_dir("pi headers");
    for (name, content, error) in [
        ("empty", "", "missing Pi session header"),
        ("invalid", "not-json", "invalid Pi session header"),
        ("wrong-type", r#"{"type":"message"}"#, "expected type=session"),
        ("missing-id", r#"{"type":"session"}"#, "missing an id"),
    ] {
        let transcript = root.join(format!("{name}.jsonl"));
        std::fs::write(&transcript, content).expect("write header");
        let out = bin()
            .args(["last", "--host", "pi", "--session"])
            .arg(&transcript)
            .args(["--session-id", PI_SESSION_ID, "--pi-leaf-id", "leaf", "--print"])
            .output()
            .expect("runs");
        assert!(out.stdout.is_empty(), "{name} header must fail closed");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(error),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let missing = root.join("missing.jsonl");
    let out = bin()
        .args(["last", "--host", "pi", "--session"])
        .arg(&missing)
        .args(["--session-id", PI_SESSION_ID, "--pi-leaf-id", "leaf", "--print"])
        .output()
        .expect("runs");
    assert!(out.stdout.is_empty(), "a missing file is never replaced by a discovered session");
    assert!(String::from_utf8_lossy(&out.stderr).contains("reading "));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn pi_leaf_flags_reject_missing_values_duplicates_and_stdin() {
    for (args, error) in [
        (&["last", "--host", "pi", "--pi-leaf-id"][..], "--pi-leaf-id needs a value"),
        (&["last", "--host", "pi", "--pi-leaf-id", "--print"][..], "--pi-leaf-id needs a value"),
        (
            &["last", "--host", "pi", "--pi-leaf-id", "leaf", "--pi-leaf-id", "other"][..],
            "may be used only once",
        ),
        (&["last", "--host", "pi", "--pi-leaf-id", "leaf", "--stdin"][..], "cannot be used with --stdin"),
    ] {
        let out = bin().args(args).output().expect("runs");
        assert!(!out.status.success(), "{args:?} should be rejected");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(error),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn pi_leaf_flag_requires_an_explicit_transcript_and_session_id() {
    let transcript = fixtures().join("pi.jsonl");
    for (args, error) in [
        (
            vec!["last", "--host", "pi", "--session-id", PI_SESSION_ID, "--pi-leaf-id", "leaf", "--print"],
            "--pi-leaf-id needs --session",
        ),
        (
            vec![
                "last",
                "--host",
                "pi",
                "--session",
                transcript.to_str().expect("path"),
                "--pi-leaf-id",
                "leaf",
                "--print",
            ],
            "--pi-leaf-id needs --session-id",
        ),
    ] {
        let out = bin().args(args).output().expect("runs");
        assert!(out.status.success(), "--print keeps its exit-zero contract");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(error),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
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

/// No session id from Herdr: the pane's cwd selects the session under `$HOME/.commandcode`.
#[cfg(unix)]
#[test]
fn command_code_reads_the_session_started_in_the_agents_cwd() {
    let home = temp_dir("cmd home");
    let projects = home.join(".commandcode/projects");
    std::fs::create_dir_all(&projects).expect("projects");
    let source = fixtures().join("commandcode/users-me-my-proj");
    let target = projects.join("users-me-my-proj");
    std::fs::create_dir_all(&target).expect("project");
    let name = "0198f0a1-7c2e-7b10-9a4d-3c5e8f1a2b3c.jsonl";
    std::fs::copy(source.join(name), target.join(name)).expect("copy");
    let out = bin()
        .env("HOME", &home)
        .env("PLANNOTATOR_TUI_CWD", "/Users/me/MyProj")
        .env_remove("PLANNOTATOR_TUI_HOST")
        .args(["last", "--host", "cmd", "--print"])
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        "# Done\n\nassistant text 2, longer than the viewport\nsecond block",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let elsewhere = bin()
        .env("HOME", &home)
        .env("PLANNOTATOR_TUI_CWD", "/Users/me/other")
        .args(["last", "--host", "cmd", "--print"])
        .output()
        .expect("runs");
    assert!(elsewhere.status.success(), "exit 0 is the contract");
    assert_eq!(String::from_utf8_lossy(&elsewhere.stdout), "");
    assert!(String::from_utf8_lossy(&elsewhere.stderr).contains("no Command Code session"));
    std::fs::remove_dir_all(&home).expect("cleanup");
}

/// Herdr's conversation id reads its transcript directly; without one, the conversation the
/// prompt history ties to the pane's cwd is read.
#[test]
fn antigravity_reads_the_conversation_herdr_names_or_the_one_started_in_the_cwd() {
    let id = "4f1c9e2a-7b3d-4c8e-9a10-2b6d8e0f1a3c";
    let app = temp_dir("agy");
    let relative = format!("brain/{id}/.system_generated/logs/transcript_full.jsonl");
    let target = app.join(&relative);
    std::fs::create_dir_all(target.parent().expect("logs")).expect("logs dir");
    std::fs::copy(fixtures().join("antigravity").join(&relative), &target).expect("copy");
    let newest = "Done: both settings are applied in `retroarch.cfg`.";
    let by_id = bin()
        .env("ANTIGRAVITY_CLI_HOME", &app)
        .env("PLANNOTATOR_TUI_CWD", "/w/unrelated")
        .args(["last", "--host", "agy", "--session-id", id, "--print"])
        .output()
        .expect("runs");
    assert_eq!(
        String::from_utf8_lossy(&by_id.stdout).trim_end(),
        newest,
        "{}",
        String::from_utf8_lossy(&by_id.stderr)
    );
    let unvouched = bin()
        .env("ANTIGRAVITY_CLI_HOME", &app)
        .env("PLANNOTATOR_TUI_CWD", "/w/proj")
        .args(["last", "--host", "agy", "--print"])
        .output()
        .expect("runs");
    assert!(unvouched.status.success(), "exit 0 is the contract");
    assert_eq!(String::from_utf8_lossy(&unvouched.stdout), "");
    assert!(
        String::from_utf8_lossy(&unvouched.stderr).contains("no Antigravity conversation started in /w/proj")
    );
    std::fs::write(
        app.join("history.jsonl"),
        format!("{{\"display\":\"Apply it.\",\"timestamp\":1,\"workspace\":\"/w/proj\",\"conversationId\":\"{id}\"}}\n"),
    )
    .expect("history");
    let by_cwd = bin()
        .env("ANTIGRAVITY_CLI_HOME", &app)
        .env("PLANNOTATOR_TUI_CWD", "/w/proj")
        .args(["last", "--host", "agy", "--print"])
        .output()
        .expect("runs");
    assert_eq!(
        String::from_utf8_lossy(&by_cwd.stdout).trim_end(),
        newest,
        "{}",
        String::from_utf8_lossy(&by_cwd.stderr)
    );
    std::fs::remove_dir_all(&app).expect("cleanup");
}

/// Run from a shell `agy` started, its exported conversation id picks the transcript exactly,
/// with no host named and a newer conversation in the same folder.
#[test]
fn inside_an_agy_shell_the_exported_conversation_id_is_read() {
    let id = "4f1c9e2a-7b3d-4c8e-9a10-2b6d8e0f1a3c";
    let newer = "22222222-2222-4222-8222-222222222222";
    let app = temp_dir("agy shell");
    let relative = |id: &str| format!("brain/{id}/.system_generated/logs/transcript_full.jsonl");
    let target = app.join(relative(id));
    std::fs::create_dir_all(target.parent().expect("logs")).expect("logs dir");
    std::fs::copy(fixtures().join("antigravity").join(relative(id)), &target).expect("copy");
    let other = app.join(relative(newer));
    std::fs::create_dir_all(other.parent().expect("logs")).expect("logs dir");
    std::fs::write(&other, "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"content\":\"NEWER\"}\n")
        .expect("newer");
    std::fs::write(
        app.join("history.jsonl"),
        format!(
            "{{\"workspace\":\"/w/proj\",\"conversationId\":\"{id}\"}}\n{{\"workspace\":\"/w/proj\",\"conversationId\":\"{newer}\"}}\n"
        ),
    )
    .expect("history");
    let run = |conversation: &str| {
        bin()
            .env_remove("PLANNOTATOR_TUI_HOST")
            .env_remove("CODEX_THREAD_ID")
            .env_remove("COPILOT_CLI")
            .env_remove("ANTIGRAVITY_CLI_HOME")
            .env("ANTIGRAVITY_APP_DATA_DIR", &app)
            .env("ANTIGRAVITY_AGENT", "1")
            .env("ANTIGRAVITY_CONVERSATION_ID", conversation)
            .env("PLANNOTATOR_TUI_CWD", "/w/proj")
            .args(["last", "--print"])
            .output()
            .expect("runs")
    };
    let exact = run(id);
    assert_eq!(
        String::from_utf8_lossy(&exact.stdout).trim_end(),
        "Done: both settings are applied in `retroarch.cfg`.",
        "{}",
        String::from_utf8_lossy(&exact.stderr)
    );
    let missing = run("33333333-3333-4333-8333-333333333333");
    assert_eq!(String::from_utf8_lossy(&missing.stdout), "", "an unknown id never falls back to the folder");
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no Antigravity conversation 33333333"));
    std::fs::remove_dir_all(&app).expect("cleanup");
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

#[test]
fn herdr_open_rejects_newest_by_name() {
    let out = bin().args(["herdr", "open", "--newest"]).output().expect("runs");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("only for `herdr last`"), "{stderr}");
    assert!(!stderr.contains("unknown flag"), "{stderr}");

    let out = bin().args(["herdr", "open", "--bogus"]).output().expect("runs");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown flag --bogus"));
}
