//! `plannotator-tui herdr terminal` through the real binary against a recording fake `herdr`.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const OUTPUT: &str = "\u{1b}[32m$ cargo test\u{1b}[0m   \r\ntest result: ok. 3 passed\n\n\n";

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("plannotator terminal {tag} ü-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root");
    root
}

/// The fake compiled into `root/fake herdr/`, where it also reads fixtures and logs calls.
fn compile_fake(root: &Path) -> PathBuf {
    let dir = root.join("fake herdr");
    std::fs::create_dir_all(&dir).expect("fake dir");
    let executable = dir.join(format!("herdr{}", std::env::consts::EXE_SUFFIX));
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake-herdr.rs");
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("run rustc");
    assert!(output.status.success(), "rustc failed: {}", String::from_utf8_lossy(&output.stderr));
    executable
}

fn calls(fake: &Path) -> Vec<Value> {
    let log = fake.parent().expect("fake dir").join("calls.jsonl");
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON call"))
        .collect()
}

/// Run `herdr terminal` as a plugin action would, focused on `w1:p1` where codex runs.
fn terminal(root: &Path, fake: &Path, extra: &[&str]) -> Output {
    let context =
        json!({"focused_pane_id": "w1:p1", "focused_pane_agent": "codex", "focused_pane_cwd": root});
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(["herdr", "terminal"])
        .args(extra)
        .current_dir(root)
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", fake)
        .env("HERDR_PLUGIN_CONTEXT_JSON", context.to_string())
        .env("HERDR_PANE_ID", "w1:p9")
        .env("PLANNOTATOR_TUI_CONFIG", root.join("absent.toml"))
        .env_remove("PLANNOTATOR_TUI_PLACEMENT")
        .output()
        .expect("runs")
}

#[test]
fn print_shows_the_focused_panes_cleaned_output_as_one_code_block() {
    let root = temp_root("print");
    let fake = compile_fake(&root);
    std::fs::write(fake.with_file_name("pane-read.txt"), OUTPUT).expect("fixture");

    let out = terminal(&root, &fake, &["--lines", "40", "--print"]);

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "```text\n$ cargo test\ntest result: ok. 3 passed\n```\n"
    );
    let calls = calls(&fake);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        calls[0]["argv"],
        json!(["pane", "read", "w1:p1", "--source", "recent-unwrapped", "--lines", "40", "--format", "text"])
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_review_pane_opens_on_the_output_and_sends_to_the_agent_that_produced_it() {
    let root = temp_root("launch");
    let fake = compile_fake(&root);
    std::fs::write(fake.with_file_name("pane-read.txt"), OUTPUT).expect("fixture");

    let out = terminal(&root, &fake, &[]);

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let calls = calls(&fake);
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0]["argv"][6], "200", "the default line count");
    let open: Vec<&str> =
        calls[1]["argv"].as_array().expect("argv").iter().filter_map(Value::as_str).collect();
    assert_eq!(open.get(..3), Some(&["plugin", "pane", "open"][..]));
    for env in [
        "PLANNOTATOR_TUI_TERMINAL_PANE=w1:p1",
        "PLANNOTATOR_TUI_TERMINAL_LINES=200",
        "PLANNOTATOR_TUI_DELIVER_TO=w1:p1",
        "PLANNOTATOR_TUI_DELIVER_AGENT=codex",
    ] {
        assert!(open.contains(&env), "{env} missing from {open:?}");
    }
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_pane_with_nothing_on_it_fails_without_opening_a_review() {
    let root = temp_root("empty");
    let fake = compile_fake(&root);
    std::fs::write(fake.with_file_name("pane-read.txt"), "\n  \n\u{1b}[0m\n").expect("fixture");

    let out = terminal(&root, &fake, &[]);

    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pane w1:p1 has no output to review"));
    assert_eq!(calls(&fake).len(), 1, "no pane opened");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_failed_read_reports_herdrs_error_without_opening_a_review() {
    let root = temp_root("error");
    let fake = compile_fake(&root);
    std::fs::write(
        fake.with_file_name("pane-read.error"),
        r#"{"error":{"code":"pane_not_found","message":"pane w1:p1 not found"}}"#,
    )
    .expect("fixture");

    let out = terminal(&root, &fake, &[]);

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("herdr pane read w1:p1") && stderr.contains("pane_not_found"), "{stderr}");
    assert_eq!(calls(&fake).len(), 1, "no pane opened");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn an_action_without_a_focused_pane_reads_nothing() {
    let root = temp_root("unfocused");
    let fake = compile_fake(&root);

    let out = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(["herdr", "terminal", "--print"])
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", &fake)
        .env("HERDR_PLUGIN_CONTEXT_JSON", r#"{"workspace_id":"w1"}"#)
        .env("HERDR_PANE_ID", "w1:p9")
        .output()
        .expect("runs");

    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no focused pane to read"));
    assert!(calls(&fake).is_empty());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn line_counts_must_be_positive_numbers() {
    let out = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(["herdr", "terminal", "--lines", "0"])
        .output()
        .expect("runs");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--lines takes a number from 1"));
}
