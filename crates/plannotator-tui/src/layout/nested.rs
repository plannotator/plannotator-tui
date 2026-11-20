//! Fenced code inside a list item or a quote, given the look of top-level code: its fence
//! lines are dropped, a bar runs beside it, inside the bullets' indent or the quote bars,
//! and it is coloured by language when drawn.
//!
//! The renderer leaves the fences in a list or quote's lines; after decoration (`decorate`)
//! a line's container prefix is `hang.lead` characters long, and what follows is the line's
//! own text. The code's text is read from the source, without the container syntax, so it
//! can be coloured on its own; a drawn cell finds its colour by the source byte it shows.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};

use super::code::{BAR_PREFIX, BAR_WIDTH, Code, Shade, paint_row};
use crate::decorate::Decorated;
use crate::doc::parse_options;
use crate::srcmap::LineOffsets;
use crate::theme::palette;
use crate::wrap::{Hang, Row};

/// One fenced code block inside a list or quote.
#[derive(Debug)]
pub(super) struct Nested {
    code: Code,
    /// Absolute source bytes from the first character of code to the end of the last.
    range: Range<usize>,
    /// The code's lines as written, container syntax left out, and their source bytes.
    lines: Vec<Line<'static>>,
    offsets: Vec<LineOffsets>,
    /// Source byte each line starts at.
    starts: Vec<usize>,
}

/// The fenced code blocks inside a list or quote whose source is `source`, at absolute `base`.
pub(super) fn find(source: &str, base: usize) -> Vec<Nested> {
    let mut found = Vec::new();
    let mut open: Option<Nested> = None;
    let mut line = (String::new(), Vec::new());
    for (event, range) in Parser::new_ext(source, parse_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let code = Code::new(info.trim());
                open = Some(Nested {
                    code,
                    range: 0..0,
                    lines: Vec::new(),
                    offsets: Vec::new(),
                    starts: Vec::new(),
                });
            }
            Event::Text(text) => {
                let Some(nested) = open.as_mut() else { continue };
                // The parser's text is the source's, byte for byte, unless it expanded a tab.
                let mapped = source.get(range.clone()) == Some(text.as_ref());
                for (at, ch) in text.char_indices() {
                    let offset = mapped.then_some(base + range.start + at);
                    if ch == '\n' {
                        nested.push(std::mem::take(&mut line));
                        continue;
                    }
                    if line.1.is_empty() {
                        nested.starts.push(offset.unwrap_or(base + range.start));
                    }
                    line.0.push(ch);
                    line.1.push(offset);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(mut nested) = open.take() {
                    if !line.1.is_empty() {
                        nested.push(std::mem::take(&mut line));
                    }
                    let first = nested.offsets.iter().flatten().flatten().next().copied();
                    let last = nested.offsets.iter().flatten().flatten().next_back().copied();
                    if let (Some(first), Some(last)) = (first, last) {
                        nested.range = first..last + 1;
                    }
                    found.push(nested);
                }
            }
            _ => {}
        }
    }
    found
}

impl Nested {
    fn push(&mut self, (text, offsets): (String, LineOffsets)) {
        if offsets.is_empty() {
            // A blank line of code: it starts where the previous one ended.
            let start = self.starts.last().copied().unwrap_or(self.range.start);
            self.starts.push(start);
        }
        self.lines.push(Line::raw(text));
        self.offsets.push(offsets);
    }

    fn holds(&self, offset: usize) -> bool {
        self.range.contains(&offset)
    }
}

/// What a decorated line is, around nested code.
enum Kind {
    Fence,
    Code,
    Other,
}

