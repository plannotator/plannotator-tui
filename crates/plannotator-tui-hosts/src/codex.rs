//! Codex: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread>.jsonl`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Message, Role};

/// The transcript files of one thread, oldest first. With `thread_id`, every rollout whose
/// filename carries that id (a thread can span files). Without it, the newest rollout that
/// is not a subagent's, plus any sibling files of its thread.
pub fn find_transcripts(codex_home: &Path, thread_id: Option<&str>) -> Vec<PathBuf> {
    let mut rollouts = all_rollouts(&codex_home.join("sessions"));
    rollouts.sort();
    let thread = match thread_id {
        Some(id) => id.to_owned(),
        None => match rollouts.iter().rev().find(|p| !is_subagent(p)).and_then(|p| thread_of(p)) {
            Some(id) => id,
            None => return Vec::new(),
        },
    };
    rollouts.into_iter().filter(|p| thread_of(p).as_deref() == Some(thread.as_str())).collect()
}

/// The transcript files for one validated exact thread id, oldest first.
pub fn find_transcripts_by_id(codex_home: &Path, session_id: &str) -> Result<Vec<PathBuf>, crate::HostError> {
    let id = crate::validate_session_id(session_id)?;
    Ok(find_transcripts(codex_home, Some(id)))
}

fn all_rollouts(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(all_rollouts(&path));
        } else if path.extension().is_some_and(|x| x == "jsonl")
            && path.file_stem().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-"))
        {
            out.push(path);
        }
    }
    out
}

/// The thread id is the trailing uuid of `rollout-<timestamp>-<uuid>.jsonl`.
fn thread_of(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.rsplitn(6, '-').collect();
    (parts.len() == 6).then(|| parts.iter().take(5).rev().copied().collect::<Vec<_>>().join("-"))
}

/// Subagent rollouts (reviews, guardians) record `source.subagent` in their session meta.
fn is_subagent(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    text.lines()
        .next()
        .and_then(|line| serde_json::from_str::<Value>(line).ok())
        .and_then(|v| v.pointer("/payload/source/subagent").cloned())
        .is_some()
}

/// The newest `n` assistant messages of the active turn, newest first. `files` are the
/// thread's transcripts oldest first; entries are read across them in order.
///
/// The active turn starts at the newest `task_started` that follows the newest
/// `task_complete`; when every turn has completed, the whole thread counts.
pub fn parse_messages(files: &[String], n: usize) -> Vec<Message> {
    let entries: Vec<Value> =
        files.iter().flat_map(|f| f.lines()).filter_map(|l| serde_json::from_str(l.trim()).ok()).collect();
    let event = |v: &Value, name: &str| {
        v.get("type").and_then(Value::as_str) == Some("event_msg")
            && v.pointer("/payload/type").and_then(Value::as_str) == Some(name)
    };
    let last_complete = entries.iter().rposition(|v| event(v, "task_complete"));
    let active_start = entries
        .iter()
        .enumerate()
        .rev()
        .find(|(i, v)| event(v, "task_started") && last_complete.is_none_or(|c| *i > c))
        .map(|(i, _)| i);
    let scope = entries.get(active_start.unwrap_or(0)..).unwrap_or_default();
    scope
        .iter()
        .rev()
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("response_item"))
        .filter(|v| v.pointer("/payload/type").and_then(Value::as_str) == Some("message"))
        .filter(|v| v.pointer("/payload/role").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|v| {
            let text: Vec<&str> = v
                .pointer("/payload/content")?
                .as_array()?
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("output_text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            (!text.is_empty()).then(|| Message {
                id: v.pointer("/payload/id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                role: Role::Assistant,
                text: text.join("\n\n"),
                at: v.get("timestamp").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .take(n)
        .collect()
}
