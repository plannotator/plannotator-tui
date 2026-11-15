//! Bullets, task boxes, quote bars and hanging indents for list and quote blocks.
//!
//! `tui-markdown` leaves `- `, `1. `, `[x] ` and `>` in its output. After alignment, each
//! rendered line's leading run of container syntax (as [`Syntax`] reports it from the
//! parser) is replaced by a prefix built from the containers around the line's text: a bar
//! per quote, then per list item either its marker or, after its first line, blank space of
//! the same width. Wrapped rows repeat that blank-and-bar prefix. Characters that stay keep
//! their offsets, and the new prefix maps to no source byte.
//!
//! Every glyph here is one column wide even where East Asian Ambiguous characters are drawn
//! wide, so a decorated row never outgrows the width it was wrapped to.

mod syntax;
#[cfg(test)]
mod tests;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use unicode_width::UnicodeWidthStr as _;

pub(crate) use syntax::Syntax;
use syntax::{Container, Kind, Part};

use crate::srcmap::LineOffsets;
use crate::theme::palette;
use crate::wrap::Hang;

const BULLETS: [char; 3] = ['∙', '◦', '▪'];
const TASK_OPEN: char = '☐';
const TASK_DONE: char = '☑';
const QUOTE_BAR: char = '╎';
/// Drawn across the width for a thematic break.
pub(crate) const RULE: char = '╌';

type StyledChar = (char, Style, Option<usize>);

/// A block's decorated lines, their offsets, and how each line's wrapped rows start.
pub(crate) struct Decorated {
    pub(crate) text: Text<'static>,
    pub(crate) offsets: Vec<LineOffsets>,
    pub(crate) hangs: Vec<Hang>,
}

/// Decorate the rendered lines of a block whose source is `source`, at absolute `base`.
pub(crate) fn decorate(
    syntax: &Syntax,
    source: &str,
    base: usize,
    text: Text<'static>,
    offsets: Vec<LineOffsets>,
) -> Decorated {
    let mut started = vec![false; syntax.containers.len()];
    let mut context = None;
    let mut out =
        Decorated { text: Text::default().style(text.style), offsets: Vec::new(), hangs: Vec::new() };
    for (line, offsets) in text.lines.into_iter().zip(offsets) {
        let chars = styled_chars(&line, &offsets);
        let (line, offsets, hang) =
            match decorate_line(syntax, source, base, &chars, &mut started, &mut context) {
                Some((chars, hang)) => {
                    let (line, offsets) = rebuild(chars, line.style);
                    (line, offsets, hang)
                }
                None => (line, offsets, Hang::default()),
            };
        out.text.lines.push(line);
        out.offsets.push(offsets);
        out.hangs.push(hang);
    }
    out
}

/// The line with its container prefix rebuilt, or `None` to keep it as rendered. `context` is
/// the source offset the previous line was placed by, for a line with no source of its own.
fn decorate_line(
    syntax: &Syntax,
    source: &str,
    base: usize,
    chars: &[StyledChar],
    started: &mut [bool],
    context: &mut Option<usize>,
) -> Option<(Vec<StyledChar>, Hang)> {
    let offsets = || chars.iter().filter_map(|&(_, _, offset)| offset);
    let here = offsets().find(|&o| syntax.is_leaf(o)).or_else(|| offsets().next_back());
    *context = here.or(*context);
    let chain = syntax.chain((*context)?);
    if chain.is_empty() {
        return None;
    }
    let quotes = chain.iter().filter(|&&i| syntax.containers.get(i).is_some_and(Container::is_quote)).count();
    let leading = leading_syntax(syntax, source, base, chars, quotes);
    let body = chars.get(leading.len..).unwrap_or_default();
    // A line with no text (a blank line between blocks) shows the bars it was written with.
    let bars = if body.is_empty() { leading.quote_marks } else { quotes };

    let mut lead: Vec<(char, Style)> = Vec::new();
    let mut rest: Vec<(char, Style)> = Vec::new();
    let mut intact = true;
    let mut drawn = 0;
    for index in chain {
        let Some(container) = syntax.containers.get(index) else { continue };
        let Some(mark) = mark(container) else {
            if drawn == bars {
                break;
            }
            drawn += 1;
            let bar = [(QUOTE_BAR, Style::new().fg(palette().idle_fg)), (' ', Style::new())];
            lead.extend(bar);
            rest.extend(bar);
            continue;
        };
        let blank = std::iter::repeat_n((' ', Style::new()), mark.0.width() + 1);
        rest.extend(blank.clone());
        match started.get_mut(index) {
            Some(started) if !*started => {
                *started = true;
                // The rendered marker must be the one being replaced, or it would show twice.
                intact &= leading.claimed.contains(&index);
                lead.extend(mark.0.chars().map(|ch| (ch, mark.1)));
                lead.push((' ', Style::new()));
            }
            _ => lead.extend(blank),
        }
    }
    if !intact {
        return None;
    }
    let hang = Hang {
        lead: lead.len(),
        rest: rest.into_iter().map(|(ch, style)| Span::styled(ch.to_string(), style)).collect(),
    };
    let decorated =
        lead.into_iter().map(|(ch, style)| (ch, style, None)).chain(body.iter().copied()).collect();
    Some((decorated, hang))
}

