//! The changed files, on the left of the diff (`f`): a folder tree whose width fits its
//! names, and a filter by name (`/`), by kind (`t`) and by viewed mark (`H`).
//!
//! While a filter is on, the diff shows only the files it lets through, in the list's
//! order, so `n`/`N`, `]`/`[` and `ctrl+n` move among them. Notes are never filtered: the
//! rail keeps them all, Send sends them all, and the list says how many unsent ones sit in
//! hidden files.
//!
//! - [`kind`]: what kind of file a path is. [`filter`]: what passes.
//! - [`tree`]: the rows. [`fit`]: the width, and cutting long names.
//! - `draw` and `input`: the list on screen and its keys; `kinds`: the kind picker.

mod draw;
pub(super) mod filter;
pub(super) mod fit;
mod input;
pub(super) mod kind;
mod kinds;
pub(super) mod tree;

use std::collections::BTreeSet;

use anyhow::Result;
use diffs_rs::engine::Command;
use plannotator_tui_review::{NoteScope, Side};

use super::Screen;
use filter::Filter;
use kind::Kind;

/// The file list's state.
#[derive(Debug, Default)]
pub(super) struct FileList {
    /// Closed with `toggle-files`; `toggle-files` opens it again.
    pub(super) closed: bool,
    /// The filter, as typed so far.
    pub(super) filter: Filter,
    /// The paths the diff shows, in the list's order: the filter as last applied.
    pub(super) shown: Vec<String>,
    /// Each capture file's kind, in capture order.
    kinds: Vec<Kind>,
    /// Folders closed in the list, by row key.
    collapsed: BTreeSet<String>,
    /// The row under the list's cursor, by key.
    pub(super) cursor: Option<String>,
    /// First list row drawn.
    scroll: usize,
    /// The cursor the list last scrolled to show.
    scrolled_to: Option<String>,
    /// The file the diff's cursor is in.
    pub(super) current: Option<String>,
    /// The width chosen with `narrow-column` and `widen-column`.
    chosen_width: Option<u16>,
    /// The width drawn last frame; 0 when hidden.
    pub(super) drawn_width: u16,
    /// A file to show once the diff, replaced for a new filter, has it.
    pending: Option<String>,
}

impl FileList {
    pub(super) fn chosen_width(&self) -> Option<u16> {
        self.chosen_width
    }

    /// The list was drawn `width` wide (0: hidden). A chosen width the room could not give
    /// comes down to what it gave, so the next `narrow-column` narrows what is seen.
    pub(super) fn drawn(&mut self, width: u16) {
        self.drawn_width = width;
        if width > 0 {
            self.chosen_width = self.chosen_width.map(|chosen| chosen.min(width));
        }
    }

    /// After the diff was replaced for a new capture: come back to the file it was on.
    pub(super) fn retarget(&mut self) {
        self.pending = self.current.clone().filter(|p| self.shown.contains(p));
    }
}

impl Screen {
    /// Classify the capture's files and show them all, in the list's order. Call it when
    /// the capture is new; the filter stays as it was.
    pub(super) fn files_for_capture(&mut self) {
        self.files.kinds = self.capture.files.iter().map(|f| kind::classify(&f.path)).collect();
        self.files.shown = self.filtered(&self.files.filter.clone());
        if self.files.current.as_ref().is_none_or(|c| !self.files.shown.contains(c)) {
            self.files.current = self.files.shown.first().cloned();
        }
    }

    /// The capture's paths that pass `filter`, in the list's order.
    fn filtered(&self, filter: &Filter) -> Vec<String> {
        let paths: Vec<&str> = self.capture.files.iter().map(|f| f.path.as_str()).collect();
        let keep = filter.keep(&paths, &self.files.kinds, |p| self.progress.is_viewed(p));
        tree::order(&paths)
            .into_iter()
            .filter(|&i| keep.get(i).copied().unwrap_or(false))
            .filter_map(|i| paths.get(i).map(|p| (*p).to_owned()))
            .collect()
    }

    /// The paths the list shows now: the filter as typed, which the diff takes on `Enter`.
    pub(super) fn listed(&self) -> Vec<String> {
        self.filtered(&self.files.filter)
    }

    /// Show in the diff what the filter lets through, then `show` (or the file the diff was
    /// on, if it still passes). An unchanged set only moves to `show`.
    pub(super) fn apply_filter(&mut self, show: Option<String>) -> Result<()> {
        let shown = self.listed();
        let target = show.or_else(|| self.files.current.clone()).filter(|p| shown.contains(p));
        if shown == self.files.shown {
            if let Some(path) = target {
                self.goto_file(&path)?;
            }
            return Ok(());
        }
        self.diff.replace(&self.capture, &shown)?;
        self.files.shown = shown;
        self.files.current = target.clone().or_else(|| self.files.shown.first().cloned());
        self.files.pending = target;
        Ok(())
    }

    /// After the diff took more files: go to the file a new filter was waiting to show.
    pub(super) fn show_pending_file(&mut self) -> Result<()> {
        let Some(path) = self.files.pending.clone() else { return Ok(()) };
        if self.diff.paths().contains(&path) {
            self.files.pending = None;
            self.goto_file(&path)?;
        } else if self.diff.files_shown() >= self.files.shown.len() {
            self.files.pending = None;
        }
        Ok(())
    }

