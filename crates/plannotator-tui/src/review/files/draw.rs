//! The file list on screen: a heading with the count, the filter when one is on, then the
//! rows. A file row is its name, then its status letter, `+12 −3` and the viewed tick at
//! the right edge; the name has the room first and is cut in the middle when it must be.

use std::collections::HashMap;
use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use unicode_width::UnicodeWidthStr as _;

use super::fit;
use super::row::{RowLook, counts, draw_row, indent};
use super::tree::{self, Node, Row};
use crate::review::{Focus, Mode, Screen};
use crate::theme::palette;

/// What the list draws: its rows and the counts' column widths. Built when the files, the
/// filter, the folds or the viewed marks change, and kept between frames.
#[derive(Debug)]
pub(in crate::review) struct Listing {
    pub(in crate::review) paths: Vec<String>,
    pub(in crate::review) rows: Vec<Row>,
    /// Each row's viewed mark: a file's own, a folder's when every file in it is viewed.
    viewed: Vec<bool>,
    /// Columns of the widest `+N` and `−N`.
    pub(super) plus: usize,
    pub(super) minus: usize,
    /// Whether any file is viewed, so the tick column is kept.
    ticks: bool,
}

impl Listing {
    /// The columns right of a name: the status letter and the counts, each with its gap.
    pub(super) fn status_width() -> usize {
        2
    }

    pub(super) fn counts_width(&self) -> usize {
        1 + self.plus + 1 + self.minus
    }

    pub(super) fn tick_width(&self) -> usize {
        if self.ticks { 2 } else { 0 }
    }
}

impl Screen {
    /// The list's rows for the filter as typed, built again only when what they show
    /// changed.
    pub(in crate::review) fn listing(&self) -> Rc<Listing> {
        if let Some((key, listing)) = self.files.rows.borrow().as_ref()
            && key.version == self.files.version
            && key.filter == self.files.filter
            && key.collapsed == self.files.collapsed
        {
            return Rc::clone(listing);
        }
        let built = super::Built {
            version: self.files.version,
            filter: self.files.filter.clone(),
            collapsed: self.files.collapsed.clone(),
        };
        let listing = Rc::new(self.listing_of(self.listed(), &self.files.collapsed));
        *self.files.rows.borrow_mut() = Some((built, Rc::clone(&listing)));
        listing
    }

    /// The columns the rows of the files the diff shows need, every folder open: the
    /// list's width, which stays while a filter is typed or a folder is closed.
    pub(in crate::review) fn shown_width(&self) -> u16 {
        let key = (self.files.version, self.files.shown_version);
        if let Some((version, shown, width)) = self.files.shown_width.get()
            && (version, shown) == key
        {
            return width;
        }
        let open = std::collections::BTreeSet::new();
        let width = self.files_content_width(&self.listing_of(self.files.shown().to_vec(), &open));
        self.files.shown_width.set(Some((key.0, key.1, width)));
        width
    }

    fn listing_of(&self, paths: Vec<String>, collapsed: &std::collections::BTreeSet<String>) -> Listing {
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let rows = tree::rows(&refs, collapsed);
        let (mut plus, mut minus) = (0, 0);
        for path in &paths {
            if let Some(change) = self.change_of(path) {
                let (p, m) = counts(&change);
                plus = plus.max(p.width());
                minus = minus.max(m.width());
            }
        }
        let ticks = self.capture.files.iter().any(|f| self.progress.is_viewed(&f.path));
        // A folder is viewed when every file in it is, closed or open.
        let mut folders: HashMap<&str, bool> = rows
            .iter()
            .filter(|r| matches!(r.node, Node::Folder { .. }))
            .map(|r| (r.key.as_str(), true))
            .collect();
        for path in &paths {
            if self.progress.is_viewed(path) {
                continue;
            }
            for (at, _) in path.match_indices('/') {
                if let Some(all) = path.get(..=at).and_then(|folder| folders.get_mut(folder)) {
                    *all = false;
                }
            }
        }
        let viewed = rows
            .iter()
            .map(|row| match row.node {
                Node::File(i) => paths.get(i).is_some_and(|p| self.progress.is_viewed(p)),
                Node::Folder { .. } => folders.get(row.key.as_str()).copied().unwrap_or(false),
            })
            .collect();
        Listing { paths, rows, viewed, plus, minus, ticks }
    }

