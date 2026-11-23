//! Viewed marks: `x` marks the file under the cursor viewed (or not), `ctrl+n` goes to the
//! next file not marked, and the header counts them (`7/12 viewed`). Marks are saved as the
//! browser app's review-progress records (`plannotator_tui_review::progress`,
//! `docs/spec-review-progress.md`), so a review continues in either app; a mark stops
//! counting once its file changes.
//!
//! The file list (`files`) ticks viewed files and dims their rows, and can hide them; the
//! footer also ticks the file under the cursor.

use std::path::Path;

use anyhow::Result;
use plannotator_tui_review::progress;

use super::Screen;

/// Whether marks are saved: `PLANNOTATOR_REVIEW_PROGRESS`, else `reviewProgress` in the data
/// dir's `config.json`, else yes, as in the browser.
pub(super) fn saving(data_dir: &Path) -> bool {
    let env = std::env::var("PLANNOTATOR_REVIEW_PROGRESS").ok();
    let config = std::fs::read_to_string(data_dir.join("config.json")).ok();
    progress::enabled(env.as_deref(), config.as_deref())
}

impl Screen {
    /// `3/12 viewed`, for the header.
    pub(super) fn viewed_label(&self) -> String {
        let total = self.capture.files.len();
        let viewed = self.progress.count(self.capture.files.iter().map(|f| f.path.as_str()));
        format!("{viewed}/{total} viewed")
    }

    /// Whether the file under the cursor is marked viewed, for the footer's tick.
    pub(super) fn cursor_file_viewed(&self) -> bool {
        self.file_under_cursor().is_some_and(|p| self.progress.is_viewed(&p))
    }

    /// The file under the selection or the cursor, on every kind of row, as the viewer says
    /// (`diff::place`).
    pub(super) fn file_under_cursor(&self) -> Option<String> {
        if let Some(lines) = self.diff.selection() {
            return Some(lines.path);
        }
        self.diff.cursor_file().and_then(|i| self.diff.paths().get(i).cloned())
    }

    /// `x`: flip the file's mark and save it.
    pub(super) fn toggle_viewed(&mut self) {
        let Some(path) = self.file_under_cursor() else {
            self.status = Some("move the cursor onto a line of the file first".into());
            return;
        };
        self.toggle_viewed_of(&path);
    }

    /// Flip `path`'s mark and save it; the whole-file view's `x` too.
    pub(super) fn toggle_viewed_of(&mut self, path: &str) {
        let viewed = !self.progress.is_viewed(path);
        let saved = self.progress.set(path, viewed);
        let count = self.viewed_label();
        self.status = Some(match (saved, viewed) {
            (Err(err), _) => format!("{path}: for this session only, the mark was not saved: {err}"),
            (Ok(()), true) if !self.progress.is_saved(path) => {
                format!("\u{2713} {path} viewed for this session \u{b7} {count}")
            }
            (Ok(()), true) => format!("\u{2713} {path} viewed \u{b7} {count}"),
            (Ok(()), false) => format!("{path} not viewed \u{b7} {count}"),
        });
        if let Err(err) = self.after_viewed_change(path) {
            self.status = Some(format!("{err:#}"));
        }
    }

    /// `ctrl+n` (and `previous-unviewed`): the next file in diff order that is not marked
    /// viewed, wrapping around; its heading comes to the top. With a filter on, only the
    /// files it shows count.
    pub(super) fn jump_unviewed(&mut self, forward: bool) -> Result<()> {
        let paths = self.diff.paths().to_vec();
        if paths.is_empty() {
            return Ok(());
        }
        // By the viewer's index: a symlink replaced by a file is two files of one path.
        let here = match self.diff.selection() {
            Some(lines) => paths.iter().position(|q| *q == lines.path),
            None => self.diff.cursor_file(),
        };
        let count = paths.len();
        let start = here.unwrap_or(if forward { count - 1 } else { 0 });
        let target = (1..=count)
            .map(|step| if forward { (start + step) % count } else { (start + count - step % count) % count })
            .find(|&i| paths.get(i).is_some_and(|p| !self.progress.is_viewed(p)));
        let Some(index) = target.filter(|&i| Some(i) != here) else {
            let all = paths.iter().all(|p| self.progress.is_viewed(p));
            self.status = Some(if all { "every file is viewed" } else { "no other file to view" }.into());
            return Ok(());
        };
        let path = paths.get(index).cloned().unwrap_or_default();
        self.goto_file(&path)?;
        let left = paths.iter().filter(|p| !self.progress.is_viewed(p)).count();
        self.status = Some(format!("{path} \u{b7} {left} not viewed"));
        Ok(())
    }
}
