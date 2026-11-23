//! A drag held past the diff pane's top or bottom scrolls the view a line at a time, on the
//! event loop's tick and at most once per [`STEP`], however often the terminal reports the
//! pointer: a terminal can report each pixel it moves, and many reports between two frames.
//! It scrolls only while there are lines of the anchor's hunk that way, the cursor inside it;
//! the selection never leaves the hunk, so the view does not either.

use std::time::{Duration, Instant};

use anyhow::Result;
use diffs_rs::engine::Command;
use ratatui::layout::Rect;

use super::Drag;
use crate::review::Screen;

/// The time between two lines scrolled while the pointer is held past an edge.
pub(in crate::review) const STEP: Duration = Duration::from_millis(50);

/// The edge a drag is held past, and when it last scrolled the view.
#[derive(Debug, Clone, Copy)]
pub(super) struct Edge {
    down: bool,
    stepped: Option<Instant>,
}

impl Edge {
    /// The edge of `pane` a pointer on screen row `row` is past, if any. The same edge as
    /// `held` keeps its pace: a pointer moving along it scrolls no faster.
    pub(super) fn past(row: u16, pane: Rect, held: Option<Self>) -> Option<Self> {
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
    pub(super) fn down(self) -> bool {
        self.down
    }
}

impl Screen {
    /// Whether a drag is held past the pane's edge, so the loop ticks every [`STEP`].
    pub(in crate::review) fn held_at_edge(&self) -> bool {
        self.drag.as_ref().is_some_and(|drag| drag.edge.is_some())
    }

    /// At `now`, scroll one line toward the edge a drag is held past, unless it did less than
    /// [`STEP`] ago or the anchor's hunk has no more lines that way. True when it scrolled.
    pub(in crate::review) fn step_at_edge(&mut self, now: Instant) -> Result<bool> {
        let Some(mut drag) = self.drag.take() else { return Ok(false) };
        let stepped = self.step(&mut drag, now);
        self.drag = Some(drag);
        if matches!(stepped, Ok(true)) {
            self.sync_markers();
        }
        stepped
    }

    fn step(&mut self, drag: &mut Drag, now: Instant) -> Result<bool> {
        let Some(edge) = drag.edge else { return Ok(false) };
        if edge.stepped.is_some_and(|at| now.saturating_duration_since(at) < STEP) {
            return Ok(false);
        }
        let done = if edge.down { drag.end >= drag.span.1 } else { drag.end <= drag.span.0 };
        let Some(diff) = self.active_diff_mut() else { return Ok(false) };
        let in_hunk =
            diff.cursor_row().is_some_and(|row| row.path == drag.anchor.path && row.hunk == Some(drag.hunk));
        if done || !in_hunk {
            return Ok(false);
        }
        diff.run(if edge.down { Command::Down } else { Command::Up })?;
        drag.follow(diff)?;
        drag.edge = Some(Edge { stepped: Some(now), ..edge });
        Ok(true)
    }
}
