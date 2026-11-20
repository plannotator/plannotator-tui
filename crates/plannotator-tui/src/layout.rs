//! Per-block rendering and the row map.
//!
//! Each block is rendered in isolation through `tui-markdown` (cached, width-independent),
//! aligned back to its source bytes, and wrapped to the current column width. List and quote
//! blocks are decorated in between (`decorate`). Every screen cell therefore knows its block
//! and, where it shows real text, its source byte. A long document lays out its first
//! screens before the first frame and the rest in the background (`layout::background`).

use std::borrow::Cow;
use std::ops::Range;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use tui_markdown::{Options, StyleSheet};

use crate::cjk_emphasis::{protect, strip_marks};
use crate::decorate::{self, Decorated, RULE, Syntax};
use crate::doc::{BlockKind, Document};
use crate::srcmap::{LineOffsets, align};
use crate::theme::palette;
use crate::wrap::{Hang, Row, clip_line, rule_row, wrap_line, wrap_line_hanging, wrap_table};

mod background;

use background::{Rest, each_in_parallel, lay_out};

/// Rows of vertical space between blocks.
const BLOCK_GAP: usize = 1;

/// Rows [`DocLayout::start`] lays out before the first frame: more than any terminal shows.
const FIRST_ROWS: usize = 500;

/// House style: no `#` markers, headings carry weight through bold/underline rather than
/// background color, so the palette stays available for selection and annotations.
#[derive(Debug, Clone)]
struct Styles;

