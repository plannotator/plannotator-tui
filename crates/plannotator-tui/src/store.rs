//! Annotations for one document, saved automatically on every change.
//!
//! The record is `annotations.json` under the Plannotator data directory, keyed the way
//! Plannotator keys files (`plannotator_tui_schema::annotations_dir`). It holds
//! `plannotator_tui_schema::Annotation` values — the Workspaces wire shape — so a local record
//! and a server row are interchangeable. Resolution against the current source happens on
//! load; an annotation whose text is gone is kept as an orphan.
//!
//! A phase-2 sidecar (`<file>.annotations.json` next to the document) is imported once and
//! left alone; nothing is written next to the document any more.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use plannotator_tui_schema::{Anchor, Annotation, Kind, Resolution, State, resolve};
use serde::{Deserialize, Serialize};

use crate::doc::Document;

#[derive(Debug)]
pub(crate) struct Store {
    /// `None` for transient documents: nothing is ever written.
    path: Option<PathBuf>,
    /// The document this store's record belongs to (absolute), for the record's `path` field.
    document: Option<PathBuf>,
    annotations: Vec<Annotation>,
    /// Parallel to `annotations`.
    resolved: Vec<Resolution>,
    deliveries: Vec<Delivered>,
}

#[derive(Default, Serialize, Deserialize)]
struct Record {
    /// The absolute document path this record belongs to. Written since 0.5.0 so folder
    /// sends can enumerate annotated files without walking the tree; absent in older
    /// records, which are then only found through listed tree rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    annotations: Vec<Annotation>,
    /// Every send, newest last. Lets the UI say "sent" across restarts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    deliveries: Vec<Delivered>,
}

/// One send of the feedback: when, where, and which annotations it covered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Delivered {
    pub(crate) at: String,
    pub(crate) target: String,
    pub(crate) annotation_ids: Vec<String>,
}

/// A resolved annotation: the record plus where it currently sits in the source.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placed<'a> {
    pub(crate) annotation: &'a Annotation,
    pub(crate) range: &'a Range<usize>,
}

impl Placed<'_> {
    pub(crate) fn kind(&self) -> Kind {
        self.annotation.anchor.kind()
    }
}

/// Where a document's annotations live, and its legacy sidecar if one exists.
#[derive(Debug, Clone)]
pub(crate) struct Location {
    pub(crate) record: PathBuf,
    pub(crate) legacy_sidecar: Option<PathBuf>,
    /// The document the record is for; written into the record so folder sends can find
    /// annotated files without walking the tree.
    pub(crate) document: Option<PathBuf>,
}

impl Location {
    /// `<data-dir>/clients/plannotator-tui/annotations/<project>/<slug>/annotations.json`, plus
    /// the phase-2 sidecar path when a file exists there.
    pub(crate) fn for_file(data_dir: &Path, project: &str, doc_path: &Path) -> Self {
        let resolved = doc_path.to_string_lossy();
        let record =
            plannotator_tui_schema::annotations_dir(data_dir, project, &resolved).join("annotations.json");
        let mut name = doc_path.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
        name.push(".annotations.json");
        let sidecar = doc_path.with_file_name(name);
        Self {
            record,
            legacy_sidecar: sidecar.is_file().then_some(sidecar),
            document: Some(doc_path.to_path_buf()),
        }
    }
}

