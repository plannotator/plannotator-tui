//! Claude Code: `~/.claude/sessions/<pid>.json` names a running session, and
//! `~/.claude/projects/<slug>/<session>.jsonl` is its transcript.

mod entries;
mod ladder;
mod messages;

use std::path::{Path, PathBuf};

use crate::SessionMeta;

pub use ladder::find_transcript;
pub use messages::parse_messages;

/// Parse one `sessions/<pid>.json`. Fields beyond the four we use are ignored.
pub fn parse_session_meta(json: &str) -> Option<SessionMeta> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(SessionMeta {
        pid: u32::try_from(value.get("pid")?.as_u64()?).ok()?,
        session_id: value.get("sessionId")?.as_str()?.to_owned(),
        cwd: PathBuf::from(value.get("cwd")?.as_str()?),
        started_at: value.get("startedAt").and_then(serde_json::Value::as_u64).unwrap_or(0),
    })
}

/// Claude Code's project directory name for a cwd: every byte outside `[A-Za-z0-9-]`
/// becomes `-` (so `/Users/me/repo` is `-Users-me-repo`).
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect()
}

/// `ps -eo pid=,ppid=` output → `(pid, ppid)` pairs. Tolerates padding and blank lines.
pub fn parse_ps(output: &str) -> Vec<(u32, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, parent))
        })
        .collect()
}
