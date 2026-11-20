//! A cheap fingerprint of what a scope's diff depends on, so the screen can say "the diff
//! changed on disk" without capturing the diff again.
//!
//! One `git status --porcelain=v2 -z --branch --untracked-files=all` (with
//! `--no-optional-locks`, so it never takes the index lock or rewrites the index), plus the
//! modification time and size of each file it lists. Git's own stat cache (and fsmonitor
//! or the untracked cache, where the repository has them) makes the status cheap; the stat
//! calls cover a file that was already changed and changed again, which status alone reports
//! the same way.
//!
//! | Scope | Watched |
//! |---|---|
//! | Uncommitted, Branch | HEAD, every changed or untracked file, and their mtime and size |
//! | Staged | HEAD and the index side of each staged file (its blob id) |
//! | Commit, Pull request | nothing: a commit does not change on disk |
//!
//! A branch review does not notice its base ref moving; `r` still recaptures it.

use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::time::UNIX_EPOCH;

use crate::capture::Repo;
use crate::error::Result;
use crate::git;
use crate::scope::Scope;

/// What a scope's diff depends on, at one moment. Equal stamps mean nothing the diff shows
/// changed in between (up to the limits in the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskStamp(u64);

impl Scope {
    /// Whether anything on disk can change this scope's diff.
    pub fn watches_disk(&self) -> bool {
        matches!(self, Self::Uncommitted | Self::Staged | Self::Branch { .. })
    }
}

impl Repo {
    /// The stamp of `scope` now; `None` for a scope nothing on disk changes.
    pub fn disk_stamp(&self, scope: &Scope) -> Result<Option<DiskStamp>> {
        if !scope.watches_disk() {
            return Ok(None);
        }
        let mut cmd = git::command(self.root());
        cmd.args(["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all", "--no-renames"]);
        let status = git::run(cmd, "status --porcelain=v2")?;
        Ok(Some(stamp_of(self, scope, &status)))
    }
}

fn stamp_of(repo: &Repo, scope: &Scope, status: &[u8]) -> DiskStamp {
    let staged_only = *scope == Scope::Staged;
    let mut hasher = DefaultHasher::new();
    for record in status.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        if let Some(header) = record.strip_prefix(b"# ") {
            // HEAD's commit; the other headers name the branch and its upstream.
            if header.starts_with(b"branch.oid ") {
                header.hash(&mut hasher);
            }
            continue;
        }
        let fields: Vec<&[u8]> = record.splitn(fields_before_path(record), |b| *b == b' ').collect();
        let Some(path) = fields.last().copied() else { continue };
        if staged_only {
            // `1 XY sub mH mI mW hH hI path`, `u XY sub m1 m2 m3 mW h1 h2 h3 path`: the index
            // status letter and every blob id; untracked files and the work tree do not count.
            let index_letter = fields.get(1).and_then(|xy| xy.first());
            if matches!(record.first(), Some(b'1' | b'u')) && index_letter != Some(&b'.') {
                index_letter.hash(&mut hasher);
                fields.iter().filter(|f| f.len() >= 40).for_each(|f| f.hash(&mut hasher));
                path.hash(&mut hasher);
            }
            continue;
        }
        record.hash(&mut hasher);
        file_stamp(repo, path).hash(&mut hasher);
    }
    DiskStamp(hasher.finish())
}

/// How many space-separated fields a porcelain v2 record has, its path last (a path may hold
/// spaces): `1` has 9, `u` 11, `?` and `!` 2. Renames are off (`--no-renames`), so no `2`.
fn fields_before_path(record: &[u8]) -> usize {
    match record.first() {
        Some(b'1') => 9,
        Some(b'u') => 11,
        _ => 2,
    }
}

/// A work-tree file's modification time (nanoseconds) and size; `None` when it is missing.
fn file_stamp(repo: &Repo, path: &[u8]) -> Option<(u128, u64)> {
    let path = std::str::from_utf8(path).ok()?;
    let meta = std::fs::symlink_metadata(repo.root().join(path)).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some((modified, meta.len()))
}

#[cfg(test)]
mod tests;
