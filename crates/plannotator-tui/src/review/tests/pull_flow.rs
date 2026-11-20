//! A pull request review on the screen, against a simulated GitHub: a local bare repository
//! with `refs/pull/7/head`, a clone whose `origin` is the GitHub URL (rewritten to the bare
//! repository by `insteadOf`), and a shell script standing in for `gh`. No network.

use std::os::unix::fs::PermissionsExt as _;

use plannotator_tui_review::{Gh, Repo, Status};

use super::*;

const URL: &str = "https://github.com/o/app";

struct Pull {
    dir: PathBuf,
    agent: Agent,
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

impl Pull {
    /// PR 7 changes `app.py` from BEFORE to AFTER on branch `feature` off `main`.
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("plannotator-review-pull-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let dir = dir.canonicalize().expect("real path");
        let pull = Self { dir, agent: Agent::default() };
        git_in(&pull.dir, &["init", "-q", "--bare", "upstream.git"]);
        git_in(&pull.dir, &["init", "-q", "-b", "main", "work"]);
        let work = pull.work();
        git_in(&work, &["remote", "add", "origin", &pull.dir.join("upstream.git").display().to_string()]);
        std::fs::write(work.join("app.py"), BEFORE).expect("write");
        git_in(&work, &["add", "-A"]);
        git_in(&work, &["commit", "-qm", "init"]);
        git_in(&work, &["push", "-q", "origin", "main"]);
        git_in(&work, &["checkout", "-q", "-b", "feature"]);
        pull.push_head(AFTER);
        git_in(&pull.dir, &["init", "-q", "-b", "mine", "clone"]);
        let clone = pull.clone_dir();
        git_in(&clone, &["remote", "add", "origin", &format!("{URL}.git")]);
        let rewrite = format!("url.{}.insteadOf", pull.dir.join("upstream.git").display());
        git_in(&clone, &["config", &rewrite, &format!("{URL}.git")]);
        let script = pull.dir.join("gh");
        let body = format!("#!/bin/sh\ncat '{}'\n", pull.dir.join("view.json").display());
        std::fs::write(&script, body).expect("gh");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        pull
    }

    fn work(&self) -> PathBuf {
        self.dir.join("work")
    }

    fn clone_dir(&self) -> PathBuf {
        self.dir.join("clone")
    }

    fn data(&self) -> PathBuf {
        self.dir.join("data")
    }

    /// Commit `app` on `feature`, push it as PR 7's head, and describe it for `gh`.
    fn push_head(&self, app: &str) {
        let work = self.work();
        std::fs::write(work.join("app.py"), app).expect("write");
        git_in(&work, &["commit", "-qam", "feature"]);
        git_in(&work, &["push", "-q", "-f", "origin", "feature:refs/pull/7/head"]);
        let view = serde_json::json!({
            "number": 7, "url": format!("{URL}/pull/7"), "title": "feat: greet from argv",
            "baseRefName": "main", "headRefName": "feature",
            "baseRefOid": git_in(&work, &["rev-parse", "main"]),
            "headRefOid": git_in(&work, &["rev-parse", "feature"]),
        });
        std::fs::write(self.dir.join("view.json"), view.to_string()).expect("view.json");
    }

