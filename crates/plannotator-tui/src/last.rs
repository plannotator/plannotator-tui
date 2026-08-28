//! `plannotator-tui last`: annotate an agent's most recent message (docs/spec-last-message.md).

use std::path::PathBuf;

use anyhow::Result;

/// What `last` was asked to open. Every field optional; detection fills the gaps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LastOptions {
    /// `claude` | `codex`; else detected from the environment.
    pub(crate) host: Option<String>,
    /// The agent process to start the transcript search from.
    pub(crate) pid: Option<u32>,
    /// An explicit transcript; skips detection.
    pub(crate) session: Option<PathBuf>,
    /// Read the document from stdin instead of a transcript.
    pub(crate) stdin: bool,
    /// Print the newest message and exit instead of opening the UI.
    pub(crate) print: bool,
    /// How many recent messages the picker offers.
    pub(crate) pick: usize,
}

#[allow(clippy::needless_pass_by_value, reason = "the real implementation consumes the options")]
pub(crate) fn run(options: LastOptions) -> Result<()> {
    anyhow::bail!("`last` is not available in this build yet ({options:?})")
}
