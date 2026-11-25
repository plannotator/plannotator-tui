//! A review's notes, saved on every change.
//!
//! One record per repository root and scope:
//! `<data-dir>/clients/plannotator-tui/reviews/<project>/<scope>-<8hex>/review.json`, where
//! `<8hex>` is `sha256(root + "\0" + scope id)`, so each worktree has its own record. The
//! file holds `{ repo, scope, annotations, deliveries, archived }` and is replaced
//! atomically (write a temp file, then rename). Deliveries and archiving work as in a file
//! review's `annotations.json`: a note is pending until a delivery at or after its last edit
//! covers it, and finishing moves sent notes to `archived` whole.
//!
//! Every change is made on a copy and committed to memory only once it is saved, so a
//! failed write leaves the store as it was.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::annotation::{CodeAnnotation, now_millis};
use crate::error::{Error, Result};
use crate::resolve::{self, Resolution};
use crate::scope::Scope;

/// One send of the feedback: when, where, and which notes it covered. The same shape as a
/// file review's deliveries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivered {
    /// RFC 3339, UTC, milliseconds: `2026-11-15T09:30:00.000Z`.
    pub at: String,
    pub target: String,
    pub annotation_ids: Vec<String>,
}

/// The `review.json` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The repository root the record belongs to.
    pub repo: String,
    pub scope: Scope,
    pub annotations: Vec<CodeAnnotation>,
    #[serde(default)]
    pub deliveries: Vec<Delivered>,
    #[serde(default)]
    pub archived: Vec<CodeAnnotation>,
}

/// The notes of one review, bound to their file.
#[derive(Debug, Clone)]
pub struct ReviewStore {
    path: PathBuf,
    record: Record,
}

/// The directory holding the record for `scope` of the repository at `repo_root`.
/// `project` is Plannotator's project name ([`project_for`]).
pub fn review_dir(data_dir: &Path, project: &str, repo_root: &str, scope: &Scope) -> PathBuf {
    let id = scope.id();
    let hex = crate::sha256_hex(&[repo_root.as_bytes(), b"\0", id.as_bytes()], 8);
    data_dir
        .join("clients")
        .join("plannotator-tui")
        .join("reviews")
        .join(project)
        .join(format!("{}-{hex}", dir_safe(&id)))
}

/// Plannotator's project name for a repository: the root's basename, sanitized.
pub fn project_for(repo_root: &Path) -> String {
    plannotator_tui_schema::project_name(Some(repo_root), repo_root)
}

/// The scope id with anything but `[A-Za-z0-9._-]` as `-`, capped at 60 characters. The
/// hash beside it keeps two ids that clean to the same name apart.
fn dir_safe(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .take(60)
        .collect();
    cleaned.trim_matches('.').to_owned()
}

