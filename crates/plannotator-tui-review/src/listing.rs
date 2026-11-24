//! What a change picker lists before anything is captured: the recent commits, the branch
//! a review can compare against, and how large each scope's diff is.
//!
//! The commits and the branch come from a few quick calls (`git log -n N` with a format
//! string, `rev-parse`, `merge-base`), so a picker can draw them at once. A scope's size is
//! `git diff --numstat`, run exactly as [`Repo::capture`] runs the diff (the same sides, the
//! same temporary index for untracked files), so a size of zero means its review would be
//! empty. That can take a while in a large working tree; a picker runs it in the background.

use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::capture::{MAX_PATCH_BYTES, Repo};
use crate::error::Result;
use crate::git;
use crate::scope::Scope;

/// One commit, as `git log` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The full id.
    pub sha: String,
    pub subject: String,
    /// The author's name, after `.mailmap`.
    pub author: String,
    /// When it was committed, in seconds since the epoch.
    pub time: i64,
}

/// How large a scope's diff is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub files: usize,
    pub additions: u64,
    pub deletions: u64,
    /// For the scopes that read the working tree or the index (uncommitted, staged,
    /// unstaged): the newest modification time of a changed file still on disk, in seconds
    /// since the epoch.
    pub newest: Option<i64>,
}

impl DiffStat {
    pub fn is_empty(&self) -> bool {
        self.files == 0
    }
}

impl Repo {
    /// Up to `count` commits reachable from HEAD, newest first, after the first `skip`. A
    /// repository with no commit yet has none.
    pub fn commits(&self, skip: usize, count: usize) -> Result<Vec<Commit>> {
        if count == 0 || self.commit("HEAD").is_err() {
            return Ok(Vec::new());
        }
        let mut cmd = git::command(self.root());
        cmd.args(["log", "-z", "--no-show-signature", "--format=%H%x1f%ct%x1f%aN%x1f%s"])
            .arg(format!("--skip={skip}"))
            .arg(format!("--max-count={count}"))
            .args(["--end-of-options", "HEAD", "--"]);
        let out = git::run(cmd, "log")?;
        Ok(String::from_utf8_lossy(&out)
            .split('\0')
            .filter_map(|record| {
                let mut fields = record.trim_start_matches('\n').splitn(4, '\u{1f}');
                let sha = fields.next().filter(|sha| !sha.is_empty())?.to_owned();
                let time = fields.next()?.parse().unwrap_or(0);
                let author = fields.next()?.to_owned();
                let subject = fields.next().unwrap_or_default().to_owned();
                Some(Commit { sha, subject, author, time })
            })
            .collect())
    }

    /// The branch review a picker offers: the default base ([`crate::DEFAULT_BASE`]) as
    /// [`Self::branch_scope`] names it, when it resolves and its merge base with HEAD is not
    /// HEAD itself (otherwise it shows what the uncommitted review shows).
    pub fn branch_choice(&self) -> Option<Scope> {
        let scope = self.branch_scope(None).ok()?;
        let head = self.commit("HEAD").ok()?;
        let merge_base = self.merge_base(scope.base()?).ok()?;
        (merge_base != head).then_some(scope)
    }

    /// The size of `scope`'s diff, without capturing it. A pull request has no local size.
    pub fn diff_stat(&self, scope: &Scope) -> Result<DiffStat> {
        let (_, _, out) = self.scope_diff(scope, &["--numstat", "-z"], MAX_PATCH_BYTES)?;
        let mut stat = DiffStat::default();
        let mut paths = Vec::new();
        for (additions, deletions, path) in numstat(&out) {
            stat.files += 1;
            stat.additions += additions;
            stat.deletions += deletions;
            paths.push(path);
        }
        if matches!(scope, Scope::Uncommitted | Scope::Staged | Scope::Unstaged) {
            stat.newest = paths.iter().filter_map(|path| modified(&self.root().join(path))).max();
        }
        Ok(stat)
    }
}

/// `git diff --numstat -z`: `added\tdeleted\tpath\0` per file, or for a rename or copy
/// `added\tdeleted\t\0old\0new\0`. A binary file counts `-` lines, read as zero.
fn numstat(out: &[u8]) -> Vec<(u64, u64, String)> {
    let mut fields = out.split(|b| *b == 0);
    let mut files = Vec::new();
    while let Some(head) = fields.next() {
        let head = String::from_utf8_lossy(head);
        let mut parts = head.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            let _old = fields.next();
            fields.next().map(|new| String::from_utf8_lossy(new).into_owned()).unwrap_or_default()
        } else {
            path.to_owned()
        };
        files.push((added.parse().unwrap_or(0), deleted.parse().unwrap_or(0), path));
    }
    files
}

/// A file's modification time in seconds since the epoch; `None` when it is gone.
fn modified(path: &Path) -> Option<i64> {
    let at = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
    i64::try_from(at.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()
}

#[cfg(all(test, unix))]
mod tests;
