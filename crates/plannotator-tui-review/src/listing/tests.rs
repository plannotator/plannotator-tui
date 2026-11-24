//! The picker's listing against real repositories in the temp dir. Each test returns early
//! (and says so) when git is not installed.
#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
#![allow(clippy::print_stderr, reason = "a skipped test says why")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

struct TestRepo {
    dir: PathBuf,
}

impl TestRepo {
    fn new(tag: &str) -> Option<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is not installed; skipping");
            return None;
        }
        let dir = std::env::temp_dir().join(format!(
            "pt-listing-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let repo = Self { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Test"),
            ("user.email", "test@example.com"),
            ("core.autocrlf", "false"),
            ("commit.gpgsign", "false"),
        ] {
            repo.git(&["config", key, value]);
        }
        Some(repo)
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(&self.dir).args(args).output().expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.dir.join(path), content).expect("write");
    }

    fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn repo(&self) -> Repo {
        Repo::discover(&self.dir).expect("a repo")
    }

    /// Files changed in each working-tree scope: uncommitted, unstaged, staged.
    fn sizes(&self) -> [usize; 3] {
        let repo = self.repo();
        [Scope::Uncommitted, Scope::Unstaged, Scope::Staged]
            .map(|scope| repo.diff_stat(&scope).expect("a size").files)
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_clean_tree_has_nothing_in_the_working_tree_scopes_and_lists_its_commits() {
    let Some(t) = TestRepo::new("clean") else { return };
    t.write("a.txt", "one\n");
    t.commit_all("first");
    t.write("a.txt", "one\ntwo\n");
    let sha = t.commit_all("second: add two");
    assert_eq!(t.sizes(), [0, 0, 0]);
    let commits = t.repo().commits(0, 20).expect("commits");
    assert_eq!(commits.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["second: add two", "first"]);
    assert_eq!((commits[0].sha.as_str(), commits[0].author.as_str()), (sha.as_str(), "Test"));
    assert!(commits[0].time > 1_600_000_000, "{}", commits[0].time);
    let stat = t.repo().diff_stat(&Scope::Commit { sha }).expect("size");
    assert_eq!((stat.files, stat.additions, stat.deletions, stat.newest), (1, 1, 0, None));
}

#[test]
fn staged_unstaged_and_untracked_changes_each_count_where_their_review_shows_them() {
    let Some(t) = TestRepo::new("parts") else { return };
    t.write("a.txt", "one\n");
    t.write("b.txt", "bee\n");
    t.commit_all("init");

    t.write("a.txt", "one staged\n");
    t.git(&["add", "a.txt"]);
    assert_eq!(t.sizes(), [1, 0, 1], "staged only");

    t.git(&["reset", "-q"]);
    assert_eq!(t.sizes(), [1, 1, 0], "unstaged only");

    t.git(&["checkout", "-q", "--", "a.txt"]);
    t.write("new.txt", "fresh\nfile\n");
    assert_eq!(t.sizes(), [1, 1, 0], "untracked only");
    let unstaged = t.repo().diff_stat(&Scope::Unstaged).expect("size");
    assert_eq!((unstaged.additions, unstaged.deletions), (2, 0));
    assert!(unstaged.newest.is_some(), "the file's time");

    // A file staged and then edited again is in all three.
    t.write("b.txt", "bee staged\n");
    t.git(&["add", "b.txt"]);
    t.write("b.txt", "bee edited after\n");
    assert_eq!(t.sizes(), [2, 2, 1]);
    for scope in [Scope::Uncommitted, Scope::Unstaged, Scope::Staged] {
        let size = t.repo().diff_stat(&scope).expect("size").files;
        let review = t.repo().capture(&scope).expect("capture").files.len();
        assert_eq!(size, review, "{scope:?}: the size counts the files its review shows");
    }
}

#[test]
fn a_repository_without_commits_lists_none_and_its_files_are_uncommitted() {
    let Some(t) = TestRepo::new("empty") else { return };
    assert_eq!(t.repo().commits(0, 20).expect("commits"), []);
    assert_eq!(t.repo().branch_choice(), None);
    assert_eq!(t.sizes(), [0, 0, 0]);
    t.write("a.txt", "one\n");
    assert_eq!(t.sizes(), [1, 1, 0]);
}

#[test]
fn a_detached_head_lists_the_commits_behind_it() {
    let Some(t) = TestRepo::new("detached") else { return };
    t.write("a.txt", "one\n");
    let first = t.commit_all("first");
    t.write("a.txt", "two\n");
    t.commit_all("second");
    t.git(&["checkout", "-q", "--detach", &first]);
    let commits = t.repo().commits(0, 20).expect("commits");
    assert_eq!(commits.iter().map(|c| c.sha.as_str()).collect::<Vec<_>>(), [first.as_str()]);
}

#[test]
fn the_branch_is_offered_only_when_the_base_resolves_and_head_has_moved_past_it() {
    let Some(t) = TestRepo::new("branch") else { return };
    t.write("a.txt", "one\n");
    let base = t.commit_all("base");
    assert_eq!(t.repo().branch_choice(), None, "no origin");
    t.git(&["update-ref", "refs/remotes/origin/main", &base]);
    t.git(&["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    assert_eq!(t.repo().branch_choice(), None, "HEAD is the base");
    t.write("a.txt", "two\n");
    t.commit_all("on the branch");
    let branch = t.repo().branch_choice().expect("offered");
    assert_eq!(branch, Scope::Branch { base: "origin/main".into() });
    assert_eq!(t.repo().diff_stat(&branch).expect("size").files, 1);
}

#[test]
fn a_merge_commit_is_listed_and_sized_against_its_first_parent() {
    let Some(t) = TestRepo::new("merge") else { return };
    t.write("a.txt", "one\n");
    t.commit_all("init");
    t.git(&["checkout", "-q", "-b", "side"]);
    t.write("side.txt", "from the side\n");
    t.commit_all("side work");
    t.git(&["checkout", "-q", "main"]);
    t.write("a.txt", "one\nmain\n");
    t.commit_all("main work");
    t.git(&["merge", "-q", "--no-edit", "--no-ff", "side"]);
    let merge = t.git(&["rev-parse", "HEAD"]);
    let commits = t.repo().commits(0, 20).expect("commits");
    assert_eq!(commits[0].sha, merge);
    assert_eq!(commits.len(), 4);
    let stat = t.repo().diff_stat(&Scope::Commit { sha: merge.clone() }).expect("size");
    assert_eq!((stat.files, stat.additions), (1, 1), "side.txt, what the merge brought into main");
    let review = t.repo().capture(&Scope::Commit { sha: merge }).expect("capture");
    assert_eq!(review.files.len(), 1);
}

#[test]
fn commits_page_by_skip_and_count() {
    let Some(t) = TestRepo::new("pages") else { return };
    for n in 1..=25 {
        t.write("a.txt", &format!("{n}\n"));
        t.commit_all(&format!("commit {n}"));
    }
    let repo = t.repo();
    let first = repo.commits(0, 20).expect("first page");
    assert_eq!(first.len(), 20);
    assert_eq!(first[0].subject, "commit 25");
    let more = repo.commits(20, 20).expect("second page");
    assert_eq!(
        more.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
        ["commit 5", "commit 4", "commit 3", "commit 2", "commit 1"]
    );
    assert_eq!(repo.commits(25, 20).expect("past the end"), []);
}

#[test]
fn numstat_reads_renames_and_binary_files() {
    let out = b"3\t1\ta.txt\0-\t-\timg.png\x000\t0\t\0old name.txt\0new name.txt\0";
    assert_eq!(
        numstat(out),
        [(3, 1, "a.txt".to_owned()), (0, 0, "img.png".to_owned()), (0, 0, "new name.txt".to_owned())]
    );
}
