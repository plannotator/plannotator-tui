//! Finding the transcript for the agent we were launched from.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use plannotator_tui_hosts::claude::{find_transcript, parse_ps, parse_session_meta, project_slug};

const TRANSCRIPT: &str = include_str!("fixtures/claude-code.jsonl");

struct Home {
    root: PathBuf,
}

impl Home {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("plannotator-tui-hosts-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sessions")).expect("sessions");
        fs::create_dir_all(root.join("projects")).expect("projects");
        Self { root }
    }
    fn sessions(&self) -> PathBuf {
        self.root.join("sessions")
    }
    fn projects(&self) -> PathBuf {
        self.root.join("projects")
    }
    fn session(&self, pid: u32, session_id: &str, cwd: &Path, started_at: u64) {
        let json = format!(
            r#"{{"pid":{pid},"sessionId":"{session_id}","cwd":"{}","startedAt":{started_at},"version":"2.1.250"}}"#,
            cwd.display()
        );
        fs::write(self.sessions().join(format!("{pid}.json")), json).expect("session json");
    }
    /// A transcript in the project dir for `cwd`, with content and an mtime offset in seconds.
    fn transcript(&self, cwd: &Path, session_id: &str, content: &str, age_secs: u64) -> PathBuf {
        self.transcript_in(&project_slug(cwd), session_id, content, age_secs)
    }
    fn transcript_in(&self, dir: &str, session_id: &str, content: &str, age_secs: u64) -> PathBuf {
        let dir = self.projects().join(dir);
        fs::create_dir_all(&dir).expect("project dir");
        let path = dir.join(format!("{session_id}.jsonl"));
        fs::write(&path, content).expect("transcript");
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        File::open(&path).expect("open").set_modified(when).expect("mtime");
        path
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn session_meta_parses_the_four_fields_we_use() {
    let meta = parse_session_meta(r#"{"pid":86521,"sessionId":"abc","cwd":"/Users/x/repo","startedAt":1787878328768,"kind":"interactive"}"#).expect("meta");
    assert_eq!((meta.pid, meta.session_id.as_str(), meta.started_at), (86521, "abc", 1_787_878_328_768));
    assert_eq!(meta.cwd, Path::new("/Users/x/repo"));
    assert!(parse_session_meta(r#"{"pid":1}"#).is_none());
}

#[test]
fn the_project_slug_replaces_everything_outside_alnum_and_dash() {
    assert_eq!(project_slug(Path::new("/Users/me/oss/my repo.v2")), "-Users-me-oss-my-repo-v2");
}

#[test]
fn ps_output_parses_with_padding_and_blank_lines() {
    let table = parse_ps("  100     1\n  200   100\n\n300 200 extra\n");
    assert_eq!(table, vec![(100, 1), (200, 100), (300, 200)]);
}

#[test]
fn a_direct_pid_hit_is_the_herdr_case() {
    let home = Home::new("direct");
    let cwd = Path::new("/w/repo");
    home.session(500, "s500", cwd, 10);
    let expected = home.transcript(cwd, "s500", TRANSCRIPT, 60);
    let found = find_transcript(&home.sessions(), &home.projects(), cwd, &[], 500);
    assert_eq!(found, Some(expected));
}

#[test]
fn an_ancestor_within_eight_hops_is_found_and_the_ninth_is_not() {
    let home = Home::new("ancestors");
    let cwd = Path::new("/w/repo");
    home.session(1000, "s1000", cwd, 10);
    let expected = home.transcript(cwd, "s1000", TRANSCRIPT, 60);
    // 1008 → 1007 → … → 1000: eight hops.
    let table: Vec<(u32, u32)> = (1001..=1009).map(|p| (p, p - 1)).collect();
    assert_eq!(
        find_transcript(&home.sessions(), &home.projects(), Path::new("/elsewhere"), &table, 1008),
        Some(expected)
    );
    assert_eq!(
        find_transcript(&home.sessions(), &home.projects(), Path::new("/elsewhere"), &table, 1009),
        None
    );
}

#[test]
fn a_newer_unregistered_transcript_in_the_project_is_a_clear_session_and_wins() {
    let home = Home::new("ghost");
    let cwd = Path::new("/w/repo");
    home.session(700, "s700", cwd, 10);
    home.transcript(cwd, "s700", TRANSCRIPT, 600);
    let ghost = home.transcript(cwd, "after-clear", TRANSCRIPT, 5);
    assert_eq!(find_transcript(&home.sessions(), &home.projects(), cwd, &[], 700), Some(ghost));
}

#[test]
fn without_a_pid_hit_the_cwd_scan_prefers_the_newest_session() {
    let home = Home::new("cwd");
    let cwd = Path::new("/w/repo");
    home.session(10, "old", cwd, 100);
    home.session(11, "new", cwd, 200);
    home.session(12, "other", Path::new("/w/other"), 300);
    home.transcript(cwd, "old", TRANSCRIPT, 30);
    let expected = home.transcript(cwd, "new", TRANSCRIPT, 30);
    assert_eq!(find_transcript(&home.sessions(), &home.projects(), cwd, &[], 9999), Some(expected));
}

#[test]
fn slug_and_mtime_pick_the_newest_transcript_even_in_a_lowercased_dir() {
    let home = Home::new("slug");
    let cwd = Path::new("/w/Repo");
    let lower = project_slug(cwd).to_ascii_lowercase();
    home.transcript_in(&lower, "older", TRANSCRIPT, 300);
    let expected = home.transcript_in(&lower, "newer", TRANSCRIPT, 30);
    // Case-insensitive filesystems (macOS) resolve either spelling; compare the real path.
    let found = find_transcript(&home.sessions(), &home.projects(), cwd, &[], 1).expect("found");
    assert_eq!(fs::canonicalize(found).expect("real"), fs::canonicalize(expected).expect("real"));
}

#[test]
fn a_parent_directory_of_cwd_is_tried_when_cwd_has_no_project() {
    let home = Home::new("parent");
    let expected = home.transcript(Path::new("/w/repo"), "s", TRANSCRIPT, 30);
    assert_eq!(
        find_transcript(&home.sessions(), &home.projects(), Path::new("/w/repo/deep/dir"), &[], 1),
        Some(expected)
    );
}

#[test]
fn a_candidate_with_no_messages_is_skipped_for_the_next_one() {
    let home = Home::new("empty");
    let cwd = Path::new("/w/repo");
    home.transcript(cwd, "empty", "{\"type\":\"progress\"}\n", 5);
    let expected = home.transcript(cwd, "real", TRANSCRIPT, 60);
    assert_eq!(find_transcript(&home.sessions(), &home.projects(), cwd, &[], 1), Some(expected));
}
