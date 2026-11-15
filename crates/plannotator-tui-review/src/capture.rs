//! Capturing a scope's patch from git, and reading either side's full files.
//!
//! | Scope | Compares | Command |
//! |---|---|---|
//! | Uncommitted | HEAD vs working tree, untracked included | `git diff <HEAD>` on a temp index |
//! | Staged | HEAD vs index | `git diff --cached <HEAD>` |
//! | Branch | merge-base(base, HEAD) vs working tree, untracked included | `git diff <merge-base>` on a temp index |
//! | Commit | first parent vs commit | `git diff <sha>^ <sha>` |
//!
//! Untracked files are marked intent-to-add in a temporary copy of the index
//! (`GIT_INDEX_FILE`); the real index is never written. With no HEAD yet (a new repo) the
//! empty tree stands in for it, as for a root commit's parent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::annotation::{CodeAnnotation, Side};
use crate::error::{Error, Result};
use crate::git::{self, TempIndex};
use crate::patch::{self, FilePatch};
use crate::resolve::{self, Current, Resolution};
use crate::scope::Scope;

/// The largest patch a review opens: diffs-rs needs 540 to 800 MiB for its largest
/// measured patches.
pub const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;

/// The base a branch review uses when none is given.
pub const DEFAULT_BASE: &str = "origin/HEAD";

/// Flags on every `git diff`: no external diff or textconv drivers, no color, the `a/` and
/// `b/` prefixes whatever the config says, and submodules as one line.
const DIFF_FLAGS: [&str; 6] = [
    "--no-ext-diff",
    "--no-textconv",
    "--no-color",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--submodule=short",
];

/// A git work tree, found from any folder inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

/// What the person asked to review, before refs are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Uncommitted changes; when there are none and the base resolves, the branch instead.
    Uncommitted {
        base: Option<String>,
    },
    Staged,
    /// Against `base`, else [`DEFAULT_BASE`].
    Branch {
        base: Option<String>,
    },
    /// One commit, by any revision git understands (`HEAD`, a short sha).
    Commit {
        rev: String,
    },
}

/// Where one side's full file content comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideSource {
    /// A commit or tree, read with `git cat-file blob <rev>:<path>`.
    Rev(String),
    /// The index, read with `git cat-file blob :<path>`.
    Index,
    /// The file on disk.
    Worktree,
}

/// One captured diff.
#[derive(Debug, Clone)]
pub struct Capture {
    pub scope: Scope,
    /// The repository root, as `git rev-parse --show-toplevel` prints it.
    pub root: PathBuf,
    /// The raw patch, as git printed it; feed it to the viewer as a stream.
    pub patch: Vec<u8>,
    /// The browser's draft key for this patch: `sha256(patch)` hex, first 16.
    pub patch_key: String,
    /// The changed files, in patch order.
    pub files: Vec<FilePatch>,
    pub old: SideSource,
    pub new: SideSource,
    /// A commit review's subject line, for the feedback's `**Diff:**` line.
    pub commit_subject: Option<String>,
}