/// A line's leading container syntax.
struct Leading {
    /// Characters that are syntax.
    len: usize,
    /// Items whose marker or task box is among them.
    claimed: Vec<usize>,
    /// Quote marks among them.
    quote_marks: usize,
}

/// Find a line's leading container syntax.
///
/// `tui-markdown` starts a line inside `quotes` quotes with at most that many characters
/// before a space; those are the quote prefix whether or not alignment mapped them. After
/// it, mapped characters are syntax when the parser says so, and padding is whitespace
/// outside text. Text the renderer changed (an entity, say) maps to nothing and stays.
fn leading_syntax(
    syntax: &Syntax,
    source: &str,
    base: usize,
    chars: &[StyledChar],
    quotes: usize,
) -> Leading {
    let prefix = chars
        .iter()
        .take_while(|&&(ch, _, offset)| {
            !ch.is_whitespace() && offset.is_none_or(|o| syntax.part_at(source, base, o) == Some(Part::Quote))
        })
        .count();
    let mut at = if prefix <= quotes { prefix } else { 0 };
    let mut quote_marks = at;
    let mut claimed = Vec::new();
    while let Some(&(ch, _, offset)) = chars.get(at) {
        let taken = match offset.map(|o| (o, syntax.part_at(source, base, o))) {
            Some((_, Some(Part::Marker(index)))) => {
                claimed.push(index);
                true
            }
            Some((_, Some(Part::Quote))) => {
                quote_marks += 1;
                true
            }
            Some((o, None)) => ch.is_whitespace() && !syntax.is_leaf(o),
            None => ch.is_whitespace(),
        };
        if !taken {
            break;
        }
        at += 1;
    }
    Leading { len: at, claimed, quote_marks }
}

/// The text a list item's first line starts with, and its style. `None` for a quote.
fn mark(container: &Container) -> Option<(String, Style)> {
    let Kind::Item { number, task, depth, .. } = &container.kind else { return None };
    let task = task.as_ref().map(|(_, checked)| if *checked { TASK_DONE } else { TASK_OPEN });
    let text = match (number, task) {
        (Some(n), Some(task)) => format!("{n}. {task}"),
        (Some(n), None) => format!("{n}."),
        (None, Some(task)) => task.to_string(),
        (None, None) => {
            BULLETS.get(depth.saturating_sub(1) % BULLETS.len()).copied().unwrap_or('-').to_string()
        }
    };
    let style = if number.is_some() { Style::new().fg(Color::LightBlue) } else { Style::new() };
    Some((text, style))
}

fn styled_chars(line: &Line<'_>, offsets: &[Option<usize>]) -> Vec<StyledChar> {
    let mut offsets = offsets.iter();
    line.spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |ch| (ch, span.style)))
        .map(|(ch, style)| (ch, style, offsets.next().copied().flatten()))
        .collect()
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
