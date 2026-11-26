//! What a note's box in the diff says, wrapped to the box's width: a title (`comment · L42`,
//! the author when the note is not the person's own), then the comment, and for a suggestion
//! the lines it replaces and the code it suggests as `-` and `+` lines. At most
//! [`BODY_LINES`] lines show; the box says when there is more, which opening the note shows.

mod code;

use std::hash::{Hash as _, Hasher as _};

use plannotator_tui_review::{CodeAnnotation, NoteScope, NoteType, Side};
use ratatui::style::Style;
use ratatui::text::Line;

use crate::review::ask::Thread;
use crate::theme::palette;
use crate::ui::compose::TAB_WIDTH;
use crate::wrap::wrap_line;

/// The most lines of text a box shows.
pub(super) const BODY_LINES: usize = 6;

/// The columns a box keeps beside its text: a border and a space on each side.
pub(super) const CHROME: u16 = 4;

/// A note's box, laid out for one width.
#[derive(Debug, Clone)]
pub(super) struct Card {
    /// What it was laid out from: the note's revision and the box's width.
    made_for: (u64, u16),
    /// `comment · L42`.
    pub(super) label: String,
    /// Who wrote it, when the record names someone.
    pub(super) author: Option<String>,
    pub(super) body: Vec<Line<'static>>,
    /// Lines that did not fit.
    pub(super) more: bool,
}

impl Card {
    /// `note` in a box `width` columns wide, its tabs as wide as the comment box draws them,
    /// whatever the diff's tab width, so a note reads the same in both.
    pub(super) fn of(note: &CodeAnnotation, width: u16) -> Self {
        let text_width = usize::from(width.saturating_sub(CHROME)).max(1);
        let tabs = " ".repeat(TAB_WIDTH);
        let plain = |text: &str| text.replace('\t', &tabs);
        let mut body: Vec<Line<'static>> = Vec::new();
        if let Some(text) = note.text.as_deref().map(str::trim_end).filter(|t| !t.is_empty()) {
            for line in text.split('\n') {
                let line = Line::from(plain(line.trim_end_matches('\r')));
                body.extend(wrap_line(&line, &[], text_width).into_iter().map(|row| row.line));
            }
        }
        if note.kind == NoteType::Suggestion {
            let styles = (Style::new().fg(palette().delete), Style::new().fg(palette().looks_good));
            let (old, new) = (note.original_code.as_deref(), note.suggested_code.as_deref());
            body.extend(code::rows(old, new, text_width, TAB_WIDTH, styles));
        }
        let more = body.len() > BODY_LINES;
        body.truncate(BODY_LINES);
        if body.is_empty() {
            body.push(Line::default());
        }
        let author = note.author_name.as_deref().filter(|name| !name.is_empty()).map(str::to_owned);
        Self { made_for: (revision(note), width), label: label(note), author, body, more }
    }

    /// Whether this is `note`'s card at `width`, as [`Self::of`] would make it now.
    pub(super) fn is_for(&self, note: &CodeAnnotation, width: u16) -> bool {
        self.made_for == (revision(note), width)
    }

    /// An Ask thread's box at `width`: every question and all of each answer, which is what
    /// the box is for, so no [`BODY_LINES`] limit.
    pub(super) fn of_thread(thread: &Thread, width: u16) -> Self {
        let mut body = thread.body(usize::from(width.saturating_sub(CHROME)).max(1));
        if body.is_empty() {
            body.push(Line::default());
        }
        Self { made_for: (thread.revision(), width), label: thread.label(), author: None, body, more: false }
    }

    /// Whether this is `thread`'s card at `width`, as [`Self::of_thread`] would make it now.
    pub(super) fn is_for_thread(&self, thread: &Thread, width: u16) -> bool {
        self.made_for == (thread.revision(), width)
    }

    /// The rows the box takes: its text and two borders.
    pub(super) fn height(&self) -> u16 {
        self.body.len() as u16 + 2
    }
}

/// `comment · L42`, `comment · L40–45`, `suggestion · L42`, `comment · L7 old` for lines of
/// the old file, `file comment` for a note on the whole file.
pub(super) fn label(note: &CodeAnnotation) -> String {
    let kind = match note.kind {
        NoteType::Suggestion => "suggestion",
        NoteType::Comment | NoteType::Concern => "comment",
    };
    if note.scope != NoteScope::Line {
        return format!("file {kind}");
    }
    let lines = if note.line_start == note.line_end {
        format!("L{}", note.line_start)
    } else {
        format!("L{}\u{2013}{}", note.line_start, note.line_end)
    };
    let old = if note.side == Side::Old { " old" } else { "" };
    format!("{kind} \u{b7} {lines}{old}")
}

