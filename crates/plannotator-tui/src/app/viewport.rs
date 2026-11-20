//! Center the annotation surface without changing terminal coordinates or input routing.

use ratatui::layout::Rect;

use super::App;

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
}

#[cfg(test)]
mod tests;
