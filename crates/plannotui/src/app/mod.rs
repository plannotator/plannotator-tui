//! Application state. Input handling lives in `input`, drawing in `draw`; this module owns
//! the data they share and the operations that change it.

mod draw;
mod input;
mod selection;

use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plannotui_schema::{DocumentSource, Kind, Provenance};
use ratatui::layout::Rect;
use tui_input::Input;

use crate::doc::Document;
use crate::export;
use crate::layout::DocLayout;
use crate::store::Store;
use crate::tree::Tree;
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
    /// Typing a comment for the pending selection.
    Compose,
    /// Editing the body of an existing annotation (by id).
    Edit(String),
}

/// Which pane keyboard input goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tree,
    Document,
    Rail,
}

/// Screen geometry captured during the last draw, for hit-testing input.
#[derive(Debug, Default, Clone)]
struct Geometry {
    tree: Rect,
    doc: Rect,
    /// Toolbar rect and the column span of each item, in screen coordinates.
    toolbar: Option<(Rect, [Range<u16>; 3])>,
    /// Screen rects of the rail bubbles drawn last frame, with their annotation ids.
    bubbles: Vec<(Rect, String)>,
}

/// A finished selection waiting for an action.
#[derive(Debug, Clone)]
struct Pending {
    range: Range<usize>,
    /// Document (row, col) where the selection starts; anchors the toolbar and compose box.
    at: (usize, usize),
}

/// Everything about the open document; swapped wholesale when the tree switches files.
#[derive(Debug)]
struct Open {
    source: DocumentSource,
    doc: Document,
    layout: DocLayout,
    store: Store,
}

impl Open {
    fn new(source: DocumentSource, width: usize) -> Result<Self> {
        let doc = Document::parse(source.content.clone());
        let layout = DocLayout::build(&doc, width);
        let store = match (&source.provenance, source.transient) {
            (Provenance::File { path }, false) => Store::load(path, &doc)?,
            _ => Store::transient(),
        };
        Ok(Self { source, doc, layout, store })
    }
}

#[derive(Debug)]
pub(crate) struct App {
    open: Open,
    /// Present in folder mode.
    tree: Option<Tree>,
    tree_cursor: usize,
    focus: Focus,
    scroll: usize,
    selected: usize,
    selection: Option<Selection>,
    pending: Option<Pending>,
    /// Keyboard cursor for visual selection, in document (row, col).
    cursor: (usize, usize),
    /// Index into the rail's placed annotations.
    rail_cursor: usize,
    mode: Mode,
    input: Input,
    geometry: Geometry,
    status: Option<String>,
    frame_ms: f64,
    frame_max_ms: f64,
    /// Copy selections and exports to the terminal clipboard (off for headless runs).
    pub(crate) clipboard: bool,
    pub(crate) quit: bool,
}