/// A number that changes whenever what the box shows of `note` does.
fn revision(note: &CodeAnnotation) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (&note.text, &note.suggested_code, &note.original_code, &note.author_name).hash(&mut hasher);
    (note.kind == NoteType::Suggestion, note.scope == NoteScope::Line, note.side == Side::Old)
        .hash(&mut hasher);
    (note.line_start, note.line_end).hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    /// A note of `kind` on `scope`, lines `start..=end` of `side`, with the fields in `extra`.
    fn note(kind: &str, scope: &str, side: &str, (start, end): (u32, u32), extra: &str) -> CodeAnnotation {
        let json = format!(
            r#"{{"id":"n","type":"{kind}","scope":"{scope}","filePath":"a.rs","side":"{side}","lineStart":{start},"lineEnd":{end},"createdAt":1{extra}}}"#
        );
        serde_json::from_str(&json).expect("note")
    }

    fn text(card: &Card) -> Vec<String> {
        card.body.iter().map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    #[test]
    fn labels_name_the_kind_the_lines_and_an_old_side() {
        assert_eq!(label(&note("comment", "line", "new", (42, 42), "")), "comment \u{b7} L42");
        assert_eq!(label(&note("comment", "line", "new", (40, 45), "")), "comment \u{b7} L40\u{2013}45");
        assert_eq!(label(&note("comment", "line", "old", (7, 7), "")), "comment \u{b7} L7 old");
        assert_eq!(label(&note("suggestion", "line", "new", (42, 42), "")), "suggestion \u{b7} L42");
        assert_eq!(label(&note("comment", "file", "new", (1, 1), "")), "file comment");
    }

    #[test]
    fn text_wraps_to_the_box_and_stops_after_six_lines() {
        let words = "word ".repeat(40);
        let long = note("comment", "line", "new", (1, 1), &format!(r#","text":"{words}""#));
        let card = Card::of(&long, 24);
        assert_eq!(card.body.len(), BODY_LINES);
        assert!(card.more);
        assert!(text(&card).iter().all(|line| line.chars().count() <= 20), "{:?}", text(&card));
        assert_eq!(card.height(), 8);
        let short = note("comment", "line", "new", (1, 1), r#","text":"one\ntwo""#);
        let card = Card::of(&short, 24);
        assert_eq!((text(&card), card.more), (vec!["one".to_owned(), "two".to_owned()], false));
    }

    #[test]
    fn a_suggestion_shows_its_comment_then_the_old_and_the_new_code() {
        let code = r#","text":"simpler","originalCode":"\tlet a = b;","suggestedCode":"\tlet a = c;""#;
        let suggestion = note("suggestion", "line", "new", (3, 3), code);
        let card = Card::of(&suggestion, 40);
        assert_eq!(text(&card), ["simpler", "- let a = b;", "+ let a = c;"]);
        let red = card.body.get(1).and_then(|line| line.spans.first()).map(|span| span.style.fg);
        assert_eq!(red, Some(Some(palette().delete)));
        // A long line wraps with its sign on the first row only.
        let code = r#","originalCode":"abcdefghijklmnop","suggestedCode":"x""#;
        let card = Card::of(&note("suggestion", "line", "new", (3, 3), code), 14);
        assert_eq!(text(&card), ["- abcdefgh", "  ijklmnop", "+ x"]);
    }

    #[test]
    fn tabs_take_the_comment_boxs_four_columns() {
        let tabbed = note("comment", "line", "new", (1, 1), r#","text":"a\tb""#);
        assert_eq!(text(&Card::of(&tabbed, 30)), ["a    b"]);
    }

    #[test]
    fn a_card_is_made_again_when_its_note_or_its_width_changes() {
        let mut one = note("comment", "line", "new", (1, 1), r#","text":"a""#);
        let card = Card::of(&one, 30);
        assert!(card.is_for(&one, 30));
        assert!(!card.is_for(&one, 31));
        one.text = Some("b".into());
        assert!(!card.is_for(&one, 30));
    }
}
