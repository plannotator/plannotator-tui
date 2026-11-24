//! A mouse selection being dragged in the document. The head follows the pointer, kept to the
//! text's area; a report for the cell already reported changes nothing, and no report scrolls.
//! Held past the text's top or bottom, the document scrolls a row per step of the event
//! loop's tick (`ui::drag`), as far as its end.

use std::time::Instant;

use ratatui::crossterm::event::MouseEvent;

use super::App;
use crate::ui::drag::Edge;

/// Where the pointer of a mouse selection was last reported, and the edge it is held past.
#[derive(Debug, Clone, Copy)]
pub(super) struct MouseDrag {
    cell: (u16, u16),
    edge: Option<Edge>,
}

impl MouseDrag {
    /// A drag pressed at `mouse`.
    pub(super) fn pressed(mouse: MouseEvent) -> Self {
        Self { cell: (mouse.column, mouse.row), edge: None }
    }
}

impl App {
    /// The pointer moved with the button down while a selection grows.
    pub(super) fn drag_to(&mut self, mouse: MouseEvent) {
        if !self.selection.is_some_and(|s| s.dragging) {
            return;
        }
        let cell = (mouse.column, mouse.row);
        // A selection `v` started, or one a key went on with, has no pointer yet.
        let held = self.mouse_drag.filter(|drag| drag.cell != cell);
        if self.mouse_drag.is_some() && held.is_none() {
            return;
        }
        let edge = Edge::past(mouse.row, self.geometry.doc, held.and_then(|drag| drag.edge));
        self.mouse_drag = Some(MouseDrag { cell, edge });
        self.follow_pointer(cell);
    }

    /// Whether a mouse selection is held past the text's edge, so the loop ticks every step.
    pub(crate) fn held_at_edge(&self) -> bool {
        self.selection.is_some_and(|s| s.dragging) && self.mouse_drag.is_some_and(|drag| drag.edge.is_some())
    }

    /// At `now`, scroll a row toward the edge a mouse selection is held past, unless it did
    /// less than a step ago or the document ends that way. True when it scrolled.
    pub(crate) fn step_at_edge(&mut self, now: Instant) -> bool {
        let Some(drag) = self.mouse_drag.filter(|_| self.held_at_edge()) else { return false };
        let Some(edge) = drag.edge.filter(|edge| edge.due(now)) else { return false };
        let before = self.scroll;
        self.scroll_by(if edge.down() { 1 } else { -1 });
        if self.scroll == before {
            return false;
        }
        self.mouse_drag = Some(MouseDrag { edge: Some(edge.stepped(now)), ..drag });
        self.follow_pointer(drag.cell);
        true
    }

    /// The selection's head to the text under `cell`, or at the text's nearest edge.
    fn follow_pointer(&mut self, (column, row): (u16, u16)) {
        if let Some(pos) = self.doc_position(column, row, true)
            && let Some(sel) = self.selection.as_mut()
        {
            sel.set_head(pos);
        }
    }
}
