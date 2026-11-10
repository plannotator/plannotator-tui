//! `plannotator-tui herdr terminal`: review a pane's recent terminal output, read through
//! `herdr pane read`. `crate::terminal` turns the text into the review.

use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use super::context::HerdrEnv;
use crate::terminal::{DEFAULT_LINES, clean, open, title};

/// `herdr pane read <pane> --source recent-unwrapped`, cleaned; an error when Herdr fails
/// or the pane shows nothing. Unwrapped lines are the program's own lines, so a split that
/// narrows the pane before the read does not re-break them.
pub(crate) fn read(env: &HerdrEnv, pane: &str, lines: u32) -> Result<String> {
    let output = Command::new(&env.bin)
        .args(["pane", "read", pane, "--source", "recent-unwrapped", "--lines", &lines.to_string()])
        .args(["--format", "text"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {} pane read", env.bin.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("herdr pane read {pane}: {}", stderr.trim());
    }
    let text = clean(&String::from_utf8_lossy(&output.stdout));
    anyhow::ensure!(!text.is_empty(), "pane {pane} has no output to review");
    Ok(text)
}

/// The doc pane for `PLANNOTATOR_TUI_TERMINAL_PANE`: read the pane now and open the review.
/// Feedback goes where the launcher said (`PLANNOTATOR_TUI_DELIVER_TO`), else the clipboard.
pub(crate) fn run(env: &HerdrEnv, pane: &str) -> Result<()> {
    let text = read(env, pane, env.terminal_lines.unwrap_or(DEFAULT_LINES))?;
    let agent = env.deliver_agent.clone().or_else(|| env.context_agent_in(pane));
    let title = title(pane, agent.as_deref());
    crate::cli::run_ui(|width| open(&text, title, width, crate::cli::delivery(true)))
}
