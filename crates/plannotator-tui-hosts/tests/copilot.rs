//! GitHub Copilot CLI: lock-pid session detection, the cwd fallback ladder, and events.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use plannotator_tui_hosts::Role;
use plannotator_tui_hosts::copilot::{find_session, parse_messages};

const EVENTS: &str =
    include_str!("fixtures/copilot/session-state/aaaa1111-0000-4000-8000-000000000001/events.jsonl");

/// A throwaway `$COPILOT_HOME` whose session directories get explicit mtimes.
struct CopilotHome {
    root: PathBuf,
}

impl CopilotHome {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("plannotator-tui-copilot-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("session-state")).expect("state dir");
        Self { root }
    }

    /// A session with a workspace cwd, optional lock pids, and an age in seconds.
    fn session(&self, id: &str, cwd: &str, locks: &[u32], age_secs: u64) -> PathBuf {
        let dir = self.root.join("session-state").join(id);
        fs::create_dir_all(&dir).expect("session dir");
        fs::write(dir.join("workspace.yaml"), format!("id: {id}\ncwd: {cwd}\nclient_name: copilot-agent\n"))
            .expect("workspace");
        fs::write(dir.join("events.jsonl"), EVENTS).expect("events");
        for pid in locks {
            File::create(dir.join(format!("inuse.{pid}.lock"))).expect("lock");
        }
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        File::open(&dir).expect("open").set_modified(when).expect("mtime");
        dir
    }
}

impl Drop for CopilotHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn always_copilot(_: u32) -> bool {
    true
}

fn age(dir: &Path, secs: u64) {
    let when = SystemTime::now() - Duration::from_secs(secs);
    File::open(dir).expect("open").set_modified(when).expect("mtime");
}

#[test]
fn a_lock_held_by_an_ancestor_beats_every_cwd_heuristic() {
    let home = CopilotHome::new("lock");
    home.session("newer-for-cwd", "/w", &[], 10);
    let locked = home.session("locked-elsewhere", "/elsewhere", &[300], 100);
    // 4242 → 4000 → 300: the third hop owns the lock.
    let table = [(4242, 4000), (4000, 300), (300, 1)];
    assert_eq!(find_session(&home.root, Path::new("/w"), &table, 4242, always_copilot), Some(locked));
}

#[test]
fn a_stale_lock_is_skipped_and_the_walk_continues() {
    let home = CopilotHome::new("stale");
    home.session("stale", "/w", &[4000], 10);
    let live = home.session("live", "/w", &[300], 50);
    let table = [(4242, 4000), (4000, 300), (300, 1)];
    // 4000 no longer names a copilot process; 300 does.
    let found = find_session(&home.root, Path::new("/w"), &table, 4242, |pid| pid == 300);
    assert_eq!(found, Some(live));
}

#[test]
fn the_ninth_ancestor_is_out_of_reach() {
    let home = CopilotHome::new("hops");
    let near = home.session("near", "/x", &[12], 10);
    let far = home.session("far", "/x", &[9], 50);
    let table: Vec<(u32, u32)> = (1..=17).map(|p| (p + 1, p)).collect(); // 18 → 17 → … → 2
    let only_nine_is_live = |pid: u32| pid == 9;
    // From 18 the walk reaches 11; the stale lock at 12 is dropped and 9 is never seen, so
    // the cwd ladder decides: the newest active session.
    assert_eq!(find_session(&home.root, Path::new("/none"), &table, 18, only_nine_is_live), Some(near));
    // From 16 the eighth hop is 9 and its live lock wins.
    assert_eq!(find_session(&home.root, Path::new("/none"), &table, 16, only_nine_is_live), Some(far));
}

#[test]
fn without_a_lock_match_the_cwd_ladder_applies_in_order() {
    let home = CopilotHome::new("ladder");
    let newest_any = home.session("newest-any", "/other", &[], 5);
    let cwd_locked = home.session("cwd-locked", "/w", &[777], 40);
    let any_locked = home.session("any-locked", "/other", &[888], 60);
    let cwd_plain = home.session("cwd-plain", "/w", &[], 20);
    let no_pid_match = [(1, 1)];
    let find = |cwd: &str| find_session(&home.root, Path::new(cwd), &no_pid_match, 4242, always_copilot);

    assert_eq!(find("/w"), Some(cwd_locked.clone()), "an active session for the cwd wins");
    fs::remove_file(cwd_locked.join("inuse.777.lock")).expect("unlock");
    assert_eq!(find("/w"), Some(any_locked.clone()), "then any active session");
    fs::remove_file(any_locked.join("inuse.888.lock")).expect("unlock");
    // Removing locks touched those directories; restore their ages so mtime order holds.
    age(&cwd_locked, 40);
    age(&any_locked, 60);
    assert_eq!(find("/w"), Some(cwd_plain), "then the newest session for the cwd");
    assert_eq!(find("/nowhere"), Some(newest_any), "then the newest session at all");
}

#[test]
fn a_session_directory_without_events_is_not_a_candidate() {
    let home = CopilotHome::new("no-events");
    let with_events = home.session("with-events", "/w", &[], 50);
    let bare = home.session("bare", "/w", &[], 5);
    fs::remove_file(bare.join("events.jsonl")).expect("strip events");
    assert_eq!(find_session(&home.root, Path::new("/w"), &[], 1, always_copilot), Some(with_events));
}

#[test]
fn cwd_comparison_ignores_case_and_slash_direction() {
    let home = CopilotHome::new("case");
    let win = home.session("win", "C:\\Users\\Me\\Repo", &[], 10);
    home.session("other", "/other", &[], 5);
    let found = find_session(&home.root, Path::new("c:/users/me/repo"), &[], 1, always_copilot);
    assert_eq!(found, Some(win));
}

#[test]
fn newest_assistant_message_first_and_n_is_honoured() {
    let messages = parse_messages(EVENTS, 25);
    let assistant: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Assistant).map(|m| m.text.as_str()).collect();
    assert_eq!(assistant, ["assistant text 2\n\nwith a second paragraph", "assistant text 1"]);
    assert_eq!(messages[0].id, "f3f3f3f3-7666-46b3-9a91-0ca13d2321bb", "data.messageId is the id");
    assert_eq!(messages[0].at.as_deref(), Some("2026-07-08T17:10:01.000Z"));
    assert_eq!(parse_messages(EVENTS, 1).len(), 1);
}

#[test]
fn tool_only_and_system_events_are_not_messages() {
    let texts: Vec<String> = parse_messages(EVENTS, 25).into_iter().map(|m| m.text).collect();
    assert!(!texts.iter().any(|t| t.contains("system prompt")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("tool output")), "{texts:?}");
    assert!(!texts.iter().any(String::is_empty), "an assistant.message with empty content is skipped");
}

#[test]
fn user_messages_are_human_prompts() {
    let humans: Vec<String> =
        parse_messages(EVENTS, 25).into_iter().filter(|m| m.role == Role::Human).map(|m| m.text).collect();
    assert_eq!(humans, ["user prompt 2", "user prompt 1"]);
}
