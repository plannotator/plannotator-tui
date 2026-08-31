//! Oh My Pi (OMP): a pi harness with pi's session format and layout, rooted at
//! `~/.omp/agent/sessions` (`PI_CODING_AGENT_SESSION_DIR` / `PI_CODING_AGENT_DIR` apply
//! unchanged: OMP reuses pi's variable names, `oh-my-pi/packages/coding-agent/src/cli/args.ts`).
//! Under Herdr, the exact transcript path arrives as `agent_session` and no discovery runs.

use std::path::{Path, PathBuf};

use crate::{Message, pi};

/// The agent directory relative to `$HOME`.
pub const DEFAULT_AGENT_DIR: &str = ".omp/agent";

/// The exact OMP session id, preferring OMP's current home/temp/absolute cwd bucket.
pub fn find_transcript_by_id(
    sessions_dir: &Path,
    cwd: &Path,
    home: &Path,
    temp: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, crate::HostError> {
    let preferred = sessions_dir.join(encoded_dir(cwd, home, temp));
    pi::find_transcript_by_id_in(sessions_dir, &[preferred], session_id)
}

fn encoded_dir(cwd: &Path, home: &Path, temp: &Path) -> String {
    let normalized = |path: &Path| path.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_owned();
    let cwd = normalized(cwd);
    let relative = |root: &Path| {
        let root = normalized(root);
        let compare = |value: &str| {
            if cfg!(windows) { value.to_ascii_lowercase() } else { value.to_owned() }
        };
        if compare(&cwd) == compare(&root) {
            return Some(String::new());
        }
        let prefix = format!("{root}/");
        compare(&cwd)
            .strip_prefix(&compare(&prefix))
            .and_then(|suffix| cwd.get(cwd.len().saturating_sub(suffix.len())..))
            .map(str::to_owned)
    };
    let encode = |prefix: &str, relative: &str| {
        let relative = relative.replace(['/', '\\', ':'], "-");
        if relative.is_empty() {
            prefix.to_owned()
        } else if prefix.ends_with('-') {
            format!("{prefix}{relative}")
        } else {
            format!("{prefix}-{relative}")
        }
    };
    if let Some(relative) = relative(home) {
        encode("-", &relative)
    } else if let Some(relative) = relative(temp) {
        encode("-tmp", &relative)
    } else {
        format!("--{}--", cwd.trim_start_matches('/').replace(['/', '\\', ':'], "-"))
    }
}

/// The newest OMP session for `cwd`; pi's rules over OMP's root.
pub fn find_transcript(sessions_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    pi::find_transcript(sessions_dir, cwd)
}

/// OMP writes pi's entries; the same reader applies.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    pi::parse_messages(jsonl, n)
}
