//! Center the annotation surface without changing terminal coordinates or input routing.

use ratatui::layout::Rect;

use super::App;

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
}
