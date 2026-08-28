//! Feedback export: annotations as the numbered Markdown a coding agent expects.
//!
//! The shape follows Plannotator's `exportAnnotations` so an agent that has read one kind
//! of feedback reads the other the same way: a title, a count line, then one `## N.` entry
//! per annotation in document order, each quoting the text and carrying the note.

use std::fmt::Write as _;
use std::ops::Range;

use plannotui_schema::{Annotation, Kind};

/// One annotation placed in the document, as the exporter needs it.
pub(crate) struct Entry<'a> {
    pub(crate) annotation: &'a Annotation,
    pub(crate) range: Range<usize>,
    /// 1-based source line span of the annotated range.
    pub(crate) lines: (usize, usize),
}

pub(crate) fn feedback(source: &str, subject: &str, entries: &[Entry<'_>]) -> String {
    if entries.is_empty() {
        return "No changes detected.".to_owned();
    }
    let mut out = String::from("# Plan Feedback\n\n");
    let n = entries.len();
    let _ = write!(
        out,
        "I've reviewed this {subject} and have {n} piece{} of feedback:\n\n",
        if n > 1 { "s" } else { "" }
    );
    for (i, entry) in entries.iter().enumerate() {
        let quoted = source.get(entry.range.clone()).unwrap_or("");
        let line_label = match entry.lines {
            (a, b) if a == b => format!("line {a}"),
            (a, b) => format!("lines {a}\u{2013}{b}"),
        };
        let _ = write!(out, "## {}. ({line_label}) ", i + 1);
        let body = entry.annotation.body.trim();
        match entry.annotation.anchor.kind() {
            Kind::Delete => {
                out.push_str("Remove this\n");
                out.push_str(&fenced(quoted));
                let _ = writeln!(
                    out,
                    "> {}",
                    if body.is_empty() { "I don't want this in the plan." } else { body }
                );
            }
            Kind::LooksGood => {
                let _ = writeln!(out, "[Looks good] Feedback on: \"{}\"", single_line(quoted));
                if !body.is_empty() {
                    let _ = writeln!(out, "> {}", quote_lines(body));
                }
            }
            Kind::Comment => {
                let _ = writeln!(out, "Feedback on: \"{}\"", single_line(quoted));
                let _ = writeln!(out, "> {}", quote_lines(body));
            }
        }
        for reply in &entry.annotation.replies {
            let who = reply.author.as_deref().unwrap_or("reply");
            let _ = writeln!(out, "- **Reply ({who}):** {}", reply.body.replace('\n', "\n  "));
        }
        out.push('\n');
    }
    out
}

/// A fence longer than any backtick run inside the text, so quoted markdown cannot escape.
fn fenced(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{text}\n{fence}\n")
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn quote_lines(text: &str) -> String {
    text.replace('\n', "\n> ")
}

/// 1-based line numbers of the first and last byte of `range`.
pub(crate) fn line_span(source: &str, range: &Range<usize>) -> (usize, usize) {
    let line_at = |offset: usize| source.get(..offset).map_or(1, |s| s.matches('\n').count() + 1);
    (line_at(range.start), line_at(range.end.saturating_sub(1).max(range.start)))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use plannotui_schema::{Anchor, SourceRange, State};

    fn annotation(source: &str, quote: &str, kind: Kind, body: &str) -> (Annotation, Range<usize>) {
        let start = source.find(quote).expect("present");
        let range = start..start + quote.len();
        let source_range = SourceRange { start, end: range.end, version: "v".into() };
        let annotation = Annotation {
            id: "a".into(),
            document_id: String::new(),
            anchor: Anchor::new(quote, source, source_range, kind, None),
            body: body.into(),
            author: None,
            author_name: None,
            state: State::Open,
            attachments: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
            replies: Vec::new(),
            other: std::collections::BTreeMap::default(),
        };
        (annotation, range)
    }

    #[test]
    fn export_matches_the_agent_facing_shape() {
        let source = "# Title\n\nShip the login page by Friday.\n\nDrop the `legacy` path.\n";
        let (comment, r1) = annotation(source, "login page", Kind::Comment, "Which page?\nBe specific.");
        let (delete, r2) = annotation(source, "Drop the `legacy` path.", Kind::Delete, "");
        let entries = [
            Entry { annotation: &comment, lines: line_span(source, &r1), range: r1 },
            Entry { annotation: &delete, lines: line_span(source, &r2), range: r2 },
        ];
        let out = feedback(source, "plan", &entries);
        assert_eq!(
            out,
            "# Plan Feedback\n\nI've reviewed this plan and have 2 pieces of feedback:\n\n\
             ## 1. (line 3) Feedback on: \"login page\"\n> Which page?\n> Be specific.\n\n\
             ## 2. (line 5) Remove this\n```\nDrop the `legacy` path.\n```\n> I don't want this in the plan.\n\n"
        );
    }

    #[test]
    fn fences_grow_past_embedded_backticks() {
        assert!(fenced("has ``` inside").starts_with("````\n"));
    }
}
