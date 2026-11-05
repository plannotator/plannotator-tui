//! Replace the Markdown syntax `tui-markdown` leaves visible with terminal decoration.
//!
//! The renderer keeps `>` quote markers, `-` bullets, `[ ]` task boxes and code fences as
//! literal text. Runs after source alignment: a marker's replacement maps to no source byte,
//! and every character that stays keeps its offset.

use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};

use crate::doc::BlockKind;
use crate::srcmap::LineOffsets;

const QUOTE_BAR: char = '│';
const BULLETS: [char; 3] = ['•', '◦', '▪'];
const TASK_OPEN: char = '☐';
const TASK_DONE: char = '☑';
/// Columns `tui-markdown` indents each nested list level.
const LIST_INDENT: usize = 4;
const FENCE: &str = "```";

type StyledChar = (char, Style, Option<usize>);

/// Decorate a block's lines, keeping lines and offsets in step.
pub(crate) fn decorate(
    kind: BlockKind,
    text: Text<'static>,
    offsets: Vec<LineOffsets>,
) -> (Text<'static>, Vec<LineOffsets>) {
    let text_style = text.style;
    let mut lines: Vec<(Line<'static>, LineOffsets)> = text.lines.into_iter().zip(offsets).collect();
    match kind {
        BlockKind::CodeBlock => strip_outer_fences(&mut lines),
        BlockKind::List | BlockKind::BlockQuote => lines = decorate_markers(lines),
        _ => {}
    }
    let (lines, offsets): (Vec<_>, Vec<_>) = lines.into_iter().unzip();
    (Text::from(lines).style(text_style), offsets)
}

fn strip_outer_fences(lines: &mut Vec<(Line<'static>, LineOffsets)>) {
    let is_fence = |entry: Option<&(Line<'static>, LineOffsets)>| {
        entry.is_some_and(|(line, _)| line.to_string().trim_start().starts_with(FENCE))
    };
    if lines.len() >= 2 && is_fence(lines.first()) && is_fence(lines.last()) {
        lines.pop();
        lines.remove(0);
    }
}

fn styled_chars(line: &Line<'_>, offsets: &[Option<usize>]) -> Vec<StyledChar> {
    let mut offsets = offsets.iter();
    line.spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |ch| (ch, span.style)))
        .map(|(ch, style)| (ch, style, offsets.next().copied().flatten()))
        .collect()
}

/// The line after a run of quote bars, with its own text: bars and fences are found here.
fn after_quote_prefix(chars: &[StyledChar]) -> &[StyledChar] {
    let mut rest = chars;
    while let Some(((ch, ..), tail)) = rest.split_first() {
        if *ch != '>' && !ch.is_whitespace() {
            break;
        }
        rest = tail;
    }
    rest
}

fn decorate_markers(lines: Vec<(Line<'static>, LineOffsets)>) -> Vec<(Line<'static>, LineOffsets)> {
    let mut in_code = false;
    let mut out = Vec::with_capacity(lines.len());
    for (line, offsets) in lines {
        let chars = styled_chars(&line, &offsets);
        let body: String = after_quote_prefix(&chars).iter().map(|(ch, ..)| ch).collect();
        if body.starts_with(FENCE) {
            in_code = !in_code;
            continue;
        }
        out.push(rebuild(decorate_line(&chars, !in_code), line.style));
    }
    out
}

/// Rewrite a line's leading quote markers and, outside code, its list marker.
fn decorate_line(chars: &[StyledChar], list_marker: bool) -> Vec<StyledChar> {
    let mut out: Vec<StyledChar> = Vec::with_capacity(chars.len() + 2);
    let mut rest = chars;
    let mut indent = 0usize;

    while let Some((&(ch, style, _), tail)) = rest.split_first() {
        if ch == '>' {
            out.push((QUOTE_BAR, style, None));
            out.push((' ', style, None));
            rest = match tail.split_first() {
                Some(((' ', ..), after_space)) => after_space,
                _ => tail,
            };
            indent = 0;
        } else if ch == ' ' {
            out.push((ch, style, None));
            indent += 1;
            rest = tail;
        } else {
            break;
        }
    }
    if list_marker {
        rest = rewrite_list_marker(rest, indent, &mut out);
    }
    out.extend_from_slice(rest);
    out
}

/// Emit the glyph for a bullet or task box at the front of `rest`; return what follows it.
fn rewrite_list_marker<'a>(
    rest: &'a [StyledChar],
    indent: usize,
    out: &mut Vec<StyledChar>,
) -> &'a [StyledChar] {
    let Some((&(marker, style, _), tail)) = rest.split_first() else {
        return rest;
    };
    if !matches!(marker, '-' | '*' | '+') || tail.first().is_none_or(|(ch, ..)| *ch != ' ') {
        return rest;
    }
    let text = tail.get(1..).unwrap_or_default();
    if let Some((checked, after_box)) = task_box(text) {
        out.push((if checked { TASK_DONE } else { TASK_OPEN }, style, None));
        out.push((' ', style, None));
        return after_box;
    }
    let glyph = BULLETS.get((indent / LIST_INDENT) % BULLETS.len()).copied().unwrap_or(marker);
    out.push((glyph, style, None));
    out.push((' ', style, None));
    text
}

