//! Application state. Input handling lives in `input`, drawing in `draw`; this module owns
//! the data they share and the operations that change it.

mod draw;
mod input;
mod selection;

use std::ops::Range;

use anyhow::Result;
use plannotui_schema::{DocumentSource, Kind};
use ratatui::layout::Rect;
use tui_input::Input;

use crate::doc::Document;
use crate::layout::DocLayout;
use crate::store::Store;
use selection::Selection;

/// Width of the marker column left of the document.
pub(super) const GUTTER: u16 = 2;

/// Toolbar items in display order: (glyph, label, key, kind).
const TOOLBAR: [(&str, &str, char, Kind); 3] = [
    ("👍", "looks good", 'a', Kind::LooksGood),
    ("💬", "comment", 'c', Kind::Comment),
    ("✗", "delete", 'd', Kind::Delete),
];

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Browse,
    Compose,
}

/// Screen geometry captured during the last draw, for hit-testing input.
#[derive(Debug, Default, Clone)]
struct Geometry {
    doc: Rect,
    /// Toolbar rect and the column span of each item, in screen coordinates.
    toolbar: Option<(Rect, [Range<u16>; 3])>,
}

/// A finished selection waiting for an action.
#[derive(Debug, Clone)]
struct Pending {
    range: Range<usize>,
    /// Document (row, col) where the selection starts; anchors the toolbar and compose box.
    at: (usize, usize),
}

#[derive(Debug)]
pub(crate) struct App {
    source: DocumentSource,
    doc: Document,
    layout: DocLayout,
    store: Store,
    scroll: usize,
    selected: usize,
    selection: Option<Selection>,
    pending: Option<Pending>,
    mode: Mode,
    input: Input,
    geometry: Geometry,
    status: Option<String>,
    frame_ms: f64,
    frame_max_ms: f64,
    /// Copy selections to the terminal clipboard (off for headless runs).
    pub(crate) clipboard: bool,
    pub(crate) quit: bool,
}

impl App {
    pub(crate) fn open(source: DocumentSource, width: usize) -> Result<Self> {
        let doc = Document::parse(source.content.clone());
        let layout = DocLayout::build(&doc, width);
        let store = match (&source.provenance, source.transient) {
            (plannotui_schema::Provenance::File { path }, false) => Store::load(path, &doc)?,
            _ => Store::transient(),
        };
        Ok(Self {
            source,
            doc,
            layout,
            store,
            scroll: 0,
            selected: 0,
            selection: None,
            pending: None,
            mode: Mode::Browse,
            input: Input::default(),
            geometry: Geometry::default(),
            status: None,
            frame_ms: 0.0,
            frame_max_ms: 0.0,
            clipboard: false,
            quit: false,
        })
    }

    pub(crate) fn record_frame(&mut self, ms: f64) {
        self.frame_ms = if self.frame_ms == 0.0 { ms } else { self.frame_ms * 0.9 + ms * 0.1 };
        self.frame_max_ms = self.frame_max_ms.max(ms);
    }

    /// Annotate a source range: the rendered text is derived from the layout so the
    /// Workspaces web client can find it.
    fn annotate(&mut self, range: Range<usize>, kind: Kind, body: String) -> Result<()> {
        let rendered = self.layout.rendered_in_range(&self.doc.source, &range);
        self.store.add(&self.doc, range, rendered, kind, body)
    }

    /// Apply a toolbar action to the pending selection.
    fn act(&mut self, kind: Kind) -> Result<()> {
        let Some(pending) = self.pending.clone() else { return Ok(()) };
        match kind {
            Kind::Comment => {
                self.mode = Mode::Compose;
                self.input.reset();
            }
            Kind::LooksGood | Kind::Delete => {
                self.annotate(pending.range, kind, String::new())?;
                self.clear_selection();
                self.status = Some(format!("{} saved", label(kind)));
            }
        }
        Ok(())
    }

    fn clear_selection(&mut self) {
        self.selection = None;
        self.pending = None;
    }

    fn select_block(&mut self, block: usize) {
        if self.doc.blocks.is_empty() {
            return;
        }
        self.clear_selection();
        self.selected = block.min(self.doc.blocks.len() - 1);
        self.ensure_selected_visible();
    }

