//! Capturing a scope's patch from git, and reading either side's full files.
//!
//! | Scope | Compares | Command |
//! |---|---|---|
//! | Uncommitted | HEAD vs working tree, untracked included | `git diff <HEAD>` on a temp index |
//! | Staged | HEAD vs index | `git diff --cached <HEAD>` |
//! | Unstaged | index vs working tree, untracked included | `git diff` on a temp index |
//! | Branch | merge-base(base, HEAD) vs working tree, untracked included | `git diff <merge-base>` on a temp index |
//! | Commit | first parent vs commit | `git diff <sha>^ <sha>` |
//! | Pull request | merge-base(base, head) vs head | `gh pr view`, `git fetch`, `git diff <merge-base> <head>` |
//!
//! Untracked files are marked intent-to-add in a temporary copy of the index
//! (`GIT_INDEX_FILE`); the real index is never written. With no HEAD yet (a new repo) the
//! empty tree stands in for it, as for a root commit's parent.
//!
//! A pull request ([`Repo::pull_request`]) is looked up with `gh pr view` and its head
//! fetched with `git fetch <remote> refs/pull/<n>/head` into a private ref,
//! `refs/plannotator-tui/pull/<n>/head`; no branch, index or working tree is touched, and
//! both sides are read from commits. The fetch goes into the person's clone when the folder
//! the review runs in is one (a remote names the PR's repository), else into a bare cache
//! repository per GitHub repository under the data dir, fetched with `gh` as the credential
//! helper.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::annotation::{CodeAnnotation, NoteScope, Side};
use crate::error::{Error, Result};
use crate::git::{self, TempIndex};
use crate::github::{Gh, PrRepo, PrSpec, PrView};
use crate::patch::{self, FilePatch};
use crate::resolve::{self, Current, Resolution};
use crate::scope::Scope;

/// The largest patch a review opens: diffs-rs needs 540 to 800 MiB for its largest
/// measured patches.
pub const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;

/// The largest file a whole-file view shows, per side; the browser's cap
/// (`MAX_REVIEW_FILE_CONTENT_BYTES`).
pub const MAX_WHOLE_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// One side of one file, for a whole-file view ([`Capture::read_whole`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WholeFile {
    /// The file does not exist on that side: added (no base) or deleted (no current file).
    Missing,
    /// Over [`MAX_WHOLE_FILE_BYTES`].
    TooLarge { bytes: u64 },
    /// A NUL byte in its first 8000 bytes, git's test for binary content.
    Binary,
    /// Its text, read lossily when it is not UTF-8.
    Text(String),
}

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

/// A git work tree, found from any folder inside it; for a pull request, the repository its
/// commits are fetched into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
    pull: Option<PullSource>,
}

/// Where a pull request review fetches from, so a reload can fetch again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PullSource {
    gh: Gh,
    /// A remote of the person's clone, or the clone URL when fetching into the cache.
    remote: String,
    /// Fetching into the cache repository, with `gh` as the credential helper.
    cache: bool,
    /// Where the review was opened: the clone's root, or the folder it ran in for the cache.
    workdir: PathBuf,
}

/// The pull request a capture shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// What `gh pr view` said: repository, number, link, title, branches and commits.
    pub view: PrView,
    /// The head commit fetched (gh's `headRefOid`, or newer when a push landed in between).
    pub head: String,
    /// The base commit the diff starts from: merge-base(base, head).
    pub merge_base: String,
    /// The person's clone it was fetched into; `None` for the cache repository.
    pub clone: Option<PathBuf>,
}

/// What the person asked to review, before refs are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Uncommitted changes; when there are none and the base resolves, the branch instead.
    Uncommitted {
        base: Option<String>,
    },
    Staged,
    Unstaged,
    /// Against `base`, else [`DEFAULT_BASE`].
    Branch {
        base: Option<String>,
    },
    /// One commit, by any revision git understands (`HEAD`, a short sha).
    Commit {
        rev: String,
    },
    /// A GitHub pull request: a link, `N` or `#N`. Opened with [`Repo::pull_request`].
    PullRequest {
        pr: String,
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
    /// A pull request review's PR.
    pub pull: Option<PullRequest>,
}

