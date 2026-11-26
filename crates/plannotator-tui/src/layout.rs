//! Per-block rendering and the row map.
//!
//! Each block is rendered in isolation through `tui-markdown` (cached, width-independent),
//! aligned back to its source bytes, and wrapped to the current column width. List and quote
//! blocks are decorated in between (`decorate`), definition lists indented (`definitions`),
//! and top-level code drawn beside a bar and coloured when shown (`code`). Every screen cell
//! therefore knows its block and, where it shows real text, its source byte. A long document
//! lays out its first screens before the first frame and the rest in the background
//! (`layout::background`).

mod code;
mod definitions;
mod nested;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::ops::Range;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use tui_markdown::{Options, StyleSheet};

use crate::cjk_emphasis::{protect, strip_marks};
use crate::decorate::{self, Decorated, RULE, Syntax};
use crate::doc::{BlockKind, Document, Opening};
use crate::srcmap::{LineOffsets, align};
use crate::theme::palette;
use crate::wrap::{Hang, Row, clip_line, rule_row, wrap_line, wrap_line_hanging, wrap_table};
use code::Code;
use nested::Nested;

mod background;
mod worker_panic;

use background::{Rest, each_in_parallel, lay_out};
pub(crate) use worker_panic::keep_hook_off_layout_threads;

/// Rows of vertical space between blocks.
const BLOCK_GAP: usize = 1;
/// Further rows above a level one or two heading, so a section reads as one.
const SECTION_GAP: usize = 1;
/// Drawn under a level one heading, across the reading column.
pub(crate) const TITLE_RULE: char = '╍';

/// Rows [`DocLayout::start`] lays out before the first frame: more than any terminal shows.
const FIRST_ROWS: usize = 500;

/// House style: no `#` markers, headings carry weight through bold and colour rather than
/// background color, so the palette stays available for selection and annotations.
#[derive(Debug, Clone)]
struct Styles {
    /// Whether code shows the fences it was written with. A top-level code block draws a bar
    /// instead; code inside a list or quote has nothing else to set it apart.
    fences: bool,
}

impl StyleSheet for Styles {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 | 2 => Style::new().fg(palette().heading).add_modifier(Modifier::BOLD),
            _ => Style::new().fg(palette().subheading).add_modifier(Modifier::BOLD),
        }
    }
    fn heading_marker(&self, _level: u8) -> &'static str {
        ""
    }
    fn code(&self) -> Style {
        Style::new().fg(palette().code)
    }
    fn code_block_fence(&self) -> &str {
        if self.fences { "```" } else { "" }
    }
    fn link(&self) -> Style {
        Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED)
    }
    fn blockquote(&self) -> Style {
        Style::new().fg(Color::Green).add_modifier(Modifier::ITALIC)
    }
}

#[derive(Debug)]
pub(crate) struct RenderedBlock {
    /// Width-independent styled lines from the renderer (owned, cached).
    text: Text<'static>,
    /// Per line, per char: absolute source byte offset (cached with `text`).
    offsets: Vec<LineOffsets>,
    kind: BlockKind,
    /// Per line, how its wrapped rows start; empty when nothing hangs.
    hangs: Vec<Hang>,
    /// A heading's level.
    heading: Option<u8>,
    /// A top-level code block's language and colours.
    code: Option<Code>,
    /// Fenced code inside a list or quote.
    nested: Vec<Nested>,
    /// Rows for the current width.
    pub(crate) rows: Vec<Row>,
    /// First screen row of this block in document coordinates.
    pub(crate) first_row: usize,
}

/// A document laid out for one width. A long one opened with [`DocLayout::start`] holds
/// only its first blocks until [`DocLayout::finish`] adds the rest from the background.
#[derive(Debug)]
pub(crate) struct DocLayout {
    pub(crate) width: usize,
    pub(crate) blocks: Vec<RenderedBlock>,
    pub(crate) total_rows: usize,
    /// The blocks after `blocks`, still being laid out on another thread.
    rest: Option<Rest>,
}

