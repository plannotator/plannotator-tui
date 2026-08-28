//! The active branch of a transcript, rendered into messages.

use std::collections::{HashMap, HashSet};

use super::entries::Entry;
use crate::{Message, Role};

/// The newest `n` rendered messages, newest first.
///
/// The active branch is the `parentUuid` chain from the newest entry that has a uuid.
/// File order is used instead when that chain cannot be trusted (missing ids, a dangling
/// parent, a cycle) or when it holds no assistant text — right after `/compact` the branch
/// may be nothing but the summary, and an empty result helps nobody.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    let entries: Vec<Entry> = jsonl.lines().filter_map(Entry::parse).collect();
    if entries.is_empty() {
        return Vec::new();
    }
    let rendered = match active_branch(&entries) {
        Some(active) => {
            let on_branch: Vec<&Entry> =
                entries.iter().filter(|e| e.uuid.as_ref().is_some_and(|u| active.contains(u))).collect();
            let branch = render(&on_branch);
            if branch.iter().any(|m| m.role == Role::Assistant) { branch } else { render_all(&entries) }
        }
        None => render_all(&entries),
    };
    rendered.into_iter().rev().take(n).collect()
}

/// The newest `n` rendered messages in plain file order, newest first — for transcripts
/// in Claude's shape that have no rewind tree (Droid).
pub fn parse_messages_file_order(jsonl: &str, n: usize) -> Vec<Message> {
    let entries: Vec<Entry> = jsonl.lines().filter_map(Entry::parse).collect();
    render_all(&entries).into_iter().rev().take(n).collect()
}

fn render_all(entries: &[Entry]) -> Vec<Message> {
    render(&entries.iter().collect::<Vec<_>>())
}

/// Uuids on the chain from the newest entry to a root; `None` when the chain is untrusted.
fn active_branch(entries: &[Entry]) -> Option<HashSet<String>> {
    let newest = entries.iter().rev().find(|e| e.uuid.is_some())?;
    let by_uuid: HashMap<&str, &Entry> =
        entries.iter().filter_map(|e| e.uuid.as_deref().map(|u| (u, e))).collect();
    let mut chain = HashSet::new();
    let mut current = newest;
    loop {
        let uuid = current.uuid.as_deref()?;
        if !chain.insert(uuid.to_owned()) {
            return None; // cycle
        }
        match current.parent.as_deref() {
            None => return Some(chain),
            Some(parent) => current = *by_uuid.get(parent)?, // dangling parent → untrusted
        }
    }
}

/// Messages in file order. Assistant text blocks that share a `message.id` (streamed
/// chunks, interleaved with tool calls) become one message.
fn render(entries: &[&Entry]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    let mut by_message: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        if entry.is_assistant_text() {
            // Streamed chunks share `message.id`; Droid entries carry only their own `id`.
            let id = entry
                .message_id
                .clone()
                .or_else(|| entry.id.clone())
                .or_else(|| entry.uuid.clone())
                .unwrap_or_default();
            let text = entry.texts.join("\n\n");
            if let Some(message) = by_message.get(&id).and_then(|&index| out.get_mut(index)) {
                message.text.push_str("\n\n");
                message.text.push_str(&text);
            } else {
                by_message.insert(id.clone(), out.len());
                out.push(Message { id, role: Role::Assistant, text, at: entry.timestamp.clone() });
            }
        } else if entry.is_human_prompt() {
            out.push(Message {
                id: entry.uuid.clone().unwrap_or_default(),
                role: Role::Human,
                text: entry.texts.join("\n\n"),
                at: entry.timestamp.clone(),
            });
        }
    }
    out
}
