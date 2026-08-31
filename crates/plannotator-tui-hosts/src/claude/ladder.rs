//! Which transcript belongs to the agent we were launched from.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{parse_messages, parse_session_meta, project_slug};
use crate::SessionMeta;

const MAX_ANCESTOR_HOPS: usize = 8;

/// The ladder, most precise first; the first candidate that yields a message wins:
/// 1. `sessions/<pid>.json` for `start_pid` and up to eight of its ancestors — with a ghost
///    check: a newer transcript in the same project dir that no running session claims is a
///    `/clear` session and is preferred;
/// 2. every session whose `cwd` is ours, newest `startedAt` first;
/// 3. the project dir for our cwd (case-insensitive fallback), newest transcript first;
/// 4. the same for each parent directory of cwd.
pub fn find_transcript(
    sessions_dir: &Path,
    projects_dir: &Path,
    cwd: &Path,
    process_table: &[(u32, u32)],
    start_pid: u32,
) -> Option<PathBuf> {
    let sessions = registered_sessions(sessions_dir);
    let registered: HashSet<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();

    let mut pid = Some(start_pid);
    for _ in 0..=MAX_ANCESTOR_HOPS {
        let Some(current) = pid else { break };
        if let Some(meta) = sessions.iter().find(|s| s.pid == current)
            && let Some(found) = transcript_for_session(projects_dir, meta, &registered)
        {
            return Some(found);
        }
        pid = process_table.iter().find(|(p, _)| *p == current).map(|(_, ppid)| *ppid).filter(|&p| p > 1);
    }

    let mut same_cwd: Vec<&SessionMeta> = sessions.iter().filter(|s| same_dir(&s.cwd, cwd)).collect();
    same_cwd.sort_by_key(|s| std::cmp::Reverse(s.started_at));
    for meta in same_cwd {
        if let Some(found) = transcript_for_session(projects_dir, meta, &registered) {
            return Some(found);
        }
    }

    cwd.ancestors().find_map(|dir| newest_with_messages(&project_dir(projects_dir, dir)?, None))
}

fn registered_sessions(sessions_dir: &Path) -> Vec<SessionMeta> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|json| parse_session_meta(&json))
        .collect()
}

fn transcript_for_session(
    projects_dir: &Path,
    meta: &SessionMeta,
    registered: &HashSet<&str>,
) -> Option<PathBuf> {
    let dir = project_dir(projects_dir, &meta.cwd)?;
    let own = dir.join(format!("{}.jsonl", meta.session_id));
    let own_mtime = mtime(&own);
    // A newer transcript nobody registered is a `/clear` session in the same process.
    if let Some(ghost) = newest_with_messages(&dir, Some(registered))
        && own_mtime.is_none_or(|own| mtime(&ghost).is_some_and(|g| g > own))
    {
        return Some(ghost);
    }
    (own_mtime.is_some() && has_messages(&own)).then_some(own)
}

/// The project dir for `cwd`, exact name first, then case-insensitively (Windows lowercases).
pub(super) fn project_dir(projects_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    let slug = project_slug(cwd);
    let exact = projects_dir.join(&slug);
    if exact.is_dir() {
        return Some(exact);
    }
    std::fs::read_dir(projects_dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(&slug)) && p.is_dir()
    })
}

/// Newest transcript in `dir` that renders at least one message. With `unregistered`, only
/// transcripts whose session id is not in that set count.
fn newest_with_messages(dir: &Path, unregistered: Option<&HashSet<&str>>) -> Option<PathBuf> {
    let mut files: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter(|p| {
            unregistered
                .is_none_or(|set| p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| !set.contains(s)))
        })
        .filter_map(|p| Some((mtime(&p)?, p)))
        .collect();
    files.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    files.into_iter().map(|(_, p)| p).find(|p| has_messages(p))
}

fn has_messages(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| !parse_messages(&text, 1).is_empty())
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['/', '\\']).to_owned();
    norm(a) == norm(b)
}
