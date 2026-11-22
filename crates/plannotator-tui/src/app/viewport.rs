//! Center the annotation surface without changing terminal coordinates or input routing.

use ratatui::layout::Rect;

use super::{App, Focus};
use crate::notes_rail::NotesRail;

/// Columns `narrow-column` and `widen-column` move the reading column by.
const COLUMN_STEP: u16 = 8;
/// The narrowest reading column `narrow-column` goes to.
const NARROWEST_COLUMN: u16 = 20;

impl App {
    /// The part of the terminal the app draws in: all of it by default, or a centered
    /// column narrowed by `[ui] horizontal_margin` and `[ui] max_width`. Both yield before
    /// the viewport drops below 80 columns, so a narrow terminal keeps the rail.
    pub(super) fn viewport(&self, area: Rect) -> Rect {
        let minimum = area.width.min(80);
        let margin = self.ui.horizontal_margin.min(area.width.saturating_sub(minimum) / 2);
        let available = area.width.saturating_sub(margin * 2);
        let width =
            if self.ui.max_width == 0 { available } else { available.min(self.ui.max_width.max(minimum)) };
        Rect { x: area.x + (area.width - width) / 2, width, ..area }
    }

    /// `narrow-column` and `widen-column`: move `[ui] doc_width` by one step for this run.
    /// The step starts from the column on screen, so a cap the pane already overrules
    /// narrows at once, and a column wider than the pane fills it.
    pub(super) fn resize_column(&mut self, wider: bool) {
        // The cap, unless the pane is narrower; keys that arrive before the next frame build
        // on the cap they set.
        let room = match self.geometry.column_room {
            0 => u16::try_from(self.open.layout.width).unwrap_or(u16::MAX),
            room => room,
        };
        let now = if self.ui.doc_width > 0 { self.ui.doc_width.min(room) } else { room };
        let next = if wider {
            now.saturating_add(COLUMN_STEP)
        } else {
            now.saturating_sub(COLUMN_STEP).max(NARROWEST_COLUMN)
        };
        self.ui.doc_width = next;
        self.status = Some(format!("reading column {next} wide"));
    }

    /// Start the notes rail as the last session left it, else as `[ui] notes_rail` says.
    pub(crate) fn start_notes_rail(&mut self) {
        self.notes_rail = NotesRail::load(self.ui.notes_rail, &self.data_dir);
    }

    /// Whether the open document has a note the rail would show: orphans have no place there.
    fn has_notes(&self) -> bool {
        self.open.store.len() > self.open.store.orphans()
    }

    /// Whether the notes rail is shown, width allowing. Hidden, its notes keep their marks in
    /// the text; only the bubbles go, and the text takes their columns.
    pub(super) fn rail_shown(&self) -> bool {
        self.notes_rail.shown(self.has_notes())
    }

    /// `toggle-notes`: close the notes rail, or open it again, and remember which. Only the
    /// state changes here; the next frame finds the text column wider or narrower and lays the
    /// text out again, the way a resize does.
    pub(super) fn toggle_notes(&mut self) {
        let remembered = self.notes_rail.toggle(self.has_notes(), &self.data_dir);
        self.after_rail_change(remembered);
    }

    /// Moving the focus to the notes opens a closed rail.
    pub(super) fn reveal_notes(&mut self) {
        let remembered = self.notes_rail.reveal(self.has_notes(), &self.data_dir);
        self.after_rail_change(remembered);
    }

    fn after_rail_change(&mut self, remembered: anyhow::Result<()>) {
        if let Err(error) = remembered {
            self.status = Some(format!("{error:#}"));
        }
        if !self.rail_shown() && self.focus == Focus::Rail {
            self.focus = Focus::Document;
        }
    }
}

#[cfg(test)]
mod tests;
