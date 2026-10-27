//! Annotations, replies, and the request/response bodies, in the Workspaces wire shape.
//!
//! One type serves both the local sidecar and the server row: an annotation saved next to
//! a file is exactly what `POST .../annotations` returns. Responses may grow fields;
//! unknown keys are preserved rather than rejected.

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
    /// plannotator-tui's own fields. Workspaces stores annotations opaquely beyond its own
    /// keys; an empty namespace is omitted, so a note without them keeps the server shape.
    #[serde(default, skip_serializing_if = "NoteExtras::is_empty")]
    pub plannotator_tui: NoteExtras,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// The `plannotator_tui` namespace on a root annotation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteExtras {
    /// Absolute paths of local image files attached to the note, in the order attached.
    /// References only: the files are never copied or uploaded, and Workspaces' own
    /// `attachments` (uploaded `https://` URLs) is a different field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

impl NoteExtras {
    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.other.is_empty()
    }
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
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn note(extras: &str) -> String {
        format!(
            r#"{{"id":"a1","document_id":"d1","anchor":{{"originalText":"hello"}},"body":"b","author":null,
               "state":"open","created_at":"t","updated_at":"t","replies":[]{extras}}}"#
        )
    }

    #[test]
    fn a_note_without_images_keeps_the_server_shape() {
        let parsed: Annotation = serde_json::from_str(&note("")).expect("an older record parses");
        assert_eq!(parsed.plannotator_tui, NoteExtras::default());
        let json = serde_json::to_value(&parsed).expect("serializable");
        assert!(json.get("plannotator_tui").is_none(), "{json}");
    }

    #[test]
    fn images_round_trip_beside_unknown_namespace_fields() {
        let text = note(r#","plannotator_tui":{"images":["/tmp/a.png"],"later":1}"#);
        let parsed: Annotation = serde_json::from_str(&text).expect("parses");
        assert_eq!(parsed.plannotator_tui.images, ["/tmp/a.png"]);
        assert_eq!(parsed.attachments, Vec::<String>::new(), "local paths never become Workspaces URLs");
        let back = serde_json::to_value(&parsed).expect("serializable");
        assert_eq!(back, serde_json::from_str::<Value>(&text).expect("JSON"));
    }
}
