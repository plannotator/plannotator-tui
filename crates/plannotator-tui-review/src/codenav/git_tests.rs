//! Searches against real repositories in the temp dir, with the `git` on `PATH`: each side
//! of a diff, a bare repository, and the browser's real-ripgrep exclusion cases (#1558)
//! replayed through `git grep`. Each test returns early (and says so) when git is missing.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "tests assert by panicking")]
#![allow(clippy::print_stderr, reason = "a skipped test says why")]

use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

struct TestRepo {
    dir: PathBuf,
    /// `HOME` and `PLANNOTATOR_DATA_DIR` for every git this test runs.
    scratch: PathBuf,
}

impl TestRepo {
    fn new(tag: &str) -> Option<Self> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is not installed; skipping");
            return None;
        }
        let base = std::env::temp_dir().join(format!("pt-codenav-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = Self { dir: base.join("repo"), scratch: base.join("home") };
        std::fs::create_dir_all(&repo.dir).expect("repo dir");
        std::fs::create_dir_all(&repo.scratch).expect("scratch home");
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

    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("HOME", &self.scratch)
            .env("PLANNOTATOR_DATA_DIR", self.scratch.join("data"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.dir, args)
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
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        if let Some(base) = self.dir.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

fn find(repo: &Path, side: &SideSource, file: &str, line: u32, word: &str, changed: &[String]) -> Found {
    search(repo, side, &Query { file, line, word, changed }).expect("search runs")
}

fn places(found: &Found) -> Vec<String> {
    found.candidates().map(|c| format!("{}:{}", c.path, c.line)).collect()
}

#[test]
fn worktree_side_sees_edits_and_untracked_files() {
    let Some(repo) = TestRepo::new("worktree") else { return };
    repo.write("src/lib.rs", "pub fn charge(amount: u64) -> u64 {\n    amount\n}\n");
    repo.write("src/main.rs", "fn main() {\n    let total = charge(3);\n}\n");
    repo.commit_all("one");
    repo.write("src/extra.rs", "fn twice() -> u64 { charge(2) * 2 }\n"); // untracked
    repo.write(".gitignore", "ignored.rs\n");
    repo.write("ignored.rs", "fn charge() {}\n"); // ignored: rg would not see it either

    let found = find(&repo.dir, &SideSource::Worktree, "src/main.rs", 2, "charge", &[]);
    assert_eq!(found.definitions.len(), 1);
    let def = &found.definitions[0];
    assert_eq!(
        (def.path.as_str(), def.line, def.column, def.kind, def.symbol),
        ("src/lib.rs", 1, 7, Kind::Definition, Some(SymbolKind::Function))
    );
    // The origin file's own use first (+1000), then the untracked file (same folder, +200).
    assert_eq!(places(&found), ["src/lib.rs:1", "src/main.rs:2", "src/extra.rs:1"]);
    assert!(found.complete);
    let query = Query { file: "src/main.rs", line: 2, word: "charge", changed: &[] };
    assert_eq!(found.best(&query).map(|c| c.path.as_str()), Some("src/lib.rs"));
}

#[test]
fn staged_side_sees_the_index_not_the_disk() {
    let Some(repo) = TestRepo::new("staged") else { return };
    repo.write("a.py", "def old_name():\n    pass\n");
    repo.commit_all("one");
    repo.write("a.py", "def staged_name():\n    pass\n");
    repo.git(&["add", "a.py"]);
    repo.write("a.py", "def disk_name():\n    pass\n");

    let staged = |word| find(&repo.dir, &SideSource::Index, "a.py", 1, word, &[]).definitions.len();
    assert_eq!((staged("staged_name"), staged("disk_name"), staged("old_name")), (1, 0, 0));
    let disk = find(&repo.dir, &SideSource::Worktree, "a.py", 1, "disk_name", &[]);
    assert_eq!(disk.definitions.len(), 1);
}

#[test]
fn a_revision_side_searches_that_version() {
    let Some(repo) = TestRepo::new("rev") else { return };
    repo.write("pkg/server.go", "package pkg\n\nfunc Start() error { return nil }\n");
    let first = repo.commit_all("one");
    repo.write("pkg/server.go", "package pkg\n\nfunc Begin() error { return nil }\n");
    repo.commit_all("two");

    let old = SideSource::Rev(first.clone());
    let found = find(&repo.dir, &old, "pkg/server.go", 3, "Start", &[]);
    assert_eq!(places(&found), ["pkg/server.go:3"]);
    assert_eq!(found.definitions[0].symbol, Some(SymbolKind::Function));
    assert_eq!(
        places(&find(&repo.dir, &SideSource::Rev("HEAD".into()), "pkg/server.go", 3, "Start", &[])),
        Vec::<String>::new()
    );
    // A revision that looks like an option is refused, not passed to git.
    assert!(
        search(
            &repo.dir,
            &SideSource::Rev("--output=x".into()),
            &Query { file: "a", line: 1, word: "x", changed: &[] }
        )
        .is_err()
    );
}

#[test]
fn a_bare_repository_is_searched_by_revision() {
    let Some(repo) = TestRepo::new("bare") else { return };
    repo.write("src/gateway.ts", "export class Gateway {\n  charge() {}\n}\n");
    repo.write("src/use.ts", "import { Gateway } from './gateway';\nnew Gateway();\n");
    let head = repo.commit_all("one");
    let bare = repo.dir.with_file_name("cache.git");
    repo.git_in(repo.dir.parent().expect("base"), &["clone", "-q", "--bare", "repo", "cache.git"]);

    let found = find(&bare, &SideSource::Rev(head), "src/use.ts", 2, "Gateway", &["src/use.ts".to_owned()]);
    assert_eq!(found.definitions.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["src/gateway.ts"]);
    assert_eq!(found.references.len(), 2);
    // No working tree: the worktree side is an error, not a silent empty answer.
    assert!(
        search(&bare, &SideSource::Worktree, &Query { file: "a", line: 1, word: "x", changed: &[] }).is_err()
    );
}

#[test]
fn the_language_limits_files_and_hidden_and_excluded_dirs_are_skipped() {
    let Some(repo) = TestRepo::new("filters") else { return };
    repo.write("src/a.ts", "export function render() {}\n");
    repo.write("src/a.tsx", "render();\n");
    repo.write("docs/notes.md", "render\n");
    repo.write("node_modules/dep/index.ts", "export function render() {}\n");
    repo.write("dist/out.ts", "function render() {}\n");
    repo.write("lib/dist/kept.ts", "render();\n");
    repo.write(".github/script.ts", "render();\n");
    repo.commit_all("one");

    let found = find(&repo.dir, &SideSource::Worktree, "src/a.ts", 1, "render", &[]);
    let mut paths: Vec<String> = found.candidates().map(|c| c.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, ["lib/dist/kept.ts", "src/a.ts", "src/a.tsx"]);
    // With no language every file is searched.
    let any = find(&repo.dir, &SideSource::Worktree, "README", 1, "render", &[]);
    assert!(any.candidates().any(|c| c.path == "docs/notes.md"));
}

#[test]
fn whole_words_only() {
    let Some(repo) = TestRepo::new("words") else { return };
    repo.write("a.rs", "fn foo() {}\nfn foobar() {}\nlet x = foo_bar + foo;\n");
    repo.commit_all("one");
    let found = find(&repo.dir, &SideSource::Worktree, "a.rs", 1, "foo", &[]);
    assert_eq!(places(&found), ["a.rs:1", "a.rs:3"]);
    assert_eq!(found.references[0].column, 18);
}

#[test]
fn per_file_and_result_caps_hold() {
    let Some(repo) = TestRepo::new("caps") else { return };
    repo.write("big.txt", &"needle\n".repeat(80));
    repo.commit_all("one");
    // 80 matches in one file, 50 kept: exactly the ranking cap, so nothing more was dropped.
    let found = find(&repo.dir, &SideSource::Worktree, "x.txt", 1, "needle", &[]);
    assert_eq!((found.references.len(), found.capped, found.complete), (PER_FILE_CAP, false, true));

    for i in 0..12 {
        repo.write(&format!("f{i:02}.txt"), &"needle\n".repeat(50));
    }
    repo.commit_all("two");
    // 650 matches: the parse cap of 500 stops git, and ranking keeps 50.
    let found = find(&repo.dir, &SideSource::Worktree, "x.txt", 1, "needle", &[]);
    assert_eq!((found.references.len(), found.capped, found.complete), (RESULT_CAP, true, false));
}

#[test]
fn out_of_time_answers_with_what_it_has() {
    let Some(repo) = TestRepo::new("timeout") else { return };
    repo.write("a.rs", "fn foo() {}\n");
    repo.commit_all("one");
    let query = Query { file: "a.rs", line: 1, word: "foo", changed: &[] };
    let found = search_with(&repo.dir, &SideSource::Worktree, &query, Duration::ZERO).expect("no error");
    assert!(!found.complete);
    let found = search_with(&repo.dir, &SideSource::Worktree, &query, HOVER_TIMEOUT).expect("runs");
    assert_eq!((found.definitions.len(), found.complete), (1, true));
}

#[test]
fn an_empty_word_finds_nothing_without_running_git() {
    let found = search(
        Path::new("/nonexistent"),
        &SideSource::Worktree,
        &Query { file: "a", line: 1, word: " ", changed: &[] },
    )
    .expect("no git needed");
    assert_eq!(found.candidates().count(), 0);
}

// --- The browser's "resolveCodeNav against real ripgrep (#1558)" cases, through git grep ---

fn java_repo() -> Option<TestRepo> {
    let repo = TestRepo::new("java")?;
    let source = |class: &str| {
        format!("public class {class} {{\n    public String fetchContent() {{ return \"example\"; }}\n}}\n")
    };
    repo.write("src/main/java/com/example/vendor/app/ExampleService.java", &source("ExampleService"));
    repo.write("vendor/third_party/Vendored.java", &source("Vendored"));
    repo.write("src/app/node_modules/dep/Dep.java", &source("Dep"));
    repo.write("src/other/Other.java", &source("Other"));
    repo.commit_all("one");
    Some(repo)
}

fn java_paths(repo: &TestRepo, origin: &str) -> Vec<String> {
    find(&repo.dir, &SideSource::Worktree, origin, 1, "fetchContent", &[])
        .candidates()
        .map(|c| c.path.clone())
        .collect()
}

#[test]
fn java_vendor_package_and_exclusions() {
    let Some(repo) = java_repo() else { return };
    let found = java_paths(&repo, "src/main/java/com/example/Caller.java");
    // finds a symbol in a Java package containing a `vendor` segment
    assert!(found.iter().any(|p| p.contains("com/example/vendor/app/ExampleService.java")));
    // a root-level vendor/ directory is still excluded
    assert!(!found.iter().any(|p| p.contains("vendor/third_party/")));
    // node_modules is excluded at any depth
    assert!(!found.iter().any(|p| p.contains("node_modules")));

    // a request from inside an excluded segment can find its own siblings
    let found = java_paths(&repo, "src/app/node_modules/dep/Caller.java");
    assert!(found.iter().any(|p| p.contains("node_modules/dep/Dep.java")));

    // a first-party `vendor` package does not un-exclude the root vendor tree
    let found = java_paths(&repo, "src/main/java/com/example/vendor/app/ExampleService.java");
    assert!(found.iter().any(|p| p.contains("com/example/vendor/app/ExampleService.java")));
    assert!(found.iter().any(|p| p.contains("src/other/")));
    assert!(!found.iter().any(|p| p.contains("vendor/third_party/")));

    // a file that really lives under root vendor/ still finds its own siblings
    let found = java_paths(&repo, "vendor/third_party/Caller.java");
    assert!(found.iter().any(|p| p.contains("vendor/third_party/Vendored.java")));
    assert!(!found.iter().any(|p| p.contains("node_modules")));
}