impl Repo {
    /// The work tree containing `dir`.
    pub fn discover(dir: &Path) -> Result<Self> {
        let mut cmd = git::command(dir);
        cmd.args(["rev-parse", "--show-toplevel"]);
        match git::run_text(cmd, "rev-parse --show-toplevel") {
            Ok(root) if !root.is_empty() => Ok(Self { root: PathBuf::from(root), pull: None }),
            Ok(_) => Err(Error::NotARepo { dir: dir.to_path_buf(), detail: "no work tree".into() }),
            Err(Error::Git { detail, .. }) => Err(Error::NotARepo { dir: dir.to_path_buf(), detail }),
            Err(other) => Err(other),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder the review belongs to, for the agent and Pi: the work tree, or for a pull
    /// request fetched into the cache, the folder the review was opened in.
    pub fn workdir(&self) -> &Path {
        self.pull.as_ref().map_or(&self.root, |p| &p.workdir)
    }

    /// Open the pull request `text` names (a link, `N` or `#N`) from the folder `dir`: look
    /// it up with `gh`, fetch its head into `dir`'s clone when a remote of it names the PR's
    /// repository, else into a bare repository under `cache`, and capture its diff. A bare
    /// number needs `dir` to be a clone with a GitHub remote.
    pub fn pull_request(dir: &Path, text: &str, cache: &Path, gh: &Gh) -> Result<(Self, Capture)> {
        let spec = PrSpec::parse(text).ok_or_else(|| Error::NotAPull { text: text.to_owned() })?;
        let local = Self::discover(dir).ok();
        if matches!(spec, PrSpec::Number(_)) && local.is_none() {
            return Err(Error::NoGitHubRemote {
                detail: format!("{} is not inside a git repository", dir.display()),
            });
        }
        let view = gh.view(&spec, local.as_ref().map_or(dir, Self::root))?;
        let clone = local.and_then(|repo| repo.remote_for(&view.repo).map(|remote| (repo, remote)));
        let source = |remote, cache, workdir| Some(PullSource { gh: gh.clone(), remote, cache, workdir });
        let repo = if let Some((repo, remote)) = clone {
            let workdir = repo.root.clone();
            Self { root: repo.root, pull: source(remote, false, workdir) }
        } else {
            let root = cache_repo(cache, &view.repo)?;
            Self { root, pull: source(view.repo.clone_url(), true, dir.to_path_buf()) }
        };
        let capture = repo.capture_pull(view)?;
        Ok((repo, capture))
    }

    /// The first remote (by name) whose URL names `repo`, as configured (before `insteadOf`).
    fn remote_for(&self, repo: &PrRepo) -> Option<String> {
        let mut cmd = git::command(&self.root);
        cmd.args(["config", "--get-regexp", r"^remote\..*\.url$"]);
        let listed = git::run_text(cmd, "config --get-regexp remote").ok()?;
        let mut names: Vec<&str> = listed
            .lines()
            .filter_map(|line| line.split_once(' '))
            .filter(|(_, url)| repo.matches_remote(url.trim()))
            .filter_map(|(key, _)| key.strip_prefix("remote.")?.strip_suffix(".url"))
            .collect();
        names.sort_unstable();
        names.first().map(|n| (*n).to_owned())
    }

    /// Fetch the PR `view` describes and capture merge-base(base, head) against head.
    fn capture_pull(&self, view: PrView) -> Result<Capture> {
        let source = self.pull.as_ref().ok_or_else(|| Error::Git {
            command: "fetch".into(),
            detail: "this review was not opened for a pull request".into(),
        })?;
        let n = view.number;
        let head_ref = format!("refs/plannotator-tui/pull/{n}/head");
        let base_ref = format!("refs/plannotator-tui/pull/{n}/base");
        self.fetch(source, &[&format!("+refs/pull/{n}/head:{head_ref}")])?;
        let head = self.commit(&head_ref)?;
        // The base commit gh named is usually here already (it is on the base branch); else
        // fetch the base branch, then the commit itself; else diff against the branch's tip.
        if self.commit(&view.base_sha).is_err() {
            let branch = format!("+refs/heads/{}:{base_ref}", view.base_ref);
            if self.fetch(source, &[&branch]).is_err() || self.commit(&view.base_sha).is_err() {
                let _ = self.fetch(source, &[&view.base_sha]);
            }
        }
        let base = self.commit(&view.base_sha).or_else(|_| self.commit(&base_ref)).map_err(|_| {
            Error::NoBase { base: format!("{} ({}) of PR #{n}", view.base_ref, short(&view.base_sha)) }
        })?;
        let merge_base = self.merge_base_of(&base, &head)?;
        let patch = Self::diff(git::command(&self.root), &[], &[&merge_base, &head], MAX_PATCH_BYTES)?;
        let files = patch::parse(&String::from_utf8_lossy(&patch));
        let scope = Scope::PullRequest { url: view.url.clone(), number: n };
        let clone = (!source.cache).then(|| self.root.clone());
        Ok(Capture {
            scope,
            root: self.root.clone(),
            patch_key: patch_key(&patch),
            patch,
            files,
            old: SideSource::Rev(merge_base.clone()),
            new: SideSource::Rev(head.clone()),
            commit_subject: None,
            pull: Some(PullRequest { view, head, merge_base, clone }),
        })
    }

    /// `git fetch` from the PR's remote: no tags, no `FETCH_HEAD`, never a prompt.
    fn fetch(&self, source: &PullSource, refspecs: &[&str]) -> Result<()> {
        let mut cmd = git::command(&self.root);
        if source.cache && source.remote.starts_with("https://") {
            // A private repository's objects with the person's gh login, as `gh repo clone` does.
            let program = source.gh.program().to_string_lossy().replace('\'', r"'\''");
            cmd.args(["-c", "credential.helper="]);
            cmd.arg("-c").arg(format!("credential.helper=!'{program}' auth git-credential"));
        }
        cmd.args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--end-of-options",
            &source.remote,
        ])
        .args(refspecs)
        .env("GIT_TERMINAL_PROMPT", "0");
        git::run(cmd, &format!("fetch {}", source.remote)).map(drop)
    }

