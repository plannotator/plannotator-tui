//! Application state, input handling, and drawing.
//!
//! Three columns: a 2-cell gutter, the document, and a comment rail. The document is
//! drawn row-by-row from the layout's row map; every cell knows its source byte, so a
//! mouse drag becomes a source range, a toolbar appears beside it, and annotations paint
//! back onto their cells.

use std::io::Write;
use std::ops::Range;
use std::path::PathBuf;

use anyhow::Result;
use ratatui::Frame;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;
use unicode_width::UnicodeWidthStr;

use crate::comments::{Kind, Store};
use crate::doc::Document;
use crate::layout::DocLayout;
use crate::wrap::wrap_line;

const GUTTER: u16 = 2;
const RAIL_WIDTH: u16 = 36;
const RAIL_MIN_WIDTH: u16 = 28;
/// Below this the rail is dropped and comments are only marked in the gutter.
const RAIL_MIN_TOTAL_WIDTH: u16 = 80;
const COMPOSE_WIDTH: u16 = 48;

pub const COMMENT_BG: Color = Color::Indexed(58);
pub const APPROVE_BG: Color = Color::Indexed(22);
const BLOCK_BG: Color = Color::Indexed(236);
const TOOLBAR_BG: Color = Color::Indexed(238);

/// Toolbar items in display order: (glyph, label, key, kind).
const TOOLBAR: [(&str, &str, char, Kind); 3] = [
    ("👍", "looks good", 'a', Kind::Approve),
    ("💬", "comment", 'c', Kind::Comment),
    ("✗", "delete", 'd', Kind::Delete),
];

#[derive(PartialEq, Eq)]
enum Mode {
    Browse,
    Compose,
}

#[derive(Default, Clone)]
struct Geometry {
    doc: Rect,
    /// Toolbar rect and the column span of each item, in screen coordinates.
    toolbar: Option<(Rect, [Range<u16>; 3])>,
}

/// A mouse selection in document coordinates (row, column).
#[derive(Clone, Copy)]
struct Selection {
    anchor: (usize, usize),
    head: (usize, usize),
    dragging: bool,
}

impl Selection {
    fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    /// Columns of `row` covered by the selection, if any.
    fn columns_on(&self, row: usize, row_width: usize) -> Option<Range<usize>> {
        let (a, b) = self.ordered();
        if row < a.0 || row > b.0 {
            return None;
        }
        let start = if row == a.0 { a.1 } else { 0 };
        let end = if row == b.0 { b.1 + 1 } else { row_width };
        (start < end).then_some(start..end)
    }
}

/// A finished selection waiting for an action.
#[derive(Clone)]
struct Pending {
    block: usize,
    range: Range<usize>,
    /// Document (row, col) where the selection starts; anchors the toolbar and compose box.
    at: (usize, usize),
}

pub struct App {
    path: PathBuf,
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
    pub clipboard: bool,
    pub quit: bool,
}