impl ReviewStore {
    /// Load the record for `scope` of `repo_root`, or start an empty one. Nothing is written
    /// until the first change.
    pub fn load(data_dir: &Path, project: &str, repo_root: &str, scope: &Scope) -> Result<Self> {
        let path = review_dir(data_dir, project, repo_root, scope).join("review.json");
        let record = match std::fs::read_to_string(&path) {
            Ok(json) => {
                serde_json::from_str(&json).map_err(|source| Error::Json { path: path.clone(), source })?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Record {
                repo: repo_root.to_owned(),
                scope: scope.clone(),
                annotations: Vec::new(),
                deliveries: Vec::new(),
                archived: Vec::new(),
            },
            Err(source) => return Err(Error::Io { action: "reading", path, source }),
        };
        Ok(Self { path, record })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(&self) -> &Record {
        &self.record
    }

    pub fn annotations(&self) -> &[CodeAnnotation] {
        &self.record.annotations
    }

    pub fn archived(&self) -> &[CodeAnnotation] {
        &self.record.archived
    }

    pub fn deliveries(&self) -> &[Delivered] {
        &self.record.deliveries
    }

    /// Add a note and save.
    pub fn add(&mut self, note: CodeAnnotation) -> Result<()> {
        self.commit(|record| {
            record.annotations.push(note);
            true
        })
    }

    /// Change one note with `edit` and save. When `edit` reports a change, `updatedAt`
    /// advances past both its old value and the note's last send, even if the clock has not,
    /// so an edited note is pending again. Returns whether anything changed.
    pub fn edit(&mut self, id: &str, edit: impl FnOnce(&mut CodeAnnotation) -> bool) -> Result<bool> {
        let floor = self.find(id).map(|note| {
            let sent = self.last_delivery(id).and_then(|d| parse_millis(&d.at));
            note.updated_at().max(sent.unwrap_or(0)) + 1
        });
        let Some(floor) = floor else { return Ok(false) };
        let mut changed = false;
        self.commit(|record| {
            let Some(note) = record.annotations.iter_mut().find(|n| n.id == id) else { return false };
            changed = edit(note);
            if changed {
                note.plannotator_tui.updated_at = Some(now_millis().max(floor));
            }
            changed
        })?;
        Ok(changed)
    }

    /// Remove a note. Returns whether it was there.
    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let mut removed = false;
        self.commit(|record| {
            let before = record.annotations.len();
            record.annotations.retain(|n| n.id != id);
            removed = record.annotations.len() != before;
            removed
        })?;
        Ok(removed)
    }

    /// Remove a note and hand it back with where it was, for [`Self::put_back`] to undo.
    /// Its send history stays, so a note put back is as sent as it was.
    pub fn take(&mut self, id: &str) -> Result<Option<(usize, CodeAnnotation)>> {
        let Some(index) = self.record.annotations.iter().position(|n| n.id == id) else { return Ok(None) };
        let mut taken = None;
        self.commit(|record| {
            taken = (index < record.annotations.len()).then(|| record.annotations.remove(index));
            taken.is_some()
        })?;
        Ok(taken.map(|note| (index, note)))
    }

    /// Put a note [`Self::take`] removed back where it was, or last when fewer notes are left.
    /// A note whose id is there again stays out. Returns whether it went back.
    pub fn put_back(&mut self, index: usize, note: CodeAnnotation) -> Result<bool> {
        if self.find(&note.id).is_some() {
            return Ok(false);
        }
        self.commit(|record| {
            let index = index.min(record.annotations.len());
            record.annotations.insert(index, note);
            true
        })?;
        Ok(true)
    }

    /// Apply resolutions (parallel to [`Self::annotations`], as
    /// [`crate::Capture::resolve_all`] returns them) and save when a note moved or changed
    /// state. Returns whether anything changed.
    pub fn apply(&mut self, resolutions: &[Resolution]) -> Result<bool> {
        let mut changed = false;
        self.commit(|record| {
            for (note, resolution) in record.annotations.iter_mut().zip(resolutions) {
                changed |= resolve::apply(note, *resolution);
            }
            changed
        })?;
        Ok(changed)
    }

    /// True until a delivery at or after the note's last edit covers it. A delivery whose
    /// time cannot be read covers nothing.
    pub fn is_pending(&self, note: &CodeAnnotation) -> bool {
        self.last_delivery(&note.id)
            .and_then(|d| parse_millis(&d.at))
            .is_none_or(|sent| note.updated_at() > sent)
    }

    /// The notes a send would carry: new or edited since their last send.
    pub fn pending(&self) -> Vec<&CodeAnnotation> {
        self.record.annotations.iter().filter(|n| self.is_pending(n)).collect()
    }

    pub fn all_delivered(&self) -> bool {
        !self.record.annotations.is_empty() && self.pending().is_empty()
    }

    /// Record a send of `ids` to `target`. Its time is never before the newest covered edit,
    /// so a clock step backwards cannot leave a sent note pending.
    pub fn record_delivery(&mut self, target: &str, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let newest =
            self.record.annotations.iter().filter(|n| ids.contains(&n.id)).map(CodeAnnotation::updated_at);
        let at = format_millis(newest.max().unwrap_or(0).max(now_millis()));
        let delivered = Delivered { at, target: target.to_owned(), annotation_ids: ids.to_vec() };
        self.commit(|record| {
            record.deliveries.push(delivered);
            true
        })
    }

    /// Finish every sent, unchanged note (outdated ones too): move it to `archived` whole.
    /// Pending notes stay. Returns the archived ids, which [`Self::restore_archived`] undoes.
    pub fn archive_sent(&mut self) -> Result<Vec<String>> {
        let ids: Vec<String> =
            self.record.annotations.iter().filter(|n| !self.is_pending(n)).map(|n| n.id.clone()).collect();
        if ids.is_empty() {
            return Ok(ids);
        }
        let selected: HashSet<&str> = ids.iter().map(String::as_str).collect();
        if let Some(clash) = self.record.archived.iter().find(|n| selected.contains(n.id.as_str())) {
            return Err(Error::ArchiveConflict { id: clash.id.clone() });
        }
        self.commit(|record| {
            let (done, active) = std::mem::take(&mut record.annotations)
                .into_iter()
                .partition::<Vec<_>, _>(|n| selected.contains(n.id.as_str()));
            record.annotations = active;
            record.archived.extend(done);
            true
        })?;
        Ok(ids)
    }

    /// Bring archived notes back, timestamps and send history untouched. A note whose id is
    /// active again stays archived. Returns how many came back.
    pub fn restore_archived(&mut self, ids: &[String]) -> Result<usize> {
        let mut restored = 0;
        self.commit(|record| {
            let active: HashSet<String> = record.annotations.iter().map(|n| n.id.clone()).collect();
            let (back, keep) = std::mem::take(&mut record.archived)
                .into_iter()
                .partition::<Vec<_>, _>(|n| ids.contains(&n.id) && !active.contains(&n.id));
            restored = back.len();
            record.archived = keep;
            record.annotations.extend(back);
            restored > 0
        })?;
        Ok(restored)
    }

    fn find(&self, id: &str) -> Option<&CodeAnnotation> {
        self.record.annotations.iter().find(|n| n.id == id)
    }

    fn last_delivery(&self, id: &str) -> Option<&Delivered> {
        self.record.deliveries.iter().rev().find(|d| d.annotation_ids.iter().any(|sent| sent == id))
    }

    /// Apply `change` to a copy; when it reports a change, save the copy and keep it.
    fn commit(&mut self, change: impl FnOnce(&mut Record) -> bool) -> Result<()> {
        let mut next = self.record.clone();
        if !change(&mut next) {
            return Ok(());
        }
        save(&self.path, &next)?;
        self.record = next;
        Ok(())
    }
}

fn io(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> Error + use<> {
    let path = path.to_path_buf();
    move |source| Error::Io { action, path, source }
}

fn save(path: &Path, record: &Record) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io("creating", dir))?;
    }
    let json = serde_json::to_string_pretty(record)
        .map_err(|source| Error::Json { path: path.to_path_buf(), source })?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(io("writing", &tmp))?;
    std::fs::rename(&tmp, path).map_err(io("replacing", path))
}

fn format_millis(ms: u64) -> String {
    let at = OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

/// Any RFC 3339 time, as Unix milliseconds rounded up, so a send recorded with finer
/// digits never sorts before the edit it covered.
fn parse_millis(value: &str) -> Option<u64> {
    let nanos = OffsetDateTime::parse(value, &Rfc3339).ok()?.unix_timestamp_nanos();
    u64::try_from((nanos + 999_999).div_euclid(1_000_000)).ok()
}

#[cfg(test)]
mod tests;
