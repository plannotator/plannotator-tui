//! Annotations, replies, and the request/response bodies, in the Workspaces wire shape.
//!
//! One type serves both local records and server rows, with additive TUI-owned fields for
//! local references. Responses may grow fields; unknown keys are preserved rather than
//! rejected.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::anchor::Anchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    #[default]
    Open,
    Resolved,
}

/// A shallow reply under a root annotation. Replies have no anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub annotation_id: String,
    pub body: String,
    /// Display label only; never an authority. `None` for an anonymous author.
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// A plannotator-tui-owned attachment. Workspaces still owns the top-level
/// `attachments: string[]` field for uploaded `https://` image URLs; local files live here
/// so a future Workspaces sync can omit or upload them rather than sending invalid URLs.
/// Type-like fields are strings, not enums, so newer attachment kinds still round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAttachment {
    #[serde(rename = "type")]
    pub attachment_type: String,
    pub source: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

impl LocalAttachment {
    pub fn image(path: String, alt: Option<String>, media_type: Option<String>) -> Self {
        Self {
            attachment_type: "image".to_owned(),
            source: "local_file".to_owned(),
            path,
            alt,
            media_type,
            other: BTreeMap::new(),
        }
    }

    pub fn is_local_image(&self) -> bool {
        self.attachment_type == "image" && self.source == "local_file" && !self.path.is_empty()
    }
}

/// plannotator-tui-owned fields on a root annotation. Empty records serialize exactly like
/// the Workspaces object, and older plannotator-tui builds preserve this object through
/// their flattened `other` map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationExtras {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<LocalAttachment>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

impl AnnotationExtras {
    pub fn is_empty(&self) -> bool {
        self.attachments.is_empty() && self.other.is_empty()
    }
}

/// A root annotation: the server's `Annotation` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    pub document_id: String,
    pub anchor: Anchor,
    pub body: String,
    /// Display label only; never an authority. `None` for an anonymous author.
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(default)]
    pub state: State,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    #[serde(default, skip_serializing_if = "AnnotationExtras::is_empty")]
    pub plannotator_tui: AnnotationExtras,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// Body of `POST .../annotations`. The whole request is capped at 32 KiB; `body` at 8 KiB;
/// the serialized `anchor` at 16 KiB; `author` at 120 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAnnotation {
    pub anchor: Anchor,
    pub body: String,
    /// Honored only for anonymous share-token callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}

/// Body of `PATCH .../annotations/{id}`. Any combination; `anchor` replaces the whole object.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PatchAnnotation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

/// The standard error envelope: `{"error": {"code", "message", "details"?}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error.code, self.error.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::{Anchor, Kind, SourceRange};

    fn annotation() -> Annotation {
        Annotation {
            id: "a1".into(),
            document_id: "d1".into(),
            anchor: Anchor::new(
                "hello",
                "hello world",
                SourceRange { start: 0, end: 5, version: "v".into() },
                Kind::Comment,
                Some(0),
            ),
            body: "see screenshot".into(),
            author: None,
            author_name: None,
            state: State::Open,
            attachments: Vec::new(),
            plannotator_tui: AnnotationExtras::default(),
            created_at: "2026-09-14T00:00:00.000Z".into(),
            updated_at: "2026-09-14T00:00:00.000Z".into(),
            replies: Vec::new(),
            other: BTreeMap::new(),
        }
    }

    #[test]
    fn empty_local_extras_do_not_change_the_workspaces_shape() {
        let json = serde_json::to_value(annotation()).expect("serializable");
        assert!(json.get("plannotator_tui").is_none());
        assert!(json.get("attachments").is_none());
    }

    #[test]
    fn local_image_attachments_round_trip_under_the_tui_namespace() {
        let mut annotation = annotation();
        annotation.plannotator_tui.attachments.push(LocalAttachment::image(
            "/tmp/screen.png".into(),
            Some("screen.png".into()),
            Some("image/png".into()),
        ));
        let json = serde_json::to_value(&annotation).expect("serializable");
        assert_eq!(json["plannotator_tui"]["attachments"][0]["type"], "image");
        assert_eq!(json["plannotator_tui"]["attachments"][0]["source"], "local_file");
        assert_eq!(json["plannotator_tui"]["attachments"][0]["path"], "/tmp/screen.png");
        assert!(json.get("attachments").is_none(), "local files must not leak into Workspaces URLs");
        let parsed: Annotation = serde_json::from_value(json).expect("parses");
        assert_eq!(parsed, annotation);
        assert!(parsed.plannotator_tui.attachments[0].is_local_image());
    }

    #[test]
    fn future_local_attachment_kinds_are_preserved() {
        let json = serde_json::json!({
            "id": "a1",
            "document_id": "d1",
            "anchor": {"originalText": "hello"},
            "body": "body",
            "author": null,
            "state": "open",
            "plannotator_tui": {"attachments": [{"type": "video", "source": "local_file", "path": "/tmp/cast.webm", "duration": 4}]},
            "created_at": "2026-09-14T00:00:00.000Z",
            "updated_at": "2026-09-14T00:00:00.000Z",
            "replies": []
        });
        let parsed: Annotation = serde_json::from_value(json.clone()).expect("parses");
        assert_eq!(serde_json::to_value(parsed).expect("serializable"), json);
    }
}
