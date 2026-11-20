//! Real repositories in the temp dir, driven by the `git` on `PATH`. Each test returns early
//! (and says so) when git is not installed.
#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
#![allow(clippy::print_stderr, reason = "a skipped test says why")]

use std::path::PathBuf;
use std::process::Command;

use super::*;
use crate::annotation::Status;
use crate::resolve;

struct TestRepo {
    dir: PathBuf,
}

impl TestRepo {
    /// An empty repository, or `None` when git is missing.
    fn new(tag: &str) -> Option<Self> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is not installed; skipping");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("pt-review-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let repo = Self { dir };
        repo.git(&["init", "-q"]);
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
        let path = self.dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("dirs");
        }
        std::fs::write(path, content).expect("write");
    }

    fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn repo(&self) -> Repo {
        Repo::discover(&self.dir).expect("a repo")
    }

    fn index_bytes(&self) -> Vec<u8> {
        std::fs::read(self.dir.join(".git").join("index")).unwrap_or_default()
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn text(capture: &Capture) -> String {
    String::from_utf8_lossy(&capture.patch).into_owned()
}

fn paths(capture: &Capture) -> Vec<&str> {
    capture.files.iter().map(|f| f.path.as_str()).collect()
}

#[test]
fn uncommitted_shows_untracked_files_without_writing_the_real_index() {
    let Some(t) = TestRepo::new("uncommitted") else { return };
    t.write("a.txt", "one\ntwo\n");
    t.write(".gitignore", "ignored.log\n");
    t.commit_all("init");
    t.write("a.txt", "one\nTWO\n");
    t.write("new dir/fresh é.txt", "hello\n");
    t.write("ignored.log", "noise\n");
    let index_before = t.index_bytes();

    let capture = t.repo().capture(&Scope::Uncommitted).expect("captures");
    assert_eq!(paths(&capture), ["a.txt", "new dir/fresh é.txt"], "{}", text(&capture));
    assert!(text(&capture).contains("+++ b/new dir/fresh é.txt"), "UTF-8 paths are not quoted");
    assert_eq!(t.index_bytes(), index_before, "the real index is untouched");
    assert!(t.git(&["status", "--porcelain"]).contains("?? \"new dir/\""), "still untracked to git");
    assert_eq!(capture.old, SideSource::Rev(t.git(&["rev-parse", "HEAD"])));
    assert_eq!(capture.new, SideSource::Worktree);
    assert_eq!(capture.patch_key, patch_key(&capture.patch));
    assert_eq!(capture.patch_key.len(), 16);
    assert_eq!(capture.patch_bytes(), capture.patch.len());
    assert_eq!(capture.totals(), (2, 1));
    assert_eq!(capture.read_side(Side::Old, "a.txt").expect("reads").as_deref(), Some("one\ntwo\n"));
    assert_eq!(capture.read_side(Side::New, "a.txt").expect("reads").as_deref(), Some("one\nTWO\n"));
    assert_eq!(capture.read_side(Side::Old, "new dir/fresh é.txt").expect("reads"), None);
}

#[test]
fn staged_compares_head_with_the_index_only() {
    let Some(t) = TestRepo::new("staged") else { return };
    t.write("a.txt", "one\n");
    t.write("b.txt", "bee\n");
    t.commit_all("init");
    t.write("a.txt", "one staged\n");
    t.git(&["add", "a.txt"]);
    t.write("a.txt", "one staged then edited\n");
    t.write("b.txt", "bee unstaged\n");

    let capture = t.repo().open(&Request::Staged).expect("captures");
    assert_eq!(capture.scope, Scope::Staged);
    assert_eq!(paths(&capture), ["a.txt"]);
    assert_eq!(capture.new, SideSource::Index);
    assert_eq!(capture.read_side(Side::New, "a.txt").expect("reads").as_deref(), Some("one staged\n"));
}

#[test]
fn a_branch_review_diffs_the_merge_base_against_the_working_tree() {
    let Some(t) = TestRepo::new("branch") else { return };
    t.write("a.txt", "base\n");
    let base = t.commit_all("base");
    t.git(&["update-ref", "refs/remotes/origin/main", &base]);
    t.git(&["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    t.write("b.txt", "committed on the branch\n");
    t.commit_all("work");
    t.write("c.txt", "untracked\n");

    let capture = t.repo().open(&Request::Branch { base: None }).expect("captures");
    assert_eq!(
        capture.scope,
        Scope::Branch { base: "origin/main".into() },
        "origin/HEAD is named by its target"
    );
    assert_eq!(paths(&capture), ["b.txt", "c.txt"]);
    assert_eq!(capture.old, SideSource::Rev(base));
    let missing = t.repo().open(&Request::Branch { base: Some("nope".into()) });
    assert!(matches!(missing, Err(Error::NoBase { ref base }) if base == "nope"), "{missing:?}");
}

#[test]
fn clean_uncommitted_falls_back_to_the_branch_when_a_base_exists() {
    let Some(t) = TestRepo::new("fallback") else { return };
    t.write("a.txt", "base\n");
    let base = t.commit_all("base");
    t.write("a.txt", "branch work\n");
    t.commit_all("work");

    let request = Request::Uncommitted { base: None };
    let no_remote = t.repo().open(&request).expect("captures");
    assert_eq!(no_remote.scope, Scope::Uncommitted, "no origin/HEAD: nothing to fall back to");
    assert!(no_remote.is_empty());

    t.git(&["update-ref", "refs/remotes/origin/main", &base]);
    t.git(&["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    let fallback = t.repo().open(&request).expect("captures");
    assert_eq!(fallback.scope, Scope::Branch { base: "origin/main".into() });
    assert_eq!(paths(&fallback), ["a.txt"]);

    t.write("a.txt", "uncommitted\n");
    assert_eq!(t.repo().open(&request).expect("captures").scope, Scope::Uncommitted);
}

#[test]
fn a_commit_review_diffs_against_its_parent_or_the_empty_tree() {
    let Some(t) = TestRepo::new("commit") else { return };
    t.write("a.txt", "one\n");
    let root = t.commit_all("root commit");
    t.write("a.txt", "two\n");
    let second = t.commit_all("fix: second");

    let capture = t.repo().open(&Request::Commit { rev: "HEAD".into() }).expect("captures");
    assert_eq!(capture.scope, Scope::Commit { sha: second.clone() });
    assert_eq!(capture.commit_subject.as_deref(), Some("fix: second"));
    assert_eq!(
        (capture.old.clone(), capture.new.clone()),
        (SideSource::Rev(root.clone()), SideSource::Rev(second))
    );
    assert_eq!(capture.read_side(Side::Old, "a.txt").expect("reads").as_deref(), Some("one\n"));

    let first =
        t.repo().open(&Request::Commit { rev: root.get(..8).expect("sha").into() }).expect("captures");
    assert_eq!(first.files.first().map(|f| f.status), Some(crate::FileStatus::Added));
    assert_eq!(first.read_side(Side::Old, "a.txt").expect("reads"), None, "the empty tree has no files");
    assert!(t.repo().open(&Request::Commit { rev: "--output=x".into() }).is_err(), "an option is not a rev");
}

#[test]
fn a_new_repository_without_commits_still_shows_its_files() {
    let Some(t) = TestRepo::new("unborn") else { return };
    t.write("first.txt", "hello\n");
    let capture = t.repo().capture(&Scope::Uncommitted).expect("captures");
    assert_eq!(paths(&capture), ["first.txt"]);
    t.git(&["add", "first.txt"]);
    assert_eq!(paths(&t.repo().capture(&Scope::Staged).expect("captures")), ["first.txt"]);
}

#[test]
fn a_patch_over_the_limit_refuses_and_says_why() {
    let Some(t) = TestRepo::new("too-large") else { return };
    t.write("a.txt", "x\n");
    t.commit_all("init");
    let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n";
    t.write("big.txt", &line.repeat(MAX_PATCH_BYTES / line.len() + 10));
    let err = t.repo().capture(&Scope::Uncommitted).expect_err("too large");
    assert!(matches!(err, Error::TooLarge { limit: MAX_PATCH_BYTES }));
    assert!(err.to_string().contains("larger than 8 MiB"), "{err}");
}

#[test]
fn a_folder_outside_any_repository_is_named_in_the_error() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("pt-review-norepo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let err = Repo::discover(&dir).expect_err("not a repo");
    let _ = std::fs::remove_dir_all(&dir);
    match err {
        Error::NotARepo { dir: named, .. } => assert_eq!(named, dir),
        // The temp dir itself may sit inside a work tree on a developer machine.
        other => panic!("expected NotARepo, got {other}"),
    }
}

#[test]
fn notes_follow_their_lines_across_a_recapture() {
    let Some(t) = TestRepo::new("recapture") else { return };
    t.write("src/lib.rs", "fn keep() {}\nfn old() {\n    legacy();\n}\n");
    t.commit_all("init");
    t.write("src/lib.rs", "fn keep() {}\nfn new() {\n    modern();\n}\n");
    let repo = t.repo();
    let first = repo.capture(&Scope::Uncommitted).expect("captures");
    let file = first.file("src/lib.rs").expect("in the diff");
    let deleted = CodeAnnotation::line_comment(file, Side::Old, 3, 3, "why drop?".into(), &first.patch_key);
    let added = CodeAnnotation::line_comment(file, Side::New, 2, 3, "naming".into(), &first.patch_key);
    let notes = vec![deleted, added];
    assert_eq!(
        first.resolve_all(&notes).expect("resolves"),
        [Resolution::StillThere, Resolution::StillThere]
    );

    // The agent adds lines above the new code and rewrites nothing the notes point at.
    t.write("src/lib.rs", "use std::fmt;\n\nfn keep() {}\nfn new() {\n    modern();\n}\n");
    let second = repo.capture(&Scope::Uncommitted).expect("recaptures");
    let resolutions = second.resolve_all(&notes).expect("resolves");
    assert_eq!(resolutions, [Resolution::StillThere, Resolution::Moved { line_start: 4, line_end: 5 }]);

    // Then rewrites the commented code.
    t.write("src/lib.rs", "use std::fmt;\n\nfn keep() {}\nfn renamed() {\n    modern();\n}\n");
    let third = repo.capture(&Scope::Uncommitted).expect("recaptures");
    let mut moved = notes.clone();
    for (note, r) in moved.iter_mut().zip(third.resolve_all(&notes).expect("resolves")) {
        resolve::apply(note, r);
    }
    let statuses: Vec<_> = moved.iter().map(CodeAnnotation::status).collect();
    assert_eq!(statuses, [Some(Status::StillThere), Some(Status::Outdated)]);
}

#[test]
fn a_whole_change_note_resolves_without_reading_a_file_in_every_scope() {
    let Some(t) = TestRepo::new("general") else { return };
    t.write("src/lib.rs", "fn keep() {}\n");
    t.commit_all("init");
    t.write("src/lib.rs", "fn keep() {}\nfn more() {}\n");
    t.git(&["add", "-A"]);
    let repo = t.repo();
    for scope in [Scope::Uncommitted, Scope::Staged] {
        let capture = repo.capture(&scope).expect("captures");
        let note = CodeAnnotation::general_comment("split it".into(), &capture.patch_key);
        assert_eq!(capture.resolve_all(&[note]).expect("resolves"), [Resolution::StillThere], "{scope:?}");
    }
    let head = t.commit_all("more");
    let capture = repo.capture(&Scope::Commit { sha: head }).expect("captures");
    let note = CodeAnnotation::general_comment("split it".into(), "another patch");
    assert_eq!(capture.resolve_all(&[note]).expect("resolves"), [Resolution::StillThere]);
}
