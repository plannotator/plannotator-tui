//! `layout = "auto"`: split or unified, decided from the terminal's width, never from the
//! diff pane's. Opening the notes, closing the file list or dragging a pane's edge then never
//! flips the diff under the reader; resizing the terminal decides again.
//!
//! The viewer's own auto splits a pane of 120 columns or more. A review opens without notes
//! and, with more than one file, with the file list on the left at 20 columns or more and a
//! one-column line beside it. So auto splits a terminal that leaves the diff 120 columns
//! beside the narrowest list: 141 columns, or 120 for a one-file review, which has no list.
//! Only a pane too narrow for the viewer's two least sides (each 20 columns, the line between
//! them) is drawn unified whatever the terminal is.

use diffs_rs::engine::SplitRatio;

use crate::config::review::{Layout, ReviewConfig};

use super::super::Screen;
use super::super::files::fit;
use super::Settings;

/// The diff's width from which the viewer's own auto splits.
const VIEWER_AUTO_WIDTH: u16 = 120;
/// The narrowest pane that holds both sides at their least and the line between them.
const PANE_MIN_WIDTH: u16 = 2 * SplitRatio::MIN_SIDE_WIDTH + 1;

/// Whether `auto` splits in a terminal `terminal` columns wide, for a review of more than one
/// file (`listed`), its diff pane `pane` columns wide.
pub(in crate::review) fn splits(terminal: u16, listed: bool, pane: u16) -> bool {
    let list = if listed { fit::MIN_WIDTH + 1 } else { 0 };
    terminal >= VIEWER_AUTO_WIDTH + list && pane >= PANE_MIN_WIDTH
}

impl Settings {
    /// The settings for the viewer: `auto` as this screen decided it, once it has drawn a
    /// frame; before that the viewer's own auto, which no frame draws with.
    pub(in crate::review) fn for_viewer(&self) -> ReviewConfig {
        let mut settings = self.now;
        if settings.layout == Layout::Auto
            && let Some(split) = self.auto_split
        {
            settings.layout = if split { Layout::Split } else { Layout::Unified };
        }
        settings
    }
}

impl Screen {
    /// Decide `auto` for a frame `terminal` columns wide whose diff pane is `pane` wide, and
    /// bring the viewers to it when that changed.
    pub(in crate::review) fn decide_auto(&mut self, terminal: u16, pane: u16) -> anyhow::Result<()> {
        let split = splits(terminal, self.capture.files.len() > 1, pane);
        if self.settings.auto_split == Some(split) {
            return Ok(());
        }
        self.settings.auto_split = Some(split);
        self.apply_settings()
    }
}

#[cfg(test)]
mod tests {
    use super::{PANE_MIN_WIDTH, splits};

    #[test]
    fn auto_splits_from_141_columns_or_from_120_with_one_file() {
        assert!(!splits(140, true, 119) && splits(141, true, 120));
        assert!(!splits(119, false, 119) && splits(120, false, 120));
        assert!(splits(160, true, 30 + PANE_MIN_WIDTH), "the pane's width is not the terminal's");
        assert!(!splits(200, true, PANE_MIN_WIDTH - 1), "a pane too narrow for both sides");
    }
}
