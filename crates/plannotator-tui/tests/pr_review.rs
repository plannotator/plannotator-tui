//! `plannotator-tui review <PR>` and `herdr review --pr`, through the real binary against a
//! simulated GitHub: a local bare repository with `refs/pull/7/head`, a shell script on
//! `PATH` standing in for `gh`, and `url.<path>.insteadOf` sending `https://github.com/o/app`
//! to the bare repository. No network.

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

mod support;

const URL: &str = "https://github.com/o/app";

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("plannotator pr review {tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root");
    root.canonicalize().expect("real path")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// `upstream.git` with `main` and PR 7, the fake `gh` in `root/fake gh/`, and a clone `app`
/// of `https://github.com/o/app` (rewritten to the bare repository) on its own branch.
fn world(root: &Path) -> PathBuf {
    git(root, &["init", "-q", "--bare", "upstream.git"]);
    git(root, &["init", "-q", "-b", "main", "work"]);
    let work = root.join("work");
    git(&work, &["remote", "add", "origin", &root.join("upstream.git").display().to_string()]);
    std::fs::write(work.join("lib.rs"), "fn a() {}\n").expect("write");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "init"]);
    git(&work, &["push", "-q", "origin", "main"]);
    std::fs::write(work.join("lib.rs"), "fn a() {}\nfn b() {}\n").expect("write");
    git(&work, &["commit", "-qam", "b"]);
    git(&work, &["push", "-q", "origin", "HEAD:refs/pull/7/head"]);
    let view = json!({
        "number": 7, "url": format!("{URL}/pull/7"), "title": "add b",
        "baseRefName": "main", "headRefName": "add-b",
        "baseRefOid": git(&work, &["rev-parse", "HEAD~1"]), "headRefOid": git(&work, &["rev-parse", "HEAD"]),
    });
    std::fs::write(root.join("view.json"), view.to_string()).expect("view.json");
    let fake = root.join("fake gh");
    std::fs::create_dir_all(&fake).expect("fake dir");
    let script = format!(
        "#!/bin/sh\necho \"$PWD $@\" >> '{}'\ncat '{}'\n",
        root.join("gh.log").display(),
        root.join("view.json").display()
    );
    support::write_script(&fake.join("gh"), &script);
    // Git config every run of the binary reads: GitHub is the bare repository.
    let config = format!("[url \"{}\"]\n\tinsteadOf = {URL}.git\n", root.join("upstream.git").display());
    std::fs::write(root.join("gitconfig"), config).expect("gitconfig");
    let clone = root.join("app");
    git(root, &["init", "-q", "-b", "mine", "app"]);
    git(&clone, &["remote", "add", "origin", &format!("{URL}.git")]);
    clone
}

/// The binary in `cwd`, with the fake `gh` first on `PATH` (unless `path` replaces it) and
/// every home, data and config folder in `root`.
fn plannotator(root: &Path, cwd: &Path, args: &[&str], path: Option<&Path>) -> Output {
    let path = match path {
        Some(path) => path.display().to_string(),
        None => format!("{}:{}", root.join("fake gh").display(), std::env::var("PATH").unwrap_or_default()),
    };
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .env("PATH", path)
        .env("HOME", root)
        .env("PLANNOTATOR_DATA_DIR", root.join("data"))
        .env("PLANNOTATOR_TUI_CONFIG", root.join("absent.toml"))
        .env("PLANNOTATOR_TUI_DELIVER_CMD", format!("cat > '{}'", root.join("delivered.md").display()))
        .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("HERDR_ENV")
        .env_remove("PLANNOTATOR_TUI_REVIEW")
        .output()
        .expect("runs")
}

