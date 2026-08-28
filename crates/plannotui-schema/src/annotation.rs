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
