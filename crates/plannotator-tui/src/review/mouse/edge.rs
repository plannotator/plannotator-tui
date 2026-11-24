//! A drag held past the diff pane's top or bottom scrolls the view a line at a time, on the
//! event loop's tick (`ui::drag`). It scrolls only while there are lines of the anchor's hunk
//! that way, the cursor inside it; the selection never leaves the hunk, so the view does not
//! either.

use std::time::Instant;

use anyhow::Result;
use diffs_rs::engine::Command;

use super::Drag;
use crate::review::Screen;

impl Screen {
    /// Whether a drag is held past the pane's edge, so the loop ticks every `ui::drag::STEP`.
    pub(in crate::review) fn held_at_edge(&self) -> bool {
        self.drag.as_ref().is_some_and(|drag| drag.edge.is_some())
    }

    /// At `now`, scroll one line toward the edge a drag is held past, unless it did less than
    /// a step ago or the anchor's hunk has no more lines that way. True when it scrolled.
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
        let Some(edge) = drag.edge.filter(|edge| edge.due(now)) else { return Ok(false) };
        let done = if edge.down() { drag.end >= drag.span.1 } else { drag.end <= drag.span.0 };
        let Some(diff) = self.active_diff_mut() else { return Ok(false) };
        let in_hunk =
            diff.cursor_row().is_some_and(|row| row.path == drag.anchor.path && row.hunk == Some(drag.hunk));
        if done || !in_hunk {
            return Ok(false);
        }
        diff.run(if edge.down() { Command::Down } else { Command::Up })?;
        drag.follow(diff)?;
        drag.edge = Some(edge.stepped(now));
        Ok(true)
    }
}
