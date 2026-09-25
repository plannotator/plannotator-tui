//! `plannotator-tui herdr terminal`: review a pane's recent terminal output.
//!
//! Herdr reads the pane (`herdr pane read`); this module cleans the text and presents it as
//! one fenced code block, so the output shows verbatim and a quote in the feedback is the
//! text as it appeared. The document is transient: nothing is written to disk.

use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use plannotator_tui_schema::{DocumentSource, Provenance};
use unicode_width::UnicodeWidthChar;

use super::context::HerdrEnv;
use crate::app::App;
use crate::delivery::Delivery;

/// Recent lines read when `--lines` is not given. Herdr caps a read at 1000.
pub(crate) const DEFAULT_LINES: u32 = 200;

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

/// Terminal text as plain lines: escape sequences and control characters removed, trailing
/// whitespace trimmed, and blank lines dropped from both ends.
pub(crate) fn clean(raw: &str) -> String {
    let plain = strip_escapes(raw);
    let lines: Vec<&str> = plain.lines().map(str::trim_end).collect();
    let first = lines.iter().position(|line| !line.is_empty());
    let last = lines.iter().rposition(|line| !line.is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines.get(first..=last).unwrap_or_default().join("\n"),
        _ => String::new(),
    }
}

/// Drop CSI, OSC, and two-byte escape sequences, and every control character but `\n` and
/// `\t`. `\r\n` becomes `\n`; a lone `\r` is dropped.
fn strip_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                // CSI: parameters and intermediates up to one final byte in `@`..=`~`.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ST (`ESC \`).
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\t' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Output re-broken to fit a width, and where it was broken inside a token.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Wrapped {
    pub(crate) text: String,
    /// Byte offsets in `text` of the newlines that split a token wider than the view. A
    /// quote across one rejoins without a space; a break at a space rejoins with one.
    pub(crate) breaks: Vec<usize>,
}

/// Break lines wider than `width` columns, at the last space that fits when there is one.
/// Code blocks clip rather than wrap, so without this a long line would be cut off.
pub(crate) fn wrap(text: &str, width: usize) -> Wrapped {
    let width = width.max(1);
    let mut out = String::with_capacity(text.len());
    let mut breaks = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut rest = line;
        while let Some(cut) = overflow(rest, width) {
            let head = rest.get(..cut).unwrap_or(rest);
            // A space inside the leading indentation is no place to break.
            let indent = head.len() - head.trim_start_matches(' ').len();
            if let Some(space) = head.rfind(' ').filter(|&space| space > indent) {
                out.push_str(head.get(..space).unwrap_or(head));
                rest = rest.get(space + 1..).unwrap_or_default();
            } else {
                out.push_str(head);
                breaks.push(out.len());
                rest = rest.get(cut..).unwrap_or_default();
            }
            out.push('\n');
        }
        out.push_str(rest);
    }
    Wrapped { text: out, breaks }
}

/// The byte offset where `line` stops fitting in `width` columns, or `None` when it fits.
fn overflow(line: &str, width: usize) -> Option<usize> {
    let mut used = 0;
    for (offset, ch) in line.char_indices() {
        used += UnicodeWidthChar::width(ch).unwrap_or(0);
        // A character wider than the whole line still has to go somewhere.
        if used > width && offset > 0 {
            return Some(offset);
        }
    }
    None
}

/// The output as Markdown: one fenced `text` block, its fence longer than any backtick run
/// inside so the output cannot close it.
pub(crate) fn document(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}text\n{text}\n{fence}\n")
}

/// What the header calls the review: `terminal · claude in w1:p2`, or the pane alone.
pub(crate) fn title(pane: &str, agent: Option<&str>) -> String {
    match agent {
        Some(agent) => format!("terminal · {agent} in {pane}"),
        None => format!("terminal · {pane}"),
    }
}

/// The document the app opens: transient, so it gets no sidecar, history, or drafts.
pub(crate) fn source(text: &str, title: String) -> DocumentSource {
    DocumentSource::new(document(text), title, true, Provenance::Stdin)
}

/// The review of `text` at `width`: wrapped to fit, with the breaks the wrapper put inside
/// tokens handed to the app so quotes across them read as printed.
pub(crate) fn open(text: &str, title: String, width: usize, delivery: Box<dyn Delivery>) -> Result<App> {
    let wrapped = wrap(text, width);
    let source = source(&wrapped.text, title);
    // The output starts after the opening fence line.
    let start = source.content.find('\n').map_or(0, |newline| newline + 1);
    let mut app = App::open(source, width, delivery)?;
    app.set_terminal_breaks(wrapped.breaks.iter().map(|offset| start + offset).collect());
    Ok(app)
}

