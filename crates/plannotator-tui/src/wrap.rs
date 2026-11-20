//! Word-wrapping of styled lines to a column width, carrying a source offset per cell.
//!
//! `tui-markdown` emits one `Line` per logical line and never wraps; the layout needs
//! exact row counts and the selection needs to know which source byte sits under each
//! screen column, so both live here. Generic text wrapping — no markdown knowledge.
//! Where a line may break is `wrap::breaks`; hanging indents are `wrap::hang`. Reflowing a rendered table is the one
//! shape-aware job, and it lives in `wrap::table`.

mod breaks;
mod hang;
mod table;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use breaks::token_len;

pub(crate) use hang::{Hang, wrap_line_hanging};
pub(crate) use table::wrap_table;

/// One screen row: styled text plus, per column, the source byte it came from.
#[derive(Debug)]
pub(crate) struct Row {
    pub(crate) line: Line<'static>,
    /// `cells[col]` is the source offset shown at that column (wide chars repeat it).
    pub(crate) cells: Vec<Option<usize>>,
}

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    width: usize,
    offset: Option<usize>,
    style: Style,
}

/// Flatten a line into cells, pairing each char with its source offset.
fn cells_of(line: &Line<'_>, offsets: &[Option<usize>]) -> Vec<Cell> {
    let mut offsets = offsets.iter();
    line.spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |ch| (ch, span.style)))
        .map(|(ch, style)| Cell {
            ch,
            width: ch.width().unwrap_or(0),
            offset: offsets.next().copied().flatten(),
            style,
        })
        .collect()
}

/// Drop trailing whitespace: a wrapped piece can end on the space it broke at.
fn trim_trailing_space(mut cells: Vec<Cell>) -> Vec<Cell> {
    while cells.last().is_some_and(|c| c.ch.is_whitespace()) {
        cells.pop();
    }
    cells
}

/// Keep the leading cells that fit `width` columns, dropping the rest.
fn clip_cells(cells: Vec<Cell>, width: usize) -> Vec<Cell> {
    let mut used = 0usize;
    cells
        .into_iter()
        .take_while(|cell| {
            let fits = used + cell.width <= width;
            if fits {
                used += cell.width;
            }
            fits
        })
        .collect()
}

/// Columns a run of cells occupies on screen.
fn cells_width(cells: &[Cell]) -> usize {
    cells.iter().map(|cell| cell.width).sum()
}

/// Turn accumulated cells into a row, merging same-style runs into spans.
fn finish_row(cells: Vec<Cell>, line_style: Style) -> Row {
    let cells = trim_trailing_space(cells);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut columns: Vec<Option<usize>> = Vec::new();
    for cell in &cells {
        match spans.last_mut() {
            Some(last) if last.style == cell.style => last.content.to_mut().push(cell.ch),
            _ => spans.push(Span::styled(cell.ch.to_string(), cell.style)),
        }
        columns.extend(std::iter::repeat_n(cell.offset, cell.width));
    }
    Row { line: Line::from(spans).style(line_style), cells: columns }
}