/// Drop the fences of `nested` code among a list or quote's decorated lines and put the bar
/// beside its lines.
pub(super) fn restyle(decorated: Decorated, nested: &[Nested]) -> Decorated {
    let Decorated { text, offsets, hangs } = decorated;
    let mut out =
        Decorated { text: Text::default().style(text.style), offsets: Vec::new(), hangs: Vec::new() };
    let bar_style = Style::new().fg(palette().code_comment);
    // Code whose opening fence has gone by and whose closing one has not.
    let mut inside = false;
    let mut next = 0;
    // A closing fence leaves a blank row, the container prefix alone, unless the block ends
    // there or a blank row follows anyway.
    let mut gap: Option<(Line<'static>, Hang)> = None;
    for ((line, offsets), hang) in text.lines.into_iter().zip(offsets).zip(hangs) {
        let chars: Vec<(char, Style)> =
            line.spans.iter().flat_map(|span| span.content.chars().map(move |ch| (ch, span.style))).collect();
        let lead = hang.lead.min(chars.len());
        let body: String = chars.iter().skip(lead).map(|&(ch, _)| ch).collect();
        let in_code = offsets.iter().skip(lead).flatten().any(|&o| nested.iter().any(|n| n.holds(o)));
        let kind = if in_code {
            Kind::Code
        } else if is_fence(&body) && (inside || next < nested.len()) {
            Kind::Fence
        } else if inside {
            Kind::Code
        } else {
            Kind::Other
        };
        let blank = body.trim().is_empty();
        if let Some((line, hang)) = gap.take().filter(|_| !blank && !matches!(kind, Kind::Fence)) {
            out.offsets.push(vec![None; line.width()]);
            out.text.lines.push(line);
            out.hangs.push(hang);
        }
        match kind {
            Kind::Fence => {
                if inside {
                    let prefix: String = chars.iter().take(lead).map(|&(ch, _)| ch).collect();
                    let style = chars.first().map_or_else(Style::new, |&(_, style)| style);
                    gap = Some((Line::from(Span::styled(prefix.trim_end().to_owned(), style)), hang));
                } else {
                    next += 1;
                }
                inside = !inside;
            }
            Kind::Other => {
                out.text.lines.push(line);
                out.offsets.push(offsets);
                out.hangs.push(hang);
            }
            Kind::Code => {
                let mut spans: Vec<Span<'static>> = Vec::new();
                let prefix = chars.iter().take(lead).map(|&(ch, style)| Span::styled(ch.to_string(), style));
                spans.extend(prefix);
                spans.push(Span::styled(BAR_PREFIX, bar_style));
                spans.extend(chars.iter().skip(lead).map(|&(ch, style)| Span::styled(ch.to_string(), style)));
                let mut cells = offsets;
                let at = lead.min(cells.len());
                cells.splice(at..at, [None; BAR_WIDTH]);
                let mut rest = hang.rest;
                rest.push(Span::styled(BAR_PREFIX, bar_style));
                out.text.lines.push(Line::from(merge(spans)).style(line.style));
                out.offsets.push(cells);
                out.hangs.push(Hang { lead: lead + BAR_WIDTH, rest });
            }
        }
    }
    out
}

/// Whether a line's own text is a code fence the renderer drew.
fn is_fence(body: &str) -> bool {
    let body = body.trim_start();
    body.starts_with("```") || body.starts_with("~~~")
}

/// Adjacent spans of one style as one.
fn merge(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(last) if last.style == span.style => last.content.to_mut().push_str(&span.content),
            _ => out.push(span),
        }
    }
    out
}

/// `row` of a list or quote holding `nested` code, in colour; `None` to draw it as laid out.
pub(super) fn paint(nested: &[Nested], row: &Row) -> Option<Line<'static>> {
    let shows_code = row.cells.iter().flatten().any(|&o| nested.iter().any(|n| n.holds(o)));
    if !shows_code {
        return None;
    }
    Some(paint_row(row, |offset| {
        let Some(nested) = nested.iter().find(|n| n.holds(offset)) else { return Shade::Keep };
        let line = nested.starts.partition_point(|&start| start <= offset).saturating_sub(1);
        nested.code.shade(&nested.lines, &nested.offsets, line, offset)
    }))
}