impl App {
    pub(crate) fn open(source: DocumentSource, width: usize) -> Result<Self> {
        Ok(Self {
            open: Open::new(source, width)?,
            tree: None,
            tree_cursor: 0,
            focus: Focus::Document,
            scroll: 0,
            selected: 0,
            selection: None,
            pending: None,
            cursor: (0, 0),
            rail_cursor: 0,
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

    /// Folder mode: a tree on the left, the first file open.
    pub(crate) fn open_folder(root: &Path, width: usize) -> Result<Self> {
        let tree = Tree::scan(root)?;
        let first =
            tree.first_file().with_context(|| format!("no markdown files under {}", root.display()))?;
        let path = first.path.clone();
        let mut app = Self::open(read_file(&path)?, width)?;
        app.tree_cursor = tree.position(&path).unwrap_or(0);
        app.tree = Some(tree);
        Ok(app)
    }

    /// Switch to the file under the tree cursor.
    fn open_tree_selection(&mut self) -> Result<()> {
        let Some(row) = self.tree.as_ref().and_then(|t| t.rows.get(self.tree_cursor)) else { return Ok(()) };
        if row.is_dir {
            return Ok(());
        }
        let path = row.path.clone();
        if matches!(&self.open.source.provenance, Provenance::File { path: p } if *p == path) {
            self.focus = Focus::Document;
            return Ok(());
        }
        let width = self.open.layout.width;
        self.open = Open::new(read_file(&path)?, width)?;
        self.scroll = 0;
        self.selected = 0;
        self.cursor = (0, 0);
        self.rail_cursor = 0;
        self.clear_selection();
        self.focus = Focus::Document;
        Ok(())
    }

    pub(crate) fn record_frame(&mut self, ms: f64) {
        self.frame_ms = if self.frame_ms == 0.0 { ms } else { self.frame_ms * 0.9 + ms * 0.1 };
        self.frame_max_ms = self.frame_max_ms.max(ms);
    }

    /// Annotate a source range: the rendered text is derived from the layout so the
    /// Workspaces web client can find it.
    fn annotate(&mut self, range: Range<usize>, kind: Kind, body: String) -> Result<()> {
        let rendered = self.open.layout.rendered_in_range(&self.open.doc.source, &range);
        self.open.store.add(&self.open.doc, range, rendered, kind, body)
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

    /// Begin editing the body of the annotation under the rail cursor.
    fn edit_selected_annotation(&mut self) {
        let placed = self.open.store.placed();
        let Some(target) = placed.get(self.rail_cursor) else { return };
        self.input = Input::new(target.annotation.body.clone());
        self.mode = Mode::Edit(target.annotation.id.clone());
    }

    fn remove_selected_annotation(&mut self) -> Result<()> {
        let id = self.open.store.placed().get(self.rail_cursor).map(|p| p.annotation.id.clone());
        let Some(id) = id else { return Ok(()) };
        if self.open.store.remove(&id)? {
            self.status = Some("annotation removed".into());
            self.rail_cursor = self.rail_cursor.min(self.open.store.placed().len().saturating_sub(1));
        }
        Ok(())
    }

    /// The feedback document for every placed annotation, in source order.
    pub(crate) fn feedback(&self) -> String {
        let source = &self.open.doc.source;
        let entries: Vec<export::Entry<'_>> = self
            .open
            .store
            .placed()
            .into_iter()
            .map(|p| export::Entry {
                annotation: p.annotation,
                lines: export::line_span(source, p.range),
                range: p.range.clone(),
            })
            .collect();
        export::feedback(source, "plan", &entries)
    }

    fn clear_selection(&mut self) {
        self.selection = None;
        self.pending = None;
    }

    fn select_block(&mut self, block: usize) {
        if self.open.doc.blocks.is_empty() {
            return;
        }
        self.clear_selection();
        self.selected = block.min(self.open.doc.blocks.len() - 1);
        if let Some(rendered) = self.open.layout.blocks.get(self.selected) {
            self.cursor = (rendered.first_row, 0);
        }
        self.ensure_selected_visible();
    }

    fn ensure_selected_visible(&mut self) {
        let height = usize::from(self.geometry.doc.height.max(1));
        let Some(block) = self.open.layout.blocks.get(self.selected) else { return };
        let first = block.first_row;
        let last = first + block.rows.len().saturating_sub(1);
        if first < self.scroll {
            self.scroll = first.saturating_sub(1);
        } else if last >= self.scroll + height {
            self.scroll = (last + 2).saturating_sub(height).min(first);
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let height = usize::from(self.geometry.doc.height.max(1));
        if self.cursor.0 < self.scroll {
            self.scroll = self.cursor.0;
        } else if self.cursor.0 >= self.scroll + height {
            self.scroll = self.cursor.0 + 1 - height;
        }
    }

    fn scroll_by(&mut self, delta: i64) {
        let height = usize::from(self.geometry.doc.height.max(1));
        let max = self.open.layout.total_rows.saturating_sub(height);
        self.scroll = (self.scroll as i64 + delta).clamp(0, max as i64) as usize;
    }

    /// Re-read the document from its provenance and re-resolve every annotation.
    fn reload(&mut self) -> Result<()> {
        let Provenance::File { path } = &self.open.source.provenance else {
            self.status = Some("not a file; nothing to reload".into());
            return Ok(());
        };
        let path = path.clone();
        self.open.source = read_file(&path)?;
        self.open.doc = Document::parse(self.open.source.content.clone());
        self.open.layout = DocLayout::build(&self.open.doc, self.open.layout.width);
        self.open.store.resolve_all(&self.open.doc);
        self.clear_selection();
        self.selected = self.selected.min(self.open.doc.blocks.len().saturating_sub(1));
        self.status = Some(format!("reloaded · {} orphaned", self.open.store.orphans()));
        Ok(())
    }

    // ----- headless helpers (bench, snapshot, scripting) -----------------------------

    /// Annotate a whole block by index.
    pub(crate) fn add_block_annotation(&mut self, block: usize, kind: Kind, body: String) -> Result<()> {
        let range = self.open.doc.blocks.get(block).map(|b| b.range.clone());
        let range = range.ok_or_else(|| {
            anyhow::anyhow!("block {block} out of range ({} blocks)", self.open.doc.blocks.len())
        })?;
        self.annotate(range, kind, body)
    }

    /// Annotate the first occurrence of `quote` in the source.
    pub(crate) fn add_quote_annotation(&mut self, quote: &str, kind: Kind, body: String) -> Result<()> {
        let start =
            self.open.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        self.annotate(start..start + quote.len(), kind, body)
    }

    pub(crate) fn describe_blocks(&self) -> Vec<String> {
        self.open
            .doc
            .blocks
            .iter()
            .zip(&self.open.layout.blocks)
            .enumerate()
            .map(|(i, (block, rendered))| {
                let first = self.open.doc.block_text(i).lines().next().unwrap_or("");
                let head: String = first.chars().take(60).collect();
                format!("{i:4} {:<10} row {:>5}  {head}", format!("{:?}", block.kind), rendered.first_row)
            })
            .collect()
    }

    /// Scroll and select the first visible block.
    pub(crate) fn scroll_for_snapshot(&mut self, delta: i64) {
        self.scroll_by(delta);
        if let Some(block) = self.open.layout.block_at_row(self.scroll) {
            self.selected = block;
        }
    }

    /// Simulate a finished drag over the first occurrence of `quote`.
    pub(crate) fn select_quote_for_snapshot(&mut self, quote: &str) -> Result<()> {
        let start =
            self.open.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        let range = start..start + quote.len();
        let cells = self.open.layout.blocks.iter().flat_map(|b| {
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

fn read_file(path: &Path) -> Result<DocumentSource> {
    let content = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(DocumentSource::file(PathBuf::from(path), content))
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
