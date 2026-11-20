//! The data side of reviewing a git diff in plannotator-tui: capture a scope's patch, make
//! notes on its lines, re-find them after the code changes, save them, and write the
//! feedback a coding agent receives.
//!
//! - [`scope`]: what a review compares, and its names (record id, browser diff type, label).
//! - [`capture`]: running git for a scope (temporary index for untracked files, size guard)
//!   and reading either side's full files; fetching a pull request.
//! - [`codenav`]: definition search on either side of a diff (`git grep`), ranked as the
//!   browser ranks it, and the identifiers on a line.
//! - [`github`]: PR links and numbers, and `gh pr view` (read-only).
//! - [`patch`]: reading a unified patch: files, the lines each side shows, anchors.
//! - [`annotation`]: a note, in the browser's `CodeAnnotation` shape.
//! - [`resolve`]: pure functions that place a note against the current files.
//! - [`store`]: `review.json`, saved on every change; sends and archiving.
//! - [`progress`]: viewed marks, in the browser's review-progress format.
//! - [`export`]: the feedback text, in the browser's `exportReviewFeedback` format.
//! - [`stamp`]: a cheap fingerprint of what a scope's diff depends on, to notice changes.
//!
//! No terminal, no diff viewer: the app draws, this crate owns the records.

pub mod annotation;
pub mod capture;
pub mod codenav;
pub mod error;
pub mod export;
mod git;
pub mod github;
pub mod patch;
pub mod progress;
pub mod resolve;
pub mod scope;
pub mod stamp;
pub mod store;

pub use annotation::{AnchorContext, CodeAnnotation, Extras, NoteScope, NoteType, Side, Status};
pub use capture::{
    Capture, DEFAULT_BASE, MAX_PATCH_BYTES, PullRequest, Repo, Request, SideSource, patch_key,
};
pub use error::{Error, Result};
pub use export::{
    OUTDATED_ANNOTATION_LABEL, REVIEW_SUFFIX, agent_feedback, pull_agent_feedback, pull_feedback,
    review_feedback,
};
pub use github::{Gh, PrRepo, PrSpec, PrView};
pub use patch::{FilePatch, FileStatus};
pub use progress::Progress;
pub use resolve::{Current, Resolution, apply, resolve};
pub use scope::Scope;
pub use stamp::DiskStamp;
pub use store::{Delivered, Record, ReviewStore, project_for, review_dir};

/// sha256 over `parts` in order, as lowercase hex, first `chars` characters.
fn sha256_hex(parts: &[&[u8]], chars: usize) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let mut hasher = sha2::Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    let mut hex = hasher.finalize().iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    });
    hex.truncate(chars);
    hex
}