#[test]
fn a_pr_number_is_fetched_into_the_clone_before_the_screen_opens() {
    let root = temp_root("number");
    let clone = world(&root);
    let out = plannotator(&root, &clone, &["review", "#7"], None);
    // Captured, then refused only because stdout is no terminal here.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("stdout is not one"), "{stderr}");
    let head = git(&root.join("work"), &["rev-parse", "HEAD"]);
    assert_eq!(git(&clone, &["rev-parse", "refs/plannotator-tui/pull/7/head"]), head);
    assert_eq!(git(&clone, &["symbolic-ref", "HEAD"]), "refs/heads/mine");
    let log = std::fs::read_to_string(root.join("gh.log")).expect("gh ran");
    assert!(log.starts_with(&format!("{} pr view 7 --json ", clone.display())), "{log}");
    assert!(!root.join("data/clients/plannotator-tui/pulls").exists());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_pr_link_outside_a_clone_is_fetched_into_the_cache() {
    let root = temp_root("link");
    world(&root);
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).expect("plain");
    let out = plannotator(&root, &plain, &["review", &format!("{URL}/pull/7")], None);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("stdout is not one"), "{stderr}");
    let cache = root.join("data/clients/plannotator-tui/pulls/github.com/o/app.git");
    let head = git(&root.join("work"), &["rev-parse", "HEAD"]);
    assert_eq!(git(&cache, &["rev-parse", "refs/plannotator-tui/pull/7/head"]), head);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn errors_name_what_is_missing() {
    let root = temp_root("errors");
    let clone = world(&root);
    // No gh on PATH: only git.
    let only_git = root.join("only git");
    std::fs::create_dir_all(&only_git).expect("dir");
    let real_git = Command::new("sh").args(["-c", "command -v git"]).output().expect("sh");
    let real_git = String::from_utf8_lossy(&real_git.stdout).trim().to_owned();
    std::os::unix::fs::symlink(&real_git, only_git.join("git")).expect("symlink");
    let out = plannotator(&root, &clone, &["review", "7"], Some(&only_git));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(stderr.contains("needs the GitHub CLI (gh)") && stderr.contains("gh auth login"), "{stderr}");
    // A number outside any clone.
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).expect("plain");
    let out = plannotator(&root, &plain, &["review", "7"], None);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("pass the PR's link instead"), "{stderr}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

/// The fake herdr, compiled into `root/fake herdr/`, where it logs its calls.
fn compile_fake_herdr(root: &Path) -> PathBuf {
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

#[test]
fn herdr_review_pr_opens_the_pane_on_the_prs_link() {
    let root = temp_root("herdr");
    let clone = world(&root);
    let fake = compile_fake_herdr(&root);
    let context =
        json!({"focused_pane_id": "w1:p1", "focused_pane_agent": "codex", "focused_pane_cwd": clone});
    let out = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(["herdr", "review", "--pr", "7"])
        .current_dir(&root)
        .stdin(Stdio::null())
        .env(
            "PATH",
            format!("{}:{}", root.join("fake gh").display(), std::env::var("PATH").unwrap_or_default()),
        )
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", &fake)
        .env("HERDR_PLUGIN_CONTEXT_JSON", context.to_string())
        .env("HERDR_PANE_ID", "w1:p9")
        .env("HOME", &root)
        .env("PLANNOTATOR_DATA_DIR", root.join("data"))
        .env("PLANNOTATOR_TUI_CONFIG", root.join("absent.toml"))
        .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("PLANNOTATOR_TUI_PLACEMENT")
        .env_remove("PLANNOTATOR_TUI_FILE")
        .env_remove("PLANNOTATOR_TUI_REVIEW")
        .output()
        .expect("runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let log = std::fs::read_to_string(fake.parent().expect("dir").join("calls.jsonl")).expect("herdr ran");
    let call: Value = serde_json::from_str(log.lines().next().expect("a call")).expect("JSON");
    let argv: Vec<&str> = call["argv"].as_array().expect("argv").iter().filter_map(Value::as_str).collect();
    let envs: Vec<&str> = argv.windows(2).filter(|pair| pair[0] == "--env").map(|pair| pair[1]).collect();
    assert_eq!(
        envs[..2],
        [
            format!("PLANNOTATOR_TUI_REVIEW=pr:{URL}/pull/7"),
            format!("PLANNOTATOR_TUI_REPO={}", clone.display())
        ]
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

/// A fake `rex` in `root/fake rex/`: the agent in the block is claude, pid 4242, in the clone.
/// It logs its arguments to `rex.log` there, and an `api call`'s stdin after them.
fn fake_rex(root: &Path, clone: &Path) -> PathBuf {
    let dir = root.join("fake rex");
    std::fs::create_dir_all(&dir).expect("fake dir");
    let log = dir.join("rex.log");
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{log}'
case "$1 $2" in
  "do com.superlogical.terminal.process") printf '{{"foreground":{{"pid":4242,"name":"claude","argv0":"claude","cwd":"{cwd}"}}}}' ;;
  "do com.superlogical.terminal.program_status") printf '{{"records":[{{"state":"idle","app":"claude-code","owner":{{"pid":4242}}}}]}}' ;;
  "api call") cat >> '{log}'; printf '\n' >> '{log}'; printf '{{}}' ;;
  *) exit 1 ;;
esac
"#,
        log = log.display(),
        cwd = clone.display()
    );
    support::write_script(&dir.join("rex"), &script);
    dir
}