/// The doc pane for `PLANNOTATOR_TUI_TERMINAL_PANE`: read the pane now and open the review.
/// Feedback goes where the launcher said (`PLANNOTATOR_TUI_DELIVER_TO`), else the clipboard.
pub(crate) fn run(env: &HerdrEnv, pane: &str) -> Result<()> {
    let text = read(env, pane, env.terminal_lines.unwrap_or(DEFAULT_LINES))?;
    let agent = env.deliver_agent.clone().or_else(|| env.context_agent_in(pane));
    let title = title(pane, agent.as_deref());
    crate::cli::run_ui(|width| open(&text, title, width, crate::cli::delivery(true)))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn cleaning_removes_escapes_controls_and_surrounding_blank_lines() {
        let raw = "\n  \n\u{1b}[1;32m$ cargo test\u{1b}[0m   \r\n\u{1b}]8;;file:///x\u{7}link\u{1b}]8;;\u{1b}\\ ok\u{7}\n\t indented\n\n\n";
        assert_eq!(clean(raw), "$ cargo test\nlink ok\n\t indented");
    }

    #[test]
    fn output_with_nothing_visible_cleans_to_empty() {
        assert_eq!(clean(""), "");
        assert_eq!(clean("\n   \n\u{1b}[2J\n"), "");
    }

    #[test]
    fn blank_lines_inside_the_output_are_kept() {
        assert_eq!(clean("one\n\n\ntwo\n"), "one\n\n\ntwo");
    }

    #[test]
    fn long_lines_break_at_a_space_and_short_lines_are_untouched() {
        let text = |input: &str, width| wrap(input, width).text;
        assert_eq!(
            wrap("the quick brown fox", 10),
            Wrapped { text: "the quick\nbrown fox".into(), breaks: vec![] }
        );
        assert_eq!(text("short\n  indented", 10), "short\n  indented");
        assert_eq!(wrap("abcdefghijkl", 5), Wrapped { text: "abcde\nfghij\nkl".into(), breaks: vec![5, 11] });
        assert_eq!(text("   abcdefghi", 6), "   abc\ndefghi");
        // Wide characters count as two columns.
        assert_eq!(text("한글한글", 4), "한글\n한글");
    }

    #[test]
    fn the_fence_outlasts_any_backticks_in_the_output() {
        assert_eq!(document("ls"), "```text\nls\n```\n");
        let doc = document("echo ```` done");
        assert!(doc.starts_with("`````text\n") && doc.ends_with("\n`````\n"), "{doc}");
    }

    #[test]
    fn the_document_renders_the_output_as_one_code_block() {
        let doc = crate::doc::Document::parse(document("# not a heading\n* not a list"));
        let kinds: Vec<_> = doc.blocks.iter().map(|block| block.kind).collect();
        assert_eq!(kinds, [crate::doc::BlockKind::CodeBlock]);
    }

    #[test]
    fn the_review_is_transient_and_titled_by_its_pane() {
        let source = source("ok", title("w1:p2", Some("claude")));
        assert!(source.transient);
        assert_eq!(source.name, "terminal · claude in w1:p2");
        assert_eq!(title("w1:p2", None), "terminal · w1:p2");
    }

    #[test]
    fn a_quote_across_a_broken_path_reads_as_printed() {
        let path = "/home/user/projects/herdr/src/deeply/nested/module/source.rs";
        assert_eq!(path.len(), 60);
        let text = format!("error in {path}\nok");
        let wrapped = wrap(&text, 24);
        // `error in` breaks at its space; the path breaks twice inside itself.
        assert_eq!(wrapped.text.lines().count(), 5, "{}", wrapped.text);
        assert_eq!(wrapped.breaks.len(), 2);
        let across: Vec<&str> = wrapped.text.lines().take(4).collect();
        let mut app =
            open(&text, title("w1:p1", None), 24, Box::new(crate::delivery::Discard)).expect("opens");
        app.add_quote_annotation_at(
            &across.join("\n"),
            1,
            plannotator_tui_schema::Kind::Comment,
            "fix".into(),
        )
        .expect("annotated");
        assert_eq!(
            app.feedback(),
            format!(
                "# Annotations on terminal · w1:p1\n\n## Annotation 1\nComment on: \"error in {path}\"\n> fix\n\n"
            )
        );
    }
}