impl Repo {
    /// The work tree containing `dir`.
    pub fn discover(dir: &Path) -> Result<Self> {
        let mut cmd = git::command(dir);
        cmd.args(["rev-parse", "--show-toplevel"]);
        match git::run_text(cmd, "rev-parse --show-toplevel") {
            Ok(root) if !root.is_empty() => Ok(Self { root: PathBuf::from(root) }),
            Ok(_) => Err(Error::NotARepo { dir: dir.to_path_buf(), detail: "no work tree".into() }),
            Err(Error::Git { detail, .. }) => Err(Error::NotARepo { dir: dir.to_path_buf(), detail }),
            Err(other) => Err(other),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve `request` to a scope and capture it. Uncommitted falls back to the branch
    /// when it is empty and the base resolves; when both are empty the empty uncommitted
    /// capture is returned, so the caller can say "No uncommitted changes".
    pub fn open(&self, request: &Request) -> Result<Capture> {
        match request {
            Request::Uncommitted { base } => {
                let uncommitted = self.capture(&Scope::Uncommitted)?;
                if !uncommitted.is_empty() {
                    return Ok(uncommitted);
                }
                let Ok(scope) = self.branch_scope(base.as_deref()) else { return Ok(uncommitted) };
                match self.capture(&scope) {
                    Ok(branch) if !branch.is_empty() => Ok(branch),
                    Err(too_large @ Error::TooLarge { .. }) => Err(too_large),
                    // No merge base (unrelated history, no HEAD yet) is no branch to show.
                    Ok(_) | Err(_) => Ok(uncommitted),
                }
            }
            Request::Staged => self.capture(&Scope::Staged),
            Request::Branch { base } => self.capture(&self.branch_scope(base.as_deref())?),
            Request::Commit { rev } => {
                let sha = self.commit(rev).map_err(|_| Error::Git {
                    command: format!("rev-parse {rev}"),
                    detail: format!("{rev} does not name a commit"),
                })?;
                self.capture(&Scope::Commit { sha })
            }
        }
    }

    /// Capture exactly `scope` (no fallback): what a reload runs.
    pub fn capture(&self, scope: &Scope) -> Result<Capture> {
        let (old, new, patch) = match scope {
            Scope::Uncommitted => {
                let head = self.head_or_empty_tree()?;
                let index = TempIndex::with_untracked(&self.root)?;
                let patch = Self::diff(index.command(&self.root), &[], &[&head])?;
                (SideSource::Rev(head), SideSource::Worktree, patch)
            }
            Scope::Staged => {
                let head = self.head_or_empty_tree()?;
                let patch = Self::diff(git::command(&self.root), &["--cached"], &[&head])?;
                (SideSource::Rev(head), SideSource::Index, patch)
            }
            Scope::Branch { base } => {
                let merge_base = self.merge_base(base)?;
                let index = TempIndex::with_untracked(&self.root)?;
                let patch = Self::diff(index.command(&self.root), &[], &[&merge_base])?;
                (SideSource::Rev(merge_base), SideSource::Worktree, patch)
            }
            Scope::Commit { sha } => {
                let parent = match self.commit(&format!("{sha}^")) {
                    Ok(parent) => parent,
                    Err(_) => self.empty_tree()?,
                };
                let patch = Self::diff(git::command(&self.root), &[], &[&parent, sha])?;
                (SideSource::Rev(parent), SideSource::Rev(sha.clone()), patch)
            }
        };
        let commit_subject = match scope {
            Scope::Commit { sha } => Some(self.subject(sha)?),
            _ => None,
        };
        let files = patch::parse(&String::from_utf8_lossy(&patch));
        Ok(Capture {
            scope: scope.clone(),
            root: self.root.clone(),
            patch_key: patch_key(&patch),
            patch,
            files,
            old,
            new,
            commit_subject,
        })
    }

    /// The branch scope for `base` (else [`DEFAULT_BASE`]), with a symbolic ref such as
    /// `origin/HEAD` named by what it points at (`origin/main`), so both spellings share a
    /// record.
    pub fn branch_scope(&self, base: Option<&str>) -> Result<Scope> {
        let base = base.unwrap_or(DEFAULT_BASE);
        let no_base = || Error::NoBase { base: base.to_owned() };
        self.commit(base).map_err(|_| no_base())?;
        let mut cmd = git::command(&self.root);
        cmd.args(["rev-parse", "--abbrev-ref", base]);
        let name = git::run_text(cmd, "rev-parse --abbrev-ref").ok().filter(|n| !n.is_empty());
        Ok(Scope::Branch { base: name.unwrap_or_else(|| base.to_owned()) })
    }

    fn diff(mut cmd: Command, extra: &[&str], revs: &[&str]) -> Result<Vec<u8>> {
        cmd.arg("diff").args(DIFF_FLAGS).args(extra).arg("--end-of-options").args(revs).arg("--");
        git::run_bounded(cmd, "diff", MAX_PATCH_BYTES)
    }

    /// The full id of the commit `rev` names. A leading `-` is refused rather than handed to
    /// git, where it would read as an option (`rev-parse` has no portable `--end-of-options`).
    fn commit(&self, rev: &str) -> Result<String> {
        if rev.starts_with('-') {
            return Err(Error::Git {
                command: "rev-parse".into(),
                detail: format!("{rev} is not a revision"),
            });
        }
        let mut cmd = git::command(&self.root);
        cmd.args(["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]);
        git::run_text(cmd, "rev-parse --verify")
    }

    fn head_or_empty_tree(&self) -> Result<String> {
        match self.commit("HEAD") {
            Ok(head) => Ok(head),
            Err(_) => self.empty_tree(),
        }
    }

    /// The empty tree's id in this repository's hash (SHA-1 or SHA-256).
    fn empty_tree(&self) -> Result<String> {
        let mut cmd = git::command(&self.root);
        cmd.args(["hash-object", "-t", "tree", "--stdin"]);
        git::run_text(cmd, "hash-object -t tree")
    }

    fn merge_base(&self, base: &str) -> Result<String> {
        let mut cmd = git::command(&self.root);
        cmd.args(["merge-base", "--end-of-options", base, "HEAD"]);
        git::run_text(cmd, &format!("merge-base {base} HEAD"))
    }

    fn subject(&self, sha: &str) -> Result<String> {
        let mut cmd = git::command(&self.root);
        cmd.args(["log", "-1", "--format=%s", "--end-of-options", sha]);
        git::run_text(cmd, "log -1")
    }
}

impl Capture {
    pub fn is_empty(&self) -> bool {
        self.patch.is_empty()
    }

    pub fn patch_bytes(&self) -> usize {
        self.patch.len()
    }

    /// The file whose (new) path is `path`.
    pub fn file(&self, path: &str) -> Option<&FilePatch> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Lines added and removed across every file.
    pub fn totals(&self) -> (u32, u32) {
        self.files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions))
    }

    pub fn source(&self, side: Side) -> &SideSource {
        match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        }
    }

    /// The full content of `path` on `side`; `None` when that side has no such file.
    /// Content that is not UTF-8 is read lossily.
    pub fn read_side(&self, side: Side, path: &str) -> Result<Option<String>> {
        let spec = match self.source(side) {
            SideSource::Worktree => {
                return match std::fs::read(self.root.join(path)) {
                    Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(source) => Err(Error::Io { action: "reading", path: self.root.join(path), source }),
                };
            }
            SideSource::Index => format!(":{path}"),
            SideSource::Rev(rev) => format!("{rev}:{path}"),
        };
        let mut exists = git::command(&self.root);
        exists.args(["cat-file", "-e", "--end-of-options", &spec]);
        if !git::succeeds(exists)? {
            return Ok(None);
        }
        let mut cmd = git::command(&self.root);
        cmd.args(["cat-file", "blob", "--end-of-options", &spec]);
        Ok(Some(String::from_utf8_lossy(&git::run(cmd, "cat-file blob")?).into_owned()))
    }

    /// Resolve every note against this capture, reading each (side, file) once.
    pub fn resolve_all(&self, notes: &[CodeAnnotation]) -> Result<Vec<Resolution>> {
        let mut contents: HashMap<(Side, String), Option<String>> = HashMap::new();
        let mut out = Vec::with_capacity(notes.len());
        for note in notes {
            let patch = self.file(&note.file_path);
            // A file renamed in this diff is read from its old path on the old side.
            let file_on_side = match (note.side, patch.and_then(|p| p.old_path.as_deref())) {
                (Side::Old, Some(old)) => old.to_owned(),
                _ => note.side_path().to_owned(),
            };
            let key = (note.side, file_on_side);
            if !contents.contains_key(&key) {
                let content = self.read_side(key.0, &key.1)?;
                contents.insert(key.clone(), content);
            }
            let content = contents.get(&key).and_then(Option::as_deref);
            out.push(resolve::resolve(note, &Current { content, patch, patch_key: &self.patch_key }));
        }
        Ok(out)
    }
}

/// The browser's `contentHash`: sha256 hex, first 16 characters.
pub fn patch_key(patch: &[u8]) -> String {
    crate::sha256_hex(&[patch], 16)
}

#[cfg(test)]
mod tests;
