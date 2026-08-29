//! Oh My Pi (OMP): a pi harness with pi's session format and layout, rooted at
//! `~/.omp/agent/sessions` (`PI_CODING_AGENT_SESSION_DIR` / `PI_CODING_AGENT_DIR` apply
//! unchanged: OMP reuses pi's variable names, `oh-my-pi/packages/coding-agent/src/cli/args.ts`).
//! Under Herdr, the exact transcript path arrives as `agent_session` and no discovery runs.

use std::path::{Path, PathBuf};

use crate::{Message, pi};

/// The agent directory relative to `$HOME`.
pub const DEFAULT_AGENT_DIR: &str = ".omp/agent";

/// The newest OMP session for `cwd`; pi's rules over OMP's root.
pub fn find_transcript(sessions_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    pi::find_transcript(sessions_dir, cwd)
}

/// OMP writes pi's entries; the same reader applies.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    pi::parse_messages(jsonl, n)
}