/// A leading `[ ] ` or `[x] `: whether it is checked, and the text after it.
fn task_box(text: &[StyledChar]) -> Option<(bool, &[StyledChar])> {
    let [(open, ..), (state, ..), (close, ..), (space, ..), after @ ..] = text else {
        return None;
    };
    if *open != '[' || *close != ']' || *space != ' ' {
        return None;
    }
    match state {
        ' ' => Some((false, after)),
        'x' | 'X' => Some((true, after)),
        _ => None,
    }
}

fn rebuild(chars: Vec<StyledChar>, line_style: Style) -> (Line<'static>, LineOffsets) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut offsets = Vec::with_capacity(chars.len());
    for (ch, style, offset) in chars {
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push(ch),
            _ => spans.push(Span::styled(ch.to_string(), style)),
        }
        offsets.push(offset);
    }
    (Line::from(spans).style(line_style), offsets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(kind: BlockKind, source: &[&str]) -> Vec<String> {
        let offsets: Vec<LineOffsets> =
            source.iter().map(|line| (0..line.chars().count()).map(Some).collect()).collect();
        let text = Text::from(source.iter().map(|line| Line::from((*line).to_owned())).collect::<Vec<_>>());
        let (text, offsets) = decorate(kind, text, offsets);
        for (line, offsets) in text.lines.iter().zip(&offsets) {
            assert_eq!(line.to_string().chars().count(), offsets.len(), "offsets out of step");
        }
        text.lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn bullets_change_by_nesting_level() {
        assert_eq!(
            lines(BlockKind::List, &["- 甲", "    - 乙", "        - 丙", "1. 丁", "    - 戊"]),
            ["• 甲", "    ◦ 乙", "        ▪ 丙", "1. 丁", "    ◦ 戊"]
        );
    }

    #[test]
    fn task_boxes_replace_the_bullet() {
        assert_eq!(lines(BlockKind::List, &["- [ ] 甲", "- [x] 乙"]), ["☐ 甲", "☑ 乙"]);
    }

    #[test]
    fn quote_markers_become_bars_and_nest() {
        assert_eq!(
            lines(BlockKind::BlockQuote, &["> 甲", ">", ">> 乙", "> - 丙"]),
            ["│ 甲", "│ ", "│ │ 乙", "│ • 丙"]
        );
    }

    #[test]
    fn a_marker_in_the_item_text_is_left_alone() {
        assert_eq!(lines(BlockKind::List, &["- > 甲 - 乙"]), ["• > 甲 - 乙"]);
    }

    #[test]
    fn code_fences_disappear_and_code_stays_literal() {
        assert_eq!(lines(BlockKind::CodeBlock, &["```python", "- x", "```"]), ["- x"]);
        assert_eq!(
            lines(BlockKind::List, &["1. 运行：", "    ```bash", "    - x", "    ```", "2. 完成"]),
            ["1. 运行：", "    - x", "2. 完成"]
        );
    }

    #[test]
    fn kept_characters_keep_their_offsets() {
        let (_, offsets) = decorate(
            BlockKind::List,
            Text::from(Line::from("- 甲".to_owned())),
            vec![vec![Some(0), Some(1), Some(2)]],
        );
        assert_eq!(offsets, [vec![None, None, Some(2)]]);
    }
}
