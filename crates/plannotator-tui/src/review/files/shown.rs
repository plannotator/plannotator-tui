//! What the diff shows: the capture's files that pass the filter, in the list's order, and
//! their kinds, by name at once and by `.gitattributes` once git answers.

use std::collections::HashMap;

use anyhow::Result;
use plannotator_tui_review::NoteScope;

use super::filter::Filter;
use super::row::Change;
use super::{attrs, kind, tree};
use crate::review::Screen;

impl Screen {
    /// Classify the capture's files and show them all, in the list's order. Call it when
    /// the capture is new; the filter stays as it was.
    pub(in crate::review) fn files_for_capture(&mut self) {
        let paths: Vec<&str> = self.capture.files.iter().map(|f| f.path.as_str()).collect();
        self.files.kinds = paths.iter().map(|p| kind::classify(p)).collect();
        self.files.index = HashMap::new();
        for (i, path) in paths.iter().enumerate() {
            self.files.index.entry((*path).to_owned()).or_default().push(i);
        }
        self.files.order = tree::order(&paths);
        self.files.version += 1;
        let owned = paths.iter().map(|p| (*p).to_owned()).collect();
        let answer = attrs::start(self.repo.root(), owned, self.wake.clone());
        self.files.attributes = Some((self.files.version, answer));
        let shown = self.filtered(&self.files.filter.clone());
        self.set_shown(shown);
        if self.files.current.as_ref().is_none_or(|c| !self.files.shown_set.contains(c)) {
            self.files.current = self.files.shown.first().cloned();
        }
    }

    /// Git's `linguist-generated` answer, once it came: the kinds follow it, and so does
    /// the diff when the kinds picked hide or show a file it moved. True when it came.
    pub(in crate::review) fn take_attributes(&mut self) -> Result<bool> {
        let Some((version, receiver)) = &self.files.attributes else { return Ok(false) };
        let answer = match receiver.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(false),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => HashMap::new(),
        };
        let current = *version == self.files.version;
        self.files.attributes = None;
        if !current || answer.is_empty() {
            return Ok(false);
        }
        self.files.kinds = self
            .capture
            .files
            .iter()
            .map(|f| kind::classify_with(&f.path, answer.get(&f.path).copied()))
            .collect();
        self.files.version += 1;
        if !self.files.filter.all_kinds() {
            self.apply_filter(None)?;
        }
        Ok(true)
    }

    /// The diff shows `shown` now.
    fn set_shown(&mut self, shown: Vec<String>) {
        let files = &self.capture.files;
        self.files.shown_totals = shown
            .iter()
            .filter_map(|p| self.files.index.get(p))
            .flatten()
            .filter_map(|&i| files.get(i))
            .fold((0, 0), |(a, r), f| (a.saturating_add(f.additions), r.saturating_add(f.deletions)));
        self.files.shown_set = shown.iter().cloned().collect();
        self.files.shown = shown;
        self.files.shown_version += 1;
    }

    /// The change the capture makes to `path`, for its row.
    pub(super) fn change_of(&self, path: &str) -> Option<Change> {
        let indexes = self.files.index.get(path)?;
        Change::of(indexes.iter().filter_map(|&i| self.capture.files.get(i)))
    }

    /// The capture's paths that pass `filter`, in the list's order.
    fn filtered(&self, filter: &Filter) -> Vec<String> {
        let paths: Vec<&str> = self.capture.files.iter().map(|f| f.path.as_str()).collect();
        let keep = filter.keep(&paths, &self.files.kinds, |p| self.progress.is_viewed(p));
        self.files
            .order
            .iter()
            .filter(|&&i| keep.get(i).copied().unwrap_or(false))
            // A path the patch names twice is one row, at its first section.
            .filter(|&&i| {
                paths.get(i).and_then(|p| self.files.index.get(*p)).and_then(|v| v.first()) == Some(&i)
            })
            .filter_map(|&i| paths.get(i).map(|p| (*p).to_owned()))
            .collect()
    }

    /// The paths the list shows now: the filter as typed, which the diff takes on `Enter`.
    pub(in crate::review) fn listed(&self) -> Vec<String> {
        self.filtered(&self.files.filter)
    }

    /// Show in the diff what the filter lets through, then `show` (or the file the diff was
    /// on, if it still passes). An unchanged set only moves to `show`.
    pub(in crate::review) fn apply_filter(&mut self, show: Option<String>) -> Result<()> {
        let shown = self.listed();
        let target = show.clone().or_else(|| self.files.current.clone()).filter(|p| shown.contains(p));
        if shown == self.files.shown {
            // The diff already shows these files: only a file asked for moves it.
            if let Some(path) = show.filter(|p| self.files.is_shown(p)) {
                self.goto_file(&path)?;
            }
            return Ok(());
        }
        self.diff.replace(&self.capture, &shown)?;
        self.set_shown(shown);
        self.files.current = target.clone().or_else(|| self.files.shown.first().cloned());
        self.files.pending = target;
        Ok(())
    }

    /// Unsent notes on files the filter hides, so the list can say they are there.
    pub(in crate::review) fn notes_in_hidden_files(&self) -> usize {
        self.store
            .pending()
            .iter()
            .filter(|note| note.scope != NoteScope::General)
            // A file the capture has and the diff does not: one the filter hides. A note on
            // a file no longer changed, or on an unchanged file, is not hidden by it.
            .filter(|note| {
                self.files.index.contains_key(&note.file_path) && !self.files.is_shown(&note.file_path)
            })
            .count()
    }

    /// `hide-viewed`: hide the viewed files, or show them again.
    pub(in crate::review) fn toggle_hide_viewed(&mut self) -> Result<()> {
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
    pub(in crate::review) fn after_viewed_change(&mut self, path: &str) -> Result<()> {
        self.files.viewed_changed();
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
        // The list's cursor on the file that leaves moves on with the diff.
        if self.files.cursor.as_deref() == Some(path) {
            self.files.cursor.clone_from(&next);
        }
        self.apply_filter(next)
    }
}
