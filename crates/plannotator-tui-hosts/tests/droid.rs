//! Droid (Factory): the newest log for the cwd's slug, read in file order.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use plannotator_tui_hosts::droid::{find_transcript, parse_messages};
use plannotator_tui_hosts::{Role, claude};

const LOG: &str =
    include_str!("fixtures/droid/sessions/-Users-me-repo/be4202cc-4266-4e3b-b0f1-9324af19e4be.jsonl");
const CLAUDE_REWIND: &str = include_str!("fixtures/claude-code.jsonl");

struct FactoryDir {
    root: PathBuf,
}

impl FactoryDir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("plannotator-tui-droid-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sessions")).expect("sessions");
        Self { root }
    }

    fn log(&self, slug: &str, id: &str, content: &str, age_secs: u64) -> PathBuf {
        let dir = self.root.join("sessions").join(slug);
        fs::create_dir_all(&dir).expect("slug dir");
        let path = dir.join(format!("{id}.jsonl"));
        fs::write(&path, content).expect("log");
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        File::open(&path).expect("open").set_modified(when).expect("mtime");
        path
    }
}

impl Drop for FactoryDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_slug_is_claude_codes_rule() {
    assert_eq!(claude::project_slug(Path::new("/Users/me/repo")), "-Users-me-repo");
}

#[test]
fn the_newest_log_for_the_exact_cwd_wins_even_when_empty() {
    let factory = FactoryDir::new("exact");
    factory.log("-Users-me-repo", "older", LOG, 100);
    let newest_empty = factory.log("-Users-me-repo", "newest", "", 10);
    // Plannotator selects the newest log and reads only it; it never falls back to a sibling.
    assert_eq!(find_transcript(&factory.root, Path::new("/Users/me/repo")), Some(newest_empty));
}

#[test]
fn a_subdirectory_falls_back_to_the_first_ancestor_with_logs() {
    let factory = FactoryDir::new("ancestor");
    let repo = factory.log("-Users-me-repo", "s1", LOG, 10);
    factory.log("-Users-me", "home", LOG, 5); // nearer to root but not the first ancestor
    assert_eq!(find_transcript(&factory.root, Path::new("/Users/me/repo/src/deep")), Some(repo));
    assert_eq!(find_transcript(&factory.root, Path::new("/elsewhere")), None);
}

#[test]
fn a_lowercased_slug_directory_still_matches() {
    let factory = FactoryDir::new("case");
    let log = factory.log("-users-me-repo", "s1", LOG, 10);
    // Case-insensitive filesystems resolve the exact name too; compare the real file.
    let found = find_transcript(&factory.root, Path::new("/Users/me/repo")).expect("found");
    assert_eq!(fs::canonicalize(found).expect("real"), fs::canonicalize(log).expect("real"));
}

#[test]
fn messages_come_in_file_order_newest_first_with_tool_and_hidden_entries_dropped() {
    let messages = parse_messages(LOG, 25);
    let assistant: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Assistant).map(|m| m.text.as_str()).collect();
    assert_eq!(assistant, ["assistant text 3 (final)", "assistant text 2", "assistant text 1"]);
    let humans: Vec<&str> =
        messages.iter().filter(|m| m.role == Role::Human).map(|m| m.text.as_str()).collect();
    assert_eq!(humans, ["user prompt 1"], "llm_only reminders and tool results are not prompts");
    assert_eq!(messages[0].id, "b062ee96-658e-4697-90d0-5ba3d1cc1afa", "the entry id names the message");
    assert_eq!(parse_messages(LOG, 2).len(), 2);
}

#[test]
fn droid_ignores_rewind_branches_where_claude_follows_them() {
    // The Claude fixture has an abandoned branch; Claude's reader drops it, Droid's reads all.
    let via_droid = parse_messages(CLAUDE_REWIND, 100);
    let via_claude = claude::parse_messages(CLAUDE_REWIND, 100);
    assert!(via_droid.len() > via_claude.len(), "droid={} claude={}", via_droid.len(), via_claude.len());
}
