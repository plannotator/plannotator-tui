//! The feedback text a coding agent receives.
//!
//! The body is the browser's `exportReviewFeedback` for a local review
//! (`packages/review-editor/utils/exportFeedback.ts`), so an agent that reads
//! `plannotator review` output reads this unchanged: `# Code Review Feedback`, the
//! `**Diff:**` line, a `## <file>` section per file with file comments first and then line
//! notes by line, the outdated label, and the `**Replaces:**` / `**Suggested code:**`
//! blocks. Two additions, both in the browser's own shapes:
//!
//! - a line note without a suggestion carries its lines as the browser's
//!   `**Highlighted text:**` block, capped at [`HIGHLIGHT_MAX_LINES`], so the agent can see
//!   deleted (old-side) lines;
//! - a credited note says `From: <authorName>` under its heading, as file reviews do.
//!
//! Files are sorted by path. [`agent_feedback`] adds the `Review directory:` header the
//! browser's CLI writes (`withReviewDirectory`) and appends the review suffix unchanged.
//! Browser-only fields (conventional labels, call-flow targets, token selections) are not
//! written: notes made here never carry them.

use std::fmt::Write as _;

use crate::annotation::{CodeAnnotation, NoteScope};

/// The browser's `OUTDATED_ANNOTATION_LABEL`; its wording is fixed.
pub const OUTDATED_ANNOTATION_LABEL: &str = "[Outdated — the code changed after this comment]";

/// The browser's `GLOBAL_COMMENTS_HEADING`, for review-level notes.
pub const GLOBAL_COMMENTS_HEADING: &str = "Global comments";

/// The browser's `DEFAULT_REVIEW_DENIED_SUFFIX` (`packages/shared/prompts.ts`): the agent
/// treats findings as unverified and changes no code until they are discussed.
pub const REVIEW_SUFFIX: &str = "\n\nTreat the findings above as unverified review input. Inspect every finding against the actual code; do not assume automated feedback is correct. For each finding, give a clear verdict (Confirmed / Partly / Not a bug / Intended) with concise code evidence. Say whether it was introduced by the current changes, was pre-existing, or reflects deliberate scope.\n\nReview only the incoming findings. Do not independently review the rest of the diff or search for issues that were not submitted.\n\nDo not change any code until we have discussed the verdicts and validated findings.";

/// Lines of a note's code quoted in its `**Highlighted text:**` block.
pub const HIGHLIGHT_MAX_LINES: usize = 40;

/// What the agent receives: `Review directory: <root>`, the feedback, and the suffix.
/// `diff` is the `**Diff:**` description ([`crate::Scope::describe`]).
pub fn agent_feedback(repo_root: &str, diff: &str, notes: &[&CodeAnnotation]) -> String {
    format!("Review directory: {repo_root}\n\n{}{REVIEW_SUFFIX}", review_feedback(notes, Some(diff)))
}

/// The browser's `exportReviewFeedback(notes, null, diff)`, with the two additions above.
pub fn review_feedback(notes: &[&CodeAnnotation], diff: Option<&str>) -> String {
    if notes.is_empty() {
        return "# Code Review\n\nNo feedback provided.".to_owned();
    }
    let mut out = String::from("# Code Review Feedback\n\n");
    if let Some(diff) = diff {
        let _ = write!(out, "**Diff:** {diff}\n\n");
    }
    let (general, mut placed): (Vec<&CodeAnnotation>, Vec<&CodeAnnotation>) =
        notes.iter().partition(|n| n.scope == NoteScope::General);
    if !general.is_empty() {
        let _ = write!(out, "## {GLOBAL_COMMENTS_HEADING}\n\n");
        for note in general {
            credit(&mut out, note);
            body(&mut out, note);
            out.push('\n');
        }
    }
    placed.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    for file in placed.chunk_by(|a, b| a.file_path == b.file_path) {
        let Some(first) = file.first() else { continue };
        let _ = write!(out, "## {}\n\n", first.file_path);
        let mut file = file.to_vec();
        file.sort_by_key(|n| (n.scope != NoteScope::File, n.line_start));
        for note in file {
            if note.scope == NoteScope::File {
                file_comment(&mut out, note);
            } else {
                line_comment(&mut out, note);
            }
        }
    }
    out
}

fn file_comment(out: &mut String, note: &CodeAnnotation) {
    out.push_str("### File Comment\n");
    credit(out, note);
    body(out, note);
    suggestion_blocks(out, note);
    out.push('\n');
}

fn line_comment(out: &mut String, note: &CodeAnnotation) {
    let lines = if note.line_start == note.line_end {
        format!("Line {}", note.line_start)
    } else {
        format!("Lines {}-{}", note.line_start, note.line_end)
    };
    let _ = writeln!(out, "### {lines} ({})", note.side.as_str());
    if note.outdated {
        let _ = writeln!(out, "{OUTDATED_ANNOTATION_LABEL}");
    }
    credit(out, note);
    body(out, note);
    if note.original_code.is_none()
        && let Some(code) = note.anchor_text.as_deref()
    {
        highlighted(out, code);
    }
    suggestion_blocks(out, note);
    out.push('\n');
}

fn credit(out: &mut String, note: &CodeAnnotation) {
    if let Some(name) = note.author_name.as_deref().filter(|n| !n.is_empty()) {
        let _ = writeln!(out, "From: {name}");
    }
}

fn body(out: &mut String, note: &CodeAnnotation) {
    if let Some(text) = present(note.text.as_deref()) {
        let _ = writeln!(out, "{text}");
    }
}

/// The browser's `formatSelectedTextBlock`, over the first [`HIGHLIGHT_MAX_LINES`] lines.
fn highlighted(out: &mut String, code: &str) {
    let lines: Vec<&str> = code.split('\n').collect();
    let shown = lines.iter().take(HIGHLIGHT_MAX_LINES).copied().collect::<Vec<_>>().join("\n");
    out.push_str("\n**Highlighted text:**\n");
    fenced(out, &shown);
    let hidden = lines.len().saturating_sub(HIGHLIGHT_MAX_LINES);
    if hidden > 0 {
        let _ = writeln!(out, "_({hidden} more lines not shown)_");
    }
}

/// The browser's `formatSuggestionBlocks`.
fn suggestion_blocks(out: &mut String, note: &CodeAnnotation) {
    let suggested = present(note.suggested_code.as_deref());
    if let Some(original) = present(note.original_code.as_deref())
        && (suggested.is_some() || present(note.text.as_deref()).is_some())
    {
        out.push_str("\n**Replaces:**\n");
        fenced(out, original);
    }
    if let Some(code) = suggested {
        out.push_str("\n**Suggested code:**\n");
        fenced(out, code);
    }
}

/// A fence longer than any backtick run in `code` (the browser's `codeFenceFor`).
fn fenced(out: &mut String, code: &str) {
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    let _ = write!(out, "{fence}\n{code}\n{fence}\n");
}

/// JavaScript truthiness for an optional string: absent and empty are both false.
fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests;
