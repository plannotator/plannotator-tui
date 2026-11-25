//! A suggestion's code in a note's box: the lines it replaces as `-` lines, then the code it
//! suggests as `+` lines, with the indentation they all share taken off so the code gets the
//! width. A line too long for the box breaks after a space or punctuation near the end of
//! the row when there is one, else where the row is full; its sign is on its first row only.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar as _;

/// How far back from a full row a break after a space or punctuation is looked for.
const LOOK_BACK: usize = 12;

/// The `-` and `+` rows for `original` and `suggested`, `width` columns wide, tabs `tab`
/// columns.
pub(super) fn rows(
    original: Option<&str>,
    suggested: Option<&str>,
    width: usize,
    tab: usize,
    (old, new): (Style, Style),
) -> Vec<Line<'static>> {
    let tabs = " ".repeat(tab);
    let lines = |code: Option<&str>, sign: &'static str, style: Style| {
        code.into_iter()
            .flat_map(|code| code.split('\n'))
            .map(|line| (sign, style, line.trim_end_matches('\r').replace('\t', &tabs)))
            .collect::<Vec<_>>()
    };
    let mut all = lines(original, "- ", old);
    all.extend(lines(suggested, "+ ", new));
    let indent = all
        .iter()
        .filter(|(_, _, line)| !line.trim().is_empty())
        .map(|(_, _, line)| line.len() - line.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0);
    let room = width.saturating_sub(2).max(1);
    let mut rows = Vec::new();
    for (sign, style, line) in all {
        let line = line.get(indent..).unwrap_or_default();
        for (index, piece) in pieces(line, room).into_iter().enumerate() {
            let lead = if index == 0 { sign } else { "  " };
            rows.push(Line::from(Span::styled(format!("{lead}{piece}"), style)));
        }
    }
    rows
}

/// `line` in rows of at most `room` columns, every character kept.
fn pieces(line: &str, room: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut rest: Vec<char> = line.chars().collect();
    loop {
        let mut used = 0;
        let mut fits = 0;
        for &ch in &rest {
            let wide = ch.width().unwrap_or(0);
            if used + wide > room && fits > 0 {
                break;
            }
            used += wide;
            fits += 1;
        }
        if fits >= rest.len() {
            rows.push(rest.into_iter().collect());
            return rows;
        }
        let soft = (fits.saturating_sub(LOOK_BACK).max(1)..fits).rev().find(|&cut| {
            rest.get(cut - 1).is_some_and(|&ch| ch == ' ' || (ch.is_ascii_punctuation() && ch != '_'))
        });
        let cut = soft.unwrap_or(fits);
        let tail = rest.split_off(cut);
        rows.push(rest.into_iter().collect());
        rest = tail;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Line<'_>]) -> Vec<String> {
        rows.iter().map(|row| row.spans.iter().map(|span| span.content.as_ref()).collect()).collect()
    }

    #[test]
    fn the_shared_indentation_goes_and_long_lines_break_after_punctuation() {
        let styles = (Style::new(), Style::new());
        let old = "        if ready {\n            send(Error::from(e));\n        }";
        let new = "        if ready { send(Error::Route(e)) }";
        let rows = rows(Some(old), Some(new), 22, 4, styles);
        assert_eq!(
            text(&rows),
            [
                "- if ready {",
                "-     send(Error::",
                "  from(e));",
                "- }",
                "+ if ready { send(",
                "  Error::Route(e)) }"
            ]
        );
    }

    #[test]
    fn a_line_with_no_break_near_the_end_breaks_where_the_row_is_full() {
        assert_eq!(pieces("abcdefghijklmnopqrstuvwxyz", 10), ["abcdefghij", "klmnopqrst", "uvwxyz"]);
        assert_eq!(pieces("", 10), [""]);
        assert_eq!(pieces("a b", 1), ["a", " ", "b"]);
    }
}
