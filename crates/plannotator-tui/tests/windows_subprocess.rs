//! Native Windows proof that Herdr commands cross `CreateProcess` as separate argv values.

#![cfg(windows)]
#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
const NEWER_ID: &str = "22222222-2222-4222-8222-222222222222";

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
}

fn temp_root(tag: &str) -> PathBuf {
    let base = std::env::var_os("RUNNER_TEMP").map_or_else(std::env::temp_dir, PathBuf::from);
    let root = base.join(format!("plannotator Windows {tag} ü-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root");
    root
}

fn compile_fake(root: &Path) -> PathBuf {
    let dir = root.join("fake herdr");
    std::fs::create_dir_all(&dir).expect("fake dir");
    let executable = dir.join("herdr.exe");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake-herdr.rs");
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("run rustc");
    assert!(output.status.success(), "rustc failed: {}", String::from_utf8_lossy(&output.stderr));
    std::fs::copy(&executable, dir.join("ps.exe")).expect("ps recorder");
    executable
}

fn calls(fake: &Path) -> Vec<Value> {
    let log = fake.parent().expect("fake dir").join("calls.jsonl");
    std::fs::read_to_string(log)
        .expect("call log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON call"))
        .collect()
}

fn context(cwd: &Path, clicked_url: Option<String>) -> String {
    json!({
        "focused_pane_id": "w1:p1",
        "focused_pane_agent": "codex",
        "focused_pane_cwd": cwd,
        "workspace_cwd": cwd,
        "clicked_url": clicked_url,
    })
    .to_string()
}

fn file_url(path: &Path) -> String {
    format!("file:///{}", path.display().to_string().replace('\\', "/").replace(' ', "%20"))
}

#[test]
fn clicked_file_and_exact_session_launches_preserve_argv_without_process_info() {
    let root = temp_root("launcher");
    let fake = compile_fake(&root);
    let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    let clicked_root =
        PathBuf::from(format!(r"{system_drive}\plannotator clicked proof ü-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&clicked_root);
    std::fs::create_dir_all(&clicked_root).expect("clicked root");
    let clicked = clicked_root.join("plan with spaces.md");
    std::fs::write(&clicked, "# plan\n").expect("clicked file");
    let plugin_root = root.join("plugin root with spaces ü");
    std::fs::create_dir_all(&plugin_root).expect("plugin root");

    let open = bin()
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", &fake)
        .env("HERDR_PLUGIN_ID", "annotate")
        .env("HERDR_PLUGIN_ROOT", &plugin_root)
        .env("HERDR_PLUGIN_CONTEXT_JSON", context(&clicked_root, Some(file_url(&clicked))))
        .args(["herdr", "open"])
        .output()
        .expect("open launcher");
    assert!(open.status.success(), "{}", String::from_utf8_lossy(&open.stderr));

    let last = bin()
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", &fake)
        .env("HERDR_PLUGIN_ID", "annotate")
        .env("HERDR_PLUGIN_ROOT", &plugin_root)
        .env("HERDR_PLUGIN_CONTEXT_JSON", context(&clicked_root, None))
        .args(["herdr", "last"])
        .output()
        .expect("last launcher");
    assert!(last.status.success(), "{}", String::from_utf8_lossy(&last.stderr));

    let calls = calls(&fake);
    assert_eq!(
        calls[0]["argv"],
        json!([
            "plugin",
            "pane",
            "open",
            "--plugin",
            "annotate",
            "--entrypoint",
            "doc",
            "--placement",
            "overlay",
            "--focus",
            "--cwd",
            clicked_root.display().to_string(),
            "--env",
            format!("PLANNOTATOR_TUI_FILE={}", clicked.display()),
            "--env",
            "PLANNOTATOR_TUI_DELIVER_TO=w1:p1",
            "--env",
            "PLANNOTATOR_TUI_DELIVER_AGENT=codex",
        ])
    );
    assert_eq!(calls[1]["argv"], json!(["agent", "get", "w1:p1"]));
    assert_eq!(
        calls[2]["argv"],
        json!([
            "plugin",
            "pane",
            "open",
            "--plugin",
            "annotate",
            "--entrypoint",
            "doc",
            "--placement",
            "overlay",
            "--focus",
            "--cwd",
            clicked_root.display().to_string(),
            "--env",
            "PLANNOTATOR_TUI_HOST=codex",
            "--env",
            format!("PLANNOTATOR_TUI_CWD={}", clicked_root.display()),
            "--env",
            format!("PLANNOTATOR_TUI_SESSION_ID={SESSION_ID}"),
            "--env",
            "PLANNOTATOR_TUI_DELIVER_TO=w1:p1",
            "--env",
            "PLANNOTATOR_TUI_DELIVER_AGENT=codex",
        ])
    );
    assert!(
        calls.iter().all(|call| {
            call["argv"].as_array().is_none_or(|args| !args.iter().any(|arg| arg == "process-info"))
        }),
        "exact identity must not call pane process-info: {calls:?}"
    );
    assert!(
        calls[2]["argv"]
            .as_array()
            .expect("argv")
            .iter()
            .all(|arg| !arg.as_str().is_some_and(|arg| arg.starts_with("PLANNOTATOR_TUI_MESSAGE_PID=")))
    );

    std::fs::remove_dir_all(clicked_root).expect("clicked cleanup");
    std::fs::remove_dir_all(root).expect("cleanup");
}

fn codex_rollout(home: &Path, day: &str, id: &str, text: &str) {
    let file =
        home.join("sessions/2026/08").join(day).join(format!("rollout-2026-08-{day}T10-00-00-{id}.jsonl"));
    std::fs::create_dir_all(file.parent().expect("parent")).expect("sessions");
    std::fs::write(
        file,
        format!(
            "{{\"timestamp\":\"2026-08-{day}T10:00:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}\n{{\"timestamp\":\"2026-08-{day}T10:00:00Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"id\":\"m-{id}\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{text}\"}}]}}}}\n"
        ),
    )
    .expect("rollout");
}

#[test]
fn exact_print_uses_the_named_rollout_and_never_attempts_ps() {
    let root = temp_root("last print");
    let fake = compile_fake(&root);
    let home = root.join("Codex Home ü");
    codex_rollout(&home, "30", SESSION_ID, "NAMED WINDOWS SESSION");
    codex_rollout(&home, "31", NEWER_ID, "NEWER UNRELATED SESSION");
    let fake_dir = fake.parent().expect("fake dir");
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let search_path = std::env::join_paths(
        std::iter::once(fake_dir.to_path_buf()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("PATH");
    let output = bin()
        .env("CODEX_HOME", &home)
        .env("PATH", search_path)
        .env_remove("CODEX_THREAD_ID")
        .args(["last", "--host", "codex", "--session-id", SESSION_ID, "--print"])
        .output()
        .expect("last print");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "NAMED WINDOWS SESSION");
    let log = fake_dir.join("calls.jsonl");
    if log.exists() {
        let calls = calls(&fake);
        assert!(calls.iter().all(|call| call["program"] != "ps"), "exact last attempted ps: {calls:?}");
    }
    std::fs::remove_dir_all(root).expect("cleanup");
}
