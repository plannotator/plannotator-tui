//! Command Code: pi's entries under `~/.commandcode/projects/<slug>/<uuid>.jsonl`, chosen by
//! the cwd its `session` header records.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_hosts::{Host, HostError, Role, commandcode, pi, session_id_of, sniff};

const FIXTURE_ID: &str = "0198f0a1-7c2e-7b10-9a4d-3c5e8f1a2b3c";
const OLD: &str = "11111111-1111-4111-8111-111111111111";
const NEW: &str = "22222222-2222-4222-8222-222222222222";
const EMPTY: &str = "33333333-3333-4333-8333-333333333333";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/commandcode")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-cmd-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A session file: the header names `cwd`; one assistant reply saying `reply` unless empty.
fn session(dir: &Path, id: &str, cwd: &str, reply: &str, modified: u64) -> PathBuf {
    std::fs::create_dir_all(dir).expect("project dir");
    let header = format!(r#"{{"type":"session","version":3,"id":"{id}","timestamp":"t","cwd":"{cwd}"}}"#);
    let message = format!(
        r#"{{"type":"message","id":"m-{reply}","parentId":null,"message":{{"role":"assistant","content":[{{"type":"text","text":"{reply}"}}]}}}}"#
    );
    let text = if reply.is_empty() { format!("{header}\n") } else { format!("{header}\n{message}\n") };
    let path = dir.join(format!("{id}.jsonl"));
    std::fs::write(&path, text).expect("write");
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified);
    std::fs::File::options().write(true).open(&path).expect("open").set_modified(at).expect("mtime");
    path
}

fn newest_reply(path: &Path) -> String {
    let text = std::fs::read_to_string(path).expect("session");
    commandcode::parse_messages(&text, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("an assistant reply")
        .text
}

/// Expected values come from `@sindresorhus/slugify` 2.2.1, the library Command Code uses.
#[test]
fn the_slug_is_command_codes_slugify_of_the_cwd() {
    let cases = [
        ("/Users/me/proj", "users-me-proj"),
        ("/Users/Me/My Proj", "users-me-my-proj"),
        ("/Users/me/herdrAnnotate", "users-me-herdr-annotate"),
        ("/Users/me/my.app_v2", "users-me-my-app-v2"),
        ("/Users/me/Bob's stuff", "users-me-bobs-stuff"),
        ("/w/a't", "w-at"),
        ("/w/R&D", "w-r-and-d"),
        ("/w/APIs", "w-apis"),
        ("/w/XMLHttpRequest", "w-xml-http-request"),
        ("/w/ABC123def", "w-abc-123def"),
        ("/w/fooBARBaz", "w-foo-bar-baz"),
        ("/w/ab1Cx", "w-ab1-cx"),
        ("/w/IOS17App", "w-ios-17-app"),
        ("/w/v2ToV3", "w-v2-to-v3"),
        ("/w/--x--/", "w-x"),
        (r"C:\Users\Me\Proj", "c-users-me-proj"),
        ("/", "root"),
    ];
    for (cwd, slug) in cases {
        assert_eq!(commandcode::project_slug(Path::new(cwd)), slug, "{cwd}");
    }
}

#[test]
fn the_fixture_resolves_by_its_slug_and_reads_the_last_reply_like_pi() {
    let found = commandcode::find_transcript(&fixtures(), Path::new("/Users/me/MyProj")).expect("session");
    assert_eq!(found.file_stem().and_then(|s| s.to_str()), Some(FIXTURE_ID));
    let text = std::fs::read_to_string(&found).expect("fixture");
    let messages = commandcode::parse_messages(&text, 25);
    assert_eq!(messages, pi::parse_messages(&text, 25));
    let replies: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Assistant).map(|m| m.text.as_str()).collect();
    // The trailing `session_info` entry is on the branch but is not a message; the
    // toolCall-only reply has no text and takes no slot.
    assert_eq!(
        replies,
        ["# Done\n\nassistant text 2, longer than the viewport\nsecond block", "assistant text 1"]
    );
    // A path handed over without a host is read with pi's reader, with the same result.
    assert_eq!(sniff(&text), Some(Host::Pi));
    assert_eq!(session_id_of(Host::CommandCode, &found).as_deref(), Some(FIXTURE_ID));
}

#[test]
fn the_last_written_session_with_a_reply_wins() {
    let root = temp_dir("newest");
    let dir = root.join("w-proj");
    session(&dir, OLD, "/w/proj", "resumed", 1_000_060);
    session(&dir, NEW, "/w/proj", "idle", 1_000_000);
    session(&dir, EMPTY, "/w/proj", "", 1_000_120);
    let found = commandcode::find_transcript(&root, Path::new("/w/proj")).expect("session");
    assert_eq!(newest_reply(&found), "resumed", "last write wins and a session without messages is skipped");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn a_session_recorded_for_another_cwd_is_never_chosen() {
    let root = temp_dir("verify");
    // `/w/My Proj` and `/w/my-proj` share a slug; only the header tells them apart.
    let dir = root.join("w-my-proj");
    session(&dir, OLD, "/w/My Proj", "mine", 1_000_000);
    session(&dir, NEW, "/w/my-proj", "theirs, newer", 1_000_060);
    let found = commandcode::find_transcript(&root, Path::new("/w/My Proj/")).expect("session");
    assert_eq!(newest_reply(&found), "mine");
    assert_eq!(commandcode::find_transcript(&root, Path::new("/w/elsewhere")), None, "no unrelated fallback");
    // Sidecars and files not named by a uuid are not sessions, whatever their first line says.
    session(&dir, &format!("{OLD}.checkpoints"), "/w/x", "sidecar", 1_000_120);
    session(&dir, "notes", "/w/x", "notes", 1_000_120);
    assert_eq!(commandcode::find_transcript(&root, Path::new("/w/x")), None);
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn a_session_outside_the_computed_slug_is_found_by_its_header() {
    let root = temp_dir("scan");
    // slugify transliterates `ü` to `ue`; we do not, so this is found by the header scan.
    session(&root.join("w-ueber"), OLD, "/w/über", "found by header", 1_000_000);
    session(&root.join("w-other"), NEW, "/w/other", "unrelated", 1_000_060);
    let found = commandcode::find_transcript(&root, Path::new("/w/über")).expect("session");
    assert_eq!(newest_reply(&found), "found by header");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn an_exact_id_prefers_the_cwds_slug_directory_then_any_project() {
    let root = temp_dir("exact");
    let here = session(&root.join("w-proj"), OLD, "/w/proj", "here", 1_000_000);
    session(&root.join("a-elsewhere"), OLD, "/a/elsewhere", "elsewhere", 1_000_060);
    let other = session(&root.join("a-elsewhere"), NEW, "/a/elsewhere", "other", 1_000_060);
    let find =
        |cwd: &str, id: &str| commandcode::find_transcript_by_id(&root, Path::new(cwd), id).expect("valid");
    assert_eq!(find("/w/proj", OLD), Some(here));
    assert_eq!(find("/w/proj", NEW), Some(other));
    assert_eq!(find("/w/proj", EMPTY), None);
    assert!(matches!(
        commandcode::find_transcript_by_id(&root, Path::new("/w/proj"), "../w-proj/x"),
        Err(HostError::InvalidSessionId(_))
    ));
    std::fs::remove_dir_all(&root).expect("cleanup");
}