impl RenderedBlock {
    /// Render block `index` of `doc` (the expensive part) and lay it out `width` columns wide.
    fn new(doc: &Document, index: usize, width: usize) -> Self {
        let mut block = render_block(doc, index);
        block.wrap(width);
        block
    }

    /// Lay the block out `width` columns wide.
    fn wrap(&mut self, width: usize) {
        let lines = || self.text.lines.iter().zip(&self.offsets);
        self.rows = if self.kind == BlockKind::Rule {
            let offset = self.offsets.iter().flatten().flatten().next().copied();
            vec![rule_row(RULE, Style::new().fg(palette().idle_fg), offset, width)]
        } else if let Some(code) = &self.code {
            code.rows(&self.text.lines, &self.offsets, width)
        } else if self.kind == BlockKind::Table {
            wrap_table(&self.text.lines, &self.offsets, width)
        } else if self.kind.preserves_columns() {
            lines().map(|(l, o)| clip_line(l, o, width)).collect()
        } else if self.hangs.is_empty() {
            lines().flat_map(|(l, o)| wrap_line(l, o, width)).collect()
        } else {
            lines().zip(&self.hangs).flat_map(|((l, o), h)| wrap_line_hanging(l, o, h, width)).collect()
        };
        if self.heading == Some(1) {
            self.rows.push(rule_row(TITLE_RULE, Style::new().fg(palette().heading), None, width));
        }
    }
}

fn render_block(doc: &Document, index: usize) -> RenderedBlock {
    let source = doc.block_text(index);
    let kind = doc.blocks.get(index).map_or(BlockKind::Other, |b| b.kind);
    let base = doc.blocks.get(index).map_or(0, |b| b.range.start);
    let opening = doc.blocks.get(index).map(|b| &b.opening);
    let protected = protect(source);
    let styles = Styles { fences: kind != BlockKind::CodeBlock };
    let text = own(tui_markdown::from_str_with_options(&protected, &Options::new(styles)));
    let text = if matches!(protected, Cow::Owned(_)) { strip_marks(text) } else { text };
    let plain: Vec<String> = text.lines.iter().map(ToString::to_string).collect();
    let block = |Decorated { text, offsets, hangs }, nested| RenderedBlock {
        text,
        offsets,
        kind,
        hangs,
        heading: match opening {
            Some(Opening::Heading(level)) => Some(*level),
            _ => None,
        },
        code: match opening {
            Some(Opening::Code(info)) => Some(Code::new(info)),
            _ => None,
        },
        nested,
        rows: Vec::new(),
        first_row: 0,
    };
    match kind {
        BlockKind::List | BlockKind::BlockQuote => {
            let syntax = Syntax::parse(source, base);
            let offsets = align(&plain, &syntax.aligned_source(source, base), base);
            let decorated = decorate::decorate(&syntax, source, base, text, offsets);
            let nested = nested::find(source, base);
            if nested.is_empty() {
                block(decorated, nested)
            } else {
                block(nested::restyle(decorated, &nested), nested)
            }
        }
        BlockKind::DefinitionList => {
            let offsets = align(&plain, source, base);
            let definitions = definitions::ranges(source, base);
            let (lines, offsets, hangs) = definitions::indent(text.lines, offsets, &definitions);
            block(Decorated { text: Text::from(lines).style(text.style), offsets, hangs }, Vec::new())
        }
        _ => {
            let offsets = align(&plain, source, base);
            block(Decorated { text, offsets, hangs: Vec::new() }, Vec::new())
        }
    }
}

/// Languages a reader meets first: the info string and code of the first few code blocks
/// that name different ones.
fn first_code(doc: &Document) -> Vec<(String, String)> {
    const LANGUAGES: usize = 4;
    let mut samples: Vec<(String, String)> = Vec::new();
    for (index, block) in doc.blocks.iter().enumerate() {
        let Opening::Code(info) = &block.opening else { continue };
        if info.is_empty() || samples.iter().any(|(seen, _)| **seen == **info) {
            continue;
        }
        let code = doc.block_text(index).split_once('\n').map_or("", |(_, code)| code);
        let end = code.match_indices('\n').nth(crate::highlight::WARM_LINES).map_or(code.len(), |(at, _)| at);
        samples.push((info.to_string(), code.get(..end).unwrap_or(code).to_owned()));
        if samples.len() == LANGUAGES {
            break;
        }
    }
    samples
}