impl StyleSheet for Styles {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().fg(palette().heading).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::new().fg(palette().heading).add_modifier(Modifier::BOLD),
            _ => Style::new().fg(palette().subheading).add_modifier(Modifier::BOLD),
        }
    }
    fn heading_marker(&self, _level: u8) -> &'static str {
        ""
    }
    fn code(&self) -> Style {
        Style::new().fg(Color::LightYellow)
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

fn render_block(doc: &Document, index: usize) -> Decorated {
    let source = doc.block_text(index);
    let protected = protect(source);
    let text = own(tui_markdown::from_str_with_options(&protected, &Options::new(Styles)));
    let text = if matches!(protected, Cow::Owned(_)) { strip_marks(text) } else { text };
    let plain: Vec<String> = text.lines.iter().map(ToString::to_string).collect();
    let block = doc.blocks.get(index);
    let base = block.map_or(0, |b| b.range.start);
    if !block.is_some_and(|b| matches!(b.kind, BlockKind::List | BlockKind::BlockQuote)) {
        let offsets = align(&plain, source, base);
        return Decorated { text, offsets, hangs: Vec::new() };
    }
    let syntax = Syntax::parse(source, base);
    let offsets = align(&plain, &syntax.aligned_source(source, base), base);
    decorate::decorate(&syntax, source, base, text, offsets)
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

impl RenderedBlock {
    /// Render block `index` of `doc` (the expensive part) and wrap it to `width`.
    fn new(doc: &Document, index: usize, width: usize) -> Self {
        let Decorated { text, offsets, hangs } = render_block(doc, index);
        let kind = doc.blocks.get(index).map_or(BlockKind::Other, |b| b.kind);
        let mut block = Self { text, offsets, kind, hangs, rows: Vec::new(), first_row: 0 };
        block.wrap(width);
        block
    }

    /// Re-wrap the cached lines for `width`.
    fn wrap(&mut self, width: usize) {
        let lines = || self.text.lines.iter().zip(&self.offsets);
        self.rows = if self.kind == BlockKind::Rule {
            let offset = self.offsets.iter().flatten().flatten().next().copied();
            vec![rule_row(RULE, Style::new().fg(palette().idle_fg), offset, width)]
        } else if self.kind == BlockKind::Table {
            wrap_table(&self.text.lines, &self.offsets, width)
        } else if self.kind.preserves_columns() {
            lines().map(|(l, o)| clip_line(l, o, width)).collect()
        } else if self.hangs.is_empty() {
            lines().flat_map(|(l, o)| wrap_line(l, o, width)).collect()
        } else {
            lines().zip(&self.hangs).flat_map(|((l, o), h)| wrap_line_hanging(l, o, h, width)).collect()
        };
    }
}

impl DocLayout {
    /// Render and lay out every block for `width` before returning.
    pub(crate) fn build(doc: &Document, width: usize) -> Self {
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
    #[cfg(test)]
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

    /// Give each block its first row, one gap row apart.
    fn place(&mut self) {
        let mut row = 0usize;
        for block in &mut self.blocks {
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

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn rendered_text_strips_markup_and_joins_blocks_without_separator() {
        let doc = Document::parse("Ship the **login page**\nby Friday.\n\nNext para.\n".to_owned());
        let layout = DocLayout::build(&doc, 80);
        let whole = 0..doc.source.len();
        assert_eq!(layout.rendered_in_range(&doc.source, &whole), "Ship the login page by Friday.Next para.");
        let bold = doc.source.find("**login").expect("present");
        let bold_range = bold..bold + "**login page**".len();
        assert_eq!(layout.rendered_in_range(&doc.source, &bold_range), "login page");
    }

    #[test]
    fn every_cell_of_a_long_table_maps_to_its_source() {
        let doc = Document::parse(
            "| Item | Status | Next step |\n|---|---|---|\n\
| A. Four changes from 3/7 (release checklist handoff, the 3/4 planning call, the 3/6 search demo, \
the 3/4-3/7 progress review and rollout advice) | Scope partly confirmed; never added to the \
draft | 1.0d design + 1.0d build + 1.0d review + 2.5d rollout = **5.5d** |\n\
| B. 3/14 cache troubleshooting and the `api.example.com` proxy | Owner and priority not set | Your call |\n\
| C. 3/19-20 export access in the admin panel (read and write) | Agreed at 1.0d, done once it's \
tested; it's live, but QA hasn't finished acceptance testing | Close it now, or wait for the next sprint |\n\
| D. 3/21 lookup and report tools in the CLI; 3/23 plugin consolidation | Marked \"not confirmed\" | Your call |\n"
                .to_owned(),
        );
        let layout = DocLayout::build(&doc, 100);
        let table = layout.blocks.iter().find(|b| b.kind == BlockKind::Table).expect("a table");
        let mut cells = 0;
        for row in &table.rows {
            let text: String = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
            for (ch, offset) in text.chars().zip(&row.cells) {
                if ch.is_alphanumeric() {
                    let source = offset.and_then(|o| doc.source.get(o..));
                    assert!(source.is_some_and(|s| s.starts_with(ch)), "{ch:?} is unmapped in {text:?}");
                    cells += 1;
                }
            }
        }
        assert!(cells > 300, "only {cells} cells checked");
    }

    fn rows(layout: &DocLayout) -> Vec<(usize, String, Vec<Option<usize>>)> {
        layout
            .blocks
            .iter()
            .flat_map(|b| b.rows.iter().map(|r| (b.first_row, r.line.to_string(), r.cells.clone())))
            .collect()
    }

    /// A long document opens on its first screens only, and the rest, laid out in the
    /// background (here for a width the first frame changed), is exactly a full build.
    #[test]
    fn a_long_document_lays_out_its_first_screens_first_and_the_rest_matches_a_full_build() {
        let section = "## Step\n\nSome **bold** words that wrap at a narrow width, and more.\n\n\
- one\n- two\n\n| a | b |\n|---|---|\n| x | y |\n\n```\ncode\n```\n\n---\n\n";
        let doc = Document::parse(section.repeat(400));
        let mut lazy = DocLayout::start(&doc, 70);
        assert!(!lazy.is_complete());
        assert!(lazy.blocks.len() < doc.blocks.len() / 10, "laid out {} blocks up front", lazy.blocks.len());
        assert!(lazy.total_rows >= FIRST_ROWS);
        lazy.reflow(50);
        lazy.finish();
        assert!(lazy.is_complete());
        let full = DocLayout::build(&doc, 50);
        assert_eq!(lazy.blocks.len(), doc.blocks.len());
        assert_eq!(lazy.total_rows, full.total_rows);
        assert!(rows(&lazy) == rows(&full), "rows differ from a full build");
    }
}
