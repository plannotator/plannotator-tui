//! Antigravity CLI: `brain/<id>/.system_generated/logs/transcript_full.jsonl`, matched to a cwd
//! through `conversation_summaries.db` and `history.jsonl`.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_hosts::{Host, HostError, Role, antigravity, session_id_of, sniff};
use rusqlite::{Connection, params};

const FIXTURE_ID: &str = "4f1c9e2a-7b3d-4c8e-9a10-2b6d8e0f1a3c";
const OLD: &str = "11111111-1111-4111-8111-111111111111";
const NEW: &str = "22222222-2222-4222-8222-222222222222";
const OTHER: &str = "33333333-3333-4333-8333-333333333333";
const CHILD: &str = "44444444-4444-4444-8444-444444444444";

/// The columns this reader touches, as `agy` 1.2.7 declares them.
const SCHEMA: &str = "CREATE TABLE `conversation_summaries` (`conversation_id` text, \
    `last_modified_time` datetime NOT NULL, `workspace_uris` text NOT NULL, \
    `parent_conversation_id` text NOT NULL DEFAULT \"\", PRIMARY KEY (`conversation_id`))";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/antigravity")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-agy-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A conversation whose one reply says `reply` (none when empty), last written at `modified`.
fn conversation(app: &Path, id: &str, reply: &str, modified: u64) -> PathBuf {
    let path = antigravity::transcript_path(app, id);
    std::fs::create_dir_all(path.parent().expect("logs")).expect("logs dir");
    let prompt = r#"{"step_index":0,"type":"USER_INPUT","content":"<USER_REQUEST>\nhi\n</USER_REQUEST>"}"#;
    let text = if reply.is_empty() {
        format!("{prompt}\n")
    } else {
        format!("{prompt}\n{{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"content\":\"{reply}\"}}\n")
    };
    std::fs::write(&path, text).expect("transcript");
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified);
    std::fs::File::options().write(true).open(&path).expect("open").set_modified(at).expect("mtime");
    path
}

fn summaries(app: &Path, rows: &[(&str, &str, &str)]) {
    let c = Connection::open(app.join(antigravity::SUMMARIES_DB)).expect("db");
    c.execute(SCHEMA, []).expect("schema");
    for (id, uris, parent) in rows {
        c.execute(
            "INSERT INTO conversation_summaries (conversation_id, last_modified_time, workspace_uris, \
             parent_conversation_id) VALUES (?1, '2026-10-21 06:00:00+00:00', ?2, ?3)",
            params![id, uris, parent],
        )
        .expect("row");
    }
}

fn reply(path: &Path) -> String {
    let text = std::fs::read_to_string(path).expect("transcript");
    antigravity::parse_messages(&text, 25)
        .into_iter()
        .find(|m| m.role == Role::Assistant)
        .expect("a reply")
        .text
}

#[test]
fn replies_are_planner_responses_with_content_newest_first() {
    let path = antigravity::transcript_path(&fixtures(), FIXTURE_ID);
    let text = std::fs::read_to_string(&path).expect("fixture");
    let messages = antigravity::parse_messages(&text, 25);
    let replies: Vec<(&str, &str)> = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .map(|m| (m.id.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        replies,
        [
            ("7", "Done: both settings are applied in `retroarch.cfg`."),
            (
                "3",
                "Here is the recommended configuration update:\n\n### Changes\n- Set `video_threaded = \"false\"`\n- Set `video_vsync = \"true\"`"
            ),
        ],
        "tool-call-only steps and tool results are not replies"
    );
    let prompts = messages.iter().filter(|m| m.role == Role::Human).count();
    assert_eq!(prompts, 2);
    assert_eq!(messages.first().and_then(|m| m.at.as_deref()), Some("2026-10-21T06:01:15Z"));
    assert_eq!(antigravity::parse_messages(&text, 1).len(), 1, "pick caps the replies");
}

/// The newest step is a prompt still being answered: one pick is still the reply before it.
#[test]
fn n_counts_replies_so_a_waiting_prompt_does_not_hide_the_last_reply() {
    let path = antigravity::transcript_path(&fixtures(), FIXTURE_ID);
    let text = std::fs::read_to_string(&path).expect("fixture");
    let waiting = format!("{text}{}\n", r#"{"step_index":8,"type":"USER_INPUT","content":"and the audio?"}"#);
    let newest = antigravity::parse_messages(&waiting, 1);
    let roles: Vec<Role> = newest.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::Human, Role::Assistant]);
    assert_eq!(
        newest.last().map(|m| m.text.as_str()),
        Some("Done: both settings are applied in `retroarch.cfg`.")
    );
}

