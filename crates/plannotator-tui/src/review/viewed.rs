//! Viewed marks: `x` marks the file under the cursor viewed (or not), `ctrl+n` goes to the
//! next file not marked, and the header counts them (`7/12 viewed`). Marks are saved as the
//! browser app's review-progress records (`plannotator_tui_review::progress`,
//! `docs/spec-review-progress.md`), so a review continues in either app; a mark stops
//! counting once its file changes.
//!
//! The file list is drawn by the diff viewer, which has no way to decorate a file yet, so the
//! tick shows in the footer for the file under the cursor.

use std::path::Path;

use anyhow::Result;
use diffs_rs::engine::Command;
use plannotator_tui_review::{Side, progress};

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

    /// Whether the file under the cursor is marked viewed, for the footer's tick. Reads the
    /// cursor without moving it, so on a heading it says nothing.
    pub(super) fn cursor_file_viewed(&self) -> bool {
        let path = match self.diff.selection() {
            Some(lines) => Some(lines.path),
            None => self.diff.cursor().ok().flatten().map(|lines| lines.path),
        };
        path.is_some_and(|p| self.progress.is_viewed(&p))
    }

    /// The file under the selection or the cursor. On a file or hunk heading, which the
    /// viewer does not tie to a file, the cursor steps down to the first line below it.
    pub(super) fn file_under_cursor(&mut self) -> Result<Option<String>> {
        let mut path = match self.diff.selection() {
            Some(lines) => Some(lines.path),
            None => self.diff.cursor()?.map(|lines| lines.path),
        };
        for _ in 0..3 {
            if path.is_some() {
                break;
            }
            self.diff.run(Command::Down)?;
            path = self.diff.cursor()?.map(|lines| lines.path);
        }
        Ok(path)
    }

    /// `x`: flip the file's mark and save it.
    pub(super) fn toggle_viewed(&mut self) -> Result<()> {
        let Some(path) = self.file_under_cursor()? else {
            self.status = Some("move the cursor onto a line of the file first".into());
            return Ok(());
        };
        let viewed = !self.progress.is_viewed(&path);
        let saved = self.progress.set(&path, viewed);
        let count = self.viewed_label();
        self.status = Some(match (saved, viewed) {
            (Err(err), _) => format!("{path}: for this session only, the mark was not saved: {err}"),
            (Ok(()), true) if !self.progress.is_saved(&path) => {
                format!("\u{2713} {path} viewed for this session \u{b7} {count}")
            }
            (Ok(()), true) => format!("\u{2713} {path} viewed \u{b7} {count}"),
            (Ok(()), false) => format!("{path} not viewed \u{b7} {count}"),
        });
        Ok(())
    }

    /// `ctrl+n` (and `previous-unviewed`): the next file in diff order that is not marked
    /// viewed, wrapping around; the cursor lands on its first line.
    pub(super) fn jump_unviewed(&mut self, forward: bool) -> Result<()> {
        let paths = self.diff.paths().to_vec();
        if paths.is_empty() {
            return Ok(());
        }
        let here = self.file_under_cursor()?.and_then(|p| paths.iter().position(|q| *q == p));
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
        self.show_file(&path, index)?;
        let left = paths.iter().filter(|p| !self.progress.is_viewed(p)).count();
        self.status = Some(format!("{path} \u{b7} {left} not viewed"));
        Ok(())
    }

    /// Put the cursor on the first line `path` shows; a file with none (binary, mode only)
    /// gets its heading, by walking file by file from the first.
    fn show_file(&mut self, path: &str, index: usize) -> Result<()> {
        let first = self.capture.file(path).and_then(|file| {
            let line = |side| file.lines(side).next().map(|(n, _)| (side, n));
            line(Side::New).or_else(|| line(Side::Old))
        });
        if let Some((side, line)) = first
            && self.diff.reveal(path, side, line).is_ok()
        {
            return Ok(());
        }
        for _ in 0..self.diff.paths().len() {
            self.diff.run(Command::PreviousFile)?;
        }
        for _ in 0..index {
            self.diff.run(Command::NextFile)?;
        }
        Ok(())
    }
}
