//! The picker's rows, in their fixed order, and the words each shows.

use plannotator_tui_review::{Commit, Scope};

use super::source::Size;

/// What one row reviews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Kind {
    Uncommitted,
    Unstaged,
    Staged,
    Branch(Scope),
    Commit(Commit),
    /// `more…`: list the next page of commits.
    More,
}

/// How much of a row's size is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Sized {
    /// Not asked yet.
    Waiting,
    Asking,
    Known(Size),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) kind: Kind,
    pub(super) size: Sized,
}

impl Row {
    pub(super) fn new(kind: Kind) -> Self {
        Self { kind, size: Sized::Waiting }
    }

    /// The scope a review of this row captures; `None` for `more…`.
    pub(super) fn scope(&self) -> Option<Scope> {
        Some(match &self.kind {
            Kind::Uncommitted => Scope::Uncommitted,
            Kind::Unstaged => Scope::Unstaged,
            Kind::Staged => Scope::Staged,
            Kind::Branch(scope) => scope.clone(),
            Kind::Commit(commit) => Scope::Commit { sha: commit.sha.clone() },
            Kind::More => return None,
        })
    }

    /// Known to have nothing to review.
    pub(super) fn is_empty(&self) -> bool {
        matches!(&self.size, Sized::Known(size) if size.stat.is_empty())
    }

    /// The scope's own name, as the review's header has it: `Uncommitted`, `Commit a1b2c3d`.
    pub(super) fn name(&self) -> String {
        self.scope().map_or_else(|| "more".to_owned(), |scope| scope.title())
    }

    /// The short id and the subject of a commit row; the scope's name for the others.
    pub(super) fn title(&self) -> (Option<String>, String) {
        match &self.kind {
            Kind::Commit(commit) => {
                (Some(commit.sha.get(..7).unwrap_or(&commit.sha).to_owned()), commit.subject.clone())
            }
            Kind::More => (None, "more\u{2026}".to_owned()),
            _ => (None, self.name()),
        }
    }

    /// `+12 −3 · 4 files`, `(no changes)` or `…` while it is worked out; `None` for `more…`.
    pub(super) fn stat(&self) -> Option<Stat> {
        if self.kind == Kind::More {
            return None;
        }
        Some(match &self.size {
            Sized::Waiting | Sized::Asking => Stat::Pending,
            Sized::Failed(_) => Stat::Failed,
            Sized::Known(size) if size.stat.is_empty() => Stat::Empty,
            Sized::Known(size) => Stat::Lines {
                added: size.stat.additions,
                removed: size.stat.deletions,
                files: size.stat.files,
            },
        })
    }

    /// When it last changed, in seconds since the epoch: a commit's time, the newest changed
    /// file for the working tree, the branch's newest commit (`head`).
    pub(super) fn time(&self, head: Option<i64>) -> Option<i64> {
        match (&self.kind, &self.size) {
            (Kind::Commit(commit), _) => Some(commit.time),
            (Kind::Branch(_), _) => head,
            (Kind::More, _) => None,
            (_, Sized::Known(size)) => size.stat.newest,
            _ => None,
        }
    }

    pub(super) fn author(&self) -> Option<&str> {
        match &self.kind {
            Kind::Commit(commit) => Some(&commit.author),
            _ => None,
        }
    }

    pub(super) fn unsent(&self) -> usize {
        match &self.size {
            Sized::Known(size) => size.unsent,
            _ => 0,
        }
    }
}

/// A row's size, in the words it is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stat {
    Pending,
    Failed,
    Empty,
    Lines { added: u64, removed: u64, files: usize },
}

impl Stat {
    /// The counts, and the file count after them when `files` is on.
    pub(super) fn text(self, files: bool) -> String {
        match self {
            Self::Pending => "\u{2026}".to_owned(),
            Self::Failed => "(unreadable)".to_owned(),
            Self::Empty => "(no changes)".to_owned(),
            Self::Lines { added, removed, files: count } if files => {
                let noun = if count == 1 { "file" } else { "files" };
                format!("+{added} \u{2212}{removed} \u{b7} {count} {noun}")
            }
            Self::Lines { added, removed, .. } => format!("+{added} \u{2212}{removed}"),
        }
    }
}

/// How long ago `then` was, `now` being the present, both in seconds since the epoch: `now`,
/// `5m`, `2h`, `3d`, `4w`, `5mo`, `2y`.
pub(super) fn age(then: i64, now: i64) -> String {
    let seconds = now.saturating_sub(then).max(0);
    let (minutes, hours, days) = (seconds / 60, seconds / 3600, seconds / 86_400);
    match () {
        () if minutes < 1 => "now".to_owned(),
        () if hours < 1 => format!("{minutes}m"),
        () if hours < 48 => format!("{hours}h"),
        () if days < 14 => format!("{days}d"),
        () if days < 60 => format!("{}w", days / 7),
        () if days < 730 => format!("{}mo", days / 30),
        () => format!("{}y", days / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_in_the_largest_unit_that_keeps_them_short() {
        let now = 10_000_000;
        let cases = [
            (0, "now"),
            (59, "now"),
            (5 * 60, "5m"),
            (2 * 3600, "2h"),
            (47 * 3600, "47h"),
            (3 * 86_400, "3d"),
            (20 * 86_400, "2w"),
            (90 * 86_400, "3mo"),
            (800 * 86_400, "2y"),
            (-30, "now"),
        ];
        for (ago, text) in cases {
            assert_eq!(age(now - ago, now), text, "{ago} s ago");
        }
    }

    #[test]
    fn a_size_reads_as_counts_then_files_or_says_why_there_are_none() {
        let lines = Stat::Lines { added: 12, removed: 3, files: 4 };
        assert_eq!(lines.text(true), "+12 \u{2212}3 \u{b7} 4 files");
        assert_eq!(lines.text(false), "+12 \u{2212}3");
        assert_eq!(Stat::Lines { added: 1, removed: 0, files: 1 }.text(true), "+1 \u{2212}0 \u{b7} 1 file");
        assert_eq!(Stat::Empty.text(true), "(no changes)");
        assert_eq!(Stat::Pending.text(true), "\u{2026}");
    }
}