    fn open(&self) -> Screen {
        let gh = Gh::at(self.dir.join("gh"));
        let (repo, capture) =
            Repo::pull_request(&self.clone_dir(), "#7", &self.dir.join("cache"), &gh).expect("the PR opens");
        let (wake, _ready) = std::sync::mpsc::sync_channel(1);
        let mut screen =
            Screen::open(repo, capture, &self.data(), Box::new(self.agent.clone()), wake).expect("screen");
        settle(&mut screen);
        screen
    }
}

impl Drop for Pull {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_pr_review_names_the_pr_and_sends_the_browsers_pr_feedback() {
    let pull = Pull::new("send");
    let mut screen = pull.open();
    let rows = frame(&mut screen, 120, 40);
    let header = &rows[0];
    assert!(header.starts_with(" PR #7 · feat: greet from argv · main ← feature · 1 file +"), "{header}");
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    keys(&mut screen, "E");
    let sent = pull.agent.sent.borrow().clone();
    let [text] = sent.as_slice() else { panic!("one send: {sent:?}") };
    let clone = pull.clone_dir().display().to_string();
    let head = format!(
        "Review directory: {clone}\n\n# PR Review: o/app#7\n\n**feat: greet from argv**\nBranch: `feature` → `main`\n{URL}/pull/7\n\n## app.py\n\n### Line 9 (new)\nname the total\n"
    );
    assert!(text.starts_with(&head), "{text}");
    // Notes are kept per PR, beside its GitHub repository rather than the clone.
    let record = read(screen.store.path());
    assert!(
        screen.store.path().display().to_string().contains("/reviews/app/pr-7-"),
        "{}",
        screen.store.path().display()
    );
    let record: serde_json::Value = serde_json::from_str(&record).expect("JSON");
    assert_eq!(record["repo"], URL);
    assert_eq!(
        record["scope"],
        serde_json::json!({"kind": "pull_request", "url": format!("{URL}/pull/7"), "number": 7})
    );
    // The archive names the PR as the browser's PR review does.
    let index = pull.data().join("feedback").join("app").join("index.jsonl");
    let line: serde_json::Value = serde_json::from_str(read(&index).trim()).expect("one JSON line");
    let review = &line["target"]["review"];
    assert_eq!(review["diffType"], "merge-base");
    assert_eq!(review["gitRef"], "PR #7");
    assert_eq!(review["pr"], serde_json::json!({"provider": "github", "repo": "o/app", "number": 7}));
    assert_eq!(review["base"], git_in(&pull.work(), &["rev-parse", "main"]));
    assert_eq!(review["cwd"], clone);
}

#[test]
fn a_reload_fetches_a_new_push_and_the_note_moves_with_its_line() {
    let pull = Pull::new("reload");
    let mut screen = pull.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    pull.push_head(&format!("#!/usr/bin/env python3\n# greeter\n{AFTER}"));
    keys(&mut screen, "r");
    settle(&mut screen);
    let [note] = screen.store.annotations() else { panic!("one note") };
    assert_eq!((note.line_start, note.line_end), (11, 11));
    assert_eq!(note.status(), Some(Status::Moved));
    let head = git_in(&pull.work(), &["rev-parse", "feature"]);
    assert_eq!(screen.capture.pull.as_ref().map(|p| p.head.as_str()), Some(head.as_str()));
    // The person's clone kept its own branch.
    assert_eq!(git_in(&pull.clone_dir(), &["symbolic-ref", "HEAD"]), "refs/heads/mine");
}

#[test]
fn a_pr_review_with_no_target_asks_which_agent_and_sends_the_pr_feedback() {
    use super::send_flow::FallbackCopy;
    use crate::herdr::agents::{HerdrAgents, fake_herdr};
    use crate::review::Mode;
    use crate::ui::agent_picker::{AgentFinder, Reach};
    let pull = Pull::new("ask-agent");
    let mut screen = pull.open();
    let copy = FallbackCopy::default();
    screen.delivery = Box::new(copy.clone());
    let root = screen.real_root();
    let fake = pull.dir.join("fake herdr");
    let bin = fake_herdr(
        &fake,
        &serde_json::json!({"result": {"type": "agent_list", "agents": [
            {"agent": "claude", "agent_status": "idle", "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2", "cwd": root},
        ]}}),
    );
    let finder = AgentFinder {
        pi_root: Some(pull.dir.join("registry")),
        herdr: Some(HerdrAgents { bin, own_pane: Some("w1:p9".into()) }),
        ask_on_send: true,
    };
    screen.reach = Reach::start_with(finder, root.clone());
    screen.reach.find(&root);
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" PR #7 · feat: greet from argv · main ← feature · 1 file +"), "{}", rows[0]);
    assert!(row_with(&rows, "Send 1 new").contains("choose agent"));

    keys(&mut screen, "E");
    assert_eq!(screen.mode, Mode::PickTarget, "Send opens the picker instead of copying");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "Copy to clipboard").contains('│'));
    press(&mut screen, KeyCode::Enter);
    let prompts = read(&fake.join("prompts"));
    assert!(prompts.starts_with("w1:p2\n"), "{prompts}");
    assert!(prompts.contains("# PR Review: o/app#7") && prompts.contains("name the total"), "{prompts}");
    assert_eq!(screen.status.as_deref(), Some("sent 1 note(s) → claude in w1:p2"));
    assert_eq!(copy.copied.borrow().len(), 0);
}
