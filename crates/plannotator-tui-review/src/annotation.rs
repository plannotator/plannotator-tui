//! A code-review note, in the browser's `CodeAnnotation` shape.
//!
//! Field names and values follow Plannotator's `packages/ui/types.ts` (`CodeAnnotation`):
//! camelCase keys, `type` of `comment` | `suggestion` | `concern`, `scope` of `line` |
//! `file` | `general`, `side` of `old` | `new`, `createdAt` in Unix milliseconds. The anchor
//! fields (`anchorText`, `anchorContext`, `outdated`) are the browser's own (#1590), so a
//! note made here and a browser draft are the same record. Everything plannotator-tui adds
//! rides under one `plannotator_tui` key, and keys this crate does not know are kept as
//! they are, so a browser-written note survives a round trip.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::patch::FilePatch;

/// Which side of the diff a note's lines are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Old,
    New,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Old => "old",
            Self::New => "new",
        }
    }
}

/// The browser's `CodeAnnotationType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteType {
    #[default]
    Comment,
    Suggestion,
    Concern,
}

/// The browser's `CodeAnnotationScope`. Absent means `line` (the browser's default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteScope {
    #[default]
    Line,
    File,
    /// A review-level note with no file. The browser writes `filePath: ""` and lines `0`.
    General,
}

/// Up to two lines before and after the anchor on the same side (`None` where the line was
/// not in the patch, or before line 1), and the hunk header's function context.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnchorContext {
    pub before: Vec<Option<String>>,
    pub after: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunk: Option<String>,
}

/// Where a note stood when it was last resolved (see [`crate::resolve`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    StillThere,
    Moved,
    NotInDiff,
    Outdated,
}

/// The `plannotator_tui` namespace on a note.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extras {
    /// The browser's draft key of the patch the note was made on: `sha256(patch)[..16]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_key: Option<String>,
    /// The file's old path when it was renamed in the diff the note was made on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    /// Last edit, Unix milliseconds. With the record's deliveries this is the sent state: a
    /// note is pending until a delivery at or after this time covers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// The last resolution against the diff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Status>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

impl Extras {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One note on a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnnotation {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: NoteType,
    #[serde(default)]
    pub scope: NoteScope,
    pub file_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub side: Side,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_code: Option<String>,
    /// The lines a suggestion replaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_code: Option<String>,
    /// The anchored lines' text, joined with `\n`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_context: Option<AnchorContext>,
    /// The browser's flag for a note whose code changed. Kept in step with the resolution.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outdated: bool,
    /// The browser's flag for a note on lines outside the diff, made in a whole-file view: a
    /// new-side line note whose code is in `anchorText` (and `originalCode`), with no
    /// `anchorContext`. The feedback quotes the code; the diff never shows it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside_diff: bool,
    pub created_at: u64,
    /// Who wrote it, as an id (`pi`); display only, never an authority. The browser writes
    /// its reviewer identity here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// The name the feedback credits (`From: <authorName>`); display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(rename = "plannotator_tui", default, skip_serializing_if = "Extras::is_empty")]
    pub plannotator_tui: Extras,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

impl CodeAnnotation {
    /// A comment on lines `first..=last` of one side of `file`, anchored the way the browser
    /// anchors one (`captureAnchor`): the lines' text and context as the patch shows them.
    pub fn line_comment(
        file: &FilePatch,
        side: Side,
        first: u32,
        last: u32,
        text: String,
        patch_key: &str,
    ) -> Self {
        let mut note = Self::blank(file, NoteType::Comment, NoteScope::Line, side, patch_key);
        note.line_start = first.min(last);
        note.line_end = first.max(last);
        note.text = Some(text);
        if let Some((anchor_text, context)) = file.anchor(side, note.line_start, note.line_end) {
            note.anchor_text = Some(anchor_text);
            note.anchor_context = Some(context);
        }
        note
    }

    /// A suggested replacement for new-side lines `first..=last`. `originalCode` is the
    /// lines as the patch shows them; `text` is an optional comment beside the code.
    pub fn suggestion(
        file: &FilePatch,
        first: u32,
        last: u32,
        suggested_code: String,
        text: Option<String>,
        patch_key: &str,
    ) -> Self {
        let mut note = Self::line_comment(file, Side::New, first, last, String::new(), patch_key);
        note.kind = NoteType::Suggestion;
        note.text = text.filter(|t| !t.is_empty());
        note.original_code = note.anchor_text.clone();
        note.suggested_code = Some(suggested_code);
        note
    }

