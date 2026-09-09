//! Select feedback once: the body, count, history and delivery ids all cover that set.
//! Folder counts are cached on open/change/reload, never read from disk while drawing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plannotator_tui_schema::{Kind, Provenance};

use super::App;
use crate::archive::AnnotationRecord;
use crate::doc::Document;
use crate::export;
use crate::store::{Location, Store};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SendScope {
    Pending,
    All,
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct ReviewCounts {
    pub(super) pending: usize,
    pub(super) sent: usize,
    pub(super) archived: usize,
}

impl ReviewCounts {
    pub(super) fn for_store(store: &Store) -> Self {
        let placed = store.placed();
        let pending = placed.iter().filter(|p| store.is_pending(p.annotation)).count();
        Self { pending, sent: placed.len() - pending, archived: store.archived().len() }
    }
}

#[derive(Debug)]
pub(super) struct FeedbackPart {
    pub(super) path: Option<PathBuf>,
    pub(super) store: Store,
    pub(super) ids: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct Feedback {
    pub(super) text: String,
    pub(super) count: usize,
    pub(super) parts: Vec<FeedbackPart>,
    pub(super) annotations: Vec<AnnotationRecord>,
    pub(super) counts: HashMap<PathBuf, ReviewCounts>,
}

impl Feedback {
    fn add(&mut self, path: Option<PathBuf>, name: &str, doc: &Document, store: Store, scope: SendScope) {
        if let Some(path) = &path {
            self.counts.insert(path.clone(), ReviewCounts::for_store(&store));
        }
        let entries: Vec<export::Entry<'_>> = store
            .placed()
            .into_iter()
            .filter(|p| scope == SendScope::All || store.is_pending(p.annotation))
            .map(|p| export::Entry {
                annotation: p.annotation,
                lines: export::line_span(&doc.source, p.range),
                range: p.range.clone(),
            })
            .collect();
        if entries.is_empty() {
            return;
        }
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(&export::feedback(&doc.source, name, &entries));
        let ids = entries.iter().map(|e| e.annotation.id.clone()).collect();
        self.count += entries.len();
        self.annotations.extend(entries.iter().map(|entry| {
            let a = entry.annotation;
            AnnotationRecord {
                id: Some(a.id.clone()),
                kind: Some(
                    match a.anchor.kind() {
                        Kind::Comment => "comment",
                        Kind::LooksGood => "looks-good",
                        Kind::Delete => "delete",
                    }
                    .to_owned(),
                ),
                text: (!a.body.is_empty()).then(|| a.body.clone()),
                original_text: (!a.anchor.original_text.is_empty()).then(|| a.anchor.original_text.clone()),
            }
        }));
        self.parts.push(FeedbackPart { path, store, ids });
    }

    fn exported_text(self) -> String {
        if self.text.is_empty() { "No annotations.".to_owned() } else { self.text }
    }
}

impl App {
    pub(super) fn is_file_review(&self) -> bool {
        self.tree.is_some()
            || (!self.open.source.transient && matches!(self.open.source.provenance, Provenance::File { .. }))
    }

    pub(super) fn review_counts(&self) -> ReviewCounts {
        if self.tree.is_none() {
            return ReviewCounts::for_store(&self.open.store);
        }
        self.folder_counts.values().fold(ReviewCounts::default(), |mut sum, count| {
            sum.pending += count.pending;
            sum.sent += count.sent;
            sum.archived += count.archived;
            sum
        })
    }

    pub(super) fn pending_file_count(&self) -> usize {
        self.folder_counts.values().filter(|c| c.pending > 0).count()
    }

    /// Both active and archive-only records belong to the review. Restrict project-wide
    /// records to this folder; a sibling in the same git project must not be sent.
    pub(super) fn review_files(&self) -> Vec<PathBuf> {
        let Some(tree) = &self.tree else {
            return match &self.open.source.provenance {
                Provenance::File { path } => vec![path.clone()],
                _ => Vec::new(),
            };
        };
        let mut paths = Store::annotated_documents(&self.data_dir, &self.project);
        paths.extend(tree.rows.iter().filter(|r| !r.is_dir && r.annotations > 0).map(|r| r.path.clone()));
        if let Provenance::File { path } = &self.open.source.provenance {
            paths.push(path.clone());
        }
        paths.sort();
        paths.dedup();
        paths.retain(|p| p.starts_with(tree.root()));
        paths
    }

    pub(super) fn review_file_name(&self, path: &Path) -> String {
        if let Some(tree) = &self.tree {
            path.strip_prefix(tree.root()).unwrap_or(path).display().to_string()
        } else {
            path.file_name()
                .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
        }
    }

    pub(super) fn is_open(&self, path: &Path) -> bool {
        matches!(&self.open.source.provenance, Provenance::File { path: p } if p == path)
    }

    /// Other files need source and anchor resolution, not a rendered document layout.
    pub(super) fn load_review_file(&self, path: &Path) -> Result<(Document, Store)> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            // A deleted file's finished notes remain recoverable. Restoring them makes
            // them orphans until their source is available again.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        let doc = Document::parse(content);
        let store = Store::load(&Location::for_file(&self.data_dir, &self.project, path), &doc)?;
        Ok((doc, store))
    }

    /// Count what can be read. A file whose document or record cannot be loaded (permission
    /// denied, a corrupt record) is left out and remembered; it never stops the folder from
    /// opening, reloading or expanding. The status names the skipped files once, when the
    /// set changes; sending still reports such a file as an error.
    pub(super) fn refresh_review_counts(&mut self) {
        if self.tree.is_none() {
            return;
        }
        let mut counts = HashMap::new();
        let mut unreadable = Vec::new();
        for path in self.review_files() {
            let count = if self.is_open(&path) && path.is_file() {
                ReviewCounts::for_store(&self.open.store)
            } else if let Ok((_, store)) = self.load_review_file(&path) {
                ReviewCounts::for_store(&store)
            } else {
                unreadable.push(path);
                continue;
            };
            counts.insert(path, count);
        }
        self.folder_counts = counts;
        if unreadable != self.unreadable_files {
            self.unreadable_files = unreadable;
            if let Some(note) = self.unreadable_note() {
                self.status = Some(note);
            }
        }
    }

    /// One line naming the files the counts had to skip.
    pub(super) fn unreadable_note(&self) -> Option<String> {
        if self.unreadable_files.is_empty() {
            return None;
        }
        let names: Vec<String> =
            self.unreadable_files.iter().take(3).map(|path| self.review_file_name(path)).collect();
        let more = if self.unreadable_files.len() > names.len() { ", ..." } else { "" };
        Some(format!(
            "skipped {} unreadable file(s): {}{more}",
            self.unreadable_files.len(),
            names.join(", ")
        ))
    }

    pub(super) fn update_open_review_counts(&mut self) {
        if self.tree.is_some()
            && let Provenance::File { path } = &self.open.source.provenance
        {
            let mut counts = ReviewCounts::for_store(&self.open.store);
            if !path.is_file() {
                counts.pending = 0;
                counts.sent = 0;
            }
            self.folder_counts.insert(path.clone(), counts);
        }
    }

    fn file_feedback(&self, scope: SendScope) -> Feedback {
        let path = match &self.open.source.provenance {
            Provenance::File { path } => Some(path.clone()),
            _ => None,
        };
        let mut feedback = Feedback::default();
        feedback.add(path, &self.open.source.name, &self.open.doc, self.open.store.clone(), scope);
        feedback
    }

    pub(super) fn prepare_feedback(&self, scope: SendScope) -> Result<Feedback> {
        let Some(tree) = &self.tree else { return Ok(self.file_feedback(scope)) };
        let mut feedback = Feedback::default();
        for path in self.review_files() {
            if !path.is_file() {
                let (_, store) = self.load_review_file(&path)?;
                feedback.counts.insert(path, ReviewCounts::for_store(&store));
                continue;
            }
            let name = path.strip_prefix(tree.root()).unwrap_or(&path).display().to_string();
            if self.is_open(&path) {
                feedback.add(Some(path), &name, &self.open.doc, self.open.store.clone(), scope);
            } else {
                let (doc, store) = self.load_review_file(&path)?;
                feedback.add(Some(path), &name, &doc, store, scope);
            }
        }
        // Folder feedback has always ended each file's block with one extra newline, so
        // the body and `--export <folder>` keep the shape earlier releases produced.
        if !feedback.text.is_empty() {
            feedback.text.push('\n');
        }
        Ok(feedback)
    }

    /// Headless export retains the full active review and does not record a delivery.
    pub(crate) fn feedback(&self) -> String {
        self.file_feedback(SendScope::All).exported_text()
    }

    pub(crate) fn folder_feedback(&self) -> Result<String> {
        Ok(self.prepare_feedback(SendScope::All)?.exported_text())
    }
}
