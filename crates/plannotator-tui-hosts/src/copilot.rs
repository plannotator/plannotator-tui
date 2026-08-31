//! GitHub Copilot CLI: `$COPILOT_HOME/session-state/<uuid>/` holds `events.jsonl`,
//! `workspace.yaml` (a `cwd:` line) and, while the session runs, `inuse.<pid>.lock`.
//!
//! Copilot sets no environment marker, so a session is found by matching the lock pids
//! against our ancestor pids (Plannotator's `copilot-session.ts`); the cwd heuristic is the
//! fallback. Events have no branch tree: messages are read in file order.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::{Message, Role};

const MAX_ANCESTOR_HOPS: usize = 8;

/// Find `$COPILOT_HOME/session-state/<id>` when it contains an events transcript.
pub fn find_session_by_id(
    copilot_home: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, crate::HostError> {
    let id = crate::validate_session_id(session_id)?;
    let dir = copilot_home.join("session-state").join(id);
    Ok(dir.join("events.jsonl").is_file().then_some(dir))
}

/// The session directory for the Copilot process we were launched from.
///
/// 1. Walk `start_pid` and up to eight ancestors; the first pid that owns an
///    `inuse.<pid>.lock` wins, provided `is_copilot(pid)` confirms the pid still names a
///    Copilot process (locks outlive sessions and pids get reused; a stale match is dropped
///    and the walk continues).
/// 2. Else by cwd, newest directory first: a locked session for `cwd`, any locked session,
///    a session for `cwd`, the newest session at all.
pub fn find_session(
    copilot_home: &Path,
    cwd: &Path,
    process_table: &[(u32, u32)],
    start_pid: u32,
    is_copilot: impl Fn(u32) -> bool,
) -> Option<PathBuf> {
    let state_dir = copilot_home.join("session-state");
    let sessions = list_sessions(&state_dir);

    let mut chain = ancestor_chain(process_table, start_pid);
    let locks = lock_owners(&sessions);
    while let Some(&pid) = chain.iter().find(|pid| locks.contains_key(pid)) {
        if is_copilot(pid) {
            return locks.get(&pid).cloned();
        }
        chain.retain(|&p| p != pid);
    }

    let wanted = normalize(cwd);
    let matches = |s: &Session| s.cwd.as_deref().is_some_and(|c| normalize(Path::new(c)) == wanted);
    sessions
        .iter()
        .find(|s| s.locked && matches(s))
        .or_else(|| sessions.iter().find(|s| s.locked))
        .or_else(|| sessions.iter().find(|s| matches(s)))
        .or_else(|| sessions.first())
        .map(|s| s.dir.clone())
}

/// Human prompts and assistant replies from `events.jsonl`, newest first, at most `n`.
/// `assistant.message` carries its text as a plain string; empty ones are skipped.
pub fn parse_messages(events_jsonl: &str, n: usize) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    for line in events_jsonl.lines().rev() {
        if out.len() >= n {
            break;
        }
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let role = match kind {
            "assistant.message" => Role::Assistant,
            "user.message" => Role::Human,
            _ => continue,
        };
        let data = event.get("data");
        let Some(text) = data.and_then(|d| d.get("content")).and_then(Value::as_str) else { continue };
        if text.trim().is_empty() {
            continue;
        }
        let id = data
            .and_then(|d| d.get("messageId"))
            .or_else(|| event.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let at = event.get("timestamp").and_then(Value::as_str).map(str::to_owned);
        out.push(Message { id, role, text: text.to_owned(), at });
    }
    out
}

struct Session {
    dir: PathBuf,
    cwd: Option<String>,
    locked: bool,
    lock_pids: Vec<u32>,
}

/// Every session directory that has an `events.jsonl`, newest modification first. Other
/// clients (`gh copilot`) create session directories with a workspace but no events; those
/// hold nothing to read and are not candidates.
fn list_sessions(state_dir: &Path) -> Vec<Session> {
    let Ok(entries) = std::fs::read_dir(state_dir) else { return Vec::new() };
    let mut sessions: Vec<(SystemTime, Session)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("events.jsonl").is_file())
        .filter_map(|dir| {
            let modified = std::fs::metadata(&dir).ok()?.modified().ok()?;
            let lock_pids = lock_pids_in(&dir);
            Some((
                modified,
                Session { cwd: workspace_cwd(&dir), locked: !lock_pids.is_empty(), lock_pids, dir },
            ))
        })
        .collect();
    sessions.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    sessions.into_iter().map(|(_, s)| s).collect()
}

/// The `cwd:` line of `workspace.yaml`, if any.
fn workspace_cwd(dir: &Path) -> Option<String> {
    let yaml = std::fs::read_to_string(dir.join("workspace.yaml")).ok()?;
    yaml.lines()
        .find_map(|line| line.strip_prefix("cwd:"))
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
}

/// Pids of `inuse.<pid>.lock` files in `dir`; malformed names are ignored.
fn lock_pids_in(dir: &Path) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| name.strip_prefix("inuse.")?.strip_suffix(".lock")?.parse::<u32>().ok())
        .collect()
}

/// pid → session dir; the first directory claiming a pid keeps it.
fn lock_owners(sessions: &[Session]) -> HashMap<u32, PathBuf> {
    let mut owners = HashMap::new();
    for session in sessions {
        for &pid in &session.lock_pids {
            owners.entry(pid).or_insert_with(|| session.dir.clone());
        }
    }
    owners
}

/// `start_pid` and up to eight ancestors, stopping at pid 1 or a cycle.
fn ancestor_chain(process_table: &[(u32, u32)], start_pid: u32) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut pid = start_pid;
    while chain.len() < MAX_ANCESTOR_HOPS && pid > 1 && !chain.contains(&pid) {
        chain.push(pid);
        let Some(&(_, parent)) = process_table.iter().find(|(p, _)| *p == pid) else { break };
        pid = parent;
    }
    chain
}

/// Forward slashes, lowercase, no trailing separator — Plannotator's comparison.
fn normalize(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()
}
