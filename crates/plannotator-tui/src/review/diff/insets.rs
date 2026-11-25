//! Rows the viewer keeps free between the diff's rows, for the notes drawn there
//! (`review::inline`): diffs-rs's insets, by this app's paths. The viewer keeps the reader's
//! place whenever they change (the cursor's line stays on its screen row), so setting them
//! is safe at any time.

use anyhow::Result;
use diffs_rs::engine::{DiffRowKind, Inset, InsetAnchor, InsetSpan};
use plannotator_tui_review::Side;
use ratatui::layout::Rect;

use super::{Diff, engine_side};

pub(in crate::review) use diffs_rs::engine::InsetArea as BoxArea;

/// Rows kept for one box: under `line` of `side`, or under the file's heading (`None`); in
/// a split diff over the column of `over`'s side, or across both sides (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::review) struct Slot<'a> {
    pub(in crate::review) id: u64,
    pub(in crate::review) path: &'a str,
    pub(in crate::review) line: Option<(Side, u32)>,
    pub(in crate::review) over: Option<Side>,
    pub(in crate::review) height: u16,
}

impl Diff {
    /// Keep rows for `slots`, in their order where several share a row: in a split diff,
    /// side-by-side when they are over one side each. Slots in files the viewer does not have
    /// (not arrived, filtered out) keep none until the next call. True when the set changed;
    /// the same set again changes nothing.
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
                Some(Inset::new(review, slot.id, file, anchor, slot.height).with_span(span(slot.over)))
            })
            .collect();
        if self.engine.insets() == insets.as_slice() {
            return Ok(false);
        }
        self.engine.set_insets(&insets)?;
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

    /// Whether box `id` keeps rows in the diff now: set, and its line shown.
    pub(in crate::review) fn keeps_box(&self, id: u64) -> bool {
        self.engine.insets().iter().any(|inset| inset.id == id) && !self.engine.hidden_insets().contains(&id)
    }

    /// The width a box in the file at `path` has over `over`'s side (across the diff for
    /// `None`), in a frame drawn into `area` with the settings as they are now. `None` for a
    /// file the viewer does not have and an area too small for the diff.
    pub(in crate::review) fn box_width(&self, area: Rect, path: &str, over: Option<Side>) -> Option<u16> {
        self.engine.inset_width_in(area, self.index(path)?, span(over))
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

fn span(over: Option<Side>) -> InsetSpan {
    over.map_or(InsetSpan::Full, |side| InsetSpan::Side(engine_side(side)))
}

#[cfg(test)]
impl Diff {
    /// The viewer's boxes as set, for tests.
    pub(in crate::review) fn boxes_set(&self) -> Vec<Inset> {
        self.engine.insets().to_vec()
    }
}
