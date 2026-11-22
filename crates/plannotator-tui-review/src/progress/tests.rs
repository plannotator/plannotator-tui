//! Viewed marks against the browser's algorithm.
//!
//! The expected keys, identities and fingerprints below were derived from the browser
//! source by hand and cross-checked by running its hashing code: the bodies of
//! `parseRawDiffEntries`, `resolveReviewFileIdentities` (record + index-line check),
//! `localGitReviewIdentity` and `hash` copied into a standalone Node script, run on a scratch
//! repository built exactly as [`fixture`] builds it. Blob ids depend only on content, so
//! every identity and fingerprint is the same on any machine; only a key that includes the
//! temp folder's real path is rebuilt from that path here.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "tests assert by panicking")]
#![allow(clippy::print_stderr, reason = "a skipped test says why")]

use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
#[cfg(unix)]
use crate::capture::Repo;

/// `sha256(JSON.stringify(["git","/private/var/tmp/x","refs/heads/main",["uncommitted",null]]))`.
/// The JSON is `["git","/private/var/tmp/x","refs/heads/main",["uncommitted",null]]`: no
/// spaces, the base `null` because `uncommitted` is not a base-relative diff type.
const KEY_UNCOMMITTED: &str = "bd7c32c1a35025fd223e5b1cbe947aaacbc8471d9d02bd4516243a2be5616c93";
/// `sha256('["git","/private/var/tmp/x","refs/heads/feat",["since-base","origin/main"]]')`.
const KEY_SINCE_BASE: &str = "f49f2b185e1ac3d014f285bb559de041041f50f8b5324f2d22512b56d0fc4f19";
/// `sha256('["pr","https://github.com/o/r/pull/12","layer"]')`.
const KEY_PR: &str = "296b6d97ab8c0ba6a15d2538812abf4ba60ffc62a2f768a4dd0ae8b55435020c";

#[test]
fn keys_hash_the_identity_as_json_stringify_writes_it() {
    let uncommitted = json!(["git", "/private/var/tmp/x", "refs/heads/main", ["uncommitted", null]]);
    assert_eq!(
        uncommitted.to_string(),
        r#"["git","/private/var/tmp/x","refs/heads/main",["uncommitted",null]]"#
    );
    assert_eq!(review_key(&uncommitted), KEY_UNCOMMITTED);
    let since = json!(["git", "/private/var/tmp/x", "refs/heads/feat", ["since-base", "origin/main"]]);
    assert_eq!(review_key(&since), KEY_SINCE_BASE);
    assert_eq!(review_key(&pull_identity("https://github.com/o/r/pull/12")), KEY_PR);
}

