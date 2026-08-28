//! Per-block rendering and the row map.
//!
//! Each block is rendered in isolation through `tui-markdown` (cached, width-independent),
//! aligned back to its source bytes, and wrapped to the current column width. Every screen
//! cell therefore knows its block and, where it shows real text, its source byte.

use std::ops::Range;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use tui_markdown::{Options, StyleSheet};

use crate::doc::{BlockKind, Document};
use crate::srcmap::{LineOffsets, align};
use crate::wrap::{Row, clip_line, wrap_line};

/// Rows of vertical space between blocks.
const BLOCK_GAP: usize = 1;

/// House style: no `#` markers, headings carry weight through bold/underline rather than
/// background color, so the palette stays available for selection and comments.
#[derive(Clone)]
struct Styles;

impl StyleSheet for Styles {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            _ => Style::new().fg(Color::LightCyan).add_modifier(Modifier::BOLD),
        }
    }
    fn heading_marker(&self, _level: u8) -> &str {
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

pub struct RenderedBlock {
    /// Width-independent styled lines from the renderer (owned, cached).
    text: Text<'static>,
    /// Per line, per char: absolute source byte offset (cached with `text`).
    offsets: Vec<LineOffsets>,
    kind: BlockKind,
    /// Rows for the current width.
    pub rows: Vec<Row>,
    /// First screen row of this block in document coordinates.
    pub first_row: usize,
}

pub struct DocLayout {
    pub width: usize,
    pub blocks: Vec<RenderedBlock>,
    pub total_rows: usize,
}

fn render_block(doc: &Document, index: usize) -> (Text<'static>, Vec<LineOffsets>) {
    let source = doc.block_text(index);
    let text = own(tui_markdown::from_str_with_options(source, &Options::new(Styles)));
    let plain: Vec<String> = text.lines.iter().map(|l| l.to_string()).collect();
    let offsets = align(&plain, source, doc.blocks[index].range.start);
    (text, offsets)
}

fn own(text: Text<'_>) -> Text<'static> {
    let lines = text
        .lines
        .into_iter()
        .map(|line| {
            let spans = line
                .spans
                .into_iter()
                .map(|s| ratatui::text::Span::styled(s.content.into_owned(), s.style))
                .collect::<Vec<_>>();
            Line::from(spans).style(line.style)
        })
        .collect::<Vec<_>>();
    Text::from(lines).style(text.style)
}

impl DocLayout {
    /// Render every block once (the expensive part) and lay out for `width`.
    pub fn build(doc: &Document, width: usize) -> Self {
        let blocks = (0..doc.blocks.len())
            .map(|i| {
                let (text, offsets) = render_block(doc, i);
                RenderedBlock { text, offsets, kind: doc.blocks[i].kind, rows: Vec::new(), first_row: 0 }
            })
            .collect();
        let mut layout = Self { width: 0, blocks, total_rows: 0 };
        layout.reflow(width);
        layout
    }

    /// Re-wrap for a new width without re-rendering markdown.
    pub fn reflow(&mut self, width: usize) {
        let width = width.max(1);
        self.width = width;
        let mut row = 0usize;
        for block in &mut self.blocks {
            block.first_row = row;
            let lines = block.text.lines.iter().zip(&block.offsets);
            block.rows = if block.kind.preserves_columns() {
                lines.map(|(l, o)| clip_line(l, o, width)).collect()
            } else {
                lines.flat_map(|(l, o)| wrap_line(l, o, width)).collect()
            };
            row += block.rows.len() + BLOCK_GAP;
        }
        self.total_rows = row.saturating_sub(BLOCK_GAP);
    }

    /// Which block owns a document row (gap rows belong to nobody).
    pub fn block_at_row(&self, row: usize) -> Option<usize> {
        let idx = self.blocks.partition_point(|b| b.first_row <= row);
        let i = idx.checked_sub(1)?;
        let block = &self.blocks[i];
        (row < block.first_row + block.rows.len()).then_some(i)
    }

    /// The row at document coordinate `row`, if it exists (None for gap rows).
    pub fn row(&self, row: usize) -> Option<&Row> {
        let i = self.block_at_row(row)?;
        let block = &self.blocks[i];
        block.rows.get(row - block.first_row)
    }

    /// First document row on which any cell falls inside `range`.
    pub fn first_row_in_range(&self, block: usize, range: &Range<usize>) -> Option<usize> {
        let b = self.blocks.get(block)?;
        b.rows
            .iter()
            .position(|r| r.cells.iter().any(|c| c.is_some_and(|o| range.contains(&o))))
            .map(|i| b.first_row + i)
    }
}
