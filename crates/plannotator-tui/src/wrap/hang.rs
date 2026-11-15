//! Hanging indents: a line whose first cells are a prefix, and whose wrapped rows start with
//! another prefix of the same width, so text lines up under text. The caller says how long
//! the prefix is and what the wrapped rows start with; nothing here looks at the characters.

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar as _;

use super::{Cell, Row, cells_of, cells_width, finish_row, wrap_cells, wrap_line};

/// How a line's wrapped rows start. The default hangs nothing.
#[derive(Debug, Default, Clone)]
pub(crate) struct Hang {
    /// Characters at the front of the line that are prefix, not text.
    pub(crate) lead: usize,
    /// What each wrapped row starts with instead: no source, the same width as the lead.
    pub(crate) rest: Vec<Span<'static>>,
}

/// Like [`wrap_line`], with the rows after the first starting with `hang.rest`.
pub(crate) fn wrap_line_hanging(
    line: &Line<'_>,
    offsets: &[Option<usize>],
    hang: &Hang,
    width: usize,
) -> Vec<Row> {
    let cells = cells_of(line, offsets);
    let rest: Vec<Cell> = hang
        .rest
        .iter()
        .flat_map(|span| span.content.chars().map(|ch| (ch, span.style)))
        .map(|(ch, style)| Cell { ch, width: ch.width().unwrap_or(0), offset: None, style })
        .collect();
    let (lead, body) = cells.split_at(hang.lead.min(cells.len()));
    let indent = cells_width(lead).max(cells_width(&rest));
    // A narrow column would leave too little room for text: use all of it instead.
    if hang.lead == 0 || indent * 2 > width {
        return wrap_line(line, offsets, width);
    }
    wrap_cells(body, width - indent)
        .into_iter()
        .enumerate()
        .map(|(row, piece)| {
            let start = if row == 0 { lead } else { rest.as_slice() };
            finish_row([start, piece.as_slice()].concat(), line.style)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::{identity, plain};
    use super::*;

    fn hang(lead: usize, rest: &str) -> Hang {
        Hang { lead, rest: vec![Span::raw(rest.to_owned())] }
    }

    #[test]
    fn wrapped_rows_start_with_the_rest_prefix_and_it_has_no_source() {
        let source = "∙ one two three";
        let rows = wrap_line_hanging(&Line::from(source), &identity(15), &hang(2, "  "), 10);
        assert_eq!(plain(&rows), ["∙ one two", "  three"]);
        assert_eq!(rows.get(1).map(|r| r.cells.iter().take(2).flatten().count()), Some(0));
        assert_eq!(rows.get(1).and_then(|r| r.cells.get(2).copied()), Some(Some(10)));
    }

    #[test]
    fn a_narrow_column_gives_up_the_hanging_indent() {
        let source = "    ∙ one two";
        let rows = wrap_line_hanging(&Line::from(source), &identity(13), &hang(6, "      "), 10);
        assert_eq!(plain(&rows), ["    ∙ one", "two"]);
    }
}