#[test]
fn json_escapes_match_json_stringify() {
    // JSON.stringify(["tab\there","quote\"","back\\slash","ctl\u001f","ü/€",null]): short
    // escapes for \t \" \\, \u00XX in lowercase hex for other controls, `/` and non-ASCII
    // as is.
    let value = json!(["tab\there", "quote\"", "back\\slash", "ctl\u{1f}", "ü/€", null]);
    assert_eq!(value.to_string(), r#"["tab\there","quote\"","back\\slash","ctl\u001f","ü/€",null]"#);
}

#[test]
fn records_are_named_by_the_paths_hash_and_written_in_the_browsers_field_order() {
    // sha256("src/a b/ü.txt"), UTF-8 bytes.
    assert_eq!(
        record_path(Path::new("/d"), "src/a b/ü.txt"),
        PathBuf::from("/d/add7fac2c2354369adb0439ab90b857068b3c6b5b50b027608eb28d8bc454b44.json")
    );
    let dir = scratch("record");
    save_mark(&dir, "a.txt", "f", true).expect("save");
    let text = std::fs::read_to_string(record_path(&dir, "a.txt")).expect("record");
    // JSON.stringify({ v: 1, path, fingerprint, viewed, suppressed: !viewed })
    assert_eq!(text, r#"{"v":1,"path":"a.txt","fingerprint":"f","viewed":true,"suppressed":false}"#);
    save_mark(&dir, "a.txt", "f", false).expect("save");
    let text = std::fs::read_to_string(record_path(&dir, "a.txt")).expect("record");
    assert_eq!(text, r#"{"v":1,"path":"a.txt","fingerprint":"f","viewed":false,"suppressed":true}"#);
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("dir")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| Path::new(n).extension().is_some_and(|e| e.eq_ignore_ascii_case("tmp")))
        .collect();
    assert_eq!(leftovers, Vec::<String>::new());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(record_path(&dir, "a.txt")).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn a_record_counts_only_when_valid_and_its_fingerprint_matches() {
    let dir = scratch("load");
    let fingerprints: BTreeMap<String, String> =
        [("a", "fa"), ("b", "fb"), ("c", "fc"), ("d", "fd"), ("e", "fe"), ("f", "ff")]
            .into_iter()
            .map(|(p, f)| (p.to_owned(), f.to_owned()))
            .collect();
    let write = |path: &str, text: &str| std::fs::write(record_path(&dir, path), text).expect("write");
    std::fs::create_dir_all(&dir).expect("dir");
    write("a", r#"{"v":1,"path":"a","fingerprint":"fa","viewed":true,"suppressed":false}"#);
    write("b", r#"{"v":1,"path":"b","fingerprint":"old","viewed":true,"suppressed":false}"#);
    write("c", r#"{"v":1,"path":"other","fingerprint":"fc","viewed":true,"suppressed":false}"#);
    write("d", r#"{"v":2,"path":"d","fingerprint":"fd","viewed":true,"suppressed":false}"#);
    write("e", r#"{"v":1,"path":"e","fingerprint":"fe","viewed":true}"#);
    write("f", "{not json");
    let viewed: Vec<String> = load_viewed(&dir, &fingerprints).into_iter().collect();
    assert_eq!(viewed, ["a"]);
}

/// Unix only: Windows refuses to rename over a file another writer is renaming at that
/// instant, so there the losing write fails instead of being overwritten.
#[cfg(unix)]
#[test]
fn concurrent_writers_leave_one_whole_record() {
    let dir = scratch("race");
    let writers: Vec<_> = (0..4)
        .map(|n| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                for i in 0..50 {
                    save_mark(&dir, "a.txt", &format!("f{n}"), (i + n) % 2 == 0).expect("save");
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("writer");
    }
    let text = std::fs::read_to_string(record_path(&dir, "a.txt")).expect("record");
    let value: Value = serde_json::from_str(&text).expect("whole JSON");
    assert_eq!(value["v"], 1);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("dir")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    assert_eq!(names.len(), 1, "no temp files left: {names:?}");
}

#[test]
fn the_setting_follows_the_browsers_order() {
    assert!(enabled(None, None));
    assert!(!enabled(Some("off"), None));
    assert!(!enabled(Some(" Disabled "), Some(r#"{"reviewProgress":true}"#)));
    assert!(enabled(Some("1"), Some(r#"{"reviewProgress":false}"#)));
    assert!(!enabled(None, Some(r#"{"reviewProgress":false}"#)));
    assert!(!enabled(Some("maybe"), Some("\u{feff}{\"reviewProgress\":\"0\"}")));
    assert!(enabled(None, Some(r#"{"reviewProgress":"sometimes"}"#)));
    assert!(enabled(None, Some("not json")));
}

#[test]
fn raw_records_read_as_parse_raw_diff_entries_does() {
    let z40 = "0".repeat(40);
    let raw = format!(
        ":100644 100644 {a} {z40} M\0a.txt\0:000000 100644 {z40} {b} A\0n.txt\0\
         :100644 000000 {a} {z40} D\0d.txt\0:100644 100644 {a} {a} R087\0old name\0new name\0",
        a = "1".repeat(40),
        b = "2".repeat(40)
    );
    let entries = parse_raw(raw.as_bytes());
    let paths: Vec<(Option<&str>, Option<&str>, &str)> =
        entries.iter().map(|e| (e.old_path.as_deref(), e.new_path.as_deref(), e.status.as_str())).collect();
    assert_eq!(
        paths,
        [
            (Some("a.txt"), Some("a.txt"), "M"),
            (None, Some("n.txt"), "A"),
            (Some("d.txt"), None, "D"),
            (Some("old name"), Some("new name"), "R087"),
        ]
    );
    assert_eq!(entries[2].path(), Some("d.txt"));
    assert_eq!(
        entries[1].identity("x"),
        format!(r#"[null,"n.txt","000000","100644","{z40}","x"]"#),
        "an added file's old path is null, not missing"
    );
}

/// What the Node cross-check printed for [`fixture`]'s working tree: the identity
/// `JSON.stringify([oldPath, newPath, oldMode, newMode, oldOid, newOid])` per file.
/// - `a.txt`: modified; the worktree side is hashed (`git hash-object` of "a\nb\n").
/// - `gone.txt`: deleted; new path null, new mode `000000`, new id all zeros.
/// - `new.txt`: `git mv old.txt new.txt`, an unchanged rename (`R100`), both paths kept.
/// - `run.sh`: mode only (755 to 644); no index line, so the hash must equal the old id.
/// - `link`, `u.txt`, `ux.sh`: untracked (`--no-index` against /dev/null): old path null,
///   old mode `000000`, mode from the file (symlink `120000`, executable `100755`), the
///   symlink hashed from its target text "a.txt".
/// - `ignored.log` is ignored and has none.
#[cfg(unix)]
const UNCOMMITTED_IDENTITIES: [(&str, &str, &str); 7] = [
    (
        "a.txt",
        r#"["a.txt","a.txt","100644","100644","78981922613b2afb6025042ff6bd878ac1994e85","422c2b7ab3b3c668038da977e4e93a5fc623169c"]"#,
        "3cf634874b56e302ed90872ee7c33124f8d7982b89fbf6ab556203926f7346eb",
    ),
    (
        "gone.txt",
        r#"["gone.txt",null,"100644","000000","b023018cabc396e7692c70bbf5784a93d3f738ab","0000000000000000000000000000000000000000"]"#,
        "6261ac85725b7eec7bb46faf9ffe07cffb0585f19e8ba1083b70f08f010c47c4",
    ),
    (
        "link",
        r#"[null,"link","000000","120000","0000000000000000000000000000000000000000","8d14cbf983b3fad683171c9418998d9f68340823"]"#,
        "fed44564c3e13b09e27a13ceed1697bc9f01a8864d5eccf8abc3fc0eff46096f",
    ),
    (
        "new.txt",
        r#"["old.txt","new.txt","100644","100644","d73ce63d0c5b9baf534778807bdf49bddb328367","d73ce63d0c5b9baf534778807bdf49bddb328367"]"#,
        "b252df9bd59db1c85b9b831286b48b1255d0e9fc8afb478341eabd887ef3cecc",
    ),
    (
        "run.sh",
        r#"["run.sh","run.sh","100755","100644","587be6b4c3f93f93c489c0111bba5596147a26cb","587be6b4c3f93f93c489c0111bba5596147a26cb"]"#,
        "290f346bd60a727b9e19e57f1f10a2ad6d5d3da4ed5d730d45181b25ff1c0157",
    ),
    (
        "u.txt",
        r#"[null,"u.txt","000000","100644","0000000000000000000000000000000000000000","3e757656cf36eca53338e520d134963a44f793f8"]"#,
        "6b819458ccea3b0480200f46fd4197b6cdf4aff1c1863a2fca5f6121da61cd7a",
    ),
    (
        "ux.sh",
        r#"[null,"ux.sh","000000","100755","0000000000000000000000000000000000000000","1a2485251c33a70432394c93fb89330ef214bfc9"]"#,
        "501f2a617d798c3cd6fddd452a28289afab65326286c7c68ad2ef06c05106a3f",
    ),
];

#[cfg(unix)]
#[test]
fn identities_and_fingerprints_match_the_browser_for_uncommitted_changes() {
    let Some(dir) = fixture("uncommitted") else { return };
    let repo = Repo::discover(&dir).expect("repo");
    let capture = repo.capture(&Scope::Uncommitted).expect("capture");
    let identities = file_identities(&capture);
    let expected: BTreeMap<String, String> =
        UNCOMMITTED_IDENTITIES.iter().map(|(p, id, _)| ((*p).to_owned(), (*id).to_owned())).collect();
    assert_eq!(identities, expected);
    for (path, identity, fingerprint) in UNCOMMITTED_IDENTITIES {
        assert_eq!(sha256(identity), fingerprint, "{path}");
    }
    // ["git", realpath(root), "refs/heads/feat", ["uncommitted", null]]
    let root = std::fs::canonicalize(&dir).expect("real path").to_string_lossy().into_owned();
    let identity = local_identity(&capture.root, &capture.scope).expect("identity");
    assert_eq!(identity, json!(["git", root, "refs/heads/feat", ["uncommitted", null]]));
}

#[cfg(unix)]
#[test]
fn since_base_adds_the_branchs_commits_and_keys_on_the_base() {
    let Some(dir) = fixture("since-base") else { return };
    let repo = Repo::discover(&dir).expect("repo");
    let capture = repo.capture(&Scope::Branch { base: "main".into() }).expect("capture");
    let mut expected: BTreeMap<String, String> =
        UNCOMMITTED_IDENTITIES.iter().map(|(p, id, _)| ((*p).to_owned(), (*id).to_owned())).collect();
    // feat.txt, committed on the branch: the browser's fingerprint
    // 528805709eb46ac2cf0136a54446e3d64d982c55075b331fbd1ecf2c3edec76d.
    let feat = r#"[null,"feat.txt","000000","100644","0000000000000000000000000000000000000000","a7453f07505c42ea8d6fdda75fa91710c81c53d6"]"#;
    expected.insert("feat.txt".into(), feat.into());
    assert_eq!(file_identities(&capture), expected);
    assert_eq!(sha256(feat), "528805709eb46ac2cf0136a54446e3d64d982c55075b331fbd1ecf2c3edec76d");
    let root = std::fs::canonicalize(&dir).expect("real path").to_string_lossy().into_owned();
    let identity = local_identity(&capture.root, &capture.scope).expect("identity");
    assert_eq!(identity, json!(["git", root, "refs/heads/feat", ["since-base", "main"]]));
}

#[cfg(unix)]
#[test]
fn a_mark_is_saved_read_back_and_dropped_when_the_file_changes() {
    let Some(dir) = fixture("roundtrip") else { return };
    let data = scratch("roundtrip-data");
    let repo = Repo::discover(&dir).expect("repo");
    let capture = repo.capture(&Scope::Uncommitted).expect("capture");
    let mut progress = Progress::open(&capture, &data, true);
    let identity = local_identity(&capture.root, &capture.scope).expect("identity");
    let folder = data.join("review-progress").join(review_key(&identity));
    assert_eq!(progress.dir(), Some(folder.as_path()));
    progress.set("a.txt", true).expect("save");
    let record = std::fs::read_to_string(record_path(&folder, "a.txt")).expect("record");
    assert_eq!(
        record,
        r#"{"v":1,"path":"a.txt","fingerprint":"3cf634874b56e302ed90872ee7c33124f8d7982b89fbf6ab556203926f7346eb","viewed":true,"suppressed":false}"#
    );

    let again = Progress::open(&capture, &data, true);
    assert!(again.is_viewed("a.txt"));
    assert_eq!(again.count(capture.files.iter().map(|f| f.path.as_str())), 1);

    // Edit the file: a new capture has a new fingerprint, and the old record no longer counts.
    std::fs::write(dir.join("a.txt"), "a\nb\nc\n").expect("edit");
    let edited = repo.capture(&Scope::Uncommitted).expect("capture");
    let after = again.reopen(&edited, &data);
    assert!(!after.is_viewed("a.txt"));
    assert!(record_path(&folder, "a.txt").is_file(), "the record stays; it just does not match");

    // Turned off: nothing is read or written, marks last for the session.
    let mut off = Progress::open(&capture, &data, false);
    assert_eq!(off.dir(), None);
    assert!(!off.is_viewed("a.txt"));
    off.set("u.txt", true).expect("in memory");
    assert!(off.is_viewed("u.txt"));
    assert!(!record_path(&folder, "u.txt").exists());
}

#[cfg(unix)]
#[test]
fn a_detached_checkout_is_keyed_by_its_commit_and_staged_by_the_index() {
    let Some(dir) = fixture("detached") else { return };
    let sha = git(&dir, &["rev-parse", "HEAD"]);
    git(&dir, &["add", "a.txt"]);
    git(&dir, &["checkout", "-q", "--detach"]);
    let repo = Repo::discover(&dir).expect("repo");
    let capture = repo.capture(&Scope::Staged).expect("capture");
    let root = std::fs::canonicalize(&dir).expect("real path").to_string_lossy().into_owned();
    assert_eq!(
        local_identity(&capture.root, &capture.scope),
        Some(json!(["git", root, sha, ["staged", null]]))
    );
    // Staged: the index blob, no hashing; the staged rename comes along.
    let identities = file_identities(&capture);
    assert_eq!(
        identities.get("a.txt").map(String::as_str),
        Some(UNCOMMITTED_IDENTITIES[0].1),
        "the staged a.txt is the same content as the working copy"
    );
    assert_eq!(identities.get("new.txt").map(String::as_str), Some(UNCOMMITTED_IDENTITIES[3].1));
    assert_eq!(identities.get("u.txt"), None, "staged reviews have no untracked files");
}

#[cfg(unix)]
#[test]
fn a_commit_is_keyed_by_its_full_sha_and_fingerprinted_from_its_blobs() {
    let Some(dir) = fixture("commit") else { return };
    let sha = git(&dir, &["rev-parse", "HEAD"]);
    let repo = Repo::discover(&dir).expect("repo");
    let capture = repo.capture(&Scope::Commit { sha: sha.clone() }).expect("capture");
    let identity = local_identity(&capture.root, &capture.scope).expect("identity");
    assert_eq!(identity[3], json!([format!("commit:{sha}"), null]));
    // feat.txt "feature\n", added by the commit: both ids known, nothing hashed.
    let identities = file_identities(&capture);
    assert_eq!(
        identities.get("feat.txt").map(String::as_str),
        Some(
            r#"[null,"feat.txt","000000","100644","0000000000000000000000000000000000000000","a7453f07505c42ea8d6fdda75fa91710c81c53d6"]"#
        )
    );
}

#[test]
fn pull_request_marks_live_in_this_apps_own_folder() {
    let data = scratch("pull-data");
    let capture = Capture {
        scope: Scope::PullRequest { url: "https://github.com/o/r/pull/12".into(), number: 12 },
        root: scratch("pull-root"),
        patch: Vec::new(),
        patch_key: String::new(),
        files: Vec::new(),
        old: crate::SideSource::Rev("a".into()),
        new: crate::SideSource::Rev("b".into()),
        commit_subject: None,
        pull: None,
    };
    let progress = Progress::open(&capture, &data, true);
    let expected = data.join("clients").join("plannotator-tui").join("review-progress").join(KEY_PR);
    assert_eq!(progress.dir(), Some(expected.as_path()));
}

fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pt-progress-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[cfg(unix)]
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@e", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The cross-check repository: `main` with a.txt, run.sh (755), gone.txt, old.txt and a
/// `.gitignore`; branch `feat` adds feat.txt; then a.txt edited, run.sh made 644, gone.txt
/// deleted, old.txt `git mv`ed to new.txt, and u.txt, ux.sh (755), a symlink `link -> a.txt`
/// and an ignored ignored.log left untracked. `None` when git is missing.
#[cfg(unix)]
fn fixture(tag: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("git is not installed; skipping");
        return None;
    }
    let dir = scratch(tag);
    let write = |path: &str, text: &str| std::fs::write(dir.join(path), text).expect("write");
    let chmod = |path: &str, mode| {
        std::fs::set_permissions(dir.join(path), std::fs::Permissions::from_mode(mode)).expect("chmod");
    };
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "core.autocrlf", "false"]);
    git(&dir, &["config", "core.fileMode", "true"]);
    write("a.txt", "a\n");
    write("run.sh", "x\n");
    chmod("run.sh", 0o755);
    write("gone.txt", "bye\n");
    write("old.txt", "rename me please, this line is long enough to be similar\n");
    write(".gitignore", "ignored.log\n");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "base"]);
    git(&dir, &["checkout", "-q", "-b", "feat"]);
    write("feat.txt", "feature\n");
    git(&dir, &["add", "feat.txt"]);
    git(&dir, &["commit", "-qm", "feat"]);
    write("a.txt", "a\nb\n");
    chmod("run.sh", 0o644);
    std::fs::remove_file(dir.join("gone.txt")).expect("rm");
    git(&dir, &["mv", "old.txt", "new.txt"]);
    write("u.txt", "new\n");
    write("ux.sh", "#!/bin/sh\n");
    chmod("ux.sh", 0o755);
    std::os::unix::fs::symlink("a.txt", dir.join("link")).expect("symlink");
    write("ignored.log", "noise\n");
    Some(dir)
}
