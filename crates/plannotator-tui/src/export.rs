//! Feedback export: annotations as numbered Markdown a coding agent reads without a schema.
//!
//! `# Annotations on <name>`, then one `## Annotation N (line X)` per annotation in document
//! order: what kind of note it is, the quoted text, and the body as a blockquote. Deleted
//! text is fenced so quoted markdown cannot escape.

use std::fmt::Write as _;
use std::ops::Range;

use plannotator_tui_schema::{Annotation, Kind};

/// One annotation placed in the document, as the exporter needs it.
pub(crate) struct Entry<'a> {
    pub(crate) annotation: &'a Annotation,
    /// The annotated source text, as the agent should read it.
    pub(crate) quote: String,
    /// 1-based source line span of the annotated range; `None` where source lines mean
    /// nothing to the reader (a terminal review), and the heading omits them.
    pub(crate) lines: Option<(usize, usize)>,
}

pub(crate) fn feedback(name: &str, entries: &[Entry<'_>]) -> String {
    if entries.is_empty() {
        return "No annotations.".to_owned();
    }
    let mut out = format!("# Annotations on {name}\n\n");
    for (i, entry) in entries.iter().enumerate() {
        let quoted = entry.quote.as_str();
        let line_label = match entry.lines {
            Some((a, b)) if a == b => format!(" (line {a})"),
            Some((a, b)) => format!(" (lines {a}\u{2013}{b})"),
            None => String::new(),
        };
        let _ = writeln!(out, "## Annotation {}{line_label}", i + 1);
        let body = entry.annotation.body.trim();
        match entry.annotation.anchor.kind() {
            Kind::Delete => {
                out.push_str("Remove this:\n");
                out.push_str(&fenced(quoted));
                let _ = writeln!(out, "> {}", if body.is_empty() { "I don't want this." } else { body });
            }
            Kind::LooksGood => {
                let _ = writeln!(out, "Looks good: \"{}\"", single_line(quoted));
                if !body.is_empty() {
                    let _ = writeln!(out, "> {}", quote_lines(body));
                }
            }
            Kind::Comment => {
                let _ = writeln!(out, "Comment on: \"{}\"", single_line(quoted));
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

/// The source text under `range`, less the line breaks at `joins`: breaks a wrapper
/// inserted inside a token, which were never in the text the reader saw printed.
pub(crate) fn quote(source: &str, range: &Range<usize>, joins: &[usize]) -> String {
    let text = source.get(range.clone()).unwrap_or("");
    text.char_indices()
        .filter(|(offset, _)| joins.binary_search(&(range.start + offset)).is_err())
        .map(|(_, ch)| ch)
        .collect()
}

/// 1-based line numbers of the first and last byte of `range`.
pub(crate) fn line_span(source: &str, range: &Range<usize>) -> (usize, usize) {
    let line_at = |offset: usize| source.get(..offset).map_or(1, |s| s.matches('\n').count() + 1);
    (line_at(range.start), line_at(range.end.saturating_sub(1).max(range.start)))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use plannotator_tui_schema::{Anchor, SourceRange, State};

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
            Entry { annotation: &comment, lines: Some(line_span(source, &r1)), quote: source[r1].to_owned() },
            Entry { annotation: &delete, lines: Some(line_span(source, &r2)), quote: source[r2].to_owned() },
        ];
        let out = feedback("plan.md", &entries);
        assert_eq!(
            out,
            "# Annotations on plan.md\n\n\
             ## Annotation 1 (line 3)\nComment on: \"login page\"\n> Which page?\n> Be specific.\n\n\
             ## Annotation 2 (line 5)\nRemove this:\n```\nDrop the `legacy` path.\n```\n> I don't want this.\n\n"
        );
    }

    #[test]
    fn an_entry_without_lines_has_no_line_label() {
        let source = "$ ls\n";
        let (comment, range) = annotation(source, "ls", Kind::Comment, "why");
        let entries = [Entry { annotation: &comment, lines: None, quote: source[range].to_owned() }];
        assert_eq!(
            feedback("terminal · w1:p1", &entries),
            "# Annotations on terminal · w1:p1\n\n## Annotation 1\nComment on: \"ls\"\n> why\n\n"
        );
    }

    #[test]
    fn fences_grow_past_embedded_backticks() {
        assert!(fenced("has ``` inside").starts_with("````\n"));
    }
}
