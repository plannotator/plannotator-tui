//! Comments and their anchors.
//!
//! An anchor is quote-primary: the selected source text plus a little context on either
//! side. The block index is only a hint. On load, each comment is re-resolved against the
//! current document; a comment whose quote cannot be found is kept but marked orphaned.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::doc::Document;

const CONTEXT_CHARS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    pub block_hint: usize,
}

/// What the annotation says about its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Comment,
    Approve,
    Delete,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Comment => "comment",
            Kind::Approve => "looks good",
            Kind::Delete => "delete this",
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            Kind::Comment => "💬",
            Kind::Approve => "👍",
            Kind::Delete => "✗",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    #[serde(default)]
    pub kind: Kind,
    pub anchor: Anchor,
    pub body: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Absolute source byte range inside `block`.
    Range { block: usize, range: Range<usize> },
    Orphan,
}

impl Resolution {
    pub fn block(&self) -> Option<usize> {
        match self {
            Resolution::Range { block, .. } => Some(*block),
            Resolution::Orphan => None,
        }
    }
}

pub struct Store {
    path: PathBuf,
    pub comments: Vec<Comment>,
    /// Parallel to `comments`.
    pub resolved: Vec<Resolution>,
}

#[derive(Default, Serialize, Deserialize)]
struct FileFormat {
    comments: Vec<Comment>,
}

impl Store {
    pub fn sidecar_path(doc_path: &Path) -> PathBuf {
        let mut name = doc_path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".annotations.json");
        doc_path.with_file_name(name)
    }

    pub fn load(doc_path: &Path, doc: &Document) -> Result<Self> {
        let path = Self::sidecar_path(doc_path);
        let comments = match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str::<FileFormat>(&json)
                .with_context(|| format!("parsing {}", path.display()))?
                .comments,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut store = Self { path, comments, resolved: Vec::new() };
        store.resolve_all(doc);
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&FileFormat { comments: self.comments.clone() })?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Annotate an absolute source range that starts inside `block`.
    pub fn add(&mut self, doc: &Document, block: usize, range: Range<usize>, kind: Kind, body: String) -> Result<()> {
        let anchor = make_anchor(doc, block, &range);
        let id = self.comments.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let created_at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.comments.push(Comment { id, kind, anchor, body, created_at });
        self.resolved.push(Resolution::Range { block, range });
        self.save()
    }

    pub fn remove_on_block(&mut self, block: usize) -> Result<usize> {
        let before = self.comments.len();
        let keep: Vec<bool> = self.resolved.iter().map(|r| r.block() != Some(block)).collect();
        let mut i = 0;
        self.comments.retain(|_| {
            let k = keep[i];
            i += 1;
            k
        });
        self.resolved.retain(|r| r.block() != Some(block));
        self.save()?;
        Ok(before - self.comments.len())
    }

    pub fn resolve_all(&mut self, doc: &Document) {
        self.resolved = self.comments.iter().map(|c| resolve(doc, &c.anchor)).collect();
    }

    /// Comments resolved into `block`, with their ranges, in source order.
    pub fn on_block(&self, block: usize) -> Vec<(&Comment, Range<usize>)> {
        let mut out: Vec<_> = self
            .comments
            .iter()
            .zip(&self.resolved)
            .filter_map(|(c, r)| match r {
                Resolution::Range { block: b, range } if *b == block => Some((c, range.clone())),
                _ => None,
            })
            .collect();
        out.sort_by_key(|(_, r)| r.start);
        out
    }

    /// Every resolved range with its kind, for painting.
    pub fn ranges(&self) -> impl Iterator<Item = (Range<usize>, Kind)> + '_ {
        self.comments.iter().zip(&self.resolved).filter_map(|(c, r)| match r {
            Resolution::Range { range, .. } => Some((range.clone(), c.kind)),
            Resolution::Orphan => None,
        })
    }

    pub fn orphans(&self) -> usize {
        self.resolved.iter().filter(|r| **r == Resolution::Orphan).count()
    }
}

