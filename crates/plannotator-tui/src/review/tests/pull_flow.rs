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
        rex: None,
        ask_on_send: true,
    };
    screen.reach = Reach::start_with(finder, root.clone(), None);
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

#[test]
fn a_pr_review_from_one_rex_agent_can_go_to_another_picked_in_rex() {
    use crate::review::Mode;
    use crate::rex::agent::RexAgent;
    use crate::rex::agents::{FIXTURE_OWN_BLOCK, FIXTURE_SESSION, RexAgents, fake_rex};
    use crate::rex::command::Rex;
    use crate::ui::agent_picker::{AgentFinder, Reach};
    let pull = Pull::new("rex-picker");
    let mut screen = pull.open();
    let fake = pull.dir.join("fake rex");
    let bin = fake_rex(&fake);
    // As `rex review 7` opens it beside the Claude in the fixtures' `work › api`.
    let claude = "block:01a1f658-997c-79c0-ae26-1bf77b9ba798".to_owned();
    let rex = Rex::new(bin.clone(), FIXTURE_SESSION.into());
    screen.delivery = Box::new(RexAgent::picked(rex, claude, "claude".into(), 94903));
    let root = screen.real_root();
    let finder = AgentFinder {
        pi_root: Some(pull.dir.join("registry")),
        herdr: None,
        rex: Some(RexAgents {
            bin,
            session: FIXTURE_SESSION.into(),
            own_block: Some(FIXTURE_OWN_BLOCK.into()),
        }),
        ask_on_send: true,
    };
    screen.reach = Reach::start_with(finder, root.clone(), None);
    screen.reach.find(&root);
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    assert!(screen.send_label().contains("▸ claude in 9ba798"), "{}", screen.send_label());

    keys(&mut screen, "P");
    assert_eq!(screen.mode, Mode::PickTarget);
    let rows = frame(&mut screen, 120, 40);
    if let Some(dir) = std::env::var_os("REX_PICKER_PROOF_DIR") {
        std::fs::create_dir_all(&dir).expect("proof dir");
        std::fs::write(PathBuf::from(dir).join("pr-review-p-from-claude.txt"), rows.join("\n"))
            .expect("proof");
    }
    assert!(rows[0].starts_with(" PR #7 · feat: greet from argv"), "{}", rows[0]);
    assert!(row_with(&rows, "work › api · 31496a").contains("codex"));
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    keys(&mut screen, "E");
    let pastes = read(&fake.join("pastes"));
    assert!(pastes.contains("block:01a1f658-99b3-7134-b64a-0b9d9d31496a"), "{pastes}");
    assert!(pastes.contains("# PR Review: o/app#7") && pastes.contains("name the total"), "{pastes}");
    assert_eq!(screen.status.as_deref(), Some("sent 1 note(s) → codex in 31496a"));
}

#[test]
fn a_comment_on_the_whole_pr_goes_first_under_the_pr_header() {
    let pull = Pull::new("change-note");
    let mut screen = pull.open();
    keys(&mut screen, "M");
    type_and_save(&mut screen, "please add a test");
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    keys(&mut screen, "E");
    let sent = pull.agent.sent.borrow().clone();
    let [text] = sent.as_slice() else { panic!("one send: {sent:?}") };
    let expected = format!(
        "{URL}/pull/7\n\n## Global comments\n\nplease add a test\n\n## app.py\n\n### Line 9 (new)\nname the total\n"
    );
    assert!(text.contains(&expected), "{text}");
    pull.push_head(&format!("# greeter\n{AFTER}"));
    keys(&mut screen, "r");
    settle(&mut screen);
    let general = screen.store.annotations().iter().find(|n| n.text.as_deref() == Some("please add a test"));
    assert_eq!(
        general.map(plannotator_tui_review::CodeAnnotation::status),
        Some(Some(Status::StillThere)),
        "a reload keeps it"
    );
}

#[test]
fn the_whole_file_of_a_pr_is_read_from_its_commits_in_the_cache_repository() {
    use super::whole_flow::settle_whole;
    let pull = Pull::new("whole");
    let (mut screen, merge_base) = open_in_cache(&pull);
    at(&mut screen, 9);
    keys(&mut screen, "o");
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Whole file · new · app.py "), "{:?}", rows[0]);
    assert!(rows.iter().any(|r| r.contains("total = a + b")), "the head's version: {rows:#?}");
    keys(&mut screen, "o");
    keys(&mut screen, "O");
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    let base = format!(" Whole file · base ({}) · app.py ", &merge_base[..7]);
    assert!(rows[0].starts_with(&base), "{:?}", rows[0]);
    assert!(rows.iter().any(|r| r.contains("return a + b")), "the base's version: {rows:#?}");
    assert!(!rows.iter().any(|r| r.contains("total = a + b")));
}

#[test]
fn go_to_definition_searches_a_pr_s_commits_in_the_cache_repository() {
    let pull = Pull::new("jump");
    let (mut screen, _) = open_in_cache(&pull);
    let settle_search = |screen: &mut Screen| {
        let started = Instant::now();
        while screen.nav.notice().is_some() {
            screen.tick().expect("tick");
            assert!(started.elapsed() < Duration::from_secs(10), "no answer");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let cursor = |screen: &Screen| {
        let lines = screen.diff.cursor().expect("cursor").expect("a line");
        (lines.side, lines.first)
    };
    // The head: `greet(sys.argv[1])`, pick `greet`.
    at(&mut screen, 14);
    keys(&mut screen, "d1");
    settle_search(&mut screen);
    assert_eq!(cursor(&screen), (Side::New, 4), "{:?}", screen.status);
    // The base: the removed `greet("world")` finds the base's `def greet`.
    screen.diff.reveal("app.py", Side::Old, 10).expect("removed line");
    keys(&mut screen, "d");
    settle_search(&mut screen);
    assert_eq!(cursor(&screen), (Side::Old, 1), "{:?}", screen.status);
}

/// A screen on PR 7 fetched into a bare cache repository (no clone), and the PR's merge base.
fn open_in_cache(pull: &Pull) -> (Screen, String) {
    // No clone: the PR goes into the bare cache repository, made here first so the GitHub
    // URL can be pointed at the local upstream for this repository only.
    let outside = pull.dir.join("outside");
    std::fs::create_dir_all(&outside).expect("dir");
    let cache = pull.dir.join("cache");
    let cache_repo = cache.join("github.com").join("o").join("app.git");
    std::fs::create_dir_all(&cache_repo).expect("cache dir");
    git_in(&cache_repo, &["init", "-q", "--bare"]);
    let rewrite = format!("url.{}.insteadOf", pull.dir.join("upstream.git").display());
    git_in(&cache_repo, &["config", &rewrite, &format!("{URL}.git")]);
    let gh = Gh::at(pull.dir.join("gh"));
    let (repo, capture) = Repo::pull_request(&outside, &format!("{URL}/pull/7"), &cache, &gh).expect("opens");
    assert_eq!(capture.pull.as_ref().map(|p| p.clone.is_none()), Some(true), "fetched into the cache");
    let merge_base = capture.pull.as_ref().map(|p| p.merge_base.clone()).expect("a PR");
    let (wake, _ready) = std::sync::mpsc::sync_channel(1);
    let mut screen =
        Screen::open(repo, capture, &pull.data(), Box::new(pull.agent.clone()), wake).expect("screen");
    settle(&mut screen);
    (screen, merge_base)
}
