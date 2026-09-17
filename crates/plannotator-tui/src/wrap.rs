//! Word-wrapping of styled lines to a column width, carrying a source offset per cell.
//!
//! `tui-markdown` emits one `Line` per logical line and never wraps; the layout needs
//! exact row counts and the selection needs to know which source byte sits under each
//! screen column, so both live here. Generic text wrapping — no markdown knowledge.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

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

#[derive(Clone, Copy)]
enum TableBorder {
    Top,
    Middle,
    Bottom,
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
        columns.extend(std::iter::repeat_n(cell.offset, cell.width));
    }
    Row { line: Line::from(spans).style(line_style), cells: columns }
}

/// Wrap one logical line into as many rows as needed for `width` columns.
/// `offsets` has one entry per char of the line. An empty line yields one empty row.
pub(crate) fn wrap_line(line: &Line<'_>, offsets: &[Option<usize>], width: usize) -> Vec<Row> {
    let width = width.max(1);
    let cells = cells_of(line, offsets);
    let mut rows: Vec<Row> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut current_width = 0usize;

    // Tokens are unbreakable runs: a word, or a run of whitespace.
    let mut rest = cells.as_slice();
    while let Some(first) = rest.first() {
        let is_space = first.ch.is_whitespace();
        let len = rest.iter().take_while(|c| c.ch.is_whitespace() == is_space).count();
        let (token, tail) = rest.split_at(len);
        rest = tail;
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
pub(crate) fn clip_line(line: &Line<'_>, offsets: &[Option<usize>], width: usize) -> Row {
    let mut used = 0usize;
    let kept = cells_of(line, offsets)
        .into_iter()
        .take_while(|cell| {
            let fits = used + cell.width <= width;
            if fits {
                used += cell.width;
            }
            fits
        })
        .collect();
    finish_row(kept, line.style)
}

/// Reflow a rendered Unicode table to fit `width`, preserving cell boundaries and source offsets.
/// The Markdown renderer emits a complete table before this layer sees it, so the table is
/// identified from its box-drawing borders and each rendered cell is wrapped independently.
pub(crate) fn wrap_table(lines: &[Line<'_>], offsets: &[Vec<Option<usize>>], width: usize) -> Vec<Row> {
    let Some((original_widths, border_kind, border_style)) =
        lines.first().zip(offsets.first()).and_then(|(line, map)| table_border(line, map))
    else {
        return lines.iter().zip(offsets).flat_map(|(line, map)| wrap_line(line, map, width)).collect();
    };

    let column_widths = fit_table_columns(&original_widths, width.max(1));
    let table_wraps = lines
        .iter()
        .zip(offsets)
        .filter_map(|(line, map)| table_content_cells(line, map).map(|(cells, _)| cells))
        .any(|cells| table_content_wraps(&cells, &column_widths));
    let mut out = Vec::new();
    for (line_index, (line, map)) in lines.iter().zip(offsets).enumerate() {
        if let Some((_, kind, style)) = table_border(line, map) {
            // Keep the detected border kind for each row; the first line's kind only supplies
            // the style fallback when a renderer emits an unusual border sequence.
            let kind = if matches!(kind, TableBorder::Top | TableBorder::Middle | TableBorder::Bottom) {
                kind
            } else {
                border_kind
            };
            out.push(render_table_border(&column_widths, kind, style, border_style));
        } else if let Some((cells, style)) = table_content_cells(line, map) {
            let content_rows = render_table_content(&cells, &column_widths, style, border_style);
            out.extend(content_rows);
            let next_is_content = lines
                .get(line_index + 1)
                .zip(offsets.get(line_index + 1))
                .is_some_and(|(next_line, next_map)| table_content_cells(next_line, next_map).is_some());
            if table_wraps && next_is_content {
                out.push(render_table_border(
                    &column_widths,
                    TableBorder::Middle,
                    border_style,
                    border_style,
                ));
            }
        } else {
            out.extend(wrap_line(line, map, width));
        }
    }
    out
}

fn table_border(line: &Line<'_>, offsets: &[Option<usize>]) -> Option<(Vec<usize>, TableBorder, Style)> {
    let cells = cells_of(line, offsets);
    let first = cells.first()?.ch;
    let (kind, left, intersection, right) = match first {
        '┌' => (TableBorder::Top, '┌', '┬', '┐'),
        '├' => (TableBorder::Middle, '├', '┼', '┤'),
        '└' => (TableBorder::Bottom, '└', '┴', '┘'),
        _ => return None,
    };
    if cells.last()?.ch != right {
        return None;
    }
    let mut widths = Vec::new();
    let mut current = 0usize;
    let mut seen_left = false;
    for cell in &cells {
        if cell.ch == left && !seen_left {
            seen_left = true;
        } else if seen_left && cell.ch == '─' {
            current += cell.width;
        } else if seen_left && matches!(cell.ch, c if c == intersection || c == right) {
            if current == 0 {
                return None;
            }
            widths.push(current.saturating_sub(2).max(1));
            current = 0;
        } else if seen_left {
            return None;
        }
    }
    (!widths.is_empty()).then(|| (widths, kind, cells.first().map_or(Style::default(), |c| c.style)))
}

fn table_content_cells(line: &Line<'_>, offsets: &[Option<usize>]) -> Option<(Vec<Vec<Cell>>, Style)> {
    let cells = cells_of(line, offsets);
    if cells.first()?.ch != '│' || cells.last()?.ch != '│' {
        return None;
    }
    let separator_indices: Vec<usize> =
        cells.iter().enumerate().filter_map(|(index, cell)| (cell.ch == '│').then_some(index)).collect();
    if separator_indices.len() < 2 {
        return None;
    }
    let mut columns = Vec::with_capacity(separator_indices.len() - 1);
    for pair in separator_indices.windows(2) {
        let [left, right] = pair else { continue };
        let mut column = cells.get(left + 1..*right)?.to_vec();
        while column.first().is_some_and(|cell| cell.ch.is_whitespace()) {
            column.remove(0);
        }
        while column.last().is_some_and(|cell| cell.ch.is_whitespace()) {
            column.pop();
        }
        columns.push(column);
    }
    Some((columns, cells.first().map_or(Style::default(), |c| c.style)))
}

fn fit_table_columns(original: &[usize], width: usize) -> Vec<usize> {
    let columns = original.len();
    let overhead = columns.saturating_mul(3).saturating_add(1);
    let content_budget = width.saturating_sub(overhead).max(columns);
    let original_total: usize = original.iter().sum();
    if original_total <= content_budget {
        return original.to_vec();
    }
    let mut fitted = vec![1; columns];
    let mut remaining = content_budget.saturating_sub(columns);
    let mut order: Vec<usize> = (0..columns).collect();
    order.sort_by_key(|&index| std::cmp::Reverse(original.get(index).copied().unwrap_or(0)));
    let mut cursor = 0usize;
    while remaining > 0 && !order.is_empty() {
        let Some(&index) = order.get(cursor % order.len()) else { break };
        if let Some(slot) = fitted.get_mut(index) {
            *slot += 1;
        }
        cursor += 1;
        remaining -= 1;
    }
    fitted
}

fn render_table_border(widths: &[usize], kind: TableBorder, style: Style, fallback_style: Style) -> Row {
    let (left, intersection, right) = match kind {
        TableBorder::Top => ('┌', '┬', '┐'),
        TableBorder::Middle => ('├', '┼', '┤'),
        TableBorder::Bottom => ('└', '┴', '┘'),
    };
    let style = if style == Style::default() { fallback_style } else { style };
    let mut cells = vec![Cell { ch: left, width: 1, offset: None, style }];
    for (index, &width) in widths.iter().enumerate() {
        cells.extend((0..width + 2).map(|_| Cell { ch: '─', width: 1, offset: None, style }));
        if index + 1 < widths.len() {
            cells.push(Cell { ch: intersection, width: 1, offset: None, style });
        }
    }
    cells.push(Cell { ch: right, width: 1, offset: None, style });
    finish_row(cells, Style::default())
}

fn render_table_content(
    columns: &[Vec<Cell>],
    widths: &[usize],
    cell_style: Style,
    border_style: Style,
) -> Vec<Row> {
    let wrapped: Vec<Vec<Row>> = widths
        .iter()
        .enumerate()
        .map(|(index, &width)| {
            let line = finish_row(columns.get(index).cloned().unwrap_or_default(), cell_style);
            wrap_line(&line.line, &line.cells, width.max(1))
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    let mut rows = Vec::with_capacity(height);
    for row_index in 0..height {
        let mut cells = vec![Cell { ch: '│', width: 1, offset: None, style: border_style }];
        for (column_index, &width) in widths.iter().enumerate() {
            let row = wrapped.get(column_index).and_then(|rows| rows.get(row_index));
            let row_style =
                row.and_then(|value| value.line.spans.first().map(|span| span.style)).unwrap_or(cell_style);
            cells.push(Cell { ch: ' ', width: 1, offset: None, style: row_style });
            if let Some(row) = row {
                cells.extend(cells_from_row(row));
            }
            let used = row.map_or(0, |value| value.cells.len());
            cells.extend((0..=width.saturating_sub(used)).map(|_| Cell {
                ch: ' ',
                width: 1,
                offset: None,
                style: row_style,
            }));
            cells.push(Cell { ch: '│', width: 1, offset: None, style: border_style });
        }
        rows.push(finish_row(cells, Style::default()));
    }
    rows
}

fn table_content_wraps(columns: &[Vec<Cell>], widths: &[usize]) -> bool {
    widths.iter().enumerate().any(|(index, &width)| {
        let line = finish_row(columns.get(index).cloned().unwrap_or_default(), Style::default());
        wrap_line(&line.line, &line.cells, width.max(1)).len() > 1
    })
}

fn cells_from_row(row: &Row) -> Vec<Cell> {
    let mut offsets = row.cells.iter();
    row.line
        .spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |ch| (ch, span.style)))
        .map(|(ch, style)| {
            let offset = offsets.next().copied().flatten();
            for _ in 1..ch.width().unwrap_or(0) {
                let _ = offsets.next();
            }
            Cell { ch, width: ch.width().unwrap_or(0), offset, style }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
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

    #[test]
    fn table_cells_wrap_without_breaking_the_box() {
        let lines = [
            Line::from("┌────────────┬──────────────┐"),
            Line::from("│ Header     │ Description  │"),
            Line::from("├────────────┼──────────────┤"),
            Line::from("│ item       │ TABLE_TAIL is a long value │"),
            Line::from("│ second     │ another long value         │"),
            Line::from("│ third      │ short                      │"),
            Line::from("└────────────┴──────────────┘"),
        ];
        let offsets: Vec<Vec<Option<usize>>> =
            lines.iter().map(|line| (0..line.to_string().chars().count()).map(Some).collect()).collect();
        let rows = wrap_table(&lines, &offsets, 24);
        let rendered = rows.iter().map(|row| row.line.to_string()).collect::<Vec<_>>().join("\n");

        assert!(rows.iter().all(|row| row.cells.len() <= 24));
        assert!(
            rendered.contains("TABLE_TAI") && rendered.contains("L is a"),
            "rendered table was {rendered:?}"
        );
        assert!(rendered.contains("┌") && rendered.contains("└"));
        assert_eq!(rendered.lines().filter(|line| line.starts_with("├")).count(), 3);
    }
}
