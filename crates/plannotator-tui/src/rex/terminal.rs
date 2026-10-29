//! `plannotator-tui rex terminal`: review a block's recent output, read with `rex capture`.
//! `crate::terminal` turns the text into the review.

use anyhow::Result;

use super::agent::AGENT_VAR;
use super::command::{Rex, short_id};
use crate::terminal::{clean, open, title};

/// The block's last `lines` lines of output, cleaned; an error when the block shows nothing.
/// Unwrapped lines are the program's own lines, so the split that narrows the block before
/// the read does not re-break them.
pub(crate) fn read(rex: &Rex, block: &str, lines: u32) -> Result<String> {
    let captured = rex.run(&["capture", "-b", block, "--unwrap"], None)?;
    let text = tail(&clean(&captured), lines);
    anyhow::ensure!(!text.is_empty(), "block {} has no output to review", short_id(block));
    Ok(text)
}

/// The last `lines` lines of `text`.
fn tail(text: &str, lines: u32) -> String {
    let all: Vec<&str> = text.lines().collect();
    let keep = usize::try_from(lines).unwrap_or(usize::MAX).min(all.len());
    all.get(all.len() - keep..).unwrap_or_default().join("\n")
}

/// The split's entry point: read the block now and open the review. Feedback goes to the
/// agent the launcher found there, else the clipboard.
pub(crate) fn run(rex: &Rex, block: &str, lines: u32) -> Result<()> {
    let text = read(rex, block, lines)?;
    let agent = std::env::var(AGENT_VAR).ok().filter(|agent| !agent.is_empty());
    let title = title(short_id(block), agent.as_deref());
    crate::cli::run_ui(|width| open(&text, title, width, crate::cli::delivery(true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_lines_are_kept() {
        assert_eq!(tail("one\ntwo\nthree", 2), "two\nthree");
        assert_eq!(tail("one\ntwo", 200), "one\ntwo");
        assert_eq!(tail("", 5), "");
    }
}
