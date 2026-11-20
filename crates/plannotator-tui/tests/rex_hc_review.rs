//! `plannotator-tui rex review` and `hc review`, and the panes they open, through the real
//! binary against recording fake `rex`, `hc` and `herdr-app` scripts, on real temporary git
//! repositories.

#![cfg(unix)]
#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

mod support;

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("plannotator rex hc review {tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp root");
    root.canonicalize().expect("real path")
}

/// Fake `rex`, `hc` and `herdr-app` in `root/fake/`. Each logs its arguments, one call per
/// line, to `calls.log` there; `rex api call` logs its stdin as the next line. The agent in
/// the block or terminal is codex, pid 4242, in `$FAKE_CWD`.
fn fakes(root: &Path) -> PathBuf {
    let dir = root.join("fake");
    std::fs::create_dir_all(&dir).expect("fake dir");
    let rex = r#"#!/bin/sh
printf 'rex %s\n' "$*" >> "$FAKE_LOG"
case "$1 $2" in
  "do com.superlogical.terminal.process")
    printf '{"foreground":{"pid":4242,"name":"codex","argv0":"codex","cwd":"%s"}}' "$FAKE_CWD" ;;
  "do com.superlogical.terminal.program_status") printf '{"records":[]}' ;;
  "api call") cat >> "$FAKE_LOG"; printf '\n' >> "$FAKE_LOG"; printf '{}' ;;
  *) exit 1 ;;
esac
"#;
    let hc = r#"#!/bin/sh
printf 'hc %s\n' "$*" >> "$FAKE_LOG"
case "$1" in
  get) printf '{"foreground":{"pid":4242,"name":"codex","argv":["codex"],"cwd":"%s"}}' "$FAKE_CWD" ;;
  run) printf '{"address":"t9"}' ;;
  keys) ;;
  *) exit 1 ;;
esac
"#;
    let app = "#!/bin/sh\nprintf 'herdr-app %s\\n' \"$*\" >> \"$FAKE_LOG\"\n";
    for (name, text) in [("rex", rex), ("hc", hc), ("herdr-app", app)] {
        support::write_script(&dir.join(name), text);
    }
    dir
}

