//! Pull request reviews against a simulated GitHub: a local bare repository with a
//! `refs/pull/7/head` ref stands in for the PR's repository, a shell script stands in for
//! `gh`, and `url.<path>.insteadOf` sends `https://github.com/o/r` to the bare repository.
//! No network. Each test returns early (and says so) when git is not installed.
#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
#![allow(clippy::print_stderr, reason = "a skipped test says why")]

use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

const URL: &str = "https://github.com/o/r";

struct World {
    dir: PathBuf,
}

impl World {
    /// `upstream.git` (main + PR 7), a work repo `work` that pushes to it, and a fake `gh`
    /// answering with `view.json`. `None` when git is missing.
    fn new(tag: &str) -> Option<Self> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is not installed; skipping");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("pt-review-pr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let dir = dir.canonicalize().expect("real path");
        let world = Self { dir };
        git(&world.dir, &["init", "-q", "--bare", "upstream.git"]);
        git(&world.dir, &["init", "-q", "-b", "main", "work"]);
        let work = world.work();
        for (key, value) in [("user.name", "T"), ("user.email", "t@example.com"), ("commit.gpgsign", "false")]
        {
            git(&work, &["config", key, value]);
        }
        git(&work, &["remote", "add", "origin", &world.upstream().display().to_string()]);
        std::fs::write(work.join("app.txt"), "one\ntwo\nthree\n").expect("write");
        world.commit("init");
        git(&work, &["push", "-q", "origin", "main"]);
        git(&work, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(work.join("app.txt"), "one\nTWO\nthree\n").expect("write");
        std::fs::write(work.join("new.txt"), "fresh\n").expect("write");
        world.commit("feature");
        // main moves on after the PR branched, as it does on GitHub.
        git(&work, &["checkout", "-q", "main"]);
        std::fs::write(work.join("later.txt"), "later\n").expect("write");
        world.commit("later on main");
        git(&work, &["push", "-q", "origin", "main"]);
        world.push_pr();
        world.write_gh(0, "");
        Some(world)
    }

    fn work(&self) -> PathBuf {
        self.dir.join("work")
    }

    fn upstream(&self) -> PathBuf {
        self.dir.join("upstream.git")
    }

    fn commit(&self, message: &str) {
        git(&self.work(), &["add", "-A"]);
        git(&self.work(), &["commit", "-qm", message]);
    }

    fn rev(&self, rev: &str) -> String {
        git(&self.work(), &["rev-parse", rev])
    }

    /// Push `feature` as PR 7's head and describe it in `view.json`.
    fn push_pr(&self) {
        git(&self.work(), &["push", "-q", "-f", "origin", "feature:refs/pull/7/head"]);
        let view = serde_json::json!({
            "number": 7,
            "url": format!("{URL}/pull/7"),
            "title": "feat: shout two",
            "baseRefName": "main",
            "headRefName": "feature",
            "baseRefOid": self.rev("main"),
            "headRefOid": self.rev("feature"),
        });
        std::fs::write(self.dir.join("view.json"), view.to_string()).expect("view.json");
    }

