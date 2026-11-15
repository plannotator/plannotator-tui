//! What a review compares, and the names each consumer needs for it.

use serde::{Deserialize, Serialize};

/// What a review compares. Refs are resolved before a scope is built (see
/// [`crate::Repo::open`]): `base` is a ref name such as `origin/main`, `sha` a full commit id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// HEAD against the working tree, untracked files included.
    Uncommitted,
    /// HEAD against the index.
    Staged,
    /// merge-base(base, HEAD) against the working tree, untracked files included.
    Branch { base: String },
    /// A commit against its first parent.
    Commit { sha: String },
}

impl Scope {
    /// The record key: `uncommitted`, `staged`, `branch-<base>` or `commit-<sha12>`.
    pub fn id(&self) -> String {
        match self {
            Self::Uncommitted => "uncommitted".to_owned(),
            Self::Staged => "staged".to_owned(),
            Self::Branch { base } => format!("branch-{base}"),
            Self::Commit { sha } => format!("commit-{}", sha.get(..12).unwrap_or(sha)),
        }
    }

    /// The browser's `diffType` for the same comparison, for the feedback archive's
    /// `target.review.diffType`. Branch is the browser's `since-base` (merge-base to the
    /// working tree, untracked included), not its `branch` (`base..HEAD`).
    pub fn diff_type(&self) -> String {
        match self {
            Self::Uncommitted => "uncommitted".to_owned(),
            Self::Staged => "staged".to_owned(),
            Self::Branch { .. } => "since-base".to_owned(),
            Self::Commit { sha } => format!("commit:{sha}"),
        }
    }

    pub fn base(&self) -> Option<&str> {
        match self {
            Self::Branch { base } => Some(base),
            _ => None,
        }
    }

    /// The feedback's `**Diff:**` line: the browser's `describeDiff` for the same diff type.
    pub fn describe(&self, commit_subject: Option<&str>) -> String {
        match self {
            Self::Uncommitted => "Uncommitted changes".to_owned(),
            Self::Staged => "Staged changes".to_owned(),
            Self::Branch { base } => {
                format!("All changes since `{base}` (committed + uncommitted + untracked)")
            }
            Self::Commit { sha } => {
                let subject = commit_subject.filter(|s| !s.is_empty()).map(|s| format!(" — {s}"));
                format!("Commit `{}`{} (diff vs its parent)", short(sha), subject.unwrap_or_default())
            }
        }
    }

    /// A short name for a header: `Uncommitted`, `Staged`, `Branch vs origin/main`,
    /// `Commit 1a2b3c4`.
    pub fn title(&self) -> String {
        match self {
            Self::Uncommitted => "Uncommitted".to_owned(),
            Self::Staged => "Staged".to_owned(),
            Self::Branch { base } => format!("Branch vs {base}"),
            Self::Commit { sha } => format!("Commit {}", short(sha)),
        }
    }
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

    #[test]
    fn ids_name_the_record_directory() {
        assert_eq!(Scope::Uncommitted.id(), "uncommitted");
        assert_eq!(Scope::Staged.id(), "staged");
        assert_eq!(Scope::Branch { base: "origin/main".into() }.id(), "branch-origin/main");
        assert_eq!(Scope::Commit { sha: SHA.into() }.id(), "commit-1a2b3c4d5e6f");
    }

    #[test]
    fn descriptions_match_the_browsers_describe_diff() {
        // packages/review-editor/utils/exportFeedback.ts `describeDiff`.
        assert_eq!(Scope::Uncommitted.describe(None), "Uncommitted changes");
        assert_eq!(Scope::Staged.describe(None), "Staged changes");
        assert_eq!(
            Scope::Branch { base: "origin/main".into() }.describe(None),
            "All changes since `origin/main` (committed + uncommitted + untracked)"
        );
        let commit = Scope::Commit { sha: SHA.into() };
        assert_eq!(commit.describe(Some("fix: login")), "Commit `1a2b3c4` — fix: login (diff vs its parent)");
        assert_eq!(commit.describe(None), "Commit `1a2b3c4` (diff vs its parent)");
        assert_eq!(commit.diff_type(), format!("commit:{SHA}"));
    }

    #[test]
    fn scopes_serialize_as_tagged_objects() {
        let json = serde_json::to_string(&Scope::Branch { base: "origin/main".into() }).unwrap_or_default();
        assert_eq!(json, r#"{"kind":"branch","base":"origin/main"}"#);
    }
}