    fn merge_base_of(&self, base: &str, head: &str) -> Result<String> {
        let mut cmd = git::command(&self.root);
        cmd.args(["merge-base", "--end-of-options", base, head]);
        git::run_text(cmd, "merge-base")
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
            Request::Unstaged => self.capture(&Scope::Unstaged),
            Request::Branch { base } => self.capture(&self.branch_scope(base.as_deref())?),
            Request::Commit { rev } => {
                let sha = self.commit(rev).map_err(|_| Error::Git {
                    command: format!("rev-parse {rev}"),
                    detail: format!("{rev} does not name a commit"),
                })?;
                self.capture(&Scope::Commit { sha })
            }
            // Where a PR is fetched depends on the PR, so it is not this repository's to open.
            Request::PullRequest { pr } => Err(Error::Git {
                command: "fetch".into(),
                detail: format!("open pull request {pr} with Repo::pull_request"),
            }),
        }
    }

    /// Capture exactly `scope` (no fallback): what a reload runs. A pull request is looked up
    /// and fetched again, so new pushes show.
    pub fn capture(&self, scope: &Scope) -> Result<Capture> {
        if let Scope::PullRequest { url, number } = scope {
            let source = self.pull.as_ref().ok_or_else(|| Error::NotAPull { text: url.clone() })?;
            let spec = PrSpec::parse(url).unwrap_or(PrSpec::Number(*number));
            let view = source.gh.view(&spec, &source.workdir)?;
            return self.capture_pull(view);
        }
        let (old, new, patch) = self.scope_diff(scope, &[], MAX_PATCH_BYTES)?;
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
            pull: None,
        })
    }

    /// `git diff` for a local `scope` with `extra` flags, as [`Self::capture`] runs it: the two
    /// sides it compares and git's output, refused past `limit` bytes. A pull request is not
    /// local: [`Self::capture`] fetches it.
    pub(crate) fn scope_diff(
        &self,
        scope: &Scope,
        extra: &[&str],
        limit: usize,
    ) -> Result<(SideSource, SideSource, Vec<u8>)> {
        Ok(match scope {
            Scope::Uncommitted => {
                let head = self.head_or_empty_tree()?;
                let index = TempIndex::with_untracked(&self.root)?;
                let patch = Self::diff(index.command(&self.root), extra, &[&head], limit)?;
                (SideSource::Rev(head), SideSource::Worktree, patch)
            }
            Scope::Staged => {
                let head = self.head_or_empty_tree()?;
                let flags = [&["--cached"], extra].concat();
                let patch = Self::diff(git::command(&self.root), &flags, &[&head], limit)?;
                (SideSource::Rev(head), SideSource::Index, patch)
            }
            Scope::Unstaged => {
                // The real index is the old side; the copy only adds the untracked files to it.
                let index = TempIndex::with_untracked(&self.root)?;
                let patch = Self::diff(index.command(&self.root), extra, &[], limit)?;
                (SideSource::Index, SideSource::Worktree, patch)
            }
            Scope::Branch { base } => {
                let merge_base = self.merge_base(base)?;
                let index = TempIndex::with_untracked(&self.root)?;
                let patch = Self::diff(index.command(&self.root), extra, &[&merge_base], limit)?;
                (SideSource::Rev(merge_base), SideSource::Worktree, patch)
            }
            Scope::Commit { sha } => {
                let parent = match self.commit(&format!("{sha}^")) {
                    Ok(parent) => parent,
                    Err(_) => self.empty_tree()?,
                };
                let patch = Self::diff(git::command(&self.root), extra, &[&parent, sha], limit)?;
                (SideSource::Rev(parent), SideSource::Rev(sha.clone()), patch)
            }
            Scope::PullRequest { url, .. } => return Err(Error::NotAPull { text: url.clone() }),
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

    fn diff(mut cmd: Command, extra: &[&str], revs: &[&str], limit: usize) -> Result<Vec<u8>> {
        cmd.arg("diff").args(DIFF_FLAGS).args(extra).arg("--end-of-options").args(revs).arg("--");
        git::run_bounded(cmd, "diff", limit)
    }

    /// The full id of the commit `rev` names. A leading `-` is refused rather than handed to
    /// git, where it would read as an option (`rev-parse` has no portable `--end-of-options`).
    pub(crate) fn commit(&self, rev: &str) -> Result<String> {
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

    pub(crate) fn merge_base(&self, base: &str) -> Result<String> {
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
    /// The project and repository this capture's notes are saved under (see
    /// [`crate::ReviewStore::load`]): the work tree's name and root, or for a pull request its
    /// GitHub repository's name and link (`r`, `https://github.com/o/r`), so a PR's notes are
    /// the same whichever clone, or the cache, it was fetched into.
    pub fn record_owner(&self) -> (String, String) {
        match &self.pull {
            Some(pull) => {
                (crate::store::project_for(Path::new(&pull.view.repo.name)), pull.view.repo.web_url())
            }
            None => (crate::store::project_for(&self.root), self.root.display().to_string()),
        }
    }

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

    /// The whole of `path` on `side`, for a whole-file view: its text, or why there is none.
    /// The size is checked before anything is read.
    pub fn read_whole(&self, side: Side, path: &str) -> Result<WholeFile> {
        let bytes = match self.source(side) {
            SideSource::Worktree => {
                let full = self.root.join(path);
                let bytes = match std::fs::metadata(&full) {
                    Ok(meta) if meta.is_file() => meta.len(),
                    Ok(_) => return Ok(WholeFile::Missing),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(WholeFile::Missing),
                    Err(source) => return Err(Error::Io { action: "reading", path: full, source }),
                };
                if bytes > MAX_WHOLE_FILE_BYTES {
                    return Ok(WholeFile::TooLarge { bytes });
                }
                std::fs::read(&full).map_err(|source| Error::Io { action: "reading", path: full, source })?
            }
            source => {
                let spec = match source {
                    SideSource::Rev(rev) => format!("{rev}:{path}"),
                    SideSource::Index | SideSource::Worktree => format!(":{path}"),
                };
                let mut kind = git::command(&self.root);
                kind.args(["cat-file", "-t", "--end-of-options", &spec]);
                if !matches!(git::run_text(kind, "cat-file -t"), Ok(kind) if kind == "blob") {
                    return Ok(WholeFile::Missing);
                }
                let mut measure = git::command(&self.root);
                measure.args(["cat-file", "-s", "--end-of-options", &spec]);
                let bytes: u64 = git::run_text(measure, "cat-file -s")?.parse().unwrap_or(u64::MAX);
                if bytes > MAX_WHOLE_FILE_BYTES {
                    return Ok(WholeFile::TooLarge { bytes });
                }
                let mut cmd = git::command(&self.root);
                cmd.args(["cat-file", "blob", "--end-of-options", &spec]);
                git::run(cmd, "cat-file blob")?
            }
        };
        if bytes.iter().take(8000).any(|b| *b == 0) {
            return Ok(WholeFile::Binary);
        }
        Ok(WholeFile::Text(String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// Resolve every note against this capture, reading each (side, file) once.
    pub fn resolve_all(&self, notes: &[CodeAnnotation]) -> Result<Vec<Resolution>> {
        let mut contents: HashMap<(Side, String), Option<String>> = HashMap::new();
        let mut out = Vec::with_capacity(notes.len());
        for note in notes {
            // A note on the whole change has no file to read (its path is empty, which would
            // read the work tree's folder or a commit's tree).
            if note.scope == NoteScope::General {
                let current = Current { content: None, patch: None, patch_key: &self.patch_key };
                out.push(resolve::resolve(note, &current));
                continue;
            }
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

/// The bare repository under `cache` that `repo`'s pull requests are fetched into, made on
/// first use: `<cache>/<host>/<owner>/<name>.git`.
fn cache_repo(cache: &Path, repo: &PrRepo) -> Result<PathBuf> {
    let path = cache
        .join(path_safe(&repo.host))
        .join(path_safe(&repo.owner))
        .join(format!("{}.git", path_safe(&repo.name)));
    if !path.join("HEAD").is_file() {
        std::fs::create_dir_all(&path).map_err(|source| Error::Io {
            action: "creating the pull request cache",
            path: path.clone(),
            source,
        })?;
        let mut cmd = git::command(&path);
        cmd.args(["init", "--quiet", "--bare"]);
        git::run(cmd, "init --bare")?;
    }
    Ok(path)
}

/// A path component with anything but `[A-Za-z0-9._-]` as `-`, and no leading dot.
fn path_safe(part: &str) -> String {
    let cleaned: String = part
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    let cleaned = cleaned.trim_start_matches('.');
    if cleaned.is_empty() { "-".to_owned() } else { cleaned.to_owned() }
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The browser's `contentHash`: sha256 hex, first 16 characters.
pub fn patch_key(patch: &[u8]) -> String {
    crate::sha256_hex(&[patch], 16)
}

#[cfg(all(test, unix))]
mod pull_tests;
#[cfg(test)]
mod tests;