    /// The fake `gh`: logs its arguments, then prints `view.json`, or `stderr` with `code`.
    fn write_gh(&self, code: i32, stderr: &str) -> Gh {
        let script = self.dir.join("gh");
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nif [ {code} -ne 0 ]; then echo '{stderr}' >&2; exit {code}; fi\ncat '{view}'\n",
            log = self.dir.join("gh.log").display(),
            view = self.dir.join("view.json").display(),
        );
        crate::test_support::write_script(&script, &body);
        Gh::at(script)
    }

    fn gh(&self) -> Gh {
        Gh::at(self.dir.join("gh"))
    }

    /// A clone of the PR's repository as the person has it: remote `origin` is the GitHub
    /// URL, rewritten to the bare repository by local config; nothing fetched yet.
    fn clone_dir(&self) -> PathBuf {
        let clone = self.dir.join("clone");
        git(&self.dir, &["init", "-q", "-b", "mine", "clone"]);
        git(&clone, &["remote", "add", "origin", &format!("{URL}.git")]);
        self.rewrite(&clone);
        clone
    }

    fn rewrite(&self, repo: &Path) {
        let key = format!("url.{}.insteadOf", self.upstream().display());
        git(repo, &["config", &key, &format!("{URL}.git")]);
    }

    fn cache(&self) -> PathBuf {
        self.dir.join("cache")
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn paths(capture: &Capture) -> Vec<&str> {
    capture.files.iter().map(|f| f.path.as_str()).collect()
}

#[test]
fn a_pr_number_is_fetched_into_the_clone_without_touching_its_branch() {
    let Some(w) = World::new("clone") else { return };
    let clone = w.clone_dir();
    let (repo, capture) = Repo::pull_request(&clone, "#7", &w.cache(), &w.gh()).expect("opens");
    assert_eq!(repo.root(), clone.as_path());
    assert_eq!(repo.workdir(), clone.as_path());
    // What GitHub shows: the PR's own changes, not main's later commit.
    assert_eq!(paths(&capture), ["app.txt", "new.txt"]);
    assert_eq!(capture.scope, Scope::PullRequest { url: format!("{URL}/pull/7"), number: 7 });
    let pull = capture.pull.as_ref().expect("a PR");
    assert_eq!(pull.head, w.rev("feature"));
    assert_eq!(pull.merge_base, w.rev("main~1"));
    assert_eq!(pull.clone.as_deref(), Some(clone.as_path()));
    assert_eq!((pull.view.title.as_str(), pull.view.base_ref.as_str()), ("feat: shout two", "main"));
    assert_eq!(capture.old, SideSource::Rev(w.rev("main~1")));
    assert_eq!(capture.new, SideSource::Rev(w.rev("feature")));
    assert_eq!(capture.read_side(Side::New, "app.txt").expect("reads").as_deref(), Some("one\nTWO\nthree\n"));
    // The clone's own branch, index and work tree are as they were; only a private ref moved.
    assert_eq!(git(&clone, &["symbolic-ref", "HEAD"]), "refs/heads/mine");
    assert_eq!(git(&clone, &["status", "--porcelain"]), "");
    assert_eq!(git(&clone, &["branch", "--list"]), "");
    assert_eq!(git(&clone, &["rev-parse", "refs/plannotator-tui/pull/7/head"]), w.rev("feature"));
    assert!(!clone.join(".git/FETCH_HEAD").exists());
    assert!(!w.cache().exists(), "a clone needs no cache");
    let log = std::fs::read_to_string(w.dir.join("gh.log")).expect("gh ran");
    assert_eq!(log.trim(), "pr view 7 --json number,url,title,baseRefName,headRefName,baseRefOid,headRefOid");
}

#[test]
fn a_reload_fetches_new_pushes() {
    let Some(w) = World::new("reload") else { return };
    let clone = w.clone_dir();
    let (repo, capture) = Repo::pull_request(&clone, "7", &w.cache(), &w.gh()).expect("opens");
    assert_eq!(paths(&capture), ["app.txt", "new.txt"]);
    git(&w.work(), &["checkout", "-q", "feature"]);
    std::fs::write(w.work().join("third.txt"), "3\n").expect("write");
    w.commit("more");
    w.push_pr();
    let again = repo.capture(&capture.scope).expect("reloads");
    assert_eq!(paths(&again), ["app.txt", "new.txt", "third.txt"]);
    assert_eq!(again.pull.as_ref().map(|p| p.head.clone()), Some(w.rev("feature")));
    // The reload looks the PR up by its link.
    let log = std::fs::read_to_string(w.dir.join("gh.log")).expect("gh ran");
    assert!(log.lines().nth(1).is_some_and(|l| l.starts_with(&format!("pr view {URL}/pull/7 "))), "{log}");
}

#[test]
fn a_link_outside_any_clone_is_fetched_into_the_cache() {
    let Some(w) = World::new("cache") else { return };
    let elsewhere = w.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("dir");
    // The cache repository is made on first use; made here first only to point it at the
    // bare repository instead of GitHub.
    let cache_repo = w.cache().join("github.com/o/r.git");
    std::fs::create_dir_all(&cache_repo).expect("cache dir");
    git(&cache_repo, &["init", "-q", "--bare"]);
    w.rewrite(&cache_repo);
    let (repo, capture) =
        Repo::pull_request(&elsewhere, &format!("{URL}/pull/7/files"), &w.cache(), &w.gh()).expect("opens");
    assert_eq!(repo.root(), cache_repo.as_path());
    assert_eq!(repo.workdir(), elsewhere.as_path());
    assert_eq!(paths(&capture), ["app.txt", "new.txt"]);
    assert_eq!(capture.pull.as_ref().and_then(|p| p.clone.clone()), None);
    assert_eq!(capture.read_side(Side::Old, "app.txt").expect("reads").as_deref(), Some("one\ntwo\nthree\n"));
    // A clone of some other repository is not used either.
    let other = w.dir.join("other");
    git(&w.dir, &["init", "-q", "other"]);
    git(&other, &["remote", "add", "origin", "https://github.com/o/elsewhere.git"]);
    let (repo, _) = Repo::pull_request(&other, &format!("{URL}/pull/7"), &w.cache(), &w.gh()).expect("opens");
    assert_eq!(repo.root(), cache_repo.as_path());
}

#[test]
fn a_new_cache_repository_is_made_bare_on_first_use() {
    let Some(w) = World::new("init") else { return };
    let repo =
        cache_repo(&w.cache(), &PrRepo { host: "github.com".into(), owner: "o".into(), name: "r".into() })
            .expect("made");
    assert_eq!(repo, w.cache().join("github.com/o/r.git"));
    assert_eq!(git(&repo, &["rev-parse", "--is-bare-repository"]), "true");
    assert_eq!(
        cache_repo(&w.cache(), &PrRepo { host: "h".into(), owner: "..".into(), name: "a/b".into() })
            .expect("made"),
        w.cache().join("h/-/a-b.git")
    );
}

#[test]
fn failures_say_what_is_wrong() {
    let Some(w) = World::new("errors") else { return };
    let elsewhere = w.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("dir");
    let err = Repo::pull_request(&elsewhere, "7", &w.cache(), &w.gh()).expect_err("no repo");
    assert!(matches!(err, Error::NoGitHubRemote { .. }), "{err}");
    let err = Repo::pull_request(&elsewhere, "src", &w.cache(), &w.gh()).expect_err("no PR");
    assert!(matches!(err, Error::NotAPull { .. }), "{err}");
    let missing = Gh::at(w.dir.join("no-such-gh"));
    let err = Repo::pull_request(&w.work(), "7", &w.cache(), &missing).expect_err("no gh");
    assert!(err.to_string().contains("https://cli.github.com"), "{err}");
    let gh = w.write_gh(1, "To get started with GitHub CLI, please run:  gh auth login");
    let err = Repo::pull_request(&w.work(), "7", &w.cache(), &gh).expect_err("logged out");
    assert!(matches!(err, Error::GhNotLoggedIn { .. }), "{err}");
    let gh = w.write_gh(
        1,
        "GraphQL: Could not resolve to a PullRequest with the number of 7. (repository.pullRequest)",
    );
    let err = Repo::pull_request(&w.work(), "7", &w.cache(), &gh).expect_err("no PR");
    assert!(err.to_string().starts_with("PR #7 not found"), "{err}");
}
