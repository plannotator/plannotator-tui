//! The anchor: where an annotation points.
//!
//! Two coordinate systems live in one object, deliberately separated:
//!
//! - `original_text` / `quote` are **rendered** text — what the Workspaces web client
//!   searches for (exact substring, first occurrence, whitespace-collapsed fallback).
//!   For a selection spanning blocks, the rendered block texts are joined with **no
//!   separator**, which is what the browser's own `Range.toString()` produces.
//! - everything under `plannotator_tui` is **raw source**: byte offsets into the markdown, the
//!   32-char context on either side, and the document version those offsets belong to.
//!
//! Foreign anchors (written by the web client, MCP tools, older plannotator-tui) must always
//! deserialize: every field except `original_text` is optional and unknown keys are kept.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Characters of raw-source context captured on each side of the quote.
pub const CONTEXT_CHARS: usize = 32;

/// What the annotation says about its text. A tag, never body text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Comment,
    LooksGood,
    Delete,
}

/// A byte range into the raw markdown, pinned to the document version it was made against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub start: usize,
    pub end: usize,
    /// Git blob sha of the raw bytes, as the API's `ETag`. See [`crate::blob_sha`].
    pub version: String,
}

/// The plannotator-tui-owned part of an anchor. Ignored by the web client, read only by us.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Extras {
    #[serde(default)]
    pub kind: Kind,
    /// The selected raw-source text. The truth the resolver searches for; context and the
    /// byte range only rank and shortcut. Kept separately from `original_text` because
    /// markup (`**`, backticks) is present here and stripped there.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRange>,
    /// Raw-source text immediately before the range, up to [`CONTEXT_CHARS`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    /// Raw-source text immediately after the range, up to [`CONTEXT_CHARS`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suffix: String,
    /// Index of the top-level block the range starts in. A hint, never authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<usize>,
}

/// An annotation's target, in the Workspaces wire shape.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Anchor {
    /// Rendered text of the selection. The web client's lookup key.
    #[serde(rename = "originalText", default, skip_serializing_if = "String::is_empty")]
    pub original_text: String,
    /// Same as `original_text`; the key MCP-style clients read.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plannotator_tui: Option<Extras>,
    /// Fields other clients wrote (`startMeta`, `htmlAnchor`, ...). Preserved, never read.
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

impl Anchor {
    /// Build an anchor for a selection. `rendered` is the rendered text the web client will
    /// search for; `source` is the raw markdown the byte range indexes into.
    pub fn new(
        rendered: impl Into<String>,
        source: &str,
        range: SourceRange,
        kind: Kind,
        block: Option<usize>,
    ) -> Self {
        let rendered = rendered.into();
        let (start, end) = clamp(source, range.start, range.end);
        let (prefix, suffix) = context(source, start, end);
        Self {
            quote: rendered.clone(),
            original_text: rendered,
            plannotator_tui: Some(Extras {
                kind,
                quote: source[start..end].to_owned(),
                source: Some(range),
                prefix,
                suffix,
                block,
            }),
            other: BTreeMap::new(),
        }
    }

    /// The text the web client searches for, whichever key a client wrote.
    pub fn rendered(&self) -> &str {
        if self.original_text.is_empty() { &self.quote } else { &self.original_text }
    }

    pub fn kind(&self) -> Kind {
        self.plannotator_tui.as_ref().map_or(Kind::Comment, |e| e.kind)
    }
}

/// Clamp a byte range into `source` onto char boundaries so slicing never panics.
fn clamp(source: &str, start: usize, end: usize) -> (usize, usize) {
    let start = floor_char_boundary(source, start.min(source.len()));
    let end = floor_char_boundary(source, end.min(source.len())).max(start);
    (start, end)
}

/// Up to [`CONTEXT_CHARS`] characters of `source` on each side of `start..end`,
/// which must already lie on char boundaries.
fn context(source: &str, start: usize, end: usize) -> (String, String) {
    let prefix_start = source[..start].char_indices().rev().nth(CONTEXT_CHARS - 1).map_or(0, |(i, _)| i);
    let suffix_end = source[end..].char_indices().nth(CONTEXT_CHARS).map_or(source.len(), |(i, _)| end + i);
    (source[prefix_start..start].to_owned(), source[end..suffix_end].to_owned())
}

/// Largest char boundary at or below `index`. Stable replacement for `str::floor_char_boundary`.
pub(crate) fn floor_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Join rendered block texts the way the web client's DOM does: no separator at all.
pub fn join_rendered<'a>(blocks: impl IntoIterator<Item = &'a str>) -> String {
    blocks.into_iter().collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn range(start: usize, end: usize) -> SourceRange {
        SourceRange { start, end, version: "v".into() }
    }

    #[test]
    fn serializes_to_the_wire_shape() {
        let source = "Ship the **login page** by Friday.";
        let anchor = Anchor::new("login page", source, range(11, 21), Kind::LooksGood, Some(0));
        let json = serde_json::to_value(&anchor).expect("serializable");
        assert_eq!(json["originalText"], "login page");
        assert_eq!(json["quote"], "login page");
        assert_eq!(json["plannotator_tui"]["kind"], "looks_good");
        assert_eq!(json["plannotator_tui"]["quote"], "login page");
        assert_eq!(json["plannotator_tui"]["source"]["start"], 11);
        assert_eq!(json["plannotator_tui"]["prefix"], "Ship the **");
        assert_eq!(json["plannotator_tui"]["suffix"], "** by Friday.");
        assert!(json.get("kind").is_none(), "kind must not leak to the top level");
    }

    #[test]
    fn foreign_anchors_round_trip_without_loss() {
        let web = serde_json::json!({
            "originalText": "the quick brown fox",
            "startMeta": {"parentTagName": "P", "parentIndex": 3, "textOffset": 12},
            "endMeta": {"parentTagName": "P", "parentIndex": 3, "textOffset": 31}
        });
        let anchor: Anchor = serde_json::from_value(web.clone()).expect("web anchor parses");
        assert_eq!(anchor.rendered(), "the quick brown fox");
        assert_eq!(anchor.kind(), Kind::Comment);
        assert_eq!(serde_json::to_value(&anchor).expect("serializable"), web);
    }

    #[test]
    fn mcp_quote_only_anchor_is_readable() {
        let anchor: Anchor =
            serde_json::from_value(serde_json::json!({"quote": "Overview"})).expect("parses");
        assert_eq!(anchor.rendered(), "Overview");
    }

    #[test]
    fn context_respects_char_boundaries() {
        let source = "日本語のテキストです。選択範囲。後ろの文脈。";
        let start = source.find("選択").expect("present");
        let end = start + "選択範囲".len();
        let (prefix, suffix) = context(source, start, end);
        assert_eq!(prefix, "日本語のテキストです。");
        assert_eq!(suffix, "。後ろの文脈。");
        // An offset inside a multi-byte char is clamped, not a panic.
        assert_eq!(clamp(source, start + 1, end + 1), (start, end));
    }

    #[test]
    fn rendered_blocks_join_with_no_separator() {
        assert_eq!(join_rendered(["these words.", "The second"]), "these words.The second");
    }
}