    /// A comment on the whole file. Lines are the browser's file-comment sentinels (1, new).
    pub fn file_comment(file: &FilePatch, text: String, patch_key: &str) -> Self {
        let mut note = Self::blank(file, NoteType::Comment, NoteScope::File, Side::New, patch_key);
        note.text = Some(text);
        note
    }

    /// A comment on the whole change: the browser's review-level ("Global") comment, in its
    /// shape (`createGeneralReviewComment`): no file, lines 0, the new side. The feedback lists
    /// it under [`crate::GLOBAL_COMMENTS_HEADING`], ahead of the files.
    pub fn general_comment(text: String, patch_key: &str) -> Self {
        let mut note = Self::unplaced(String::new(), None, NoteType::Comment, NoteScope::General, patch_key);
        note.line_start = 0;
        note.line_end = 0;
        note.text = Some(text);
        note
    }

    fn blank(file: &FilePatch, kind: NoteType, scope: NoteScope, side: Side, patch_key: &str) -> Self {
        let mut note = Self::unplaced(file.path.clone(), file.old_path.clone(), kind, scope, patch_key);
        note.side = side;
        note
    }

    /// A note on `file_path` (empty for the whole change) at the file-comment sentinels.
    fn unplaced(
        file_path: String,
        old_path: Option<String>,
        kind: NoteType,
        scope: NoteScope,
        patch_key: &str,
    ) -> Self {
        let now = now_millis();
        Self {
            id: local_id(),
            kind,
            scope,
            file_path,
            line_start: 1,
            line_end: 1,
            side: Side::New,
            text: None,
            suggested_code: None,
            original_code: None,
            anchor_text: None,
            anchor_context: None,
            outdated: false,
            outside_diff: false,
            created_at: now,
            author: None,
            author_name: None,
            plannotator_tui: Extras {
                patch_key: Some(patch_key.to_owned()),
                old_path,
                updated_at: Some(now),
                resolution: Some(Status::StillThere),
                other: BTreeMap::new(),
            },
            other: BTreeMap::new(),
        }
    }

    /// The path of the note's file on its own side: the old path for an old-side note on a
    /// renamed file.
    pub fn side_path(&self) -> &str {
        match (self.side, &self.plannotator_tui.old_path) {
            (Side::Old, Some(old)) => old,
            _ => &self.file_path,
        }
    }

    /// When the note last changed: `plannotator_tui.updatedAt`, else `createdAt`.
    pub fn updated_at(&self) -> u64 {
        self.plannotator_tui.updated_at.unwrap_or(self.created_at)
    }

    pub fn status(&self) -> Option<Status> {
        self.plannotator_tui.resolution
    }

    /// A comment on new-side lines `first..=last` of `path` that the diff does not show,
    /// made in the whole file, as the browser makes one (`handleAddWholeFileAnnotation`):
    /// `outsideDiff`, the lines' text as `anchorText` and `originalCode`, no context. It is
    /// placed by its text alone, never by the patch.
    pub fn outside_diff_comment(
        path: &str,
        first: u32,
        last: u32,
        code: String,
        text: String,
        patch_key: &str,
    ) -> Self {
        let now = now_millis();
        Self {
            id: local_id(),
            kind: NoteType::Comment,
            scope: NoteScope::Line,
            file_path: path.to_owned(),
            line_start: first.min(last),
            line_end: first.max(last),
            side: Side::New,
            text: Some(text),
            suggested_code: None,
            original_code: Some(code.clone()),
            anchor_text: Some(code),
            anchor_context: None,
            outdated: false,
            outside_diff: true,
            created_at: now,
            author: None,
            author_name: None,
            plannotator_tui: Extras {
                patch_key: Some(patch_key.to_owned()),
                old_path: None,
                updated_at: Some(now),
                resolution: Some(Status::NotInDiff),
                other: BTreeMap::new(),
            },
            other: BTreeMap::new(),
        }
    }
}

/// Unix milliseconds now.
pub(crate) fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A local id: the time in nanoseconds plus a per-process counter, so two notes made in the
/// same instant still differ.
fn local_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("local_{nanos:x}{n:x}")
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    /// A note as the browser writes it (packages/review-editor/App.tsx `handleAddAnnotation`
    /// plus `captureAnchor`), with browser-only fields this crate does not model.
    const BROWSER_NOTE: &str = r#"{
        "id": "k3j9x2a",
        "type": "comment",
        "scope": "line",
        "filePath": "src/auth.ts",
        "lineStart": 12,
        "lineEnd": 13,
        "side": "new",
        "text": "Missing auth check here.",
        "anchorText": "const t = req.headers.authorization;\nnext();",
        "anchorContext": {"before": [null, "function guard(req, next) {"], "after": ["}", null], "hunk": "export function guard"},
        "createdAt": 1793000000000,
        "author": "Swift Falcon",
        "conventionalLabel": "issue",
        "decorations": ["blocking"]
    }"#;

