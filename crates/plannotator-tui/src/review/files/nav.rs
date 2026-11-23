//! Moving the diff from file to file: to a file picked in the list, or the next or the
//! previous one with `n`/`N`, and knowing which file the diff's cursor is in.

use anyhow::Result;

use crate::review::Screen;

impl Screen {
    /// After the diff took more files and drew them (it lays rows out as it draws, and
    /// the cursor moves over laid-out rows): go to the file a new filter was waiting to
    /// show. True when it went.
    pub(in crate::review) fn show_pending_file(&mut self) -> Result<bool> {
        let Some(path) = self.files.pending.clone() else { return Ok(false) };
        if !self.diff.drawn() {
            return Ok(false);
        }
        if self.diff.paths().contains(&path) {
            self.files.pending = None;
            self.goto_file(&path)?;
            return Ok(true);
        }
        if self.diff.files_shown() >= self.files.shown.len() {
            self.files.pending = None;
        }
        Ok(false)
    }

    /// Show `path` in the diff, its heading at the top and the cursor on it. A file the
    /// filter hides is not in the diff; the footer says so.
    pub(in crate::review) fn goto_file(&mut self, path: &str) -> Result<()> {
        let Some(index) = self.diff.paths().iter().position(|p| p == path) else {
            self.status = Some(format!("{path} is hidden by the file filter"));
            return Ok(());
        };
        self.goto_index(index)
    }

    /// Show the diff's file `index`, its heading at the top and the cursor on it. By index,
    /// as two files can share a path (a symlink deleted and a file added in its place).
    pub(in crate::review) fn goto_index(&mut self, index: usize) -> Result<()> {
        let Some(path) = self.diff.paths().get(index).cloned() else { return Ok(()) };
        if self.diff.selection().is_some() {
            self.diff.clear_selection()?;
        }
        self.diff.show_heading(index)?;
        self.files.current = Some(path);
        Ok(())
    }

    /// `n`/`N`: the next or previous file the diff shows, from the file its cursor is in
    /// whatever moved it there.
    pub(in crate::review) fn step_file(&mut self, forward: bool) -> Result<()> {
        if self.diff.selection().is_some() {
            self.diff.clear_selection()?;
        }
        if !self.diff.step_file(forward)? {
            self.status = Some(if forward { "last file" } else { "first file" }.into());
        }
        Ok(())
    }

    /// Note which file the diff's cursor is in, for the list and the footer. The viewer's
    /// cursor is the one place that knows; this copies its answer by path after every event
    /// and before every frame. Not while a file the list asked for waits to arrive.
    pub(in crate::review) fn track_current(&mut self) {
        if self.whole.is_some() || self.files.pending.is_some() {
            return;
        }
        if let Some(index) = self.diff.cursor_file() {
            self.files.current = self.diff.paths().get(index).cloned();
        }
    }
}