/// Run `rex review 7` against the fake Rex with `extra` arguments; the calls it logged.
fn rex_review(root: &Path, fake: &Path, extra: &[&str]) -> Vec<String> {
    let _ = std::fs::remove_file(fake.join("rex.log"));
    let out = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(["rex", "review", "7", "--block", "block:b"])
        .args(extra)
        .current_dir(root)
        .stdin(Stdio::null())
        .env(
            "PATH",
            format!("{}:{}", root.join("fake gh").display(), std::env::var("PATH").unwrap_or_default()),
        )
        .env("REX_BIN_DIR", fake)
        .env("REX_SESSION", "session:s")
        .env("HOME", root)
        .env("PLANNOTATOR_DATA_DIR", root.join("data"))
        .env("PLANNOTATOR_TUI_CONFIG", root.join("absent.toml"))
        .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("REX_BLOCK")
        .env_remove("PLANNOTATOR_TUI_PLACEMENT")
        .env_remove("PLANNOTATOR_TUI_REVIEW")
        .output()
        .expect("runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let log = std::fs::read_to_string(fake.join("rex.log")).expect("rex ran");
    log.lines().map(str::to_owned).collect()
}

/// The JSON arguments logged after the `method` call, and the command they run.
fn placed(lines: &[String], method: &str) -> (Value, Vec<String>) {
    let call = format!("api call {method} - -s session:s");
    let at = lines.iter().position(|l| *l == call).expect("the placement call");
    let args: Value = serde_json::from_str(&lines[at + 1]).expect("JSON arguments");
    let command = args
        .pointer("/layout/block/options/command")
        .and_then(Value::as_array)
        .expect("command")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    (args, command)
}

#[test]
fn rex_review_pr_opens_in_a_tab_of_its_own_and_sends_back_to_the_agent() {
    let root = temp_root("rex");
    let clone = world(&root);
    let fake = fake_rex(&root, &clone);
    let (tab, command) = placed(&rex_review(&root, &fake, &[]), "session.new_window");
    assert_eq!(tab["window_label"], "Annotate · claude");
    // The agent that opened it is the Send target, and the pane it shows again at the end; the
    // picker (`P`) offers the rest.
    assert_eq!(
        command[1..5],
        [
            "PLANNOTATOR_TUI_REX_RETURN=block:b",
            "PLANNOTATOR_TUI_REX_BLOCK=block:b",
            "PLANNOTATOR_TUI_DELIVER_AGENT=claude",
            "PLANNOTATOR_TUI_REX_PID=4242"
        ]
    );
    let tail = &command[command.len() - 3..];
    assert_eq!(tail[0], "review");
    assert!(tail[1].contains(&format!("{URL}/pull/7")), "{tail:?}");
    assert_eq!(tail[2], clone.display().to_string());

    // `--placement split` still opens beside the agent, and needs nothing shown again.
    let (split, command) = placed(&rex_review(&root, &fake, &["--placement", "split"]), "session.new_split");
    assert_eq!(split["anchor_id"], "block:b");
    assert_eq!(command[1], "PLANNOTATOR_TUI_REX_BLOCK=block:b");
    assert!(!command.iter().any(|arg| arg.starts_with("PLANNOTATOR_TUI_REX_RETURN=")), "{command:?}");
    std::fs::remove_dir_all(root).expect("cleanup");
}