impl App {
    pub fn open(path: PathBuf, width: usize) -> Result<Self> {
        let source = std::fs::read_to_string(&path)?;
        let doc = Document::parse(source);
        let layout = DocLayout::build(&doc, width);
        let store = Store::load(&path, &doc)?;
        Ok(Self {
            path,
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

    // ----- headless helpers ------------------------------------------------------

    /// Annotate a whole block by index.
    pub fn add_comment(&mut self, block: usize, kind: Kind, body: String) -> Result<()> {
        anyhow::ensure!(
            block < self.doc.blocks.len(),
            "block {block} out of range ({} blocks)",
            self.doc.blocks.len()
        );
        let range = self.doc.blocks[block].range.clone();
        self.store.add(&self.doc, block, range, kind, body)
    }

    /// Annotate the first occurrence of `quote` in the source.
    pub fn add_quote_comment(&mut self, quote: &str, kind: Kind, body: String) -> Result<()> {
        let start =
            self.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        let range = start..start + quote.len();
        let block = crate::comments::block_containing(&self.doc, start)
            .ok_or_else(|| anyhow::anyhow!("quote is not inside a block"))?;
        self.store.add(&self.doc, block, range, kind, body)
    }

    pub fn describe_blocks(&self) -> Vec<String> {
        self.doc
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let first = self.doc.block_text(i).lines().next().unwrap_or("");
                format!(
                    "{i:4} {:<10} row {:>5}  {}",
                    format!("{:?}", b.kind),
                    self.layout.blocks[i].first_row,
                    first.chars().take(60).collect::<String>()
                )
            })
            .collect()
    }

    /// Scroll and select the first visible block, for `--snapshot`.
    pub fn scroll_for_snapshot(&mut self, delta: i64) {
        self.scroll_by(delta);
        if let Some(block) = self.layout.block_at_row(self.scroll) {
            self.selected = block;
        }
    }

    /// Simulate a finished drag over `quote` (first occurrence), for `--snapshot`.
    pub fn select_quote_for_snapshot(&mut self, quote: &str) -> Result<()> {
        let start =
            self.doc.source.find(quote).ok_or_else(|| anyhow::anyhow!("quote not found: {quote:?}"))?;
        let end = start + quote.len();
        let mut first: Option<(usize, usize)> = None;
        let mut last: Option<(usize, usize)> = None;
        for (bi, block) in self.layout.blocks.iter().enumerate() {
            for (ri, row) in block.rows.iter().enumerate() {
                for (col, cell) in row.cells.iter().enumerate() {
                    if cell.is_some_and(|o| o >= start && o < end) {
                        let pos = (block.first_row + ri, col);
                        first.get_or_insert(pos);
                        last = Some(pos);
                        let _ = bi;
                    }
                }
            }
        }
        let (a, b) = first.zip(last).ok_or_else(|| anyhow::anyhow!("quote is not rendered"))?;
        self.selection = Some(Selection { anchor: a, head: b, dragging: false });
        self.finish_selection();
        Ok(())
    }

    pub fn record_frame(&mut self, ms: f64) {
        self.frame_ms = if self.frame_ms == 0.0 { ms } else { self.frame_ms * 0.9 + ms * 0.1 };
        self.frame_max_ms = self.frame_max_ms.max(ms);
    }

    // ----- input -----------------------------------------------------------------

    pub fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) if key.kind != ratatui::crossterm::event::KeyEventKind::Release => {
                match self.mode {
                    Mode::Browse => self.browse_key(key)?,
                    Mode::Compose => self.compose_key(key, &event)?,
                }
            }
            Event::Mouse(mouse) if self.mode == Mode::Browse => self.mouse(mouse)?,
            _ => {}
        }
        Ok(())
    }

    fn browse_key(&mut self, key: KeyEvent) -> Result<()> {
        let page = self.geometry.doc.height.max(1) as usize;
        // With a selection waiting, the toolbar keys win.
        if self.pending.is_some() {
            if let Some((_, _, _, kind)) = TOOLBAR.iter().find(|(_, _, k, _)| KeyCode::Char(*k) == key.code) {
                return self.act(*kind);
            }
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => self.quit = true,
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.quit = true,
            (KeyCode::Esc, _) => {
                if self.pending.is_some() || self.selection.is_some() {
                    self.clear_selection();
                } else {
                    self.quit = true;
                }
            }
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => self.select_block(self.selected + 1),
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => self.select_block(self.selected.saturating_sub(1)),
            (KeyCode::Char('d'), KeyModifiers::CONTROL) | (KeyCode::PageDown, _) => {
                self.scroll_by(page as i64 / 2)
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) | (KeyCode::PageUp, _) => {
                self.scroll_by(-(page as i64) / 2)
            }
            (KeyCode::Char('g'), _) | (KeyCode::Home, _) => self.select_block(0),
            (KeyCode::Char('G'), _) | (KeyCode::End, _) => {
                self.select_block(self.doc.blocks.len().saturating_sub(1))
            }
            (KeyCode::Char('c'), _) | (KeyCode::Enter, _) => {
                // No selection: comment on the whole selected block.
                let block = self.selected;
                let range = self.doc.blocks[block].range.clone();
                let at = (self.layout.blocks[block].first_row, 0);
                self.pending = Some(Pending { block, range, at });
                self.act(Kind::Comment)?;
            }
            (KeyCode::Char('x'), _) => {
                let removed = self.store.remove_on_block(self.selected)?;
                self.status = Some(format!("removed {removed} annotation(s) on block"));
            }
            (KeyCode::Char('r'), _) => self.reload()?,
            _ => {}
        }
        Ok(())
    }

    /// Apply a toolbar action to the pending selection.
    fn act(&mut self, kind: Kind) -> Result<()> {
        let Some(pending) = self.pending.clone() else { return Ok(()) };
        match kind {
            Kind::Comment => {
                self.mode = Mode::Compose;
                self.input.reset();
            }
            Kind::Approve | Kind::Delete => {
                self.store.add(&self.doc, pending.block, pending.range, kind, String::new())?;
                self.clear_selection();
                self.status = Some(format!("{} saved", kind.label()));
            }
        }
        Ok(())
    }

    fn compose_key(&mut self, key: KeyEvent, event: &Event) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Browse,
            KeyCode::Enter => {
                let body = self.input.value().trim().to_string();
                if !body.is_empty() {
                    if let Some(pending) = self.pending.take() {
                        self.store.add(&self.doc, pending.block, pending.range, Kind::Comment, body)?;
                        self.status = Some("comment saved".into());
                    }
                    self.clear_selection();
                }
                self.mode = Mode::Browse;
            }
            _ => {
                self.input.handle_event(event);
            }
        }
        Ok(())
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(kind) = self.toolbar_hit(mouse.column, mouse.row) {
                    return self.act(kind);
                }
                let Some(pos) = self.doc_position(mouse.column, mouse.row, false) else { return Ok(()) };
                if let Some(block) = self.layout.block_at_row(pos.0) {
                    self.selected = block;
                }
                self.pending = None;
                self.selection = Some(Selection { anchor: pos, head: pos, dragging: true });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if !self.selection.is_some_and(|s| s.dragging) {
                    return Ok(());
                }
                let doc = self.geometry.doc;
                // Dragging past the top or bottom edge scrolls the document.
                if mouse.row < doc.y {
                    self.scroll_by(-1);
                } else if mouse.row >= doc.bottom() {
                    self.scroll_by(1);
                }
                if let Some(pos) = self.doc_position(mouse.column, mouse.row, true) {
                    if let Some(sel) = self.selection.as_mut() {
                        sel.head = pos;
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.finish_selection(),
            _ => {}
        }
        Ok(())
    }

    fn toolbar_hit(&self, column: u16, row: u16) -> Option<Kind> {
        let (rect, items) = self.geometry.toolbar.as_ref()?;
        if row != rect.y || column < rect.x || column >= rect.right() {
            return None;
        }
        items.iter().zip(TOOLBAR.iter()).find(|(span, _)| span.contains(&column)).map(|(_, item)| item.3)
    }

    /// Screen cell -> document (row, column). With `clamp`, positions outside the
    /// document area snap to its nearest edge so a drag can leave the area.
    fn doc_position(&self, column: u16, row: u16, clamp: bool) -> Option<(usize, usize)> {
        let doc = self.geometry.doc;
        let inside = column >= doc.x && column < doc.right() && row >= doc.y && row < doc.bottom();
        if !inside && !clamp {
            let in_gutter = column >= doc.x.saturating_sub(GUTTER)
                && column < doc.x
                && row >= doc.y
                && row < doc.bottom();
            if !in_gutter {
                return None;
            }
        }
        let col = column.clamp(doc.x, doc.right().saturating_sub(1)) - doc.x;
        let row = row.clamp(doc.y, doc.bottom().saturating_sub(1)) - doc.y;
        let doc_row = (self.scroll + row as usize).min(self.layout.total_rows.saturating_sub(1));
        Some((doc_row, col as usize))
    }

    /// Turn the drag into a source range; it may span blocks.
    fn finish_selection(&mut self) {
        let Some(sel) = self.selection.as_mut() else { return };
        sel.dragging = false;
        let (a, b) = sel.ordered();
        if a == b {
            self.selection = None;
            return;
        }
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        for row in a.0..=b.0 {
            let Some(r) = self.layout.row(row) else { continue };
            let cols = sel.columns_on(row, r.cells.len()).unwrap_or(0..0);
            for offset in r.cells[cols.start.min(r.cells.len())..cols.end.min(r.cells.len())].iter().flatten()
            {
                lo = lo.min(*offset);
                hi = hi.max(*offset);
            }
        }
        if lo == usize::MAX {
            self.selection = None;
            return;
        }
        let end_char = self.doc.source[hi..].chars().next().map(char::len_utf8).unwrap_or(1);
        let range = lo..hi + end_char;
        let Some(block) = crate::comments::block_containing(&self.doc, range.start) else {
            self.selection = None;
            return;
        };
        if self.clipboard {
            copy_to_clipboard(&self.doc.source[range.clone()]);
        }
        self.selected = block;
        self.pending = Some(Pending { block, range, at: a });
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
        let block = &self.layout.blocks[self.selected];
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

    fn reload(&mut self) -> Result<()> {
        let source = std::fs::read_to_string(&self.path)?;
        self.doc = Document::parse(source);
        self.layout = DocLayout::build(&self.doc, self.layout.width);
        self.store.resolve_all(&self.doc);
        self.clear_selection();
        self.selected = self.selected.min(self.doc.blocks.len().saturating_sub(1));
        self.status = Some(format!("reloaded · {} orphaned", self.store.orphans()));
        Ok(())
    }

    // ----- drawing ---------------------------------------------------------------

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [header, body, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);

        let rail_width = if area.width >= RAIL_MIN_TOTAL_WIDTH {
            (area.width * 3 / 10).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH)
        } else {
            0
        };
        let [gutter, doc, _gap, rail] = Layout::horizontal([
            Constraint::Length(GUTTER),
            Constraint::Min(20),
            Constraint::Length(if rail_width > 0 { 1 } else { 0 }),
            Constraint::Length(rail_width),
        ])
        .areas(body);
        self.geometry = Geometry { doc, toolbar: None };

        if self.layout.width != doc.width as usize {
            self.layout.reflow(doc.width as usize);
            self.clear_selection();
            self.scroll_by(0);
        }

        self.draw_header(frame, header);
        self.draw_document(frame, gutter, doc);
        if rail_width > 0 {
            self.draw_rail(frame, rail);
        }
        self.draw_footer(frame, footer);
        // Floating widgets last, over everything.
        if self.mode == Mode::Compose {
            self.draw_compose(frame);
        } else if self.pending.is_some() {
            self.draw_toolbar(frame);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let title = Line::from(vec![
            Span::raw(" ").dim(),
            Span::styled(self.path.display().to_string(), Style::new().bold()),
        ]);
        let right = Line::from(Span::raw("plannotui ").dim()).right_aligned();
        frame.render_widget(Paragraph::new(title), area);
        frame.render_widget(Paragraph::new(right), area);
    }

    fn draw_document(&self, frame: &mut Frame, gutter: Rect, doc: Rect) {
        let buf = frame.buffer_mut();
        let annotations: Vec<(Range<usize>, Kind)> = self.store.ranges().collect();
        let text_selection_active = self.selection.is_some();

        for y in 0..doc.height {
            let row_index = self.scroll + y as usize;
            let Some(block) = self.layout.block_at_row(row_index) else { continue };
            let Some(row) = self.layout.row(row_index) else { continue };
            let screen_y = doc.y + y;
            buf.set_line(doc.x, screen_y, &row.line, doc.width);

            if block == self.selected && !text_selection_active && self.pending.is_none() {
                buf.set_style(
                    Rect { x: doc.x, y: screen_y, width: doc.width, height: 1 },
                    Style::new().bg(BLOCK_BG),
                );
            }

            let mut row_has_annotation = false;
            for (col, cell) in row.cells.iter().enumerate().take(doc.width as usize) {
                let Some(offset) = cell else { continue };
                // Delete wins over approve wins over comment when annotations overlap.
                let kind =
                    annotations.iter().filter(|(r, _)| r.contains(offset)).map(|(_, k)| *k).max_by_key(|k| {
                        match k {
                            Kind::Comment => 0,
                            Kind::Approve => 1,
                            Kind::Delete => 2,
                        }
                    });
                let Some(kind) = kind else { continue };
                row_has_annotation = true;
                let style = match kind {
                    Kind::Comment => Style::new().bg(COMMENT_BG),
                    Kind::Approve => Style::new().bg(APPROVE_BG),
                    Kind::Delete => {
                        Style::new().fg(Color::Red).add_modifier(Modifier::CROSSED_OUT | Modifier::DIM)
                    }
                };
                buf.set_style(Rect { x: doc.x + col as u16, y: screen_y, width: 1, height: 1 }, style);
            }

            if let Some(sel) = &self.selection {
                if let Some(cols) = sel.columns_on(row_index, row.cells.len().max(1)) {
                    let start = cols.start.min(doc.width as usize) as u16;
                    let end = cols.end.min(doc.width as usize) as u16;
                    if end > start {
                        buf.set_style(
                            Rect { x: doc.x + start, y: screen_y, width: end - start, height: 1 },
                            Style::new().add_modifier(Modifier::REVERSED),
                        );
                    }
                }
            }

            let marker = match (block == self.selected, row_has_annotation) {
                (true, _) => Span::styled("▍", Style::new().fg(Color::Cyan)),
                (false, true) => Span::styled("▍", Style::new().fg(Color::Yellow)),
                (false, false) => Span::raw(" "),
            };
            buf.set_span(gutter.x, screen_y, &marker, 1);
        }
    }

    /// Screen position for a floating widget anchored at the pending selection: one row
    /// above its first row when there is room, else just below its last row.
    fn float_origin(&self, height: u16, width: u16) -> Option<Rect> {
        let pending = self.pending.as_ref()?;
        let doc = self.geometry.doc;
        let (row, col) = pending.at;
        if row < self.scroll || row >= self.scroll + doc.height as usize {
            return None;
        }
        let screen_row = doc.y + (row - self.scroll) as u16;
        let width = width.min(doc.width);
        let x = (doc.x + col as u16).min(doc.right().saturating_sub(width));
        let y = if screen_row >= doc.y + height {
            screen_row - height
        } else {
            let last_row = self.selection.map(|s| s.ordered().1.0).unwrap_or(row);
            let below = doc.y + (last_row.saturating_sub(self.scroll) as u16) + 1;
            below.min(doc.bottom().saturating_sub(height))
        };
        Some(Rect { x, y, width, height })
    }

    fn draw_toolbar(&mut self, frame: &mut Frame) {
        let labels: Vec<String> = TOOLBAR.iter().map(|(g, l, k, _)| format!(" {g} {l} ({k}) ")).collect();
        let width: u16 = labels.iter().map(|l| l.width() as u16).sum::<u16>() + 1;
        let Some(rect) = self.float_origin(1, width) else { return };
        frame.render_widget(Clear, rect);
        let buf = frame.buffer_mut();
        buf.set_style(rect, Style::new().bg(TOOLBAR_BG));
        let mut x = rect.x + 1;
        let mut spans = [0..0, 0..0, 0..0];
        for (i, label) in labels.iter().enumerate() {
            let w = label.width() as u16;
            let color = match TOOLBAR[i].3 {
                Kind::Approve => Color::Green,
                Kind::Comment => Color::Yellow,
                Kind::Delete => Color::Red,
            };
            buf.set_span(
                x,
                rect.y,
                &Span::styled(label.as_str(), Style::new().fg(color).bg(TOOLBAR_BG).bold()),
                w,
            );
            spans[i] = x..x + w;
            x += w;
        }
        self.geometry.toolbar = Some((rect, spans));
    }

    fn draw_compose(&self, frame: &mut Frame) {
        let Some(rect) = self.float_origin(3, COMPOSE_WIDTH) else { return };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Yellow))
            .title(Span::styled(" comment · enter saves · esc cancels ", Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let width = inner.width.saturating_sub(1) as usize;
        let scroll = self.input.visual_scroll(width);
        let value: String = self.input.value().chars().skip(scroll).collect();
        frame.render_widget(
            Paragraph::new(Line::from(value)),
            Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner },
        );
        let cursor_x = inner.x + 1 + (self.input.visual_cursor().saturating_sub(scroll)) as u16;
        frame.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), inner.y));
    }

    fn draw_rail(&self, frame: &mut Frame, rail: Rect) {
        let view_end = self.scroll + rail.height as usize;
        let start = self.layout.blocks.partition_point(|b| b.first_row + b.rows.len() <= self.scroll);
        let mut next_y = rail.y;
        for (index, block) in self.layout.blocks.iter().enumerate().skip(start) {
            if block.first_row >= view_end || next_y >= rail.bottom() {
                break;
            }
            for (comment, range) in self.store.on_block(index) {
                let anchor_row = self.layout.first_row_in_range(index, &range).unwrap_or(block.first_row);
                let anchored_y = rail.y + anchor_row.saturating_sub(self.scroll) as u16;
                let y = anchored_y.max(next_y);
                let inner_width = rail.width.saturating_sub(4) as usize;
                let body = if comment.body.is_empty() {
                    comment.kind.label().to_string()
                } else {
                    comment.body.clone()
                };
                let lines: Vec<Line<'static>> = wrap_line(&Line::from(body.as_str()), &[], inner_width)
                    .into_iter()
                    .map(|r| r.line)
                    .collect();
                let height = (lines.len() as u16 + 2).min(rail.bottom().saturating_sub(y));
                if height < 3 {
                    break;
                }
                let accent = match comment.kind {
                    Kind::Comment => Color::Yellow,
                    Kind::Approve => Color::Green,
                    Kind::Delete => Color::Red,
                };
                let border = if index == self.selected {
                    Style::new().fg(accent)
                } else {
                    Style::new().fg(Color::DarkGray)
                };
                let bubble = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(border)
                    .title(Span::styled(
                        format!(" {} #{} ", comment.kind.glyph(), comment.id),
                        Style::new().fg(accent),
                    ));
                let rect = Rect { x: rail.x, y, width: rail.width, height };
                let inner = bubble.inner(rect);
                frame.render_widget(bubble, rect);
                let body_style =
                    if comment.body.is_empty() { Style::new().dim().italic() } else { Style::new() };
                frame.render_widget(
                    Paragraph::new(lines).style(body_style),
                    Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner },
                );
                next_y = y + height;
            }
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let orphans = self.store.orphans();
        let mut parts = vec![
            format!("{} blocks", self.doc.blocks.len()),
            format!(
                "{} annotations{}",
                self.store.comments.len(),
                if orphans > 0 { format!(" ({orphans} orphaned)") } else { String::new() }
            ),
            match &self.pending {
                Some(p) => format!("selected {} chars", self.doc.source[p.range.clone()].chars().count()),
                None => format!("block {}/{}", self.selected + 1, self.doc.blocks.len()),
            },
            format!("frame {:.2}ms (max {:.1})", self.frame_ms, self.frame_max_ms),
        ];
        if let Some(status) = &self.status {
            parts.push(status.clone());
        }
        if frame.area().width < RAIL_MIN_TOTAL_WIDTH {
            parts.push("rail hidden: widen to ≥80 cols".into());
        }
        let help = if self.pending.is_some() {
            "a looks good · c comment · d delete · esc clear · q quit "
        } else {
            "drag to select · j/k block · c comment block · x clear block · r reload · q quit "
        };
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Min(10), Constraint::Length(help.len() as u16)]).areas(area);
        let left = Line::from(Span::raw(format!(" {}", parts.join(" · "))).dim());
        let right = Line::from(Span::raw(help).dim()).right_aligned();
        frame.render_widget(Paragraph::new(left), left_area);
        frame.render_widget(Paragraph::new(right), right_area);
    }
}

/// OSC 52: hand the selection to the terminal's clipboard so Cmd-C habits keep working.
fn copy_to_clipboard(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | *b as u32) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn selection_columns_span_rows() {
        let sel = Selection { anchor: (3, 5), head: (5, 2), dragging: false };
        assert_eq!(sel.columns_on(2, 80), None);
        assert_eq!(sel.columns_on(3, 80), Some(5..80));
        assert_eq!(sel.columns_on(4, 80), Some(0..80));
        assert_eq!(sel.columns_on(5, 80), Some(0..3));
    }
}
