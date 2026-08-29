//! Pi (pi-mono coding agent): `<sessions>/--<encoded cwd>--/<timestamp>_<uuid>.jsonl`.
//!
//! One file per session. The first line is a `session` header carrying the `cwd`; every
//! other line is an entry with `id`/`parentId` (messages, model changes, compactions,
//! custom entries) forming a tree. There is no pid registry, so a running pi is found by
//! its cwd. Verified against pi's `session-manager`/`migrations.ts` encoding and the
//! `harness/session/types.ts` entry set.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Message, Role};

/// The directory pi writes a cwd's sessions into: two dashes, the cwd without its leading
/// slash and with every slash, backslash and colon replaced by a dash, two dashes.
pub fn encoded_dir(cwd: &Path) -> String {
    let text = cwd.to_string_lossy();
    let trimmed = text.trim_start_matches(['/', '\\']);
    let body: String = trimmed.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
    format!("--{body}--")
}

/// The newest session for `cwd` that holds at least one message: its encoded directory
/// first, then legacy flat files whose header names the cwd, else the newest session
/// anywhere under `sessions_dir`. Newest is by the timestamp in the filename.
pub fn find_transcript(sessions_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    let mut for_cwd: Vec<PathBuf> = jsonl_files(&sessions_dir.join(encoded_dir(cwd)));
    for_cwd.extend(
        jsonl_files(sessions_dir).into_iter().filter(|p| header_cwd(p).is_some_and(|c| same_dir(&c, cwd))),
    );
    if let Some(found) = newest_with_messages(for_cwd) {
        return Some(found);
    }
    let mut all = jsonl_files(sessions_dir);
    for dir in subdirs(sessions_dir) {
        all.extend(jsonl_files(&dir));
    }
    newest_with_messages(all)
}

fn newest_with_messages(mut files: Vec<PathBuf>) -> Option<PathBuf> {
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    files.dedup();
    files
        .into_iter()
        .find(|p| std::fs::read_to_string(p).is_ok_and(|text| !parse_messages(&text, 1).is_empty()))
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
}

/// The `cwd` of the `session` header on the first line.
fn header_cwd(path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(path).ok()?;
    let header: Value = serde_json::from_str(text.lines().next()?).ok()?;
    (header.get("type").and_then(Value::as_str) == Some("session"))
        .then(|| header.get("cwd").and_then(Value::as_str).map(PathBuf::from))
        .flatten()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['/', '\\']).to_owned();
    norm(a) == norm(b)
}

/// One entry line, reduced to what the chain walk and rendering need.
struct Entry {
    id: Option<String>,
    parent: Option<String>,
    /// `Some` only for `message` entries whose `content` is an array: the role and the
    /// `\n`-joined text blocks, as Plannotator's pi extension reads them.
    message: Option<(String, String)>,
    timestamp: Option<String>,
}

impl Entry {
    fn parse(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let object = value.as_object()?;
        let string = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
        let message = (object.get("type").and_then(Value::as_str) == Some("message"))
            .then(|| object.get("message").and_then(Value::as_object))
            .flatten()
            .and_then(|m| {
                let role = m.get("role").and_then(Value::as_str)?.to_owned();
                let blocks = m.get("content").and_then(Value::as_array)?;
                let text: Vec<&str> = blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect();
                Some((role, text.join("\n")))
            });
        Some(Self {
            id: string("id"),
            parent: string("parentId"),
            message,
            timestamp: object.get("timestamp").and_then(iso_timestamp),
        })
    }
}

/// Timestamps as ISO 8601: strings pass through, numbers are Unix milliseconds.
fn iso_timestamp(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => n
            .as_f64()
            .filter(|f| f.is_finite() && *f >= 0.0)
            .map(|ms| crate::time::iso_from_unix_ms(ms as u64)),
        _ => None,
    }
}

/// The newest `n` messages on the active branch, newest first. The branch is the
/// `parentId` chain from the newest entry with an id to the root — what pi's own
/// `getBranch()` returns; entries orphaned by a rewind are absent. A chain that cannot be
/// reconstructed (missing parent, cycle) yields nothing rather than the wrong messages.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    let entries: Vec<Entry> = jsonl.lines().filter_map(Entry::parse).collect();
    let Some(active) = active_branch(&entries) else { return Vec::new() };
    entries
        .iter()
        .rev()
        .filter(|e| e.id.as_ref().is_some_and(|i| active.contains(i)))
        .filter_map(|e| {
            let (role, text) = e.message.as_ref()?;
            let role = match role.as_str() {
                "assistant" => Role::Assistant,
                "user" => Role::Human,
                _ => return None,
            };
            (!text.trim().is_empty()).then(|| Message {
                id: e.id.clone().unwrap_or_default(),
                role,
                text: text.clone(),
                at: e.timestamp.clone(),
            })
        })
        .take(n)
        .collect()
}

fn active_branch(entries: &[Entry]) -> Option<std::collections::HashSet<String>> {
    let newest = entries.iter().rev().find(|e| e.id.is_some())?;
    let by_id: std::collections::HashMap<&str, &Entry> =
        entries.iter().filter_map(|e| e.id.as_deref().map(|i| (i, e))).collect();
    let mut chain = std::collections::HashSet::new();
    let mut current = newest;
    loop {
        let id = current.id.as_deref()?;
        if !chain.insert(id.to_owned()) {
            return None;
        }
        match current.parent.as_deref() {
            None => return Some(chain),
            Some(parent) => current = *by_id.get(parent)?,
        }
    }
}