fn read_record(path: &Path) -> Result<Option<Record>> {
    match std::fs::read_to_string(path) {
        Ok(json) => {
            let record: Record =
                serde_json::from_str(&json).with_context(|| format!("parsing {}", path.display()))?;
            Ok(Some(record))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

impl Store {
    /// Load the record at `location`; import the legacy sidecar when the record is absent.
    pub(crate) fn load(location: &Location, doc: &Document) -> Result<Self> {
        let (record, imported) = match read_record(&location.record)? {
            Some(found) => (found, false),
            None => match &location.legacy_sidecar {
                Some(sidecar) => (read_record(sidecar)?.unwrap_or_default(), true),
                None => (Record::default(), false),
            },
        };
        let mut store = Self {
            path: Some(location.record.clone()),
            document: location.document.clone(),
            annotations: record.annotations,
            resolved: Vec::new(),
            deliveries: record.deliveries,
        };
        store.resolve_all(doc);
        if imported && !store.annotations.is_empty() {
            store.save()?; // the import is now the record; the sidecar is left alone
        }
        Ok(store)
    }

    /// A store that never touches disk, for transient documents.
    pub(crate) fn transient() -> Self {
        Self {
            path: None,
            document: None,
            annotations: Vec::new(),
            resolved: Vec::new(),
            deliveries: Vec::new(),
        }
    }

    /// True when nothing about this store ever reaches disk.
    #[cfg(test)]
    pub(crate) fn is_transient(&self) -> bool {
        self.path.is_none()
    }

    /// Every annotated document recorded for `project`, from the records that carry their
    /// path (written since 0.5.0). Older records surface through listed tree rows instead.
    pub(crate) fn annotated_documents(data_dir: &Path, project: &str) -> Vec<PathBuf> {
        let dir = plannotator_tui_schema::annotations_dir(data_dir, project, "x");
        let Some(project_dir) = dir.parent() else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(project_dir) else { return Vec::new() };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path().join("annotations.json"))
            .filter_map(|record| read_record(&record).ok().flatten())
            .filter(|record| !record.annotations.is_empty())
            .filter_map(|record| record.path)
            .collect();
        found.sort();
        found.dedup();
        found
    }

    /// Count annotations recorded for a file without loading a document.
    pub(crate) fn count_at(location: &Location) -> usize {
        read_record(&location.record)
            .ok()
            .flatten()
            .or_else(|| location.legacy_sidecar.as_deref().and_then(|p| read_record(p).ok().flatten()))
            .map_or(0, |r| r.annotations.len())
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let record = Record {
            path: self.document.clone(),
            annotations: self.annotations.clone(),
            deliveries: self.deliveries.clone(),
        };
        let json = serde_json::to_string_pretty(&record)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Annotate `range` of the source. `rendered` is the selection's rendered text.
    pub(crate) fn add(
        &mut self,
        doc: &Document,
        range: Range<usize>,
        rendered: String,
        kind: Kind,
        body: String,
    ) -> Result<()> {
        let block = doc.block_containing(range.start);
        let source_range = plannotator_tui_schema::SourceRange {
            start: range.start,
            end: range.end,
            version: plannotator_tui_schema::blob_sha(doc.source.as_bytes()),
        };
        let anchor = Anchor::new(rendered, &doc.source, source_range, kind, block);
        let now = timestamp();
        self.annotations.push(Annotation {
            id: local_id(),
            document_id: String::new(),
            anchor,
            body,
            author: None,
            author_name: None,
            state: State::Open,
            attachments: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            replies: Vec::new(),
            other: std::collections::BTreeMap::default(),
        });
        self.resolved.push(Resolution::Range(range));
        self.save()
    }

    /// Remove every annotation resolved into `block`. Returns how many were removed.
    pub(crate) fn remove_in_block(&mut self, doc: &Document, block: usize) -> Result<usize> {
        let ids: Vec<String> = self
            .annotations
            .iter()
            .zip(&self.resolved)
            .filter(|(_, r)| matches!(r, Resolution::Range(range) if doc.block_containing(range.start) == Some(block)))
            .map(|(a, _)| a.id.clone())
            .collect();
        for id in &ids {
            self.remove_unsaved(id);
        }
        self.save()?;
        Ok(ids.len())
    }

    pub(crate) fn remove(&mut self, id: &str) -> Result<bool> {
        let removed = self.remove_unsaved(id);
        self.save()?;
        Ok(removed)
    }

    fn remove_unsaved(&mut self, id: &str) -> bool {
        let Some(index) = self.annotations.iter().position(|a| a.id == id) else { return false };
        self.annotations.remove(index);
        self.resolved.remove(index);
        true
    }

    /// Replace the body of annotation `id`.
    pub(crate) fn edit_body(&mut self, id: &str, body: String) -> Result<bool> {
        let Some(annotation) = self.annotations.iter_mut().find(|a| a.id == id) else { return Ok(false) };
        annotation.body = body;
        annotation.updated_at = timestamp();
        self.save()?;
        Ok(true)
    }

    pub(crate) fn resolve_all(&mut self, doc: &Document) {
        self.resolved = self
            .annotations
            .iter()
            .map(|a| resolve(&a.anchor, &doc.source, |o| doc.block_containing(o)))
            .collect();
    }

    /// Every resolved annotation, in source order.
    pub(crate) fn placed(&self) -> Vec<Placed<'_>> {
        let mut out: Vec<Placed<'_>> = self
            .annotations
            .iter()
            .zip(&self.resolved)
            .filter_map(|(annotation, r)| match r {
                Resolution::Range(range) => Some(Placed { annotation, range }),
                Resolution::Orphan => None,
            })
            .collect();
        out.sort_by_key(|p| p.range.start);
        out
    }

    pub(crate) fn len(&self) -> usize {
        self.annotations.len()
    }

    /// Remember that everything currently recorded was sent to `target`.
    pub(crate) fn record_delivery(&mut self, target: &str) -> Result<()> {
        self.deliveries.push(Delivered {
            at: timestamp(),
            target: target.to_owned(),
            annotation_ids: self.annotations.iter().map(|a| a.id.clone()).collect(),
        });
        self.save()
    }

    /// True when the annotations on record are exactly the set of the last send.
    pub(crate) fn all_delivered(&self) -> bool {
        let Some(last) = self.deliveries.last() else { return false };
        if self.annotations.is_empty() {
            return false;
        }
        let mut sent: Vec<&str> = last.annotation_ids.iter().map(String::as_str).collect();
        let mut have: Vec<&str> = self.annotations.iter().map(|a| a.id.as_str()).collect();
        sent.sort_unstable();
        have.sort_unstable();
        sent == have
    }

    pub(crate) fn orphans(&self) -> usize {
        self.resolved.iter().filter(|r| **r == Resolution::Orphan).count()
    }
}

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // RFC 3339 without pulling in a date crate: the API accepts and returns this form.
    let (year, month, day) = civil_from_days(secs / 86_400);
    let (hour, minute, second) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's days-to-civil, for a dependency-free UTC date.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A local id in the server's style. Replaced by the server id once synced.
fn local_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("local_{nanos:x}")
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_694), (2026, 8, 29));
    }

    #[test]
    fn saves_on_every_change_and_re_resolves_after_an_edit() {
        let root = temp_root("save");
        let data_dir = root.join("data");
        let doc_path = root.join("doc.md");
        let location = Location::for_file(&data_dir, "proj", &doc_path);
        let source = "# A\n\nfirst\n\nsecond thing\n".to_owned();
        let doc = Document::parse(source.clone());
        let mut store = Store::load(&location, &doc).expect("empty store");
        let start = source.find("second").expect("present");
        store.add(&doc, start..start + 6, "second".into(), Kind::LooksGood, String::new()).expect("saved");
        assert!(location.record.is_file(), "record written under the data dir on add");
        assert!(location.record.starts_with(data_dir.join("clients/plannotator-tui/annotations/proj")));
        assert!(!doc_path.with_file_name("doc.md.annotations.json").exists(), "no sidecar");

        let edited = Document::parse("# A\n\ninserted\n\nfirst\n\nsecond thing\n".to_owned());
        let reloaded = Store::load(&location, &edited).expect("reloads");
        let placed = reloaded.placed();
        let expected = edited.source.find("second").expect("present");
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].range.clone(), expected..expected + 6);
        assert_eq!(placed[0].kind(), Kind::LooksGood);
        assert_eq!(Store::count_at(&location), 1);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn legacy_sidecar_is_imported_once_and_left_alone() {
        let root = temp_root("import");
        let doc_path = root.join("doc.md");
        std::fs::write(&doc_path, "hello world\n").expect("doc");
        let sidecar = root.join("doc.md.annotations.json");
        let doc = Document::parse("hello world\n".to_owned());
        let mut seed = Store {
            document: None,
            path: Some(sidecar.clone()),
            annotations: Vec::new(),
            resolved: Vec::new(),
            deliveries: Vec::new(),
        };
        seed.add(&doc, 0..5, "hello".into(), Kind::Comment, "hi".into()).expect("seed sidecar");
        let before = std::fs::read_to_string(&sidecar).expect("sidecar");

        let location = Location::for_file(&root.join("data"), "proj", &doc_path);
        let store = Store::load(&location, &doc).expect("imports");
        assert_eq!(store.len(), 1);
        assert!(location.record.is_file(), "imported into the data dir");
        assert_eq!(std::fs::read_to_string(&sidecar).expect("sidecar"), before, "sidecar untouched");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_record_without_deliveries_still_loads() {
        let root = temp_root("old-shape");
        let doc_path = root.join("doc.md");
        let location = Location::for_file(&root.join("data"), "proj", &doc_path);
        std::fs::create_dir_all(location.record.parent().expect("parent")).expect("dir");
        std::fs::write(&location.record, r#"{"annotations":[]}"#).expect("old record");
        let store = Store::load(&location, &Document::parse("x\n".to_owned())).expect("loads");
        assert!(!store.all_delivered());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn all_delivered_tracks_the_last_send_across_reloads() {
        let root = temp_root("deliver");
        let doc_path = root.join("doc.md");
        let location = Location::for_file(&root.join("data"), "proj", &doc_path);
        let doc = Document::parse("one two three\n".to_owned());
        let mut store = Store::load(&location, &doc).expect("empty");
        assert!(!store.all_delivered(), "nothing to send yet");
        store.add(&doc, 0..3, "one".into(), Kind::Comment, "a".into()).expect("add");
        assert!(!store.all_delivered());
        store.record_delivery("claude in w1:p1").expect("record");
        assert!(store.all_delivered());

        let mut reloaded = Store::load(&location, &doc).expect("reload");
        assert!(reloaded.all_delivered(), "the send survives a restart");
        reloaded.add(&doc, 4..7, "two".into(), Kind::LooksGood, String::new()).expect("add");
        assert!(!reloaded.all_delivered(), "a new annotation is unsent");
        let newest = reloaded.placed().last().map(|p| p.annotation.id.clone()).expect("two");
        reloaded.remove(&newest).expect("remove");
        assert!(reloaded.all_delivered(), "back to exactly the delivered set");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
