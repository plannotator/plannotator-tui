//! One row of the file list: a folder with its chevron, or a file with its name, status
//! letter, `+12 −3` counts and viewed tick, the name cut in the middle when it must be.

use plannotator_tui_review::{FilePatch, FileStatus};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use super::draw::Listing;
use super::fit;
use super::tree::{Node, Row};
use crate::theme::palette;

/// Below this many columns inside its padding the list indents one column a level.
const NARROW: usize = 30;

/// What a file row says about the change to it. A path the patch names twice, as git does
/// when a file is replaced by a symlink (deleted, then added), is one row: `T`, a type
/// change, with both sections' counts.
#[derive(Debug, Clone, Copy)]
pub(super) struct Change {
    /// `None` for a type change.
    status: Option<FileStatus>,
    binary: bool,
    additions: u32,
    deletions: u32,
}

impl Change {
    /// The change the patch's sections for one path make.
    pub(super) fn of<'a>(mut sections: impl Iterator<Item = &'a FilePatch>) -> Option<Self> {
        let first = sections.next()?;
        let mut change = Self {
            status: Some(first.status),
            binary: first.binary,
            additions: first.additions,
            deletions: first.deletions,
        };
        for more in sections {
            change.status = None;
            change.binary |= more.binary;
            change.additions = change.additions.saturating_add(more.additions);
            change.deletions = change.deletions.saturating_add(more.deletions);
        }
        Some(change)
    }
}

/// `+12`, `−3`; a binary file has neither.
pub(super) fn counts(change: &Change) -> (String, String) {
    if change.binary {
        (String::new(), "bin".to_owned())
    } else {
        (format!("+{}", change.additions), format!("\u{2212}{}", change.deletions))
    }
}

fn status_letter(change: &Change) -> (&'static str, Color) {
    match change.status {
        Some(FileStatus::Added) => ("A", palette().looks_good),
        Some(FileStatus::Deleted) => ("D", palette().delete),
        Some(FileStatus::Modified) => ("M", Color::Blue),
        Some(FileStatus::Renamed) => ("R", palette().code_keyword),
        None => ("T", Color::Blue),
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RowLook {
    pub(super) viewed: bool,
    pub(super) current: bool,
    /// The columns every file row shows.
    pub(super) shown: fit::Shown,
}

/// A row's indent in a list `inner` columns wide: two columns a level, one in a narrow
/// list; deep rows give up indent before their name gets too narrow to read.
pub(super) fn indent(depth: usize, inner: usize) -> usize {
    let step = if inner >= NARROW { 2 } else { 1 };
    (step * depth).min(inner.saturating_sub(12))
}

/// One row in `line`, padded a column on each side.
pub(super) fn draw_row(
    buf: &mut ratatui::buffer::Buffer,
    line: Rect,
    row: &Row,
    file: Option<&Change>,
    listing: &Listing,
    look: RowLook,
) {
    let inner = usize::from(line.width.saturating_sub(2));
    let indent = indent(row.depth, inner);
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
    let shown = look.shown;
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
