//! Definition lists: the term on its own line, each definition indented under it.
//!
//! `tui-markdown` starts a definition with the `: ` it was written with. That marker is
//! dropped, every line of the definition moves in by [`INDENT`] columns, and its wrapped
//! rows hang at the same depth. Characters that stay keep their offsets; the indent maps to
//! no source byte.

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};
use ratatui::text::{Line, Span};

use crate::doc::parse_options;
use crate::srcmap::LineOffsets;
use crate::wrap::Hang;

/// Columns a definition sits in from its term.
const INDENT: usize = 4;

/// Each definition's absolute source range, in order, in a definition list whose source is
/// `source`, at absolute offset `base`.
pub(super) fn ranges(source: &str, base: usize) -> Vec<Range<usize>> {
    Parser::new_ext(source, parse_options())
        .into_offset_iter()
        .filter(|(event, _)| matches!(event, Event::Start(Tag::DefinitionListDefinition)))
        .map(|(_, range)| range.start + base..range.end + base)
        .collect()
}

/// Indent the definitions among a rendered definition list's `lines`. `definitions` are the
/// absolute source ranges the parser gave each definition.
pub(super) fn indent(
    lines: Vec<Line<'static>>,
    offsets: Vec<LineOffsets>,
    definitions: &[Range<usize>],
) -> (Vec<Line<'static>>, Vec<LineOffsets>, Vec<Hang>) {
    let mut out = (Vec::new(), Vec::new(), Vec::new());
    for (line, offsets) in lines.into_iter().zip(offsets) {
        let definition =
            offsets.iter().flatten().find_map(|o| definitions.iter().find(|range| range.contains(o)));
        let Some(definition) = definition else {
            out.0.push(line);
            out.1.push(offsets);
            out.2.push(Hang::default());
            continue;
        };
        let text: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
        // The marker is the `:` at the definition's start, or one the renderer made up.
        let marker = text.starts_with(": ")
            && offsets.first().copied().flatten().is_none_or(|o| o == definition.start);
        let skip = if marker { 2 } else { 0 };
        let spans = std::iter::once(Span::raw(" ".repeat(INDENT))).chain(drop_chars(line.spans, skip));
        let line = Line::from(spans.collect::<Vec<_>>()).style(line.style);
        let offsets = std::iter::repeat_n(None, INDENT).chain(offsets.into_iter().skip(skip)).collect();
        out.0.push(line);
        out.1.push(offsets);
        out.2.push(Hang { lead: INDENT, rest: vec![Span::raw(" ".repeat(INDENT))] });
    }
    out
}

/// `spans` without their first `count` characters.
fn drop_chars(spans: Vec<Span<'static>>, mut count: usize) -> impl Iterator<Item = Span<'static>> {
    spans.into_iter().filter_map(move |mut span| {
        if count == 0 {
            return Some(span);
        }
        let chars = span.content.chars().count();
        if chars <= count {
            count -= chars;
            return None;
        }
        let rest: String = span.content.chars().skip(count).collect();
        count = 0;
        span.content = rest.into();
        Some(span)
    })
}