/// Split cells into the runs that fit `width` columns each. One empty piece for no cells.
///
/// Cells, not `Row`s: a `Row` records one offset per *column*, so reading offsets back out
/// of one would mis-pair them after any wide or zero-width char.
fn wrap_cells(cells: &[Cell], width: usize) -> Vec<Vec<Cell>> {
    let width = width.max(1);
    let mut pieces: Vec<Vec<Cell>> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut current_width = 0usize;

    let mut rest = cells;
    while let Some(first) = rest.first() {
        let is_space = first.ch.is_whitespace();
        let (token, tail) = rest.split_at(token_len(rest));
        rest = tail;
        let token_width = cells_width(token);

        // Whitespace at a row start is dropped, except leading indentation on the first row.
        if is_space && current.is_empty() && !pieces.is_empty() {
            continue;
        }
        if current_width + token_width <= width {
            current.extend_from_slice(token);
            current_width += token_width;
            continue;
        }
        if !current.is_empty() {
            pieces.push(std::mem::take(&mut current));
            current_width = 0;
            if is_space {
                continue;
            }
        }
        if token_width <= width {
            current.extend_from_slice(token);
            current_width = token_width;
        } else {
            // Token wider than a row: hard-split by cells.
            for cell in token {
                if current_width + cell.width > width && !current.is_empty() {
                    pieces.push(std::mem::take(&mut current));
                    current_width = 0;
                }
                current.push(*cell);
                current_width += cell.width;
            }
        }
    }
    if !current.is_empty() || pieces.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// Wrap one logical line into as many rows as needed for `width` columns.
/// `offsets` has one entry per char of the line. An empty line yields one empty row.
pub(crate) fn wrap_line(line: &Line<'_>, offsets: &[Option<usize>], width: usize) -> Vec<Row> {
    wrap_cells(&cells_of(line, offsets), width)
        .into_iter()
        .map(|piece| finish_row(piece, line.style))
        .collect()
}

/// `glyph` across `width` columns, every cell pointing at `offset`.
pub(crate) fn rule_row(glyph: char, style: Style, offset: Option<usize>, width: usize) -> Row {
    let line = Line::from(Span::styled(glyph.to_string().repeat(width), style));
    Row { line, cells: vec![offset; width] }
}

/// Split one line into rows of `width` columns, each starting with `prefix`, breaking
/// anywhere and keeping every character, spaces included: code, whose columns mean
/// something. The rows go onto `rows`. The prefix maps to no source byte. An empty line
/// yields one row, the prefix.
pub(crate) fn split_line(
    rows: &mut Vec<Row>,
    prefix: &Span<'static>,
    line: &Line<'_>,
    offsets: &[Option<usize>],
    width: usize,
) {
    if let Some(row) = fitting_line(prefix, line, offsets, width) {
        rows.push(row);
        return;
    }
    let lead: Vec<Cell> = prefix
        .content
        .chars()
        .map(|ch| Cell { ch, width: ch.width().unwrap_or(0), offset: None, style: prefix.style })
        .collect();
    let room = width.saturating_sub(cells_width(&lead)).max(1);
    let mut piece = lead.clone();
    let mut used = 0;
    for cell in cells_of(line, offsets) {
        if used + cell.width > room && used > 0 {
            rows.push(finish_row(std::mem::replace(&mut piece, lead.clone()), line.style));
            used = 0;
        }
        used += cell.width;
        piece.push(cell);
    }
    rows.push(finish_row(piece, line.style));
}

/// [`split_line`]'s usual case, cheaply: a line of printable ASCII that fits beside the prefix
/// and ends on something visible, kept span for span. `None` for any other line.
fn fitting_line(
    prefix: &Span<'static>,
    line: &Line<'_>,
    offsets: &[Option<usize>],
    width: usize,
) -> Option<Row> {
    let printable = |s: &str| s.bytes().all(|b| (b' '..=b'~').contains(&b));
    let chars: usize = line.spans.iter().map(|span| span.content.len()).sum();
    let lead = prefix.width();
    let ends_visible = line
        .spans
        .iter()
        .rev()
        .find(|span| !span.content.is_empty())
        .is_some_and(|span| span.content.as_bytes().last().is_some_and(|b| *b != b' '));
    let fits = chars > 0 && lead + chars <= width && ends_visible;
    if !fits || !line.spans.iter().all(|span| printable(&span.content)) {
        return None;
    }
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(prefix.clone());
    spans.extend(line.spans.iter().map(|span| Span::styled(span.content.to_string(), span.style)));
    let mut cells = Vec::with_capacity(lead + chars);
    cells.resize(lead, None);
    cells.extend(offsets.iter().copied().chain(std::iter::repeat(None)).take(chars));
    Some(Row { line: Line::from(spans).style(line.style), cells })
}

/// Keep one row; clip anything past `width` (code and tables keep their columns).
pub(crate) fn clip_line(line: &Line<'_>, offsets: &[Option<usize>], width: usize) -> Row {
    finish_row(clip_cells(cells_of(line, offsets), width), line.style)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Stylize};

    pub(super) fn plain(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.line.to_string()).collect()
    }

    pub(super) fn identity(n: usize) -> Vec<Option<usize>> {
        (0..n).map(Some).collect()
    }

    #[test]
    fn wraps_on_word_boundaries_and_keeps_offsets() {
        let line = Line::from("the quick brown fox jumps");
        let rows = wrap_line(&line, &identity(25), 10);
        assert_eq!(plain(&rows), ["the quick", "brown fox", "jumps"]);
        assert_eq!(rows.first().map(|r| r.cells.clone()), Some(identity(9)));
        assert_eq!(rows.get(1).and_then(|r| r.cells.first().copied()), Some(Some(10)));
        assert_eq!(rows.get(2).and_then(|r| r.cells.first().copied()), Some(Some(20)));
    }

    #[test]
    fn styles_survive_wrapping_at_span_and_line_level() {
        let line = Line::from(vec!["plain ".into(), "bold text here".bold()]).style(Style::new().italic());
        let rows = wrap_line(&line, &identity(20), 11);
        assert_eq!(plain(&rows), ["plain bold", "text here"]);
        let second = rows.get(1).expect("two rows");
        assert!(second.line.style.add_modifier.contains(Modifier::ITALIC));
        assert!(second.line.spans.first().is_some_and(|s| s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn hard_splits_long_tokens_and_wide_chars_take_two_cells() {
        assert_eq!(
            plain(&wrap_line(&Line::from("abcdefghijkl"), &identity(12), 5)),
            ["abcde", "fghij", "kl"]
        );
        let row = clip_line(&Line::from("日本"), &identity(2), 10);
        assert_eq!(row.cells, [Some(0), Some(0), Some(1), Some(1)]);
    }

    #[test]
    fn empty_line_is_one_row() {
        assert_eq!(wrap_line(&Line::from(""), &[], 20).len(), 1);
    }
}