    /// The rows above the file rows: the heading, the filter, hidden notes.
    fn files_header(&self, listing: &Listing) -> Vec<(String, Style)> {
        let heading = self.files_heading(listing.paths.len());
        let focused = self.focus == Focus::Files && matches!(self.mode, Mode::Browse | Mode::FilterFiles);
        let style = if focused { Style::new().fg(palette().accent).bold() } else { Style::new().bold() };
        let mut lines = vec![(format!(" {heading}"), style)];
        let filter = &self.files.filter;
        let typing = self.mode == Mode::FilterFiles;
        if typing || filter.is_active() {
            let mut parts = Vec::new();
            if typing {
                parts.push(format!("/{}\u{2581}", filter.query));
            } else if !filter.query.trim().is_empty() {
                parts.push(format!("/{}", filter.query.trim()));
            }
            parts.extend(filter.kinds_label());
            if filter.hide_viewed {
                parts.push("not viewed".to_owned());
            }
            lines.push((format!(" {}", parts.join(" \u{b7} ")), Style::new().fg(palette().accent)));
        }
        let hidden = self.notes_in_hidden_files();
        if hidden > 0 {
            let notes =
                if hidden == 1 { "1 unsent note".to_owned() } else { format!("{hidden} unsent notes") };
            lines.push((format!(" {notes} in hidden files"), Style::new().fg(palette().comment)));
        }
        lines
    }

    /// The columns the list needs to show every row whole, padding included.
    pub(in crate::review) fn files_content_width(&self, listing: &Listing) -> u16 {
        let right = Listing::status_width() + listing.counts_width() + listing.tick_width();
        let rows = listing.rows.iter().map(|row| {
            let label = row.label.width() + 2 * row.depth;
            match row.node {
                Node::Folder { .. } => label + 2,
                Node::File(_) => label + right,
            }
        });
        let heading = self.files_heading(listing.paths.len()).width();
        let widest = rows.chain([heading]).max().unwrap_or(0);
        u16::try_from(widest + 2).unwrap_or(u16::MAX)
    }

    /// Draw the list in `area` and the divider right of it.
    pub(in crate::review) fn draw_files(&mut self, frame: &mut Frame, area: Rect, listing: &Listing) {
        let header = self.files_header(listing);
        let buf = frame.buffer_mut();
        let divider = Style::new().fg(Color::DarkGray);
        for y in area.top()..area.bottom() {
            buf.set_string(area.right(), y, "\u{2502}", divider);
        }
        for (i, (text, style)) in header.iter().enumerate().take(usize::from(area.height)) {
            let text = fit::middle(text, usize::from(area.width));
            buf.set_stringn(area.x, area.y + i as u16, text, usize::from(area.width), *style);
        }
        let top = area.y + header.len() as u16;
        let height = usize::from(area.bottom().saturating_sub(top));
        if listing.rows.is_empty() {
            if height > 0 {
                buf.set_string(area.x, top, " No file matches", Style::new().dim());
            }
            self.geometry.file_rows = Vec::new();
            return;
        }
        if self.focus != Focus::Files {
            self.files.cursor.clone_from(&self.files.current);
        }
        let at = self.files.cursor.as_deref().and_then(|key| row_of(&listing.rows, key));
        // The cursor's row comes into view when the cursor moves; the wheel scrolls freely.
        if let Some(at) = at
            && self.files.scrolled_to != self.files.cursor
        {
            self.files.scrolled_to.clone_from(&self.files.cursor);
            if at < self.files.scroll {
                self.files.scroll = at;
            } else if at >= self.files.scroll + height {
                self.files.scroll = at + 1 - height.max(1);
            }
        }
        self.files.scroll = self.files.scroll.min(listing.rows.len().saturating_sub(height));
        // Which columns the file rows show, once for all of them so they line up.
        let inner = usize::from(area.width.saturating_sub(2));
        let names: Vec<usize> = listing
            .rows
            .iter()
            .filter(|r| matches!(r.node, Node::File(_)))
            .map(|row| indent(row.depth, inner) + row.label.width())
            .collect();
        let room = inner.saturating_sub(listing.tick_width());
        let shown = fit::columns(&names, room, Listing::status_width(), listing.counts_width());
        let mut hits = Vec::new();
        for (offset, row) in listing.rows.iter().enumerate().skip(self.files.scroll).take(height) {
            let y = top + (offset - self.files.scroll) as u16;
            let line = Rect { y, height: 1, ..area };
            let under_cursor = Some(offset) == at && self.focus == Focus::Files;
            if under_cursor {
                buf.set_style(line, Style::new().bg(palette().block_bg));
            }
            let path = match row.node {
                Node::File(i) => listing.paths.get(i).map(String::as_str),
                Node::Folder { .. } => None,
            };
            let change = path.and_then(|p| self.change_of(p));
            let viewed = listing.viewed.get(offset).copied().unwrap_or(false);
            let current = path.is_some() && path == self.files.current.as_deref();
            draw_row(buf, line, row, change.as_ref(), listing, RowLook { viewed, current, shown });
            hits.push((y, row.key.clone()));
        }
        self.geometry.file_rows = hits;
    }
}

/// The index of the row for `key`, or of the folder row that holds it while it is closed.
pub(in crate::review) fn row_of(rows: &[Row], key: &str) -> Option<usize> {
    rows.iter().position(|r| r.key == key).or_else(|| {
        rows.iter()
            .enumerate()
            .rfind(|(_, r)| key.starts_with(&r.key) && r.key.ends_with('/'))
            .map(|(i, _)| i)
    })
}