    fn ensure_selected_visible(&mut self) {
        let height = self.geometry.doc.height.max(1) as usize;
        let Some(block) = self.layout.blocks.get(self.selected) else { return };
        let first = block.first_row;
        let last = first + block.rows.len().saturating_sub(1);
        if first < self.scroll {
            self.scroll = first.saturating_sub(1);
        } else if last >= self.scroll + height {
            self.scroll = (last + 2).saturating_sub(height).min(first);
        }
    }

    fn scroll_by(&mut self, delta: i64) {
        let height = self.geometry.doc.height.max(1) as usize;
        let max = self.layout.total_rows.saturating_sub(height);
        self.scroll = (self.scroll as i64 + delta).clamp(0, max as i64) as usize;
    }

    /// Re-read the document from its provenance and re-resolve every annotation.
    fn reload(&mut self) -> Result<()> {
        let plannotui_schema::Provenance::File { path } = &self.source.provenance else {
            self.status = Some("not a file; nothing to reload".into());
            return Ok(());
        };
        let content = std::fs::read_to_string(path)?;
        self.source = DocumentSource::file(path.clone(), content);
        self.doc = Document::parse(self.source.content.clone());
        self.layout = DocLayout::build(&self.doc, self.layout.width);
        self.store.resolve_all(&self.doc);
        self.clear_selection();
        self.selected = self.selected.min(self.doc.blocks.len().saturating_sub(1));
        self.status = Some(format!("reloaded · {} orphaned", self.store.orphans()));
        Ok(())
    }

    // ----- headless helpers (bench, snapshot, scripting) -----------------------------

    /// Annotate a whole block by index.
    pub(crate) fn add_block_annotation(&mut self, block: usize, kind: Kind, body: String) -> Result<()> {
        let range = self.doc.blocks.get(block).map(|b| b.range.clone());
        let range = range.ok_or_else(|| {
            anyhow::anyhow!("block {block} out of range ({} blocks)", self.doc.blocks.len())
        })?;
        self.annotate(range, kind, body)
    }

    /// Annotate the first occurrence of `quote` in the source.
    pub(crate) fn add_quote_annotation(&mut self, quote: &str, kind: Kind, body: String) -> Result<()> {
        let start =
            self.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        self.annotate(start..start + quote.len(), kind, body)
    }

    pub(crate) fn describe_blocks(&self) -> Vec<String> {
        self.doc
            .blocks
            .iter()
            .zip(&self.layout.blocks)
            .enumerate()
            .map(|(i, (block, rendered))| {
                let first = self.doc.block_text(i).lines().next().unwrap_or("");
                let head: String = first.chars().take(60).collect();
                format!("{i:4} {:<10} row {:>5}  {head}", format!("{:?}", block.kind), rendered.first_row)
            })
            .collect()
    }

    /// Scroll and select the first visible block.
    pub(crate) fn scroll_for_snapshot(&mut self, delta: i64) {
        self.scroll_by(delta);
        if let Some(block) = self.layout.block_at_row(self.scroll) {
            self.selected = block;
        }
    }

    /// Simulate a finished drag over the first occurrence of `quote`.
    pub(crate) fn select_quote_for_snapshot(&mut self, quote: &str) -> Result<()> {
        let start =
            self.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        let range = start..start + quote.len();
        let cells = self.layout.blocks.iter().flat_map(|b| {
            b.rows.iter().enumerate().flat_map(move |(ri, row)| {
                row.cells.iter().enumerate().map(move |(col, cell)| ((b.first_row + ri, col), *cell))
            })
        });
        let hits: Vec<_> =
            cells.filter(|(_, cell)| cell.is_some_and(|o| range.contains(&o))).map(|(pos, _)| pos).collect();
        let first = *hits.first().ok_or_else(|| anyhow::anyhow!("quote is not rendered"))?;
        let last = hits.last().copied().unwrap_or(first);
        self.selection = Some(Selection::finished(first, last));
        self.finish_selection();
        Ok(())
    }
}

fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Comment => "comment",
        Kind::LooksGood => "looks good",
        Kind::Delete => "delete this",
    }
}

fn glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Comment => "💬",
        Kind::LooksGood => "👍",
        Kind::Delete => "✗",
    }
}
