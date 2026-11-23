//! What a mouse drag on either screen shares: the timer that scrolls a pane while the pointer
//! is held past its top or bottom, and the reports a screen ignores.
//!
//! A terminal in a window can report each pixel the pointer moves, so many reports for one
//! cell, and many between two frames. So no report scrolls a pane: held past its edge, the
//! pane scrolls a line at a time on the event loop's tick, at most once per [`STEP`].

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

/// The time between two lines scrolled while the pointer is held past an edge.
pub(crate) const STEP: Duration = Duration::from_millis(50);

/// The edge of a pane a drag is held past, and when it last scrolled the pane.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Edge {
    down: bool,
    stepped: Option<Instant>,
}

impl Edge {
    /// The edge of `pane` a pointer on screen row `row` is past, if any. The same edge as
    /// `held` keeps its pace: a pointer moving along it scrolls no faster.
    pub(crate) fn past(row: u16, pane: Rect, held: Option<Self>) -> Option<Self> {
        let down = if row < pane.y {
            false
        } else if row >= pane.bottom() {
            true
        } else {
            return None;
        };
        Some(held.filter(|held| held.down == down).unwrap_or(Self { down, stepped: None }))
    }

    /// Whether it is the bottom edge.
    pub(crate) fn down(self) -> bool {
        self.down
    }

    /// Whether a line is due at `now`: none scrolled yet, or the last one [`STEP`] ago.
    pub(crate) fn due(self, now: Instant) -> bool {
        self.stepped.is_none_or(|at| now.saturating_duration_since(at) >= STEP)
    }

    /// The same edge, a line scrolled at `now`.
    pub(crate) fn stepped(self, now: Instant) -> Self {
        Self { stepped: Some(now), ..self }
    }
}

/// `mouse` as a screen `screen` cells large takes it. A report for a cell outside the screen
/// is none a terminal reporting cells sends (pixel positions, an encoding gone wrong) and is
/// ignored, never read as the pointer past an edge; a release there still ends a drag, at the
/// nearest cell.
pub(crate) fn on_screen(mouse: MouseEvent, screen: Rect) -> Option<MouseEvent> {
    if screen.contains(Position::new(mouse.column, mouse.row)) {
        return Some(mouse);
    }
    matches!(mouse.kind, MouseEventKind::Up(_)).then(|| MouseEvent {
        column: mouse.column.clamp(screen.left(), screen.right().saturating_sub(1)),
        row: mouse.row.clamp(screen.top(), screen.bottom().saturating_sub(1)),
        ..mouse
    })
}
