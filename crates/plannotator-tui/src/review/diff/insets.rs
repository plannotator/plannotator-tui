//! Rows the viewer keeps free between the diff's rows, for the notes drawn there
//! (`review::inline`): diffs-rs's insets, by this app's paths. The viewer keeps the reader's
//! place whenever they change (the cursor's line stays on its screen row), so setting them
//! is safe at any time.

use anyhow::Result;
use diffs_rs::engine::{self, DiffRowKind, Inset, InsetAnchor};
use plannotator_tui_review::Side;

use super::{Diff, Since, engine_side};

pub(in crate::review) use diffs_rs::engine::InsetArea as BoxArea;

/// Rows kept for one box: under `line` of `side`, or under the file's heading (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::review) struct Slot<'a> {
    pub(in crate::review) id: u64,
    pub(in crate::review) path: &'a str,
    pub(in crate::review) line: Option<(Side, u32)>,
    pub(in crate::review) height: u16,
}

/// The columns a box in one file can draw in, as the last frame laid them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::review) struct Columns {
    /// The code column of each side; unified, the one code column twice.
    pub(in crate::review) old: u16,
    pub(in crate::review) new: u16,
    /// From the old side's code to the diff's right edge; unified, the code column.
    pub(in crate::review) whole: u16,
    /// Split, each side's pane, its gutter included; `None` unified.
    pub(in crate::review) panes: Option<(u16, u16)>,
}

impl Diff {
    /// Keep rows for `slots`, in their order where several share a row. Slots in files the
    /// viewer does not have (not arrived, filtered out) keep none until the next call. True
    /// when the set changed; the same set again changes nothing.
    pub(in crate::review) fn set_boxes(&mut self, slots: &[Slot<'_>]) -> Result<bool> {
        let review = self.engine.review_id();
        let index: std::collections::HashMap<&str, usize> =
            self.paths.iter().enumerate().map(|(i, path)| (path.as_str(), i)).collect();
        let insets: Vec<Inset> = slots
            .iter()
            .filter_map(|slot| {
                let file = *index.get(slot.path)?;
                let anchor = match slot.line {
                    Some((side, line)) if line >= 1 => InsetAnchor::Line { side: engine_side(side), line },
                    Some(_) => return None,
                    None => InsetAnchor::FileHeading,
                };
                Some(Inset { review, id: slot.id, file, anchor, height: slot.height })
            })
            .collect();
        if self.engine.insets() == insets.as_slice() {
            return Ok(false);
        }
        let first = self.engine.insets().is_empty();
        self.engine.set_insets(&insets)?;
        // The view as the diff opened, its first heading at the top and the cursor on its
        // first line, now with the boxes: the viewer keeps the cursor's row when boxes come,
        // so a box under the first heading would start above the top.
        let in_first = self.engine.cursor_row().is_some_and(|row| row.file == 0);
        if first && self.since == Since::Opened && in_first && !insets.is_empty() {
            self.show_heading(0)?;
            self.engine.execute(engine::Command::FirstSource)?;
        }
        Ok(true)
    }

    /// The boxes on screen in the last frame, top to bottom.
    pub(in crate::review) fn box_areas(&self) -> &[BoxArea] {
        self.engine.inset_areas()
    }

    /// The boxes set whose lines the diff does not show now (a hunk drawn as its heading
    /// because whitespace is hidden, say): they keep no rows.
    pub(in crate::review) fn hidden_boxes(&self) -> Vec<u64> {
        self.engine.hidden_insets()
    }

    /// The columns a box in the file at `path` has, as the last frame laid them out. `None`
    /// before the first frame and for a file the viewer does not have.
    pub(in crate::review) fn box_columns(&self, path: &str) -> Option<Columns> {
        let file = self.index(path)?;
        let old = self.engine.inset_width(file, engine::Side::Old)?;
        let new = self.engine.inset_width(file, engine::Side::New)?;
        // Split, the viewer's box runs from the old side's code over the line between the
        // sides to the right edge: the old code column, then all of the new side's pane.
        let Some(line) = self.split_line() else {
            return Some(Columns { old, new, whole: new, panes: None });
        };
        let (left, right) = (line.saturating_sub(self.area.x), self.area.right().saturating_sub(line + 1));
        Some(Columns { old, new, whole: old + 1 + right, panes: Some((left, right)) })
    }

    /// The box the cursor is on, by id.
    pub(in crate::review) fn cursor_box(&self) -> Option<u64> {
        match self.engine.cursor_row()?.kind {
            DiffRowKind::Inset { id } => Some(id),
            _ => None,
        }
    }

    /// The box drawn at screen cell (`column`, `row`) in the last frame, by id.
    pub(in crate::review) fn box_at(&self, column: u16, row: u16) -> Option<u64> {
        match self.engine.row_at(column, row)?.kind {
            DiffRowKind::Inset { id } => Some(id),
            _ => None,
        }
    }

    /// The last screen row of the boxes right under screen row `row`, or `row` itself when
    /// none hangs there.
    pub(in crate::review) fn under_boxes(&self, row: u16) -> u16 {
        let column = self.area.right().saturating_sub(2);
        let mut last = row;
        while last + 1 < self.area.bottom() && self.box_at(column, last + 1).is_some() {
            last += 1;
        }
        last
    }
}

#[cfg(test)]
impl Diff {
    /// The viewer's boxes as set, for tests.
    pub(in crate::review) fn boxes_set(&self) -> Vec<Inset> {
        self.engine.insets().to_vec()
    }
}
