//! Word-wrapping of styled lines to a column width, carrying a source offset per cell.
//!
//! `tui-markdown` emits one `Line` per logical line and never wraps; the layout needs
//! exact row counts and the selection needs to know which source byte sits under each
//! screen column, so both live here.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// One screen row: styled text plus, per column, the source byte it came from.
pub struct Row {
    pub line: Line<'static>,
    /// `cells[col]` is the source offset shown at that column (wide chars repeat it).
    pub cells: Vec<Option<usize>>,
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
    let mut out = Vec::new();
    let mut i = 0;
    for span in &line.spans {
        for ch in span.content.chars() {
            let offset = offsets.get(i).copied().flatten();
            i += 1;
            out.push(Cell { ch, width: ch.width().unwrap_or(0), offset, style: span.style });
        }
    }
    out
}

/// Turn accumulated cells into a row, merging same-style runs into spans.
fn finish_row(mut cells: Vec<Cell>, line_style: Style) -> Row {
    while cells.last().is_some_and(|c| c.ch.is_whitespace()) {
        cells.pop();
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut columns: Vec<Option<usize>> = Vec::new();
    for cell in &cells {
        match spans.last_mut() {
            Some(last) if last.style == cell.style => last.content.to_mut().push(cell.ch),
            _ => spans.push(Span::styled(cell.ch.to_string(), cell.style)),
        }
        for _ in 0..cell.width {
            columns.push(cell.offset);
        }
    }
    Row { line: Line::from(spans).style(line_style), cells: columns }
}

/// Wrap one logical line into as many rows as needed for `width` columns.
/// `offsets` has one entry per char of the line. An empty line yields one empty row.
pub fn wrap_line(line: &Line<'_>, offsets: &[Option<usize>], width: usize) -> Vec<Row> {
    let width = width.max(1);
    let cells = cells_of(line, offsets);
    let mut rows: Vec<Row> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut current_width = 0usize;

    // Group into unbreakable tokens: words and whitespace runs.
    let mut i = 0;
    while i < cells.len() {
        let is_space = cells[i].ch.is_whitespace();
        let start = i;
        while i < cells.len() && cells[i].ch.is_whitespace() == is_space {
            i += 1;
        }
        let token = &cells[start..i];
        let token_width: usize = token.iter().map(|c| c.width).sum();

        // Whitespace at a row start is dropped, except leading indentation on the first row.
        if is_space && current.is_empty() && !rows.is_empty() {
            continue;
        }
        if current_width + token_width <= width {
            current.extend_from_slice(token);
            current_width += token_width;
            continue;
        }
        if !current.is_empty() {
            rows.push(finish_row(std::mem::take(&mut current), line.style));
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
                    rows.push(finish_row(std::mem::take(&mut current), line.style));
                    current_width = 0;
                }
                current.push(*cell);
                current_width += cell.width;
            }
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(finish_row(current, line.style));
    }
    rows
}

/// Keep one row; clip anything past `width` (code and tables keep their columns).
pub fn clip_line(line: &Line<'_>, offsets: &[Option<usize>], width: usize) -> Row {
    let mut kept = Vec::new();
    let mut used = 0usize;
    for cell in cells_of(line, offsets) {
        if used + cell.width > width {
            break;
        }
        used += cell.width;
        kept.push(cell);
    }
    finish_row(kept, line.style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Stylize};

    fn plain(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.line.to_string()).collect()
    }

    fn identity(n: usize) -> Vec<Option<usize>> {
        (0..n).map(Some).collect()
    }

    #[test]
    fn wraps_on_word_boundaries() {
        let line = Line::from("the quick brown fox jumps");
        assert_eq!(plain(&wrap_line(&line, &identity(25), 10)), vec!["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn cells_carry_source_offsets_across_rows() {
        let line = Line::from("the quick brown fox jumps");
        let rows = wrap_line(&line, &identity(25), 10);
        assert_eq!(rows[0].cells, identity(9)); // "the quick"
        assert_eq!(rows[1].cells[0], Some(10)); // 'b' of brown
        assert_eq!(rows[2].cells[0], Some(20)); // 'j' of jumps
    }

    #[test]
    fn preserves_styles_across_wraps() {
        let line = Line::from(vec!["plain ".into(), "bold text here".bold()]);
        let rows = wrap_line(&line, &identity(20), 11);
        assert_eq!(plain(&rows), vec!["plain bold", "text here"]);
        assert!(rows[0].line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert!(rows[1].line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn keeps_line_level_style_on_every_row() {
        let line = Line::from("a heading that wraps").style(Style::new().bold());
        let rows = wrap_line(&line, &identity(20), 10);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.line.style.add_modifier.contains(Modifier::BOLD)));
        assert!(clip_line(&line, &identity(20), 5).line.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn hard_splits_long_tokens() {
        let line = Line::from("abcdefghijkl");
        assert_eq!(plain(&wrap_line(&line, &identity(12), 5)), vec!["abcde", "fghij", "kl"]);
    }

    #[test]
    fn wide_chars_occupy_two_cells() {
        let line = Line::from("日本");
        let row = clip_line(&line, &identity(2), 10);
        assert_eq!(row.cells, vec![Some(0), Some(0), Some(1), Some(1)]);
    }

    #[test]
    fn empty_line_is_one_row() {
        assert_eq!(wrap_line(&Line::from(""), &[], 20).len(), 1);
    }
}