/// `markdown` styled as the document view styles a block, one `Line` per logical line and
/// unwrapped: an agent's answer in the code review's Ask panel.
pub(crate) fn styled(markdown: &str) -> Text<'static> {
    own(tui_markdown::from_str_with_options(markdown, &Options::new(Styles { fences: true })))
}

pub(crate) fn own(text: Text<'_>) -> Text<'static> {
    let lines = text
        .lines
        .into_iter()
        .map(|line| {
            let spans =
                line.spans.into_iter().map(|s| ratatui::text::Span::styled(s.content.into_owned(), s.style));
            Line::from(spans.collect::<Vec<_>>()).style(line.style)
        })
        .collect::<Vec<_>>();
    Text::from(lines).style(text.style)
}

impl DocLayout {
    /// Render and lay out every block for `width` before returning.
    pub(crate) fn build(doc: &Document, width: usize) -> Self {
        crate::highlight::warm_up(first_code(doc));
        let width = width.max(1);
        let mut layout =
            Self { width, blocks: lay_out(doc, 0..doc.blocks.len(), width), total_rows: 0, rest: None };
        layout.place();
        layout
    }

    /// Lay out the first screens of `doc` now and the rest on a background thread, so a
    /// long document shows without waiting for its last block. Rows already laid out never
    /// move when the rest arrives: each block's rows depend on that block alone.
    pub(crate) fn start(doc: &Document, width: usize) -> Self {
        crate::highlight::warm_up(first_code(doc));
        let width = width.max(1);
        let mut blocks = Vec::new();
        let mut rows = 0;
        for index in 0..doc.blocks.len() {
            if rows >= FIRST_ROWS {
                break;
            }
            let block = RenderedBlock::new(doc, index, width);
            rows += block.rows.len() + BLOCK_GAP;
            blocks.push(block);
        }
        let rest = (blocks.len() < doc.blocks.len()).then(|| Rest::spawn(doc, blocks.len(), width));
        let mut layout = Self { width, blocks, total_rows: 0, rest };
        layout.place();
        layout
    }

    /// Whether every block is laid out.
    pub(crate) fn is_complete(&self) -> bool {
        self.rest.is_none()
    }

    /// Wait for the blocks still being laid out and add them. Anything that needs the whole
    /// document (its last row, a block far down, the text under a range) calls this first.
    pub(crate) fn finish(&mut self) {
        let Some(rest) = self.rest.take() else { return };
        let (mut blocks, width) = rest.join();
        if width != self.width {
            each_in_parallel(&mut blocks, |block| block.wrap(self.width));
        }
        self.blocks.append(&mut blocks);
        self.place();
    }

    /// Add the background blocks if they are ready, without waiting. True when they were added.
    pub(crate) fn poll(&mut self) -> bool {
        let ready = self.rest.as_ref().is_some_and(Rest::is_finished);
        if ready {
            self.finish();
        }
        ready
    }

    /// [`DocLayout::finish`] when document row `end` lies past the rows laid out so far.
    pub(crate) fn finish_through(&mut self, end: usize) {
        if end > self.total_rows {
            self.finish();
        }
    }

    /// Re-wrap for a new width without re-rendering markdown. Blocks still in the
    /// background are re-wrapped when they arrive.
    pub(crate) fn reflow(&mut self, width: usize) {
        let width = width.max(1);
        self.width = width;
        each_in_parallel(&mut self.blocks, |block| block.wrap(width));
        self.place();
    }

    /// Give each block its first row: one gap row apart, and more above a section heading.
    fn place(&mut self) {
        let mut row = 0usize;
        for (index, block) in self.blocks.iter_mut().enumerate() {
            if index > 0 && block.heading.is_some_and(|level| level <= 2) {
                row += SECTION_GAP;
            }
            block.first_row = row;
            row += block.rows.len() + BLOCK_GAP;
        }
        self.total_rows = row.saturating_sub(BLOCK_GAP);
    }

    /// Which block owns a document row (gap rows belong to nobody).
    pub(crate) fn block_at_row(&self, row: usize) -> Option<usize> {
        let idx = self.blocks.partition_point(|b| b.first_row <= row);
        let i = idx.checked_sub(1)?;
        let block = self.blocks.get(i)?;
        (row < block.first_row + block.rows.len()).then_some(i)
    }

    /// The row at document coordinate `row`, if it exists (None for gap rows).
    pub(crate) fn row(&self, row: usize) -> Option<&Row> {
        let block = self.blocks.get(self.block_at_row(row)?)?;
        block.rows.get(row - block.first_row)
    }

    /// The row at document coordinate `row` as it is drawn: code in its language's colours.
    /// Colouring is worked out here, for the rows asked for, and kept.
    pub(crate) fn drawn_line(&self, row: usize) -> Option<Cow<'_, Line<'static>>> {
        let block = self.blocks.get(self.block_at_row(row)?)?;
        let index = row - block.first_row;
        let laid_out = block.rows.get(index)?;
        let painted = match &block.code {
            Some(code) => code.paint(&block.text.lines, &block.offsets, index, laid_out),
            None if !block.nested.is_empty() => nested::paint(&block.nested, laid_out),
            None => None,
        };
        Some(painted.map_or(Cow::Borrowed(&laid_out.line), Cow::Owned))
    }

    /// Every shown line as the renderer produced it, before wrapping, with each char's
    /// source byte. Width-independent, so whatever is built from it survives a resize. A
    /// thematic break is drawn, not rendered, so it has no line here.
    pub(crate) fn rendered_lines(&self) -> impl Iterator<Item = (&Line<'static>, &LineOffsets)> {
        self.blocks
            .iter()
            .filter(|block| block.kind != BlockKind::Rule)
            .flat_map(|block| block.text.lines.iter().zip(&block.offsets))
    }

    /// The first cell, in reading order, that shows a source byte inside `range`.
    pub(crate) fn first_cell_in_range(&self, block: usize, range: &Range<usize>) -> Option<(usize, usize)> {
        let b = self.blocks.get(block)?;
        b.rows.iter().enumerate().find_map(|(i, row)| {
            let col = row.cells.iter().position(|c| c.is_some_and(|o| range.contains(&o)))?;
            Some((b.first_row + i, col))
        })
    }

    /// First document row on which any cell falls inside `range`.
    pub(crate) fn first_row_in_range(&self, block: usize, range: &Range<usize>) -> Option<usize> {
        let b = self.blocks.get(block)?;
        b.rows
            .iter()
            .position(|r| r.cells.iter().any(|c| c.is_some_and(|o| range.contains(&o))))
            .map(|i| b.first_row + i)
    }

    /// The rendered text under a source range, in the web client's form: rendered
    /// characters whose source offset falls in `range`, blocks joined with no separator.
    ///
    /// The renderer maps a soft line break to nothing, so a break inside a wrapping block
    /// is recovered from the source: when consecutive rendered characters skip over
    /// whitespace in the source, one space is emitted (the DOM renders a soft break as a
    /// space). Code and tables keep their newlines.
    pub(crate) fn rendered_in_range(&self, source: &str, range: &Range<usize>) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            let break_char = if block.kind.preserves_columns() { '\n' } else { ' ' };
            let mut last_offset: Option<usize> = None;
            let chars = block.text.lines.iter().zip(&block.offsets).flat_map(|(line, offsets)| {
                line.spans.iter().flat_map(|s| s.content.chars()).zip(offsets.iter())
            });
            for (ch, offset) in chars {
                let Some(offset) = offset.filter(|o| range.contains(o)) else { continue };
                if let Some(prev) = last_offset
                    && let Some(skipped) = source.get(prev..offset)
                    && skipped.chars().any(char::is_whitespace)
                    && !out.ends_with(char::is_whitespace)
                {
                    out.push(break_char);
                }
                out.push(ch);
                last_offset = Some(offset + ch.len_utf8());
            }
        }
        out
    }
}