fn make_anchor(doc: &Document, block: usize, range: &Range<usize>) -> Anchor {
    let src = &doc.source;
    let prefix_start = src[..range.start].char_indices().rev().nth(CONTEXT_CHARS - 1).map(|(i, _)| i).unwrap_or(0);
    let suffix_end = src[range.end..].char_indices().nth(CONTEXT_CHARS).map(|(i, _)| range.end + i).unwrap_or(src.len());
    Anchor {
        quote: src[range.clone()].to_string(),
        prefix: src[prefix_start..range.start].to_string(),
        suffix: src[range.end..suffix_end].to_string(),
        block_hint: block,
    }
}

/// Find the quote in the source: choose among occurrences by surrounding context, then
/// by proximity to the hinted block. Whitespace-insensitive fallback, else orphan.
fn resolve(doc: &Document, anchor: &Anchor) -> Resolution {
    if anchor.quote.trim().is_empty() {
        return Resolution::Orphan;
    }
    if let Some(r) = best_occurrence(doc, anchor, &anchor.quote) {
        return r;
    }
    let trimmed = anchor.quote.trim();
    if trimmed != anchor.quote {
        if let Some(r) = best_occurrence(doc, anchor, trimmed) {
            return r;
        }
    }
    Resolution::Orphan
}

fn find_all(text: &str, needle: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = text[from..].find(needle) {
        let start = from + pos;
        out.push(start..start + needle.len());
        from = start + needle.len().max(1);
    }
    out
}

/// The block whose range contains `offset`, if any.
pub fn block_containing(doc: &Document, offset: usize) -> Option<usize> {
    doc.blocks.iter().position(|b| b.range.start <= offset && offset < b.range.end)
}

fn best_occurrence(doc: &Document, anchor: &Anchor, needle: &str) -> Option<Resolution> {
    let src = &doc.source;
    let mut best: Option<((u8, usize), usize, Range<usize>)> = None;
    for range in find_all(src, needle) {
        let Some(block) = block_containing(doc, range.start) else { continue };
        let score = src[..range.start].ends_with(&anchor.prefix) as u8 + src[range.end..].starts_with(&anchor.suffix) as u8;
        let distance = (block as i64 - anchor.block_hint as i64).unsigned_abs() as usize;
        let key = (score, usize::MAX - distance);
        if best.as_ref().is_none_or(|(k, _, _)| key > *k) {
            best = Some((key, block, range));
        }
    }
    best.map(|(_, block, range)| Resolution::Range { block, range })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range_of(doc: &Document, needle: &str) -> Range<usize> {
        let start = doc.source.find(needle).unwrap();
        start..start + needle.len()
    }

    #[test]
    fn resolves_after_block_moves() {
        let doc = Document::parse("# A\n\nfirst\n\nsecond thing\n".into());
        let anchor = make_anchor(&doc, 2, &range_of(&doc, "second"));
        let edited = Document::parse("# A\n\ninserted\n\nfirst\n\nsecond thing\n".into());
        let r = resolve(&edited, &anchor);
        assert_eq!(r, Resolution::Range { block: 3, range: range_of(&edited, "second") });
    }

    #[test]
    fn orphans_when_text_is_gone() {
        let doc = Document::parse("one\n\ntwo\n".into());
        let anchor = make_anchor(&doc, 1, &range_of(&doc, "two"));
        let edited = Document::parse("one\n\nthree\n".into());
        assert_eq!(resolve(&edited, &anchor), Resolution::Orphan);
    }

    #[test]
    fn quotes_may_span_blocks() {
        let doc = Document::parse("# Title\n\nfirst para\n\nsecond para\n".into());
        let anchor = make_anchor(&doc, 1, &range_of(&doc, "para\n\nsecond"));
        let edited = Document::parse("intro\n\n# Title\n\nfirst para\n\nsecond para\n".into());
        assert_eq!(resolve(&edited, &anchor), Resolution::Range { block: 2, range: range_of(&edited, "para\n\nsecond") });
    }

    #[test]
    fn duplicate_quotes_disambiguate_by_context() {
        let doc = Document::parse("alpha same\n\nbeta same\n".into());
        let second = doc.source.rfind("same").unwrap();
        let anchor = make_anchor(&doc, 1, &(second..second + 4));
        let edited = Document::parse("same\n\nalpha same\n\nbeta same\n".into());
        let expected = edited.source.rfind("same").unwrap();
        assert_eq!(resolve(&edited, &anchor), Resolution::Range { block: 2, range: expected..expected + 4 });
    }
}
