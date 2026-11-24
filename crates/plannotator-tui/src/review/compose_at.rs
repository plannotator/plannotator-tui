//! Where the comment box opens: next to what it is about, as a document's box opens at its
//! selection, never along the bottom of the screen.
//!
//! - Lines (`c`, `s`, the toolbar, editing a line note): directly under the last row that
//!   shows them, else above the first, so the box covers none of them; in a split diff over
//!   that side's pane, wherever the line between the sides was dragged. Lines taller than
//!   the room keep the box at the end the person last moved to, the cursor's end.
//! - A file (`C`, editing a file note): under the file's heading, else at the top of the pane.
//! - The whole change (`M`, editing its note), or lines no longer on screen: centred.
//!
//! The box only draws over the diff: opening, typing in and closing it never move the view
//! or the cursor. It grows with its text on the side it opened on and, out of room there,
//! scrolls inside rather than jump to the other side.

use plannotator_tui_review::{NoteScope, Side};
use ratatui::layout::Rect;

use super::diff::Lines;
use super::{Draft, Screen};
use crate::ui::compose;

/// The box's widest; code suggestions want the room.
pub(super) const MAX_WIDTH: u16 = 84;
/// A split diff's side narrower than this is too narrow for the box, which then spans the
/// other side too.
const MIN_SIDE_WIDTH: u16 = 48;
/// One row of text and its borders.
const MIN_HEIGHT: u16 = 3;
/// The room a side needs for the box to open there: three rows of text to grow into.
const ROOM: u16 = 5;

/// Which side of its rows the box took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    Below,
    Above,
    /// Over the rows, at the end last moved to: no side had room.
    Over,
}

/// The side the box took when it first drew for this draft in this pane, kept while it
/// stays open so it does not hop over the lines as it grows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Placed {
    draft: Draft,
    pane: Rect,
    place: Place,
}

/// What the box is next to.
enum Anchor {
    Lines(Lines),
    File(String),
    Centre,
}

impl Screen {
    /// The box's rectangle in the diff pane `pane` for `draft`, as tall as its text wants up to
    /// the room it has there.
    pub(super) fn compose_rect(&mut self, pane: Rect, draft: &Draft) -> Rect {
        let anchor = self.anchor(draft);
        // Where the viewer drew the line between a split diff's sides, wherever it was dragged.
        let middle = self.active_diff().filter(|diff| diff.split()).and_then(super::diff::Diff::split_line);
        let (x, width) = match &anchor {
            Anchor::Lines(lines) => columns(pane, middle, lines.side),
            Anchor::File(_) | Anchor::Centre => (pane.x, pane.width.min(MAX_WIDTH)),
        };
        let wrap = usize::from(width.saturating_sub(compose::CHROME));
        let wanted = compose::height(&self.compose, wrap).min(pane.height);
        let diff = self.active_diff();
        let rows = match &anchor {
            Anchor::Lines(lines) => diff.and_then(|diff| diff.rows_showing(lines)).map(|(first, last)| {
                let at_first = diff.and_then(|diff| diff.line_on(lines.side).ok().flatten());
                let top = lines.first < lines.last
                    && at_first.is_some_and(|(path, line)| path == lines.path && line == lines.first);
                (first, last, top)
            }),
            Anchor::File(path) => match diff.and_then(|diff| diff.heading_row(path)) {
                Some(row) => Some((row, row, false)),
                // No heading on screen: at the top of the pane.
                None => return Rect { x, y: pane.y, width, height: wanted },
            },
            Anchor::Centre => None,
        };
        let Some((first, last, top)) = rows else {
            let x = pane.x + (pane.width - width) / 2;
            return Rect { x, y: pane.y + (pane.height - wanted) / 2, width, height: wanted };
        };
        let held = self
            .compose_at
            .as_ref()
            .filter(|placed| placed.draft == *draft && placed.pane == pane)
            .map(|placed| placed.place);
        let (y, height, place) = vertical(pane, (first, last), top, wanted, held);
        self.compose_at = Some(Placed { draft: draft.clone(), pane, place });
        Rect { x, y, width, height }
    }

    fn anchor(&self, draft: &Draft) -> Anchor {
        match draft {
            Draft::Comment(lines) | Draft::Suggestion(lines) | Draft::Outside(lines, _) => {
                Anchor::Lines(lines.clone())
            }
            Draft::File(path) => Anchor::File(path.clone()),
            Draft::Change => Anchor::Centre,
            Draft::Edit(id) => match self.store.annotations().iter().find(|note| note.id == *id) {
                Some(note) if note.scope == NoteScope::File => Anchor::File(note.file_path.clone()),
                Some(note) if note.scope == NoteScope::Line => Anchor::Lines(Lines {
                    path: note.file_path.clone(),
                    side: note.side,
                    first: note.line_start,
                    last: note.line_end,
                }),
                _ => Anchor::Centre,
            },
        }
    }
}

/// The box's first column and width, never wider than [`MAX_WIDTH`] or the pane. In a split
/// diff, `middle` the column of the line between its sides: over the lines' side when that
/// side is wide enough, else as wide as the box can be from that side's outer edge, so it
/// still starts (old) or ends (new) over the lines. Unified, from the pane's left edge.
fn columns(pane: Rect, middle: Option<u16>, side: Side) -> (u16, u16) {
    let widest = pane.width.min(MAX_WIDTH);
    let Some(middle) = middle.filter(|&middle| (pane.left()..pane.right()).contains(&middle)) else {
        return (pane.x, widest);
    };
    let (x, width) = match side {
        Side::Old => (pane.x, middle - pane.x),
        Side::New => (middle + 1, pane.right() - (middle + 1)),
    };
    if width >= MIN_SIDE_WIDTH {
        return (x, width.min(MAX_WIDTH));
    }
    match side {
        Side::Old => (pane.x, widest),
        Side::New => (pane.right() - widest, widest),
    }
}

/// The box's first row, its height and where that puts it, for rows `first..=last` of
/// `pane` and a box `wanted` rows tall (at most the pane). Below the rows when there is room
/// to grow there, else above, else on the roomier side shrunk to fit (the text scrolls
/// inside), else over the rows at the end the person moved to last (`top`, the first row).
/// `held` is the place the box took when it opened, kept while it still fits there.
fn vertical(
    pane: Rect,
    (first, last): (u16, u16),
    top: bool,
    wanted: u16,
    held: Option<Place>,
) -> (u16, u16, Place) {
    let smallest = MIN_HEIGHT.min(pane.height);
    let below = pane.bottom().saturating_sub(last + 1);
    let above = first.saturating_sub(pane.y);
    let room = |place| match place {
        Place::Below => below,
        Place::Above => above,
        Place::Over => pane.height,
    };
    let grow = wanted.max(ROOM).min(pane.height);
    let place = held.filter(|&place| room(place) >= smallest).unwrap_or_else(|| {
        let fits = |need| [Place::Below, Place::Above].into_iter().find(|&place| room(place) >= need);
        let roomier = if below >= above { Place::Below } else { Place::Above };
        fits(grow).or_else(|| fits(wanted)).unwrap_or(if room(roomier) >= smallest {
            roomier
        } else {
            Place::Over
        })
    });
    let height = wanted.min(room(place)).max(smallest);
    let y = match place {
        Place::Below => last + 1,
        Place::Above => first - height,
        // Next to the end last moved to, on the other rows.
        Place::Over if top => (first + 1).min(pane.bottom() - height),
        Place::Over => last.saturating_sub(height).max(pane.y),
    };
    (y, height, place)
}