#[test]
fn a_half_written_last_line_is_skipped() {
    let text = "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"content\":\"whole\"}\n{\"step_index\":2,\"type\":\"PLANNER_RESPONSE\",\"cont";
    let messages = antigravity::parse_messages(text, 25);
    assert_eq!(messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["whole"]);
}

#[test]
fn an_id_names_its_transcript_directly() {
    let found = antigravity::find_transcript_by_id(&fixtures(), FIXTURE_ID).expect("valid id");
    assert_eq!(found, Some(antigravity::transcript_path(&fixtures(), FIXTURE_ID)));
    assert_eq!(antigravity::find_transcript_by_id(&fixtures(), NEW).expect("valid id"), None);
    assert!(matches!(
        antigravity::find_transcript_by_id(&fixtures(), "../brain"),
        Err(HostError::InvalidSessionId(_))
    ));
}

#[test]
fn the_transcript_path_carries_the_conversation_id_and_its_format_is_sniffed() {
    let path = antigravity::transcript_path(&fixtures(), FIXTURE_ID);
    assert_eq!(session_id_of(Host::Antigravity, &path).as_deref(), Some(FIXTURE_ID));
    let elsewhere = Path::new("/tmp").join(FIXTURE_ID).join("transcript_full.jsonl");
    assert_eq!(session_id_of(Host::Antigravity, &elsewhere), None, "only brain/<id>/.system_generated/logs");
    let head = std::fs::read_to_string(&path).expect("fixture");
    assert_eq!(sniff(&head), Some(Host::Antigravity));
}

