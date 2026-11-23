//! The file list on screen: a heading with the count, the filter when one is on, then the
//! rows. A file row is its name, then its status letter, `+12 −3` and the viewed tick at
//! the right edge; the name has the room first and is cut in the middle when it must be.

use std::collections::HashMap;

use plannotator_tui_review::{FilePatch, FileStatus};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr as _;

use super::fit;
use super::tree::{self, Node, Row};
use crate::review::{Focus, Mode, Screen};
use crate::theme::palette;

/// Below this many columns inside its padding the list indents one column a level.
const NARROW: usize = 30;

/// What the list draws this frame: its rows and the counts' column widths.
#[derive(Debug)]
pub(in crate::review) struct Listing {
    pub(in crate::review) paths: Vec<String>,
    pub(in crate::review) rows: Vec<Row>,
    /// Columns of the widest `+N` and `−N`.
    plus: usize,
    minus: usize,
    /// Whether any file is viewed, so the tick column is kept.
    ticks: bool,
}

impl Listing {
    /// The columns right of a name: the status letter and the counts, each with its gap.
    fn status_width() -> usize {
        2
    }

    fn counts_width(&self) -> usize {
        1 + self.plus + 1 + self.minus
    }

    fn tick_width(&self) -> usize {
        if self.ticks { 2 } else { 0 }
    }
}

/// `+12`, `−3`; a binary file has neither.
fn counts(file: &FilePatch) -> (String, String) {
    if file.binary {
        (String::new(), "bin".to_owned())
    } else {
        (format!("+{}", file.additions), format!("\u{2212}{}", file.deletions))
    }
}

fn status_letter(file: &FilePatch) -> (&'static str, Color) {
    match file.status {
        FileStatus::Added => ("A", palette().looks_good),
        FileStatus::Deleted => ("D", palette().delete),
        FileStatus::Modified => ("M", Color::Blue),
        FileStatus::Renamed => ("R", palette().code_keyword),
    }
}

impl Screen {
    fn patches(&self) -> HashMap<&str, &FilePatch> {
        self.capture.files.iter().map(|f| (f.path.as_str(), f)).collect()
    }

    /// The list's rows for the filter as typed.
    pub(in crate::review) fn listing(&self) -> Listing {
        self.listing_of(self.listed())
    }

    /// The list's rows for the files the diff shows: what the width fits, so the list keeps
    /// its width while a filter is typed.
    pub(in crate::review) fn shown_listing(&self) -> Listing {
        self.listing_of(self.files.shown.clone())
    }

    fn listing_of(&self, paths: Vec<String>) -> Listing {
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let rows = tree::rows(&refs, &self.files.collapsed);
        let patches = self.patches();
        let (mut plus, mut minus) = (0, 0);
        for path in &paths {
            if let Some(file) = patches.get(path.as_str()) {
                let (p, m) = counts(file);
                plus = plus.max(p.width());
                minus = minus.max(m.width());
            }
        }
        let ticks = self.capture.files.iter().any(|f| self.progress.is_viewed(&f.path));
        Listing { paths, rows, plus, minus, ticks }
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
        let patches = self.patches();
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
            let file = path.and_then(|p| patches.get(p).copied());
            // A folder is viewed when every file in it is, closed or open.
            let viewed = if let Some(p) = path {
                self.progress.is_viewed(p)
            } else {
                let mut inside = listing.paths.iter().filter(|p| p.starts_with(&row.key)).peekable();
                inside.peek().is_some() && inside.all(|p| self.progress.is_viewed(p))
            };
            let current = path.is_some() && path == self.files.current.as_deref();
            draw_row(buf, line, row, file, listing, RowLook { viewed, current });
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

#[derive(Debug, Clone, Copy)]
struct RowLook {
    viewed: bool,
    current: bool,
}

/// One row in `line`, padded a column on each side.
fn draw_row(
    buf: &mut ratatui::buffer::Buffer,
    line: Rect,
    row: &Row,
    file: Option<&FilePatch>,
    listing: &Listing,
    look: RowLook,
) {
    let inner = usize::from(line.width.saturating_sub(2));
    // A narrow list indents by one column a level; deep rows give up indent before their
    // name gets too narrow to read.
    let step = if inner >= NARROW { 2 } else { 1 };
    let indent = (step * row.depth).min(inner.saturating_sub(12));
    let x = line.x + 1 + indent as u16;
    let room = inner - indent.min(inner);
    let tick = listing.tick_width();
    let right = line.x + 1 + inner as u16;
    if tick > 0 && look.viewed {
        buf.set_string(right.saturating_sub(1), line.y, "\u{2713}", Style::new().fg(palette().looks_good));
    }
    let Some(file) = file else {
        let chevron =
            if matches!(row.node, Node::Folder { expanded: false }) { "\u{25b8} " } else { "\u{25be} " };
        buf.set_stringn(x, line.y, chevron, room, Style::new().fg(Color::DarkGray));
        let label_room = room.saturating_sub(2 + if look.viewed { tick } else { 0 });
        let style = if look.viewed { Style::new().fg(Color::DarkGray) } else { Style::new() };
        buf.set_stringn(x + 2, line.y, fit::folder(&row.label, label_room), label_room, style);
        return;
    };
    let status_width = Listing::status_width();
    let counts_width = listing.counts_width();
    let avail = room.saturating_sub(tick);
    let shown = fit::shown(row.label.width(), avail, status_width, counts_width);
    let used = if shown.status { status_width } else { 0 } + if shown.counts { counts_width } else { 0 };
    let name_room = avail.saturating_sub(used);
    let name = fit::middle(&row.label, name_room);
    let mut name_style = if look.viewed { Style::new().fg(Color::DarkGray) } else { Style::new() };
    if look.current {
        name_style = name_style.fg(palette().accent).bold();
    }
    buf.set_stringn(x, line.y, &name, name_room, name_style);
    let dim = Style::new().fg(Color::DarkGray);
    let mut end = right.saturating_sub(tick as u16);
    if shown.counts {
        let (plus, minus) = counts(file);
        let start = end.saturating_sub(counts_width as u16);
        let colour = |c: Color| if look.viewed { dim } else { Style::new().fg(c) };
        let plus = format!("{plus:>width$}", width = listing.plus);
        let minus = format!("{minus:>width$}", width = listing.minus);
        buf.set_span(
            start + 1,
            line.y,
            &Span::styled(plus, colour(palette().looks_good)),
            listing.plus as u16,
        );
        let minus_x = start + 2 + listing.plus as u16;
        buf.set_span(minus_x, line.y, &Span::styled(minus, colour(palette().delete)), listing.minus as u16);
        end = start;
    }
    if shown.status {
        let (letter, colour) = status_letter(file);
        let style = if look.viewed { dim } else { Style::new().fg(colour) };
        buf.set_string(end.saturating_sub(1), line.y, letter, style);
    }
}
