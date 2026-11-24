//! Which lines a hidden-whitespace diff does not show: the lines of a hunk whose every change
//! is whitespace only, which the viewer draws as its heading alone. The verdict is the
//! viewer's own ([`diffs_rs::engine::Engine::hunk_is_whitespace_only`]), for its current
//! setting, so the app can tell before it uses a selection or a note's lines whether they are
//! still on screen.

use plannotator_tui_review::Side;

use super::{Diff, Lines};

impl Diff {
    /// Whether the viewer, with the whitespace setting it has now, draws the hunk that holds
    /// `lines.first` of `lines.path` on `lines.side` as its heading alone. False while
    /// whitespace is shown, and before the viewer has the file.
    pub(in crate::review) fn hides(&self, lines: &Lines) -> bool {
        let Some(file) = self.paths.iter().position(|path| *path == lines.path) else { return false };
        self.hunk_of(file, lines.side == Side::Old, lines.first)
            .is_some_and(|(hunk, _)| self.engine.hunk_is_whitespace_only(file, hunk))
    }
}