#[test]
fn the_database_picks_the_newest_conversation_started_in_the_cwd() {
    let app = temp_dir("db");
    conversation(&app, OLD, "old", 1_000);
    conversation(&app, NEW, "new", 2_000);
    conversation(&app, OTHER, "other folder", 9_000);
    conversation(&app, CHILD, "subagent", 9_500);
    summaries(
        &app,
        &[
            (OLD, r#"["file:///w/my%20proj"]"#, ""),
            (NEW, r#"["file:///w/elsewhere","file:///w/my%20proj/"]"#, ""),
            (OTHER, r#"["file:///w/other"]"#, ""),
            (CHILD, r#"["file:///w/my%20proj"]"#, NEW),
        ],
    );
    let found = antigravity::find_transcript(&app, Path::new("/w/my proj")).expect("found");
    assert_eq!(reply(&found), "new", "newest by last write; subagents and other folders never");
    let inside = antigravity::find_transcript(&app, Path::new("/w/other/src")).expect("found");
    assert_eq!(reply(&inside), "other folder", "a workspace that contains the cwd counts");
    let err = antigravity::find_transcript(&app, Path::new("/w/none")).expect_err("no conversation");
    assert!(err.to_string().contains("no Antigravity conversation started in /w/none"), "{err}");
    std::fs::remove_dir_all(&app).expect("cleanup");
}

#[test]
fn an_exact_workspace_beats_a_newer_one_that_merely_contains_the_cwd() {
    let app = temp_dir("tiers");
    conversation(&app, OLD, "exact", 1_000);
    conversation(&app, NEW, "parent", 2_000);
    summaries(&app, &[(OLD, r#"["file:///w/repo/sub"]"#, ""), (NEW, r#"["file:///w/repo"]"#, "")]);
    let found = antigravity::find_transcript(&app, Path::new("/w/repo/sub")).expect("found");
    assert_eq!(reply(&found), "exact");
    std::fs::remove_dir_all(&app).expect("cleanup");
}

/// An empty transcript, or a conversation just started whose only step is the prompt, has
/// nothing to annotate: the previous conversation in the folder is the answer.
#[test]
fn a_conversation_without_a_reply_is_passed_over() {
    let app = temp_dir("empty");
    conversation(&app, OLD, "older reply", 1_000);
    let empty = antigravity::transcript_path(&app, NEW);
    std::fs::create_dir_all(empty.parent().expect("logs")).expect("logs dir");
    std::fs::write(&empty, "").expect("empty transcript");
    conversation(&app, OTHER, "", 3_000);
    summaries(
        &app,
        &[
            (OLD, r#"["file:///w/p"]"#, ""),
            (NEW, r#"["file:///w/p"]"#, ""),
            (OTHER, r#"["file:///w/p"]"#, ""),
        ],
    );
    let found = antigravity::find_transcript(&app, Path::new("/w/p")).expect("found");
    assert_eq!(reply(&found), "older reply");
    std::fs::remove_dir_all(&app).expect("cleanup");
}

#[test]
fn without_a_readable_database_only_the_history_can_vouch_for_a_folder() {
    let app = temp_dir("history");
    conversation(&app, OLD, "verified", 1_000);
    conversation(&app, NEW, "unverified but newer", 2_000);
    std::fs::write(app.join(antigravity::SUMMARIES_DB), "not a database").expect("garbage db");
    std::fs::write(
        app.join(antigravity::HISTORY_FILE),
        format!(
            "{{\"display\":\"first prompt\",\"timestamp\":1,\"workspace\":\"/w/p\"}}\n\
             {{\"display\":\"second\",\"timestamp\":2,\"workspace\":\"/w/p\",\"conversationId\":\"{OLD}\"}}\n\
             {{\"display\":\"x\",\"timestamp\":3,\"workspace\":\"/w/q\",\"conversationId\":\"{NEW}\"}}\n"
        ),
    )
    .expect("history");
    let found = antigravity::find_transcript(&app, Path::new("/w/p")).expect("found");
    assert_eq!(reply(&found), "verified");
    std::fs::remove_file(app.join(antigravity::HISTORY_FILE)).expect("drop history");
    let err = antigravity::find_transcript(&app, Path::new("/w/p")).expect_err("nothing vouches");
    let message = err.to_string();
    assert!(message.contains("no Antigravity conversation started in /w/p"), "{message}");
    assert!(message.contains("reading the Antigravity database"), "the unread database is named: {message}");
    std::fs::remove_dir_all(&app).expect("cleanup");
}

#[test]
fn a_database_without_the_subagent_column_still_matches() {
    let app = temp_dir("legacy");
    conversation(&app, OLD, "legacy", 1_000);
    let c = Connection::open(app.join(antigravity::SUMMARIES_DB)).expect("db");
    c.execute(
        "CREATE TABLE conversation_summaries (conversation_id text, workspace_uris text, last_modified_time datetime)",
        [],
    )
    .expect("schema");
    c.execute(
        "INSERT INTO conversation_summaries VALUES (?1, '[\"file:///w/p\"]', '2026-10-21 06:00:00+00:00')",
        params![OLD],
    )
    .expect("row");
    drop(c); // Windows cannot remove a database file that is still open.
    let found = antigravity::find_transcript(&app, Path::new("/w/p")).expect("found");
    assert_eq!(reply(&found), "legacy");
    std::fs::remove_dir_all(&app).expect("cleanup");
}

#[test]
fn ids_from_the_database_are_never_joined_as_paths_unchecked() {
    let app = temp_dir("traversal");
    conversation(&app, OLD, "real", 1_000);
    summaries(&app, &[("../../etc", r#"["file:///w/p"]"#, ""), (OLD, r#"["file:///w/p"]"#, "")]);
    let found = antigravity::find_transcript(&app, Path::new("/w/p")).expect("found");
    assert_eq!(reply(&found), "real");
    std::fs::remove_dir_all(&app).expect("cleanup");
}

#[test]
fn file_uris_become_local_paths() {
    let cases = [
        ("file:///Users/me/my%20proj", Some("/Users/me/my proj")),
        ("file://localhost/w/p", Some("/w/p")),
        ("file:///C:/Users/Me/Proj", Some("C:/Users/Me/Proj")),
        ("file:///c%3A/Users/Me/Proj", Some("c:/Users/Me/Proj")),
        ("file:///w/%E3%83%97%E3%83%AD", Some("/w/プロ")),
        ("file://server/share/p", None),
        ("vscode-remote://ssh-remote+box/w/p", None),
    ];
    for (uri, path) in cases {
        assert_eq!(antigravity::path_of_file_uri(uri).as_deref(), path, "{uri}");
    }
}

#[cfg(windows)]
#[test]
fn on_windows_a_workspace_matches_the_cwd_up_to_case_and_slashes() {
    let app = temp_dir("windows");
    conversation(&app, OLD, "windows", 1_000);
    summaries(&app, &[(OLD, r#"["file:///c%3A/Users/Me/Proj"]"#, "")]);
    let found = antigravity::find_transcript(&app, Path::new(r"C:\Users\me\proj\")).expect("found");
    assert_eq!(reply(&found), "windows");
    std::fs::remove_dir_all(&app).expect("cleanup");
}
