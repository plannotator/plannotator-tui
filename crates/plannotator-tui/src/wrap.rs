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

/// Drop trailing whitespace: a wrapped piece can end on the space it broke at.
fn trim_trailing_space(mut cells: Vec<Cell>) -> Vec<Cell> {
    while cells.last().is_some_and(|c| c.ch.is_whitespace()) {
        cells.pop();
    }
    cells
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

    // Tokens are unbreakable runs: a word, or a run of whitespace.
    let mut rest = cells;
    while let Some(first) = rest.first() {
        let is_space = first.ch.is_whitespace();
        let len = rest.iter().take_while(|c| c.ch.is_whitespace() == is_space).count();
        let (token, tail) = rest.split_at(len);
        rest = tail;
        let token_width: usize = token.iter().map(|c| c.width).sum();

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
    let Some((original_widths, _, border_style)) =
        lines.first().zip(offsets.first()).and_then(|(line, map)| table_border(line, map))
    else {
        return clip_table(lines, offsets, width);
    };
    // A `│` in cell text splits that line into more columns than the border has, and the
    // surplus would be dropped. One unreadable line disqualifies the whole block.
    let ragged = lines
        .iter()
        .zip(offsets)
        .filter_map(|(line, map)| table_content_cells(line, map))
        .any(|(cells, _)| cells.len() != original_widths.len());
    if ragged {
        return clip_table(lines, offsets, width);
    }

    let mut longest_word = vec![0usize; original_widths.len()];
    for (line, map) in lines.iter().zip(offsets) {
        if let Some((cells, _)) = table_content_cells(line, map) {
            for (column, longest) in cells.iter().zip(longest_word.iter_mut()) {
                *longest = (*longest).max(longest_word_width(column));
            }
        }
    }
    let column_widths = fit_table_columns(&original_widths, &longest_word, width.max(1));
    let table_wraps = lines
        .iter()
        .zip(offsets)
        .filter_map(|(line, map)| table_content_cells(line, map).map(|(cells, _)| cells))
        .any(|cells| table_content_wraps(&cells, &column_widths));
    let mut out = Vec::new();
    for (line_index, (line, map)) in lines.iter().zip(offsets).enumerate() {
        if let Some((_, kind, style)) = table_border(line, map) {
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
            out.push(clip_line(line, map, width));
        }
    }
    out
}

/// The pre-reflow behaviour: a table keeps its columns and loses what does not fit.
fn clip_table(lines: &[Line<'_>], offsets: &[Vec<Option<usize>>], width: usize) -> Vec<Row> {
    lines.iter().zip(offsets).map(|(line, map)| clip_line(line, map, width)).collect()
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

/// Column widths for a table that does not fit. Every column first gets room for its
/// longest word (capped, so one huge token cannot starve the rest); the space left over is
/// shared in proportion to how much each column asked for beyond that, so a column of short
/// words stays narrow and the long one takes the room. A column never grows past its own
/// content. Only when the words alone exceed the budget do columns shrink below them.
fn fit_table_columns(original: &[usize], longest_word: &[usize], width: usize) -> Vec<usize> {
    const WORD_CAP: usize = 20;
    let columns = original.len();
    let overhead = columns.saturating_mul(3).saturating_add(1);
    let budget = width.saturating_sub(overhead).max(columns);
    let total: usize = original.iter().sum();
    if total == 0 || total <= budget {
        return original.to_vec();
    }
    let minimum: Vec<usize> = original
        .iter()
        .enumerate()
        .map(|(i, &w)| longest_word.get(i).copied().unwrap_or(1).clamp(1, WORD_CAP).min(w).max(1))
        .collect();
    let floor: usize = minimum.iter().sum();
    if floor >= budget {
        // Even the words do not fit: scale them down, keeping every column at least one cell.
        let mut fitted: Vec<usize> =
            minimum.iter().map(|&m| (m.saturating_mul(budget) / floor).max(1)).collect();
        while fitted.iter().sum::<usize>() > budget {
            let Some((index, _)) =
                fitted.iter().enumerate().filter(|(_, w)| **w > 1).max_by_key(|(_, w)| **w)
            else {
                break;
            };
            if let Some(slot) = fitted.get_mut(index) {
                *slot -= 1;
            }
        }
        return fitted;
    }
    let spare = budget - floor;
    let wants: Vec<usize> = original.iter().zip(&minimum).map(|(&o, &m)| o.saturating_sub(m)).collect();
    let wanted: usize = wants.iter().sum::<usize>().max(1);
    let mut fitted: Vec<usize> =
        minimum.iter().zip(&wants).map(|(&m, &w)| m + w.saturating_mul(spare) / wanted).collect();
    // Rounding leaves a few cells over: give them to whichever column is squeezed the most.
    while fitted.iter().sum::<usize>() < budget {
        let deficit = |i: usize| {
            original.get(i).copied().unwrap_or(0).saturating_sub(fitted.get(i).copied().unwrap_or(0))
        };
        let Some((index, _)) =
            (0..columns).map(|i| (i, deficit(i))).filter(|(_, d)| *d > 0).max_by_key(|(_, d)| *d)
        else {
            break;
        };
        if let Some(slot) = fitted.get_mut(index) {
            *slot += 1;
        }
    }
    fitted
}

/// The widest run of non-whitespace cells in a rendered cell's content.
fn longest_word_width(cells: &[Cell]) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for cell in cells {
        if cell.ch.is_whitespace() {
            longest = longest.max(run);
            run = 0;
        } else {
            run += cell.width;
        }
    }
    longest.max(run)
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
    let wrapped: Vec<Vec<Vec<Cell>>> = widths
        .iter()
        .enumerate()
        .map(|(index, &width)| {
            wrap_cells(columns.get(index).map_or(&[][..], Vec::as_slice), width.max(1))
                .into_iter()
                .map(trim_trailing_space)
                .collect()
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    let mut rows = Vec::with_capacity(height);
    for row_index in 0..height {
        let mut cells = vec![Cell { ch: '│', width: 1, offset: None, style: border_style }];
        for (column_index, &width) in widths.iter().enumerate() {
            let piece = wrapped.get(column_index).and_then(|pieces| pieces.get(row_index));
            let row_style = piece.and_then(|p| p.first().map(|c| c.style)).unwrap_or(cell_style);
            cells.push(Cell { ch: ' ', width: 1, offset: None, style: row_style });
            let used = piece.map_or(0, |p| p.iter().map(|c| c.width).sum());
            if let Some(piece) = piece {
                cells.extend_from_slice(piece);
            }
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
        wrap_cells(columns.get(index).map_or(&[][..], Vec::as_slice), width.max(1)).len() > 1
    })
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

    /// Offsets that are unique across the whole table, so a cell offset names exactly one
    /// source char. Returns the flat source and one offset per char of each line.
    fn unique_offsets(lines: &[Line<'static>]) -> (Vec<char>, Vec<Vec<Option<usize>>>) {
        let mut source = Vec::new();
        let mut offsets = Vec::new();
        for line in lines {
            let chars: Vec<char> = line.to_string().chars().collect();
            offsets.push((source.len()..source.len() + chars.len()).map(Some).collect());
            source.extend(chars);
        }
        (source, offsets)
    }

    /// Every mapped column of every row points at the char actually drawn there.
    fn assert_cells_name_their_char(rows: &[Row], source: &[char]) {
        for row in rows {
            let mut column = 0usize;
            for ch in row.line.spans.iter().flat_map(|span| span.content.chars()) {
                let width = ch.width().unwrap_or(0);
                for slot in column..column + width {
                    if let Some(offset) = row.cells[slot] {
                        assert_eq!(
                            source.get(offset).copied(),
                            Some(ch),
                            "column {slot} of {:?} claims offset {offset} ({:?})",
                            row.line.to_string(),
                            source.get(offset)
                        );
                    }
                }
                column += width;
            }
            assert_eq!(column, row.cells.len(), "row width disagrees with its cell map");
        }
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
    fn columns_keep_their_longest_word_and_share_the_rest_in_proportion() {
        // Step 37, Owner 8, Risk 6, Notes 60 at a 68-cell document: budget is 68 - 13 = 55.
        // Longest words: "accounts" 8, "platform" 8, "medium" 6, "config/flags.toml," 18.
        let widths = fit_table_columns(&[37, 8, 6, 60], &[8, 8, 6, 18], 68);
        assert_eq!(widths.iter().sum::<usize>(), 55, "{widths:?}");
        assert_eq!(
            (widths[1], widths[2]),
            (8, 6),
            "short columns get exactly their longest word: {widths:?}"
        );
        assert!(widths[3] > widths[0], "the long column takes most of the spare room: {widths:?}");
        assert!(widths.iter().zip([37, 8, 6, 60]).all(|(w, o)| *w <= o), "never wider than the content");
        assert_eq!(
            fit_table_columns(&[10, 20], &[4, 7], 80),
            vec![10, 20],
            "a table that fits keeps its widths"
        );
        let tiny = fit_table_columns(&[30, 30, 30], &[12, 12, 12], 12);
        assert_eq!(tiny.iter().sum::<usize>(), 3, "a tiny budget still gives every column a cell: {tiny:?}");
    }

    #[test]
    fn words_stay_whole_when_a_wrapped_table_has_room_for_them() {
        let lines = [
            Line::from("┌──────────────────────────────────┬──────────┬────────┐"),
            Line::from("│ Step                             │ Owner    │ Risk   │"),
            Line::from("├──────────────────────────────────┼──────────┼────────┤"),
            Line::from("│ Enable the flag for everyone now │ platform │ medium │"),
            Line::from("└──────────────────────────────────┴──────────┴────────┘"),
        ];
        let offsets: Vec<Vec<Option<usize>>> =
            lines.iter().map(|line| (0..line.to_string().chars().count()).map(Some).collect()).collect();
        let rendered = wrap_table(&lines, &offsets, 40)
            .iter()
            .map(|row| row.line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        for word in ["Owner", "platform", "medium", "Risk"] {
            assert!(rendered.contains(word), "{word} was split: {rendered}");
        }
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
        // The long cell wraps, but its text survives intact and in order.
        let squashed: String = rendered.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
        assert!(squashed.contains("TABLE_TAILisalongvalue"), "rendered table was {rendered:?}");
        assert!(rendered.contains("┌") && rendered.contains("└"));
        assert_eq!(rendered.lines().filter(|line| line.starts_with("├")).count(), 3);
    }

    #[test]
    fn a_pipe_inside_a_cell_falls_back_to_clipping_instead_of_dropping_it() {
        // Splitting on every `│` gives this row three columns for a two-column border; the
        // surplus used to be zipped away, taking its text with it.
        let lines = [
            Line::from("┌────────────┬──────────────┐"),
            Line::from("│ Header     │ Description  │"),
            Line::from("├────────────┼──────────────┤"),
            Line::from("│ a │ b      │ pipe inside  │"),
            Line::from("└────────────┴──────────────┘"),
        ];
        let (_, offsets) = unique_offsets(&lines);
        let rendered = plain(&wrap_table(&lines, &offsets, 60)).join("\n");
        assert!(rendered.contains("│ a │ b      │ pipe inside  │"), "cell text was lost: {rendered}");
    }

    #[test]
    fn a_row_the_reflow_cannot_read_is_clipped_not_word_wrapped() {
        // The trailing space stops this line looking like a table row; wrapping it would
        // word-wrap a box-drawing row into the middle of the table.
        let lines = [
            Line::from("┌────────────┬──────────────┐"),
            Line::from("│ Header     │ Description  │"),
            Line::from("├────────────┼──────────────┤"),
            Line::from("│ CLIPME     │ a fairly long value here │ "),
            Line::from("└────────────┴──────────────┘"),
        ];
        let (_, offsets) = unique_offsets(&lines);
        let rows = wrap_table(&lines, &offsets, 24);
        let clipped: Vec<String> = plain(&rows).into_iter().filter(|row| row.contains("CLIPME")).collect();
        assert_eq!(clipped.len(), 1, "the unreadable row was wrapped: {clipped:?}");
        let row = clipped.first().expect("one row");
        assert!(lines[3].to_string().starts_with(row.as_str()), "not a prefix of the source: {row:?}");
        assert!(row.chars().count() <= 24);
    }

    #[test]
    fn wrapped_table_cells_still_name_the_source_char_under_them() {
        // Wide chars take two columns and variation selectors and ZWJ take none, so a cell
        // map built by counting columns per char drifts after the first of either.
        let lines = [
            Line::from("┌──────────────────────┬──────────────────────┐"),
            Line::from("│ Head                 │ Detail               │"),
            Line::from("├──────────────────────┼──────────────────────┤"),
            Line::from("│ alpha beta gamma del │ 中文 漢字 表格 内容 説明 │"),
            Line::from("│ ⚠️ caution ⚠️ severe │ 👨‍👩‍👧 family 👍 fine │"),
            Line::from("└──────────────────────┴──────────────────────┘"),
        ];
        let (source, offsets) = unique_offsets(&lines);
        let rows = wrap_table(&lines, &offsets, 30);
        assert!(rows.len() > lines.len(), "the fixture must actually wrap: {}", rows.len());
        assert_cells_name_their_char(&rows, &source);
    }
}
