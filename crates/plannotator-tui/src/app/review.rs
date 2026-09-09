//! Finish a file/folder review, undo it, or restore individual archived annotations.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use plannotator_tui_schema::Annotation;

use super::feedback::ReviewCounts;
use super::{App, Focus, Mode};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(super) struct ArchivedBatch {
    path: PathBuf,
    ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ArchivedItem {
    pub(super) path: PathBuf,
    pub(super) annotation: Annotation,
}

impl App {
    pub(super) fn finish_review(&mut self) {
        if !self.is_file_review() {
            return;
        }
        let mut batches = Vec::new();
        let mut errors = Vec::new();
        let mut count = 0;
        for path in self.review_files() {
            match self.archive_review_file(&path) {
                Ok(ids) if !ids.is_empty() => {
                    count += ids.len();
                    batches.push(ArchivedBatch { path, ids });
                }
                Ok(_) => {}
                Err(err) => errors.push(format!("{}: {err:#}", self.review_file_name(&path))),
            }
        }
        if !batches.is_empty() {
            self.undo_archive = batches;
        }
        self.after_review_change();
        let status = if count == 0 && errors.is_empty() {
            "no sent annotations to archive".into()
        } else {
            format!("archived {count} annotation(s)")
        };
        self.status = Some(with_errors(status, "archive", &errors));
    }

    fn archive_review_file(&mut self, path: &Path) -> Result<Vec<String>> {
        if self.is_open(path) {
            return self.open.store.archive_sent();
        }
        let (_, mut store) = self.load_review_file(path)?;
        let ids = store.archive_sent()?;
        self.folder_counts.insert(path.to_path_buf(), ReviewCounts::for_store(&store));
        Ok(ids)
    }

    pub(super) fn undo_finish_review(&mut self) {
        if self.undo_archive.is_empty() {
            self.status = Some("nothing to undo".into());
            return;
        }
        let mut count = 0;
        let mut errors = Vec::new();
        for batch in std::mem::take(&mut self.undo_archive) {
            match self.restore_review_file(&batch.path, &batch.ids) {
                Ok(restored) => count += restored,
                Err(err) => {
                    errors.push(format!("{}: {err:#}", self.review_file_name(&batch.path)));
                    self.undo_archive.push(batch);
                }
            }
        }
        self.after_review_change();
        self.status = Some(with_errors(format!("restored {count} annotation(s)"), "restore", &errors));
    }

    fn restore_review_file(&mut self, path: &Path, ids: &[String]) -> Result<usize> {
        if self.is_open(path) {
            return self.open.store.restore_archived(&self.open.doc, ids);
        }
        let (doc, mut store) = self.load_review_file(path)?;
        let restored = store.restore_archived(&doc, ids)?;
        self.folder_counts.insert(path.to_path_buf(), ReviewCounts::for_store(&store));
        Ok(restored)
    }

    fn after_review_change(&mut self) {
        let remaining = self.open.store.placed().len();
        self.rail_cursor = self.rail_cursor.min(remaining.saturating_sub(1));
        if remaining == 0 && self.focus == Focus::Rail {
            self.focus = Focus::Document;
        }
        self.clear_selection();
        self.mark_unsent();
        self.sync_tree_counts();
    }

    pub(super) fn open_archive(&mut self) {
        if !self.is_file_review() {
            return;
        }
        let errors = self.refresh_archive_items();
        if !errors.is_empty() {
            self.status = Some(with_errors(String::new(), "read archive", &errors));
        }
        self.archive_cursor = 0;
        self.mode = Mode::Archive;
    }

    fn refresh_archive_items(&mut self) -> Vec<String> {
        let mut items = Vec::new();
        let mut errors = Vec::new();
        for path in self.review_files() {
            let annotations = if self.is_open(&path) {
                self.open.store.archived().to_vec()
            } else {
                match self.load_review_file(&path) {
                    Ok((_, store)) => store.archived().to_vec(),
                    Err(err) => {
                        errors.push(format!("{}: {err:#}", self.review_file_name(&path)));
                        continue;
                    }
                }
            };
            items.extend(
                annotations
                    .into_iter()
                    .rev()
                    .map(|annotation| ArchivedItem { path: path.clone(), annotation }),
            );
        }
        self.archive_items = items;
        self.archive_cursor = self.archive_cursor.min(self.archive_items.len().saturating_sub(1));
        errors
    }

    pub(super) fn restore_selected_archived(&mut self) {
        let Some(item) = self.archive_items.get(self.archive_cursor).cloned() else { return };
        match self.restore_review_file(&item.path, std::slice::from_ref(&item.annotation.id)) {
            Ok(0) => {
                self.status = Some("could not restore: an annotation with this id is already active".into());
            }
            Ok(_) => {
                for batch in &mut self.undo_archive {
                    if batch.path == item.path {
                        batch.ids.retain(|id| id != &item.annotation.id);
                    }
                }
                self.undo_archive.retain(|batch| !batch.ids.is_empty());
                self.status = Some(format!("restored annotation in {}", self.review_file_name(&item.path)));
            }
            Err(err) => {
                self.status =
                    Some(format!("could not restore {}: {err:#}", self.review_file_name(&item.path)));
            }
        }
        self.after_review_change();
        let errors = self.refresh_archive_items();
        if !errors.is_empty() {
            self.status = Some(with_errors(self.status.take().unwrap_or_default(), "read archive", &errors));
        }
        if self.archive_items.is_empty() {
            self.mode = Mode::Browse;
        }
    }
}

fn with_errors(mut status: String, action: &str, errors: &[String]) -> String {
    if !errors.is_empty() {
        if !status.is_empty() {
            status.push_str("; ");
        }
        let _ = write!(status, "could not {action}: {}", errors.join("; "));
    }
    status
}
