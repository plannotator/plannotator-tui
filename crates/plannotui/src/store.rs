//! Annotations for one document: the sidecar file and the resolved view of it.
//!
//! The sidecar holds `plannotui_schema::Annotation` values — the same shape the Workspaces
//! API returns — so a local file and a server row are interchangeable. Resolution against
//! the current source happens on load and on reload; an annotation whose text is gone is
//! kept as an orphan.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use plannotui_schema::{Anchor, Annotation, Kind, Resolution, State, resolve};
use serde::{Deserialize, Serialize};

use crate::doc::Document;

#[derive(Debug)]
pub(crate) struct Store {
    path: Option<PathBuf>,
    annotations: Vec<Annotation>,
    /// Parallel to `annotations`.
    resolved: Vec<Resolution>,
}

#[derive(Default, Serialize, Deserialize)]
struct Sidecar {
    annotations: Vec<Annotation>,
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

impl Store {
    pub(crate) fn sidecar_path(doc_path: &Path) -> PathBuf {
        let mut name = doc_path.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
        name.push(".annotations.json");
        doc_path.with_file_name(name)
    }

    /// Load the sidecar next to `doc_path`, or an empty store if there is none.
    pub(crate) fn load(doc_path: &Path, doc: &Document) -> Result<Self> {
        let path = Self::sidecar_path(doc_path);
        let annotations = match std::fs::read_to_string(&path) {
            Ok(json) => {
                serde_json::from_str::<Sidecar>(&json)
                    .with_context(|| format!("parsing {}", path.display()))?
                    .annotations
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut store = Self { path: Some(path), annotations, resolved: Vec::new() };
        store.resolve_all(doc);
        Ok(store)
    }

    /// A store that never touches disk, for transient documents.
    pub(crate) fn transient() -> Self {
        Self { path: None, annotations: Vec::new(), resolved: Vec::new() }
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_string_pretty(&Sidecar { annotations: self.annotations.clone() })?;
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
        let source_range = plannotui_schema::SourceRange {
            start: range.start,
            end: range.end,
            version: plannotui_schema::blob_sha(doc.source.as_bytes()),
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
        let before = self.annotations.len();
        let keep: Vec<bool> = self
            .resolved
            .iter()
            .map(|r| match r {
                Resolution::Range(range) => doc.block_containing(range.start) != Some(block),
                Resolution::Orphan => true,
            })
            .collect();
        let mut keep_iter = keep.iter();
        self.annotations.retain(|_| keep_iter.next().copied().unwrap_or(true));
        let mut keep_iter = keep.iter();
        self.resolved.retain(|_| keep_iter.next().copied().unwrap_or(true));
        self.save()?;
        Ok(before - self.annotations.len())
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

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_694), (2026, 8, 29));
    }

    #[test]
    fn sidecar_round_trips_and_re_resolves_after_an_edit() {
        let dir = std::env::temp_dir().join(format!("plannotui-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let doc_path = dir.join("doc.md");
        let source = "# A\n\nfirst\n\nsecond thing\n".to_owned();
        let doc = Document::parse(source.clone());
        let mut store = Store::load(&doc_path, &doc).expect("empty store");
        let start = source.find("second").expect("present");
        store.add(&doc, start..start + 6, "second".into(), Kind::LooksGood, String::new()).expect("saved");

        let edited = Document::parse("# A\n\ninserted\n\nfirst\n\nsecond thing\n".to_owned());
        let reloaded = Store::load(&doc_path, &edited).expect("reloads");
        let placed = reloaded.placed();
        let expected = edited.source.find("second").expect("present");
        assert_eq!(placed.len(), 1);
        assert_eq!(placed.first().map(|p| p.range.clone()), Some(expected..expected + 6));
        assert_eq!(placed.first().map(Placed::kind), Some(Kind::LooksGood));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
