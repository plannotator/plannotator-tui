//! Notes drawn in the diff (`[review] notes = "inline"` or `"both"`): a box per note under
//! the last line it is about, or under its file's heading for a note on the whole file, as a
//! pull request's comments sit in its diff. The viewer keeps rows free for each box
//! (`diff::insets`) and keeps the reader's place when they come, go or change height; this
//! module decides the boxes, sizes them to the width they draw in, and draws them.
//!
//! - Several notes on one line stack in the order they were made.
//! - In a split diff a box sits over its side's half, or across both when that half is too
//!   narrow for the comment box (`compose_at`'s rule).
//! - Notes on the whole change, notes whose lines the diff does not show (outdated, made in
//!   the whole file, in a hunk hidden with whitespace changes, in a file the filter hides) get
//!   no box; the footer counts them, and the panel lists them.
//! - The cursor stops on a box once. There Enter (or the panel's edit key) opens the note in
//!   the comment box, over its box, and the panel's remove key removes it, unless the diff
//!   gives that key to something else (`x` marks the file viewed there).
//! - `inline` starts with the panel away whatever notes there are; `z` opens it for the
//!   session. `both` keeps the panel's own rule (`notes_rail`).
//! - `panel`, the default, sets no rows at all: the diff draws as it always has.
//!
//! A box is laid out once per note revision and width ([`card::Card`]), and only the boxes
//! on screen are drawn.

mod card;
mod paint;

use std::collections::HashMap;

use anyhow::Result;
use plannotator_tui_review::{CodeAnnotation, NoteScope, Side};
use ratatui::crossterm::event::KeyEvent;

use super::compose_at::{MAX_WIDTH, MIN_SIDE_WIDTH};
use super::diff::{Lines, Slot};
use super::{Screen, notes};
use crate::config::review::Notes;
use crate::keys::{self, Action};

/// Narrower than this a box would show next to nothing: the note gets none.
const MIN_WIDTH: u16 = 12;

/// The boxes' state for one review.
#[derive(Debug, Default)]
pub(super) struct Inline {
    /// Each note's box id, by note id. Ids are never reused within a review.
    ids: HashMap<String, u64>,
    notes: HashMap<u64, String>,
    /// Each box's layout, by id, for the width it last drew at.
    cards: HashMap<u64, card::Card>,
    /// Notes that got no box at the last count: on the whole change, not in the diff, or
    /// with no room.
    skipped: usize,
    /// While notes show inline only, whether the panel was opened by hand.
    pub(super) rail: bool,
    /// The Notes setting the screen last followed.
    pub(super) view: Notes,
}

impl Inline {
    pub(super) fn new(view: Notes) -> Self {
        Self { view, ..Self::default() }
    }

    fn id(&mut self, note: &str) -> u64 {
        if let Some(&id) = self.ids.get(note) {
            return id;
        }
        let id = self.ids.len() as u64 + 1;
        self.ids.insert(note.to_owned(), id);
        self.notes.insert(id, note.to_owned());
        id
    }

    /// The card for `note` at `width`, laid out again only when either changed.
    fn card(&mut self, id: u64, note: &CodeAnnotation, width: u16, tab: u8) -> &card::Card {
        let card = self.cards.entry(id).or_insert_with(|| card::Card::of(note, width, tab));
        if !card.is_for(note, width, tab) {
            *card = card::Card::of(note, width, tab);
        }
        card
    }
}

/// Where a box goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Under this line of this side.
    Line(Side, u32),
    /// Under the file's heading.
    Heading,
}

/// Where `note`'s box goes: under its last line on its side, or under its file's heading.
/// `None` for a note with no box.
fn place(note: &CodeAnnotation) -> Option<Place> {
    match note.scope {
        NoteScope::Line if notes::in_diff(note) => Some(Place::Line(note.side, note.line_end)),
        NoteScope::File if !notes::is_outdated(note) => Some(Place::Heading),
        _ => None,
    }
}

/// The widths a box in one file can have: over the old side, over the new side, across.
type Widths = [Option<u16>; 3];

/// The width of a box over `over`'s side, or across for `None`.
fn width(widths: Widths, over: Option<Side>) -> Option<u16> {
    let [old, new, across] = widths;
    match over {
        Some(Side::Old) => old,
        Some(Side::New) => new,
        None => across,
    }
}

/// Which side a box at `place` sits over in a split diff, `None` across both: its own side
/// when that side's column is wide enough for the comment box (`compose_at`'s rule), and
/// across for a file's note. Unified, every box is across.
fn over(place: Place, split: bool, widths: Widths) -> Option<Side> {
    let Place::Line(side, _) = place else { return None };
    let [old, new, _] = widths;
    let width = if side == Side::Old { old } else { new };
    (split && width.is_some_and(|w| w >= MIN_SIDE_WIDTH)).then_some(side)
}

impl Screen {
    /// Whether notes show in the diff.
    pub(super) fn notes_inline(&self) -> bool {
        self.settings.now.notes != Notes::Panel
    }

