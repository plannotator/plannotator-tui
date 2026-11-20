//! Stamps of real repositories in the temp dir. Each test returns early when git is missing.
#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
#![allow(clippy::print_stderr, reason = "a skipped test says why")]

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime};

use super::*;

struct TestRepo {
    dir: PathBuf,
}

impl TestRepo {
    fn new(tag: &str) -> Option<Self> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is not installed; skipping");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("pt-stamp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let repo = Self { dir };
        repo.git(&["init", "-q"]);
        for (key, value) in
            [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")]
        {
            repo.git(&["config", key, value]);
        }
        repo.write("a.txt", "one\n");
        repo.write("dir name/b.txt", "two\n");
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-qm", "init"]);
        Some(repo)
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(&self.dir).args(args).output().expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// Write `content`, and move the mtime on so a same-size rewrite in the same tick shows.
    fn write(&self, path: &str, content: &str) {
        let path = self.dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("dirs");
        }
        std::fs::write(&path, content).expect("write");
        let later =
            SystemTime::now() + Duration::from_secs(STEP.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        std::fs::File::options().write(true).open(&path).expect("open").set_modified(later).expect("mtime");
    }

    fn stamp(&self, scope: &Scope) -> DiskStamp {
        Repo::discover(&self.dir).expect("repo").disk_stamp(scope).expect("stamps").expect("watched")
    }
}

static STEP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn an_uncommitted_stamp_changes_with_every_edit_and_only_then() {
    let Some(t) = TestRepo::new("uncommitted") else { return };
    let clean = t.stamp(&Scope::Uncommitted);
    assert_eq!(t.stamp(&Scope::Uncommitted), clean, "nothing changed");
    t.write("a.txt", "one!\n");
    let edited = t.stamp(&Scope::Uncommitted);
    assert_ne!(edited, clean);
    // Changed again: status reads the same, the file's time and size do not.
    t.write("a.txt", "one?\n");
    let again = t.stamp(&Scope::Uncommitted);
    assert_ne!(again, edited, "a second edit of a changed file shows");
    t.write("dir name/new file.txt", "new\n");
    let untracked = t.stamp(&Scope::Uncommitted);
    assert_ne!(untracked, again, "an untracked file shows");
    std::fs::remove_file(t.dir.join("dir name/new file.txt")).expect("remove");
    assert_eq!(t.stamp(&Scope::Uncommitted), again, "and its removal");
    t.git(&["commit", "-qam", "commit the edit"]);
    assert_ne!(t.stamp(&Scope::Uncommitted), again, "a commit moves HEAD");
    assert_eq!(t.stamp(&Scope::Branch { base: "HEAD".into() }), t.stamp(&Scope::Uncommitted));
}

#[test]
fn a_staged_stamp_ignores_the_work_tree_and_follows_the_index() {
    let Some(t) = TestRepo::new("staged") else { return };
    let clean = t.stamp(&Scope::Staged);
    t.write("a.txt", "one!\n");
    t.write("c.txt", "untracked\n");
    assert_eq!(t.stamp(&Scope::Staged), clean, "unstaged edits are not in a staged diff");
    t.git(&["add", "a.txt"]);
    let staged = t.stamp(&Scope::Staged);
    assert_ne!(staged, clean);
    t.write("a.txt", "one again\n");
    assert_eq!(t.stamp(&Scope::Staged), staged);
    t.git(&["add", "a.txt"]);
    assert_ne!(t.stamp(&Scope::Staged), staged, "a new version staged");
}

#[test]
fn commits_and_pull_requests_have_nothing_to_watch() {
    let Some(t) = TestRepo::new("commit") else { return };
    let repo = Repo::discover(&t.dir).expect("repo");
    assert_eq!(repo.disk_stamp(&Scope::Commit { sha: "HEAD".into() }).expect("no error"), None);
    let pr = Scope::PullRequest { url: "https://github.com/o/r/pull/1".into(), number: 1 };
    assert_eq!(repo.disk_stamp(&pr).expect("no error"), None);
    assert!(!pr.watches_disk() && Scope::Staged.watches_disk());
}