    #[test]
    fn a_browser_note_round_trips_with_unknown_fields_kept() {
        let note: CodeAnnotation = serde_json::from_str(BROWSER_NOTE).expect("parses");
        assert_eq!(note.side, Side::New);
        assert_eq!(note.anchor_context.as_ref().expect("context").before[0], None);
        assert!(note.plannotator_tui.is_empty());
        let back = serde_json::to_value(&note).expect("serializes");
        assert_eq!(back, serde_json::from_str::<Value>(BROWSER_NOTE).expect("JSON"));
    }

    #[test]
    fn a_whole_change_note_has_the_browsers_general_comment_shape() {
        let note = CodeAnnotation::general_comment("Split this PR in two.".into(), "0123456789abcdef");
        let json = serde_json::to_value(&note).expect("serializes");
        assert_eq!(json["type"], "comment");
        assert_eq!(json["scope"], "general");
        assert_eq!(json["filePath"], "");
        assert_eq!((json["lineStart"].as_u64(), json["lineEnd"].as_u64()), (Some(0), Some(0)));
        assert_eq!(json["side"], "new");
        assert_eq!(json["text"], "Split this PR in two.");
        assert_eq!(json["plannotator_tui"]["patchKey"], "0123456789abcdef");
        assert!(json["plannotator_tui"].get("oldPath").is_none());
    }

    #[test]
    fn a_note_outside_the_diff_has_the_browsers_whole_file_shape() {
        let note = CodeAnnotation::outside_diff_comment(
            "src/auth.ts",
            40,
            41,
            "if (!token) {\n  return;".into(),
            "Why return silently?".into(),
            "0123456789abcdef",
        );
        let json = serde_json::to_value(&note).expect("serializes");
        assert_eq!(json["scope"], "line");
        assert_eq!(json["side"], "new");
        assert_eq!(json["outsideDiff"], true);
        assert_eq!(json["anchorText"], "if (!token) {\n  return;");
        assert_eq!(json["originalCode"], json["anchorText"]);
        assert!(json.get("anchorContext").is_none());
        assert_eq!(note.status(), Some(Status::NotInDiff));
        let back: CodeAnnotation = serde_json::from_value(json).expect("parses");
        assert!(back.outside_diff);
    }

    #[test]
    fn extras_ride_under_one_snake_case_key_and_scope_defaults_to_line() {
        let mut note: CodeAnnotation = serde_json::from_str(
            r#"{"id":"a","type":"suggestion","filePath":"x","lineStart":1,"lineEnd":1,"side":"old","createdAt":1}"#,
        )
        .expect("parses without scope");
        assert_eq!(note.scope, NoteScope::Line);
        note.plannotator_tui.patch_key = Some("0123456789abcdef".into());
        note.plannotator_tui.resolution = Some(Status::NotInDiff);
        note.author_name = Some("Pi".into());
        let json = serde_json::to_value(&note).expect("serializes");
        assert_eq!(json["plannotator_tui"]["patchKey"], "0123456789abcdef");
        assert_eq!(json["plannotator_tui"]["resolution"], "not_in_diff");
        assert_eq!(json["authorName"], "Pi");
        assert!(json.get("outdated").is_none(), "false is omitted, as the browser omits it");
    }
}
