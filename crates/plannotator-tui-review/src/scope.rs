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
    /// A GitHub pull request: merge-base(base, head) against its head commit, what GitHub
    /// shows. The commits are looked up again on every capture, so a push moves the notes
    /// the way a local edit does; only the PR itself is the scope.
    PullRequest { url: String, number: u64 },
}

impl Scope {
    /// The record key: `uncommitted`, `staged`, `branch-<base>`, `commit-<sha12>` or
    /// `pr-<number>` (beside the PR's repository, which keys the record too).
    pub fn id(&self) -> String {
        match self {
            Self::Uncommitted => "uncommitted".to_owned(),
            Self::Staged => "staged".to_owned(),
            Self::Branch { base } => format!("branch-{base}"),
            Self::Commit { sha } => format!("commit-{}", sha.get(..12).unwrap_or(sha)),
            Self::PullRequest { number, .. } => format!("pr-{number}"),
        }
    }

    /// The browser's `diffType` for the same comparison, for the feedback archive's
    /// `target.review.diffType`. Branch is the browser's `since-base` (merge-base to the
    /// working tree, untracked included), not its `branch` (`base..HEAD`). A pull request is
    /// the browser's `merge-base` (merge-base(base, head) to head, committed only); the
    /// browser's own PR review records no diff type of its own, and names the PR in the
    /// archive's `gitRef` and `pr` fields instead, as this app does too.
    pub fn diff_type(&self) -> String {
        match self {
            Self::Uncommitted => "uncommitted".to_owned(),
            Self::Staged => "staged".to_owned(),
            Self::Branch { .. } => "since-base".to_owned(),
            Self::Commit { sha } => format!("commit:{sha}"),
            Self::PullRequest { .. } => "merge-base".to_owned(),
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
            Self::PullRequest { url, number } => format!("PR #{number} ({url})"),
        }
    }

    /// A short name for a header: `Uncommitted`, `Staged`, `Branch vs origin/main`,
    /// `Commit 1a2b3c4`, `PR #12`.
    pub fn title(&self) -> String {
        match self {
            Self::Uncommitted => "Uncommitted".to_owned(),
            Self::Staged => "Staged".to_owned(),
            Self::Branch { base } => format!("Branch vs {base}"),
            Self::Commit { sha } => format!("Commit {}", short(sha)),
            Self::PullRequest { number, .. } => format!("PR #{number}"),
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
        let pr = Scope::PullRequest { url: "https://github.com/o/r/pull/12".into(), number: 12 };
        assert_eq!(
            (pr.id(), pr.title(), pr.diff_type()),
            ("pr-12".into(), "PR #12".into(), "merge-base".into())
        );
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