fn calls(fake: &Path) -> Vec<String> {
    std::fs::read_to_string(fake.join("calls.log")).unwrap_or_default().lines().map(str::to_owned).collect()
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

/// Run the binary with the fakes first on `PATH` and the agent working in `folder`.
fn plannotator(root: &Path, fake: &Path, folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .env("PATH", format!("{}:/usr/bin:/bin", fake.display()))
        .env("FAKE_LOG", fake.join("calls.log"))
        .env("FAKE_CWD", folder)
        .env("REX_BIN_DIR", fake)
        .env("REX_SESSION", "session:s")
        .env("PLANNOTATOR_TUI_HC_BIN", fake.join("hc"))
        .env("HOME", root)
        .env("PLANNOTATOR_DATA_DIR", root.join("data"))
        .env("PLANNOTATOR_TUI_CONFIG", root.join("absent.toml"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("REX_BLOCK")
        .env_remove("HC_TERMINAL_ID")
        .env_remove("PLANNOTATOR_TUI_PLACEMENT")
        .output()
        .expect("runs")
}

/// The split's arguments from the `session.new_split` call the fake logged.
fn new_split(calls: &[String]) -> Value {
    let at = calls.iter().position(|c| c == "rex api call session.new_split - -s session:s");
    let body = at.and_then(|at| calls.get(at + 1)).expect("a session.new_split call and its arguments");
    serde_json::from_str(body).expect("JSON arguments")
}

#[test]
fn rex_review_opens_the_blocks_repository_in_a_split_when_asked_wired_to_its_agent() {
    let root = temp_root("rex opens");
    let fake = fakes(&root);
    let repo = repo(&root);
    std::fs::write(repo.join("src/lib.rs"), "fn a() {}\nfn b() {}\n").expect("change");
    git(&repo, &["add", "-A"]);

    let out = plannotator(
        &root,
        &fake,
        &repo.join("src"),
        &["rex", "review", "--staged", "--block", "block:b", "--placement", "split"],
    );

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let split = new_split(&calls(&fake));
    assert_eq!(split.get("anchor_id").and_then(Value::as_str), Some("block:b"));
    let options = split.pointer("/layout/block/options").expect("options");
    let root_dir = repo.display().to_string();
    assert_eq!(options.get("cwd").and_then(Value::as_str), Some(root_dir.as_str()));
    let command: Vec<&str> = options
        .get("command")
        .and_then(Value::as_array)
        .expect("command")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let exe = env!("CARGO_BIN_EXE_plannotator-tui");
    assert_eq!(
        command,
        [
            "/usr/bin/env",
            "PLANNOTATOR_TUI_REX_BLOCK=block:b",
            "PLANNOTATOR_TUI_DELIVER_AGENT=codex",
            "PLANNOTATOR_TUI_REX_PID=4242",
            exe,
            "rex",
            "pane",
            "review",
            "staged",
            &root_dir,
        ]
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn rex_review_opens_nothing_for_a_clean_repository_or_a_folder_that_is_none() {
    let root = temp_root("rex refuses");
    let fake = fakes(&root);
    let repo = repo(&root);

    let out = plannotator(&root, &fake, &repo, &["rex", "review", "--block", "block:b"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("No uncommitted changes in app"), "{stderr}");

    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).expect("plain folder");
    let out = plannotator(&root, &fake, &plain, &["rex", "review", "--block", "block:b"]);
    assert!(!out.status.success());
    assert_ne!(String::from_utf8_lossy(&out.stderr).trim(), "", "says why");

    let out = plannotator(&root, &fake, &repo, &["rex", "review"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("no Rex block"), "needs a block");

    let asked: Vec<String> = calls(&fake).into_iter().filter(|c| c.starts_with("rex api")).collect();
    assert_eq!(asked, [] as [String; 0], "Rex was never asked for a tab or a split");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn hc_review_starts_a_held_terminal_in_the_repository_and_shows_it() {
    let root = temp_root("hc opens");
    let fake = fakes(&root);
    let repo = repo(&root);
    std::fs::write(repo.join("src/lib.rs"), "fn a() {}\nfn b() {}\n").expect("change");

    let out = plannotator(&root, &fake, &repo.join("src"), &["hc", "review", "--terminal", "t7"]);

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let calls = calls(&fake);
    let root_dir = repo.display().to_string();
    let exe = env!("CARGO_BIN_EXE_plannotator-tui");
    let hc = fake.join("hc").display().to_string();
    let run = format!(
        "hc run --bg --json --rm --no-notify --name annotate-t7 --cwd {root_dir} -- env \
         PLANNOTATOR_TUI_HC_TERMINAL=t7 PLANNOTATOR_TUI_DELIVER_AGENT=codex PLANNOTATOR_TUI_HC_PID=4242 \
         PLANNOTATOR_TUI_HC_BIN={hc} {exe} hc pane --held review uncommitted {root_dir}"
    );
    assert!(calls.contains(&run), "{calls:#?}");
    assert!(calls.contains(&"herdr-app do terminal.peek --terminal t9".to_owned()), "{calls:#?}");
    assert!(calls.contains(&"hc keys t9 enter".to_owned()), "{calls:#?}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn hc_review_starts_no_terminal_for_a_clean_repository() {
    let root = temp_root("hc refuses");
    let fake = fakes(&root);
    let repo = repo(&root);

    let out = plannotator(&root, &fake, &repo, &["hc", "review", "--commit", "HEAD", "--terminal", "t7"]);
    assert!(out.status.success(), "the first commit has changes: {}", String::from_utf8_lossy(&out.stderr));
    let started = calls(&fake).iter().filter(|c| c.starts_with("hc run")).count();
    assert_eq!(started, 1);

    std::fs::remove_file(fake.join("calls.log")).expect("reset log");
    let out = plannotator(&root, &fake, &repo, &["hc", "review", "--terminal", "t7"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("No uncommitted changes in app"));
    assert_eq!(calls(&fake), ["hc get --json t7"], "hc was asked about the terminal, nothing more");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_rex_and_hc_panes_open_the_review_they_were_named() {
    let root = temp_root("panes");
    let fake = fakes(&root);
    let repo = repo(&root);
    let repo_arg = repo.display().to_string();
    // A clean repository fails where the person can read it, which shows the review was chosen.
    for args in [
        ["rex", "pane", "review", "uncommitted", repo_arg.as_str()].as_slice(),
        ["hc", "pane", "review", "uncommitted", repo_arg.as_str()].as_slice(),
    ] {
        let out = plannotator(&root, &fake, &repo, args);
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("No uncommitted changes in app") && stderr.contains("press enter"),
            "{stderr}"
        );
    }
    let out = plannotator(&root, &fake, &repo, &["rex", "pane", "review", "everything", &repo_arg]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("is not a scope"));
    std::fs::remove_dir_all(root).expect("cleanup");
}