    /// Show `path` in the diff, its heading at the top and the cursor on it. A file the
    /// filter hides is not in the diff; the footer says so.
    pub(super) fn goto_file(&mut self, path: &str) -> Result<()> {
        let Some(index) = self.diff.paths().iter().position(|p| p == path) else {
            self.status = Some(format!("{path} is hidden by the file filter"));
            return Ok(());
        };
        self.files.current = Some(path.to_owned());
        if self.diff.selection().is_some() {
            self.diff.clear_selection()?;
        }
        if index == 0 {
            self.diff.run(Command::FirstSource)?;
            for _ in 0..2 {
                self.diff.run(Command::Up)?;
            }
            return Ok(());
        }
        // From a line of the file, the previous file's heading and back puts this file's
        // heading at the top; a file with no lines (binary, mode only) is reached by
        // walking from the first heading.
        let first = self.capture.file(path).and_then(|file| {
            let line = |side| file.lines(side).next().map(|(n, _)| (side, n));
            line(Side::New).or_else(|| line(Side::Old))
        });
        if let Some((side, line)) = first
            && self.diff.reveal(path, side, line).is_ok()
        {
            self.diff.run(Command::PreviousFile)?;
            return self.diff.run(Command::NextFile);
        }
        self.diff.run(Command::FirstSource)?;
        for _ in 0..index {
            self.diff.run(Command::NextFile)?;
        }
        Ok(())
    }

    /// `n`/`N`: the next or previous file the diff shows.
    pub(super) fn step_file(&mut self, forward: bool) -> Result<()> {
        let paths = self.diff.paths().to_vec();
        let here = self.files.current.as_ref().and_then(|c| paths.iter().position(|p| p == c));
        let next = match here {
            Some(i) if forward => i.checked_add(1).filter(|&n| n < paths.len()),
            Some(i) => i.checked_sub(1),
            None => Some(0),
        };
        if let Some(path) = next.and_then(|n| paths.get(n)).cloned() {
            return self.goto_file(&path);
        }
        self.status = Some(if forward { "last file" } else { "first file" }.into());
        Ok(())
    }

    /// Note which file the diff's cursor is in, after it moved. On a heading or between
    /// hunks the cursor names no file: the first line below it does, so the cursor looks
    /// there and comes back. A growing selection is left alone.
    pub(super) fn track_current(&mut self) -> Result<()> {
        if let Some(lines) = self.diff.cursor()? {
            self.files.current = Some(lines.path);
            return Ok(());
        }
        if self.diff.selecting() {
            return Ok(());
        }
        let mut moved = 0;
        let mut found = None;
        while moved < 3 && found.is_none() {
            self.diff.run(Command::Down)?;
            moved += 1;
            found = self.diff.cursor()?.map(|lines| lines.path);
        }
        for _ in 0..moved {
            self.diff.run(Command::Up)?;
        }
        if found.is_some() {
            self.files.current = found;
        }
        Ok(())
    }

    /// Unsent notes on files the filter hides, so the list can say they are there.
    pub(super) fn notes_in_hidden_files(&self) -> usize {
        self.store
            .pending()
            .iter()
            .filter(|note| note.scope != NoteScope::General && !self.files.shown.contains(&note.file_path))
            .count()
    }

    /// `hide-viewed`: hide the viewed files, or show them again.
    pub(super) fn toggle_hide_viewed(&mut self) -> Result<()> {
        self.files.filter.hide_viewed = !self.files.filter.hide_viewed;
        self.apply_filter(None)?;
        let viewed = self.capture.files.iter().filter(|f| self.progress.is_viewed(&f.path)).count();
        self.status = Some(if self.files.filter.hide_viewed {
            format!("hiding {viewed} viewed file{}", if viewed == 1 { "" } else { "s" })
        } else {
            "showing viewed files".to_owned()
        });
        Ok(())
    }

    /// After `path` was marked viewed or not: with viewed files hidden, it leaves the list
    /// and the diff goes on to the next file.
    pub(super) fn after_viewed_change(&mut self, path: &str) -> Result<()> {
        if !self.files.filter.hide_viewed {
            return Ok(());
        }
        let at = self.files.shown.iter().position(|p| p == path);
        let next = at.and_then(|i| {
            self.files
                .shown
                .iter()
                .skip(i + 1)
                .chain(self.files.shown.iter().take(i))
                .find(|p| *p != path)
                .cloned()
        });
        self.apply_filter(next)
    }

    /// Whether the list wants to show: not closed, and more than one file or a filter.
    pub(super) fn files_wanted(&self) -> bool {
        !self.files.closed
            && (self.capture.files.len() > 1
                || self.focus == super::Focus::Files
                || self.files.filter.is_active())
    }

    /// `Files · 340`, or `Files · 12 of 340` while a filter hides some.
    pub(super) fn files_heading(&self, listed: usize) -> String {
        let total = self.capture.files.len();
        if listed == total {
            format!("Files \u{b7} {total}")
        } else {
            format!("Files \u{b7} {listed} of {total}")
        }
    }
}
