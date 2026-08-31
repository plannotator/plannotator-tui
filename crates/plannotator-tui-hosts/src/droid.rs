//! Droid (Factory): `$FACTORY_CONFIG_DIR/sessions/<slug>/<session>.jsonl`, where the slug
//! is Claude Code's rule for a cwd and the entries are Claude's shape with `id`/`parentId`.
//!
//! Factory keeps no per-process session metadata, so the current session is the newest log
//! for the cwd's slug, else the newest log under the first ancestor directory that has any
//! (the user `cd`'d deeper after starting). Plannotator selects that one log and reads it
//! as is — it never falls through to an older sibling session — and so do we. Factory has
//! no rewind tree, so messages are read in file order.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::Message;
use crate::claude::{parse_messages_file_order, project_slug};

/// Find `$FACTORY_CONFIG_DIR/sessions/*/<id>.jsonl`, preferring the current cwd's slug.
pub fn find_transcript_by_id(
    factory_dir: &Path,
    cwd: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, crate::HostError> {
    let id = crate::validate_session_id(session_id)?;
    let sessions_dir = factory_dir.join("sessions");
    let preferred = slug_dir(&sessions_dir, cwd);
    if let Some(path) = preferred.as_ref().map(|dir| dir.join(format!("{id}.jsonl")))
        && path.is_file()
    {
        return Ok(Some(path));
    }
    let mut slug_dirs: Vec<PathBuf> = std::fs::read_dir(&sessions_dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect())
        .unwrap_or_default();
    slug_dirs.sort();
    Ok(slug_dirs
        .into_iter()
        .filter(|dir| preferred.as_ref() != Some(dir))
        .map(|dir| dir.join(format!("{id}.jsonl")))
        .find(|path| path.is_file()))
}

/// The current session log for `cwd`, per the rule above. `None` when no slug directory
/// on the way up holds a transcript.
pub fn find_transcript(factory_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    let sessions_dir = factory_dir.join("sessions");
    cwd.ancestors().find_map(|dir| newest_log(&slug_dir(&sessions_dir, dir)?))
}

/// The newest `n` assistant and human messages, newest first, in file order.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    parse_messages_file_order(jsonl, n)
}

/// The slug directory for `cwd`: exact name, else case-insensitive (Windows lowercases).
fn slug_dir(sessions_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    let slug = project_slug(cwd);
    let exact = sessions_dir.join(&slug);
    if exact.is_dir() {
        return Some(exact);
    }
    std::fs::read_dir(sessions_dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(&slug))
    })
}

/// The most recently modified `.jsonl` in `dir`.
fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max_by_key(|(when, _)| *when)
        .map(|(_, p): (SystemTime, PathBuf)| p)
}
