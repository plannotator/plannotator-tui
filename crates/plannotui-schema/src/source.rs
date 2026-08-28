//! What the app opens: a document source, not necessarily a file.
//!
//! A file on disk, a Workspaces document, and an agent's last reply are all sources. The
//! `transient` flag is the only behavioural switch: transient sources get no sidecar, no
//! history, no drafts. Provenance is opaque to the schema and meaningful to the delivery
//! seam (where feedback goes back to).

use crate::version::blob_sha;

/// Where a document came from. The app never interprets these beyond display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// A file on this machine.
    File { path: std::path::PathBuf },
    /// A Workspaces document.
    Workspace { workspace_id: String, document_id: String },
    /// An agent's message, handed in by a host integration.
    AgentMessage { host: String, session: Option<String>, message_id: Option<String> },
    /// Standard input or another one-off feed.
    Stdin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSource {
    /// Raw markdown. Byte offsets in anchors index into this exact string.
    pub content: String,
    /// What to call it in the UI.
    pub name: String,
    /// True when nothing about this document should be persisted.
    pub transient: bool,
    pub provenance: Provenance,
    /// Git blob sha of `content`; the `version` every anchor made against it carries.
    pub version: String,
}

impl DocumentSource {
    pub fn new(content: String, name: impl Into<String>, transient: bool, provenance: Provenance) -> Self {
        let version = blob_sha(content.as_bytes());
        Self { content, name: name.into(), transient, provenance, version }
    }

    pub fn file(path: std::path::PathBuf, content: String) -> Self {
        let name =
            path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        Self::new(content, name, false, Provenance::File { path })
    }
}
