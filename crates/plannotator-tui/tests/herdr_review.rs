//! `plannotator-tui herdr review` and the pane it opens, through the real binary against a
//! recording fake `herdr`, on real temporary git repositories.

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("plannotator herdr review {tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root");
    root.canonicalize().expect("real path")
}

/// The fake compiled into `root/fake herdr/`, where it logs its calls.
fn compile_fake(root: &Path) -> PathBuf {
    let dir = root.join("fake herdr");
    std::fs::create_dir_all(&dir).expect("fake dir");
    let executable = dir.join("herdr");
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

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A repository `app` under `root` with one commit and a `src/` folder.
fn repo(root: &Path) -> PathBuf {
    let repo = root.join("app");
    std::fs::create_dir_all(repo.join("src")).expect("repo dir");
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("src/lib.rs"), "fn a() {}\n").expect("file");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    repo
}

/// Run the binary as a plugin action would, focused on `w1:p1` (codex) in `folder`.
fn plannotator(root: &Path, fake: &Path, folder: &Path, args: &[&str]) -> Output {
    let context =
        json!({"focused_pane_id": "w1:p1", "focused_pane_agent": "codex", "focused_pane_cwd": folder});
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", fake)
        .env("HERDR_PLUGIN_CONTEXT_JSON", context.to_string())
        .env("HERDR_PANE_ID", "w1:p9")
        .env("HOME", root)
        .env("PLANNOTATOR_DATA_DIR", root.join("data"))
        .env("PLANNOTATOR_TUI_CONFIG", root.join("absent.toml"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("PLANNOTATOR_TUI_PLACEMENT")
        .env_remove("PLANNOTATOR_TUI_FILE")
        .env_remove("PLANNOTATOR_TUI_REVIEW")
        .output()
        .expect("runs")
}

#[test]
fn the_focused_panes_repository_opens_in_the_doc_pane_with_its_scope() {
    let root = temp_root("opens");
    let fake = compile_fake(&root);
    let repo = repo(&root);
    std::fs::write(repo.join("src/lib.rs"), "fn a() {}\nfn b() {}\n").expect("change");
    git(&repo, &["add", "-A"]);

    let out = plannotator(&root, &fake, &repo.join("src"), &["herdr", "review", "--staged"]);

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let calls = calls(&fake);
    assert_eq!(calls.len(), 1, "{calls:?}");
    let argv: Vec<&str> =
        calls[0]["argv"].as_array().expect("argv").iter().filter_map(Value::as_str).collect();
    assert_eq!(argv[..7], ["plugin", "pane", "open", "--plugin", "annotate", "--entrypoint", "doc"]);
    let root_dir = repo.display().to_string();
    assert!(argv.windows(2).any(|pair| pair == ["--cwd", root_dir.as_str()]), "{argv:?}");
    let envs: Vec<&str> = argv.windows(2).filter(|pair| pair[0] == "--env").map(|pair| pair[1]).collect();
    assert_eq!(
        envs,
        [
            "PLANNOTATOR_TUI_REVIEW=staged",
            &format!("PLANNOTATOR_TUI_REPO={root_dir}"),
            "PLANNOTATOR_TUI_DELIVER_TO=w1:p1",
            "PLANNOTATOR_TUI_DELIVER_AGENT=codex",
        ]
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn no_pane_opens_for_a_clean_repository_or_a_folder_that_is_none() {
    let root = temp_root("refuses");
    let fake = compile_fake(&root);
    let repo = repo(&root);

    let out = plannotator(&root, &fake, &repo, &["herdr", "review"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("No uncommitted changes in app"), "{stderr}");

    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).expect("plain folder");
    let out = plannotator(&root, &fake, &plain, &["herdr", "review", "--commit", "HEAD"]);
    assert!(!out.status.success());
    assert_ne!(String::from_utf8_lossy(&out.stderr).trim(), "", "says why");

    let out = plannotator(&root, &fake, &repo, &["herdr", "review", "src"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("takes no folder"));

    assert_eq!(calls(&fake), [] as [Value; 0], "Herdr was never asked for a pane");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_pane_opens_the_review_before_any_file_it_was_also_given() {
    let root = temp_root("pane");
    let fake = compile_fake(&root);
    let repo = repo(&root);
    std::fs::write(root.join("notes.md"), "# notes\n").expect("doc");

    // A clean repository fails where the person can read it, which shows the review was
    // chosen: the file alone would have opened.
    let out = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(["herdr", "pane"])
        .stdin(Stdio::null())
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", &fake)
        .env("HOME", &root)
        .env("PLANNOTATOR_DATA_DIR", root.join("data"))
        .env("PLANNOTATOR_TUI_CONFIG", root.join("absent.toml"))
        .env("PLANNOTATOR_TUI_REVIEW", "uncommitted")
        .env("PLANNOTATOR_TUI_REPO", &repo)
        .env("PLANNOTATOR_TUI_FILE", root.join("notes.md"))
        .env_remove("HERDR_PLUGIN_CONTEXT_JSON")
        .env_remove("PLANNOTATOR_TUI_TERMINAL_PANE")
        .env_remove("PLANNOTATOR_TUI_MESSAGE_PID")
        .env_remove("PLANNOTATOR_TUI_SESSION")
        .env_remove("PLANNOTATOR_TUI_SESSION_ID")
        .output()
        .expect("runs");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("No uncommitted changes in app") && stderr.contains("press enter"), "{stderr}");
    std::fs::remove_dir_all(root).expect("cleanup");
}
