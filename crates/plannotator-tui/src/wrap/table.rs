//! Reflowing an already-rendered Unicode table so it fits the window.
//!
//! The Markdown renderer emits the whole box before layout sees it, so this module reads
//! the box-drawing characters back: each line is classified once, columns are re-sized to
//! the content, and every cell is wrapped on its own. Anything it cannot read confidently
//! is clipped instead, which is what the table did before it could reflow.

use ratatui::style::Style;
use ratatui::text::Line;

use super::{Cell, Row, cells_of, cells_width, clip_cells, clip_line, finish_row, wrap_cells};

/// Room a column gets for its longest word before it is asked to share. One very long
/// token (a path, a URL) would otherwise starve every other column of the budget.
const WORD_CAP: usize = 20;

#[derive(Clone, Copy)]
enum TableBorder {
    Top,
    Middle,
    Bottom,
}

/// A table line, read once. Everything downstream is driven from this, never re-parsed.
enum TableLine {
    /// The column widths the border draws, its shape, and the style of its box chars.
    Border(Vec<usize>, TableBorder, Style),
    /// One trimmed run of cells per column, and the style of the row's `│` chars.
    Content(Vec<Vec<Cell>>, Style),
    /// Not part of the box as far as this module can tell.
    Other,
}

/// Reflow a rendered Unicode table to fit `width`, preserving cell boundaries and source offsets.
pub(crate) fn wrap_table(lines: &[Line<'_>], offsets: &[Vec<Option<usize>>], width: usize) -> Vec<Row> {
    let parsed: Vec<TableLine> =
        lines.iter().zip(offsets).map(|(line, map)| parse_table_line(line, map)).collect();
    let Some(TableLine::Border(original_widths, _, border_style)) = parsed.first() else {
        return clip_table(lines, offsets, width);
    };
    let (original_widths, border_style) = (original_widths.clone(), *border_style);

    // A `│` in cell text splits that line into more columns than the border has, and the
    // surplus would be dropped. One unreadable line disqualifies the whole block.
    let ragged = parsed.iter().any(|line| match line {
        TableLine::Content(columns, _) => columns.len() != original_widths.len(),
        _ => false,
    });
    if ragged {
        return clip_table(lines, offsets, width);
    }

    let mut longest_word = vec![0usize; original_widths.len()];
    for line in &parsed {
        if let TableLine::Content(columns, _) = line {
            for (column, longest) in columns.iter().zip(longest_word.iter_mut()) {
                *longest = (*longest).max(longest_word_width(column));
            }
        }
    }
    let column_widths = fit_table_columns(&original_widths, &longest_word, width.max(1));
    let table_wraps = parsed.iter().any(|line| match line {
        TableLine::Content(columns, _) => table_content_wraps(columns, &column_widths),
        _ => false,
    });

    let mut out = Vec::new();
    for (index, parsed_line) in parsed.iter().enumerate() {
        match parsed_line {
            TableLine::Border(_, kind, style) => {
                let style = if *style == Style::default() { border_style } else { *style };
                out.push(render_table_border(&column_widths, *kind, style));
            }
            TableLine::Content(columns, line_border_style) => {
                out.extend(render_table_content(columns, &column_widths, *line_border_style, border_style));
                // Wrapped rows run together without a rule between them.
                let next_is_content = matches!(parsed.get(index + 1), Some(TableLine::Content(..)));
                if table_wraps && next_is_content {
                    out.push(render_table_border(&column_widths, TableBorder::Middle, border_style));
                }
            }
            TableLine::Other => {
                if let Some((line, map)) = lines.get(index).zip(offsets.get(index)) {
                    out.push(clip_line(line, map, width));
                }
            }
        }
    }
    out
}

/// The pre-reflow behaviour: a table keeps its columns and loses what does not fit.
fn clip_table(lines: &[Line<'_>], offsets: &[Vec<Option<usize>>], width: usize) -> Vec<Row> {
    lines.iter().zip(offsets).map(|(line, map)| clip_line(line, map, width)).collect()
}

fn parse_table_line(line: &Line<'_>, offsets: &[Option<usize>]) -> TableLine {
    let cells = cells_of(line, offsets);
    parse_border(&cells).or_else(|| parse_content(&cells)).unwrap_or(TableLine::Other)
}

fn parse_border(cells: &[Cell]) -> Option<TableLine> {
    let first = cells.first()?;
    let (kind, left, intersection, right) = match first.ch {
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
    for cell in cells {
        if cell.ch == left && !seen_left {
            seen_left = true;
        } else if seen_left && cell.ch == '─' {
            current += cell.width;
        } else if seen_left && matches!(cell.ch, c if c == intersection || c == right) {
            if current == 0 {
                return None;
            }
            // The border spans the cell plus one pad column on each side.
            widths.push(current.saturating_sub(2).max(1));
            current = 0;
        } else if seen_left {
            return None;
        }
    }
    (!widths.is_empty()).then_some(TableLine::Border(widths, kind, first.style))
}

fn parse_content(cells: &[Cell]) -> Option<TableLine> {
    let first = cells.first()?;
    if first.ch != '│' || cells.last()?.ch != '│' {
        return None;
    }
    let separators: Vec<usize> =
        cells.iter().enumerate().filter_map(|(index, cell)| (cell.ch == '│').then_some(index)).collect();
    if separators.len() < 2 {
        return None;
    }
    let mut columns = Vec::with_capacity(separators.len() - 1);
    for pair in separators.windows(2) {
        let [left, right] = pair else { continue };
        let padded = cells.get(left + 1..*right)?;
        let start = padded.iter().position(|cell| !cell.ch.is_whitespace()).unwrap_or(padded.len());
        let end = padded.iter().rposition(|cell| !cell.ch.is_whitespace()).map_or(start, |i| i + 1);
        columns.push(padded.get(start..end)?.to_vec());
    }
    Some(TableLine::Content(columns, first.style))
}

/// Column widths for a table that does not fit. Every column first gets room for its
/// longest word (capped, so one huge token cannot starve the rest); the space left over is
/// shared in proportion to how much each column asked for beyond that, so a column of short
/// words stays narrow and the long one takes the room. A column never grows past its own
/// content. Only when the words alone exceed the budget do columns shrink below them.
fn fit_table_columns(original: &[usize], longest_word: &[usize], width: usize) -> Vec<usize> {
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
        spend_remainder(&mut fitted, &minimum, budget);
        return fitted;
    }
    let spare = budget - floor;
    let wants: Vec<usize> = original.iter().zip(&minimum).map(|(&o, &m)| o.saturating_sub(m)).collect();
    let wanted: usize = wants.iter().sum::<usize>().max(1);
    let mut fitted: Vec<usize> =
        minimum.iter().zip(&wants).map(|(&m, &w)| m + w.saturating_mul(spare) / wanted).collect();
    spend_remainder(&mut fitted, original, budget);
    fitted
}

/// Rounding leaves a few cells over: give them to whichever column is squeezed most
/// against what it wanted, until the budget is spent or nobody wants more.
fn spend_remainder(fitted: &mut [usize], want: &[usize], budget: usize) {
    while fitted.iter().sum::<usize>() < budget {
        let deficit =
            |i: usize| want.get(i).copied().unwrap_or(0).saturating_sub(fitted.get(i).copied().unwrap_or(0));
        let Some((index, _)) =
            (0..fitted.len()).map(|i| (i, deficit(i))).filter(|(_, d)| *d > 0).max_by_key(|(_, d)| *d)
        else {
            break;
        };
        if let Some(slot) = fitted.get_mut(index) {
            *slot += 1;
        }
    }
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

fn render_table_border(widths: &[usize], kind: TableBorder, style: Style) -> Row {
    let (left, intersection, right) = match kind {
        TableBorder::Top => ('┌', '┬', '┐'),
        TableBorder::Middle => ('├', '┼', '┤'),
        TableBorder::Bottom => ('└', '┴', '┘'),
    };
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

/// Wrap every cell of one source row and stack the results into as many screen rows as the
/// tallest cell needs. `line_border_style` is the style of this row's own `│` characters;
/// it dresses the padding of a column that has run out of content.
fn render_table_content(
    columns: &[Vec<Cell>],
    widths: &[usize],
    line_border_style: Style,
    border_style: Style,
) -> Vec<Row> {
    let wrapped: Vec<Vec<Vec<Cell>>> = widths
        .iter()
        .enumerate()
        .map(|(index, &width)| {
            let width = width.max(1);
            wrap_cells(columns.get(index).map_or(&[][..], Vec::as_slice), width)
                .into_iter()
                // A cell narrower than one char still has to stay inside its column.
                .map(|piece| clip_cells(super::trim_trailing_space(piece), width))
                .collect()
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    let mut rows = Vec::with_capacity(height);
    for row_index in 0..height {
        let mut cells = vec![Cell { ch: '│', width: 1, offset: None, style: border_style }];
        for (column_index, &width) in widths.iter().enumerate() {
            let piece = wrapped.get(column_index).and_then(|pieces| pieces.get(row_index));
            let style = piece.and_then(|p| p.first().map(|c| c.style)).unwrap_or(line_border_style);
            let used = piece.map_or(0, |p| cells_width(p));
            cells.push(Cell { ch: ' ', width: 1, offset: None, style });
            if let Some(piece) = piece {
                cells.extend_from_slice(piece);
            }
            // The columns this cell left empty, plus the pad column before the next `│`.
            let padding = width.max(1).saturating_sub(used) + 1;
            cells.extend((0..padding).map(|_| Cell { ch: ' ', width: 1, offset: None, style }));
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
    use unicode_width::UnicodeWidthChar;

    fn plain(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.line.to_string()).collect()
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
    fn a_budget_too_small_for_the_words_is_still_spent_in_full() {
        // Words need 21 cells and the budget is 10: scaling gives 3 each and the odd cell
        // used to be left on the table, shrinking the box below the window.
        let widths = fit_table_columns(&[30, 30, 30], &[7, 7, 7], 20);
        assert_eq!(widths.iter().sum::<usize>(), 10, "{widths:?}");
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
        let (_, offsets) = unique_offsets(&lines);
        let rendered = plain(&wrap_table(&lines, &offsets, 40)).join("\n");
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
        let (_, offsets) = unique_offsets(&lines);
        let rows = wrap_table(&lines, &offsets, 24);
        let rendered = plain(&rows).join("\n");

        assert!(rows.iter().all(|row| row.cells.len() <= 24));
        // The long cell wraps, but its text survives intact and in order.
        let squashed: String = rendered.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
        assert!(squashed.contains("TABLE_TAILisalongvalue"), "rendered table was {rendered:?}");
        assert!(rendered.contains("┌") && rendered.contains("└"));
        assert_eq!(rendered.lines().filter(|line| line.starts_with("├")).count(), 3);
    }

    #[test]
    fn every_row_of_a_squeezed_table_is_the_same_width() {
        // One cell per column: a wide char does not fit, and must not push the box out.
        let lines = [
            Line::from("┌──────────────┬──────────────┬──────────────┐"),
            Line::from("│ 中文表格内容 │ 日本語の内容 │ 한국어의내용 │"),
            Line::from("│ 表格内容中文 │ 内容の日本語 │ 내용의한국어 │"),
            Line::from("└──────────────┴──────────────┴──────────────┘"),
        ];
        let (_, offsets) = unique_offsets(&lines);
        let rows = wrap_table(&lines, &offsets, 13);
        let widths: Vec<usize> = rows.iter().map(|row| row.cells.len()).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "ragged table: {:?}", plain(&rows));
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