    /// Give the viewer the boxes the notes want now, sized for the columns of the last frame.
    /// True when that changed the rows it keeps; the same boxes again change nothing, so this
    /// is cheap to call on every frame.
    pub(super) fn sync_boxes(&mut self) -> bool {
        let notes = self.store.annotations();
        let mut slots = Vec::new();
        let mut skipped = 0;
        if self.settings.now.notes != Notes::Panel {
            let tab = self.settings.now.tab_width.get();
            let area = self.geometry.diff;
            let split = self.diff.split();
            let mut widths: HashMap<&str, Widths> = HashMap::new();
            let mut order: Vec<usize> = (0..notes.len()).collect();
            order.sort_by_key(|&i| (notes.get(i).map_or(0, |note| note.created_at), i));
            for note in order.into_iter().filter_map(|i| notes.get(i)) {
                let path = note.file_path.as_str();
                let widths = *widths.entry(path).or_insert_with(|| {
                    [Some(Side::Old), Some(Side::New), None].map(|over| self.diff.box_width(area, path, over))
                });
                let Some(place) = place(note) else {
                    skipped += 1;
                    continue;
                };
                let over = over(place, split, widths);
                let width = width(widths, over).unwrap_or(0).min(MAX_WIDTH);
                if width < MIN_WIDTH {
                    skipped += 1;
                    continue;
                }
                let id = self.inline.id(&note.id);
                let height = self.inline.card(id, note, width, tab).height();
                let line = match place {
                    Place::Line(side, line) => Some((side, line)),
                    Place::Heading => None,
                };
                slots.push(Slot { id, path, line, over, height });
            }
        }
        self.inline.skipped = skipped;
        match self.diff.set_boxes(&slots) {
            Ok(changed) => changed,
            Err(error) => {
                self.status = Some(format!("could not show the notes in the diff: {error:#}"));
                false
            }
        }
    }

    /// The first column and width the box of a note on `lines` would have in `pane`, for the
    /// comment box to line up with the boxes. `None` when it would get none.
    pub(super) fn box_columns(&self, pane: ratatui::layout::Rect, lines: &Lines) -> Option<(u16, u16)> {
        let split = self.diff.split();
        let widths =
            [Some(Side::Old), Some(Side::New), None].map(|over| self.diff.box_width(pane, &lines.path, over));
        let over = over(Place::Line(lines.side, lines.last), split, widths);
        let full = width(widths, over)?;
        // Every box ends at the pane's right edge but one over the old side, which ends at
        // the line between the sides.
        let right = match over {
            Some(Side::Old) => self.diff.split_line()?,
            _ => pane.right(),
        };
        Some((right.checked_sub(full)?, full.min(MAX_WIDTH))).filter(|&(_, width)| width >= MIN_WIDTH)
    }

    /// How many notes have no box in the diff now, for the footer.
    pub(super) fn notes_not_inline(&self) -> usize {
        if !self.notes_inline() {
            return 0;
        }
        self.inline.skipped + self.diff.hidden_boxes().len()
    }

    /// The note whose box the cursor is on, in the diff (not the whole file).
    pub(super) fn cursor_note(&self) -> Option<&CodeAnnotation> {
        if self.whole.is_some() || !self.notes_inline() {
            return None;
        }
        let id = self.inline.notes.get(&self.diff.cursor_box()?)?;
        self.store.annotations().iter().find(|note| note.id == *id)
    }

    /// A key on a box: Enter or the panel's edit key edits its note, the panel's remove key
    /// removes it. A key the diff gives an action of its own keeps it. True when the key was
    /// the box's.
    pub(super) fn box_key(&mut self, key: &KeyEvent) -> Result<bool> {
        let Some(id) = self.cursor_note().map(|note| note.id.clone()) else { return Ok(false) };
        let diff = self.keys.action(key, keys::DIFF);
        let free = |action| diff.is_none() && self.keys.is(action, key);
        if diff == Some(Action::Confirm) || free(Action::Edit) {
            self.edit_note(&id);
        } else if free(Action::Remove) {
            self.remove_note(&id)?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// The key help's row for the keys on a box: edit / remove.
    pub(super) fn box_help(&self) -> (String, &'static str) {
        let edit = [self.keys.hint(Action::Confirm), self.keys.hint_besides(Action::Edit, keys::DIFF)];
        let edit = edit.into_iter().filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" ");
        let remove = self.keys.hint_besides(Action::Remove, keys::DIFF);
        let keys = [edit, remove].into_iter().filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" / ");
        (keys, "on a note drawn in the diff: edit / remove it")
    }

    /// Where note `id`'s box was drawn in the last frame, when it was.
    pub(super) fn drawn_box(&self, id: &str) -> Option<ratatui::layout::Rect> {
        let id = self.inline.ids.get(id)?;
        self.geometry.boxes.iter().find(|(_, drawn)| drawn == id).map(|(rect, _)| *rect)
    }

    /// After a click: a box the cursor landed on selects its note in the panel.
    pub(super) fn follow_box(&mut self) {
        let Some(id) = self.cursor_note().map(|note| note.id.clone()) else { return };
        self.select_note(&id);
    }

    /// The panel's cursor onto the note `id`.
    pub(super) fn select_note(&mut self, id: &str) {
        if let Some(position) = self.rail_order().iter().position(|&i| self.note_id(i) == Some(id)) {
            self.rail_cursor = position;
        }
    }

    /// The Notes setting changed: the panel follows its rule for the new one, and the boxes
    /// come or go at once.
    pub(super) fn notes_view_changed(&mut self) {
        self.inline.rail = false;
        if !self.rail_shown() && self.focus == super::Focus::Notes {
            self.focus = super::Focus::Diff;
        }
        self.sync_boxes();
    }
}

#[cfg(test)]
impl Screen {
    /// The note box `id` shows, for tests.
    pub(super) fn note_of_box(&self, id: u64) -> Option<&CodeAnnotation> {
        let note = self.inline.notes.get(&id)?;
        self.store.annotations().iter().find(|n| n.id == *note)
    }

    /// What a box's title says for `note`, for tests.
    pub(super) fn box_label(note: &CodeAnnotation) -> String {
        card::label(note)
    }
}
