//! Annotation and anchor types shared by plannotui and Plannotator Workspaces.
//!
//! The wire shapes here match the Workspaces API (`Annotation`, `AnnotationReply`,
//! `CreateAnnotationRequest`, `PatchAnnotationRequest`). The anchor object is opaque to the
//! server; the web client reads `originalText`, and everything plannotui needs to re-find a
//! selection rides under one `plannotui` key. See `docs/decisions.md` for the reasoning.
//!
//! No I/O, no async, no terminal. Every function is pure over strings and values.

pub mod anchor;
pub mod annotation;
pub mod resolve;
pub mod source;
pub mod version;

pub use anchor::{Anchor, CONTEXT_CHARS, Extras, Kind, SourceRange};
pub use annotation::{Annotation, ApiError, CreateAnnotation, PatchAnnotation, Reply, State};
pub use resolve::{Resolution, resolve, web_will_match};
pub use source::{DocumentSource, Provenance};
pub use version::blob_sha;
