//! Viewed marks, in the browser app's review-progress format, so a review started in one
//! continues in the other (`docs/spec-review-progress.md`).
//!
//! Ported from Plannotator: `packages/shared/review-progress.ts` (the key, the records, how
//! they load), `packages/shared/review-core.ts` (`resolveReviewFileIdentities`, the file
//! fingerprint) and `packages/server/review.ts` (which diff type and base feed the key).
//!
//! - One record per file and review: `<data-dir>/review-progress/<key>/<sha256(path)>.json`
//!   holding `{"v":1,"path":…,"fingerprint":…,"viewed":…,"suppressed":…}`, replaced atomically
//!   (a temp file, then rename), so two processes never corrupt one; the last write wins.
//! - `key = sha256(JSON.stringify(["git", realpath(root), branch ref or HEAD sha,
//!   [diffType, base or null]]))`.
//! - `fingerprint = sha256(JSON.stringify([oldPath, newPath, oldMode, newMode, oldOid,
//!   newOid]))` from `git diff --raw`, a working-tree side hashed with `git hash-object`. A
//!   mark counts only while its fingerprint matches, so editing the file drops it.
//!
//! The JSON is built with `serde_json`, which writes what `JSON.stringify` writes for arrays
//! of strings and nulls: no spaces, `null` for a missing path, the same escapes (`\"`, `\\`,
//! `\n`, `\u001f` in lowercase hex), non-ASCII left as is.
//!
//! A pull request review keeps its marks apart, under
//! `<data-dir>/clients/plannotator-tui/review-progress/`: the browser fingerprints a PR's
//! files from GitHub's patch text, which this app never sees byte for byte, so a shared
//! record could never match.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use serde_json::{Value, json};

use crate::capture::Capture;
use crate::error::{Error, Result};
use crate::git;
use crate::patch;
use crate::scope::Scope;

/// The records' folder under the data dir (`review-progress`).
pub const PROGRESS_DIR: &str = "review-progress";

/// The browser's `MAX_REVIEW_FILE_CONTENT_BYTES`: a side larger than this is shown as a
/// stub, has no fingerprint, and is viewed for the session only.
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// The browser renders every review diff with `core.bigFileThreshold` at the size cap; the
/// raw listing here does too, so rename detection pairs the same files.
const BIG_FILE_THRESHOLD: &str = "core.bigFileThreshold=5242880";

/// Whether marks are saved: `PLANNOTATOR_REVIEW_PROGRESS` (`0`/`false`/`off`/`disabled`, or
/// `1`/`true`/`on`), else `reviewProgress` in the data dir's `config.json`, else on. The
/// browser's `resolveReviewProgress`. Off, marks last for the session only.
pub fn enabled(env: Option<&str>, config_json: Option<&str>) -> bool {
    match env.map(|v| v.trim().to_lowercase()).as_deref() {
        Some("0" | "false" | "off" | "disabled") => return false,
        Some("1" | "true" | "on") => return true,
        _ => {}
    }
    let config = config_json
        .map(|text| text.trim_start_matches('\u{feff}'))
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    match config.as_ref().and_then(|c| c.get("reviewProgress")) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !matches!(s.trim().to_lowercase().as_str(), "false" | "0"),
        _ => true,
    }
}

/// `sha256(text)` as 64 lowercase hex digits: the browser's `hash`.
pub fn sha256(text: &str) -> String {
    crate::sha256_hex(&[text.as_bytes()], 64)
}

/// The review key for an identity array: `sha256(JSON.stringify(identity))`.
pub fn review_key(identity: &Value) -> String {
    sha256(&identity.to_string())
}

/// A local review's identity, as `localGitReviewIdentity` builds it:
/// `["git", realpath(root), "refs/heads/<branch>" or the HEAD sha, [diffType, base or null]]`.
/// The base is kept only for the diff types that compare against one (`since-base`, `branch`,
/// `merge-base`). `None` for a pull request, or when git cannot name the checkout.
pub fn local_identity(root: &Path, scope: &Scope) -> Option<Value> {
    if matches!(scope, Scope::PullRequest { .. }) {
        return None;
    }
    let diff_type = scope.diff_type();
    let base = match diff_type.as_str() {
        "since-base" | "branch" | "merge-base" => scope.base().map(str::to_owned),
        _ => None,
    };
    let text = |args: &[&str]| {
        let mut cmd = git::command(root);
        cmd.args(args);
        git::run_text(cmd, "rev-parse").ok().filter(|s| !s.is_empty())
    };
    let top = text(&["rev-parse", "--show-toplevel"])?;
    // Detached checkouts have no durable branch identity: the commit stands in.
    let head = text(&["symbolic-ref", "--quiet", "HEAD"]).or_else(|| text(&["rev-parse", "HEAD"]))?;
    Some(json!(["git", canonical(Path::new(&top)), head, [diff_type, base]]))
}

/// A pull request review's identity: the browser's `["pr", url, scope]` with its default
/// `layer` scope. Kept in this app's own folder (see the module docs).
pub fn pull_identity(url: &str) -> Value {
    json!(["pr", url, "layer"])
}

/// Node's `realpathSync`, else the path as given (git prints it absolute).
fn canonical(path: &Path) -> String {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = real.to_string_lossy().into_owned();
    // Windows: `canonicalize` adds the verbatim prefix `realpathSync` does not.
    match text.strip_prefix(r"\\?\") {
        Some(rest) if cfg!(windows) && !rest.starts_with("UNC\\") => rest.to_owned(),
        _ => text,
    }
}

/// One `git diff --raw` record (`parseRawDiffEntries`): the old path is `None` for an added
/// file, the new path `None` for a deleted one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub old_mode: String,
    pub new_mode: String,
    pub old_oid: String,
    pub new_oid: String,
    pub status: String,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
}

impl RawEntry {
    /// The browser's key for the file: the new path, else the old one.
    pub fn path(&self) -> Option<&str> {
        self.new_path.as_deref().or(self.old_path.as_deref())
    }

    fn is_gitlink(&self) -> bool {
        self.old_mode == "160000" || self.new_mode == "160000"
    }

    /// The file identity: `JSON.stringify([oldPath, newPath, oldMode, newMode, oldOid,
    /// newOid])`, with `new_oid` the hashed working-tree id where git printed zeros.
    pub fn identity(&self, new_oid: &str) -> String {
        json!([self.old_path, self.new_path, self.old_mode, self.new_mode, self.old_oid, new_oid]).to_string()
    }
}

/// The records of `git diff --raw -z --no-abbrev` output. Malformed records end the list.
pub fn parse_raw(output: &[u8]) -> Vec<RawEntry> {
    let text = String::from_utf8_lossy(output);
    let mut fields = text.split('\0');
    let mut entries = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let Some(meta) = header.strip_prefix(':') else { break };
        let parts: Vec<&str> = meta.split(' ').collect();
        let [old_mode, new_mode, old_oid, new_oid, status] = parts.as_slice() else { break };
        if status.is_empty() {
            break;
        }
        let renamed = status.starts_with('R') || status.starts_with('C');
        let Some(old) = fields.next().filter(|p| !p.is_empty()) else { break };
        let new = if renamed { fields.next().filter(|p| !p.is_empty()) } else { Some(old) };
        let Some(new) = new else { break };
        entries.push(RawEntry {
            old_mode: (*old_mode).to_owned(),
            new_mode: (*new_mode).to_owned(),
            old_oid: (*old_oid).to_owned(),
            new_oid: (*new_oid).to_owned(),
            status: (*status).to_owned(),
            old_path: (!status.starts_with('A')).then(|| old.to_owned()),
            new_path: (!status.starts_with('D')).then(|| new.to_owned()),
        });
    }
    entries
}

fn is_null_oid(oid: &str) -> bool {
    !oid.is_empty() && oid.bytes().all(|b| b == b'0')
}

fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Every file's identity for `capture`, by path, as the browser records them when it builds
/// the same diff: `runGitDiff` for the scope's diff type (the tracked diff, then untracked
/// files one by one), `resolveReviewFileIdentities` for the working-tree hashes. Files the
/// browser leaves without one get none here either: submodules, a side over
/// [`MAX_FILE_BYTES`], a working-tree file that changed since the capture (its hash no longer
/// matches the patch's `index` line), paths `hash-object --stdin-paths` cannot take.
pub fn file_identities(capture: &Capture) -> BTreeMap<String, String> {
    let root = capture.root.as_path();
    let shown = patch::new_side_ids(&String::from_utf8_lossy(&capture.patch));
    let (tracked, untracked) = browser_entries(root, capture);
    let mut entries: Vec<RawEntry> = tracked.into_iter().filter(|e| !e.is_gitlink()).collect();
    entries.retain(|e| e.path().is_some_and(|p| shown.contains_key(p)));
    let sizes = object_sizes(root, &entries);
    let mut identities = BTreeMap::new();
    let mut pending: Vec<RawEntry> = Vec::new();
    for entry in entries {
        let old_size = sizes.get(&entry.old_oid).copied();
        let new_size = sizes
            .get(&entry.new_oid)
            .copied()
            .or_else(|| entry.new_path.as_deref().and_then(|p| worktree_info(root, p)).map(|info| info.size));
        if old_size.unwrap_or(0) > MAX_FILE_BYTES || new_size.unwrap_or(0) > MAX_FILE_BYTES {
            continue; // the browser shows a stub with no identity
        }
        match (&entry.new_path, is_null_oid(&entry.new_oid)) {
            (Some(_), true) => pending.push(entry),
            _ => {
                if let Some(path) = entry.path() {
                    identities.insert(path.to_owned(), entry.identity(&entry.new_oid));
                }
            }
        }
    }
    // Untracked files come after the tracked diff and replace a tracked deletion of the
    // same path, as `removeTrackedDeletions` does to the patch.
    let zero = zero_oid(root);
    for path in untracked {
        if !shown.contains_key(&path) {
            continue;
        }
        identities.remove(&path);
        pending.retain(|e| e.path() != Some(path.as_str()));
        let Some(info) = worktree_info(root, &path) else { continue };
        if info.size > MAX_FILE_BYTES {
            continue;
        }
        pending.push(RawEntry {
            old_mode: "000000".into(),
            new_mode: info.mode.into(),
            old_oid: zero.clone(),
            new_oid: zero.clone(),
            status: "A".into(),
            old_path: None,
            new_path: Some(path),
        });
    }
    for (entry, id) in hash_worktree(root, &pending) {
        let Some(path) = entry.path() else { continue };
        // The displayed patch's index line must name the same content, so an edit between
        // the diff and the hash cannot mark a newer version viewed.
        let matches = match shown.get(path) {
            Some(Some(abbrev)) => id.starts_with(abbrev.as_str()),
            Some(None) => id == entry.old_oid,
            None => false,
        };
        if matches {
            identities.insert(path.to_owned(), entry.identity(&id));
        }
    }
    identities
}

/// The browser's raw records for the scope, and its untracked files (for the scopes that
/// show them). Any git failure leaves the list short, never the review broken.
fn browser_entries(root: &Path, capture: &Capture) -> (Vec<RawEntry>, Vec<String>) {
    let succeeds = |args: &[&str]| {
        let mut cmd = git::command(root);
        cmd.args(args);
        git::succeeds(cmd).unwrap_or(false)
    };
    let has_head = || succeeds(&["rev-parse", "--verify", "--quiet", "HEAD"]);
    let raw = |args: &[&str]| -> Vec<RawEntry> {
        let mut cmd = git::command(root);
        cmd.args(["-c", BIG_FILE_THRESHOLD, "diff", "--no-ext-diff", "--raw", "-z", "--no-abbrev"])
            .args(args);
        git::run(cmd, "diff --raw").map(|out| parse_raw(&out)).unwrap_or_default()
    };
    match &capture.scope {
        Scope::Uncommitted => {
            let tracked = if has_head() { raw(&["HEAD"]) } else { Vec::new() };
            (tracked, untracked_files(root))
        }
        Scope::Staged => (raw(&["--staged"]), Vec::new()),
        Scope::Branch { base } => {
            if !has_head() {
                return (Vec::new(), untracked_files(root));
            }
            let mut cmd = git::command(root);
            cmd.args(["merge-base", "--end-of-options", base, "HEAD"]);
            let merge_base = git::run_text(cmd, "merge-base").unwrap_or_else(|_| "HEAD".to_owned());
            (raw(&["--end-of-options", &merge_base]), untracked_files(root))
        }
        Scope::Commit { sha } => {
            let parent = format!("{sha}^");
            let base = if succeeds(&["rev-parse", "--verify", "--quiet", &parent]) {
                parent
            } else {
                let mut cmd = git::command(root);
                cmd.args(["hash-object", "-t", "tree", "--stdin"]);
                git::run_text(cmd, "hash-object -t tree").unwrap_or_default()
            };
            (raw(&["--end-of-options", &format!("{base}..{sha}")]), Vec::new())
        }
        Scope::PullRequest { .. } => match &capture.pull {
            Some(pull) => {
                (raw(&["--end-of-options", &format!("{}..{}", pull.merge_base, pull.head)]), Vec::new())
            }
            None => (Vec::new(), Vec::new()),
        },
    }
}

/// `git ls-files --others --exclude-standard`, without nested repositories (`dir/`), which
/// the browser's per-file `diff --no-index` cannot show.
fn untracked_files(root: &Path) -> Vec<String> {
    let mut cmd = git::command(root);
    cmd.args(["ls-files", "--others", "--exclude-standard", "-z"]);
    let Ok(out) = git::run(cmd, "ls-files --others") else { return Vec::new() };
    String::from_utf8_lossy(&out)
        .split('\0')
        .filter(|p| !p.is_empty() && !p.ends_with('/'))
        .map(str::to_owned)
        .collect()
}

/// The null object id in this repository's hash: 40 zeros, or 64 in a SHA-256 repository.
fn zero_oid(root: &Path) -> String {
    let mut cmd = git::command(root);
    cmd.args(["rev-parse", "--show-object-format"]);
    let width = if git::run_text(cmd, "rev-parse --show-object-format").is_ok_and(|f| f == "sha256") {
        64
    } else {
        40
    };
    "0".repeat(width)
}

/// Object sizes from one `cat-file --batch-check`; ids it cannot size are left out.
fn object_sizes(root: &Path, entries: &[RawEntry]) -> HashMap<String, u64> {
    let ids: BTreeSet<&str> = entries
        .iter()
        .flat_map(|e| [e.old_oid.as_str(), e.new_oid.as_str()])
        .filter(|id| !is_null_oid(id))
        .collect();
    if ids.is_empty() {
        return HashMap::new();
    }
    let input: String = ids.iter().flat_map(|id| [*id, "\n"]).collect();
    let mut cmd = git::command(root);
    cmd.args(["cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)"]);
    let Ok((_, out)) = run_with_input(cmd, input.as_bytes()) else { return HashMap::new() };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split(' ');
            let (id, kind, size) = (parts.next()?, parts.next()?, parts.next()?);
            (kind != "missing").then_some(())?;
            Some((id.to_owned(), size.parse().ok()?))
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct WorktreeInfo {
    size: u64,
    symlink: bool,
    /// The mode git gives the file: `100644`, `100755` or `120000`.
    mode: &'static str,
}

fn worktree_info(root: &Path, path: &str) -> Option<WorktreeInfo> {
    let meta = std::fs::symlink_metadata(root.join(path)).ok()?;
    let symlink = meta.file_type().is_symlink();
    if !symlink && !meta.is_file() {
        return None;
    }
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o100 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    let mode = if symlink {
        "120000"
    } else if executable {
        "100755"
    } else {
        "100644"
    };
    Some(WorktreeInfo { size: meta.len(), symlink, mode })
}

/// The working-tree object id of each pending entry's new path: regular files in one
/// `hash-object --stdin-paths` (clean filters apply, as for the diff), a symlink's target
/// text through `hash-object --stdin`. Entries that cannot be hashed are left out.
fn hash_worktree(root: &Path, pending: &[RawEntry]) -> Vec<(RawEntry, String)> {
    let mut regular: Vec<&RawEntry> = Vec::new();
    let mut hashed = Vec::new();
    for entry in pending {
        let Some(path) = entry.new_path.as_deref() else { continue };
        let Some(info) = worktree_info(root, path) else { continue };
        if info.size > MAX_FILE_BYTES {
            continue;
        }
        if info.symlink {
            if let Some(id) = hash_symlink(root, path) {
                hashed.push((entry.clone(), id));
            }
        } else if !path.contains(['\n', '\r']) && !path.starts_with('"') {
            regular.push(entry);
        }
    }
    if !regular.is_empty() {
        let input: String =
            regular.iter().filter_map(|e| e.new_path.as_deref()).flat_map(|p| [p, "\n"]).collect();
        let mut cmd = git::command(root);
        cmd.args(["hash-object", "--stdin-paths"]);
        // A path that vanished stops git there; the complete lines before it still hold.
        if let Ok((_, out)) = run_with_input(cmd, input.as_bytes()) {
            let text = String::from_utf8_lossy(&out);
            let complete = text.split_inclusive('\n').filter(|l| l.ends_with('\n'));
            for (entry, line) in regular.into_iter().zip(complete) {
                let id = line.trim();
                if is_object_id(id) {
                    hashed.push((entry.clone(), id.to_owned()));
                }
            }
        }
    }
    hashed
}

fn hash_symlink(root: &Path, path: &str) -> Option<String> {
    let target = std::fs::read_link(root.join(path)).ok()?;
    let mut cmd = git::command(root);
    cmd.args(["hash-object", "--stdin"]);
    let (ok, out) = run_with_input(cmd, target.as_os_str().as_encoded_bytes()).ok()?;
    let id = String::from_utf8_lossy(&out).trim().to_owned();
    (ok && is_object_id(&id)).then_some(id)
}

/// Run with `input` on stdin, written from its own thread so a large list cannot deadlock
/// against git's output. Returns whether it exited 0, and its stdout.
fn run_with_input(mut cmd: std::process::Command, input: &[u8]) -> Result<(bool, Vec<u8>)> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(Error::GitMissing)?;
    let writer = child.stdin.take().map(|mut stdin| {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out);
    }
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let status = child.wait().map_err(Error::GitMissing)?;
    Ok((status.success(), out))
}

/// The record: the browser's `ViewedRecord`, fields in its order.
#[derive(Debug, Serialize)]
struct Record<'a> {
    v: u8,
    path: &'a str,
    fingerprint: &'a str,
    viewed: bool,
    suppressed: bool,
}

/// `<dir>/<sha256(path)>.json`.
pub fn record_path(dir: &Path, path: &str) -> PathBuf {
    dir.join(format!("{}.json", sha256(path)))
}

/// Save one mark atomically: a temp file beside the record (mode 0600), then rename over it.
/// `suppressed` is `!viewed`, as the browser writes it.
pub fn save_mark(dir: &Path, path: &str, fingerprint: &str, viewed: bool) -> Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let io = |action: &'static str, at: &Path| {
        let at = at.to_path_buf();
        move |source| Error::Io { action, path: at, source }
    };
    std::fs::create_dir_all(dir).map_err(io("creating", dir))?;
    let file = record_path(dir, path);
    let record = Record { v: 1, path, fingerprint, viewed, suppressed: !viewed };
    let json = serde_json::to_string(&record).map_err(|source| Error::Json { path: file.clone(), source })?;
    let mut tmp = file.clone().into_os_string();
    tmp.push(format!(".{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let tmp = PathBuf::from(tmp);
    let written = write_private(&tmp, json.as_bytes())
        .map_err(io("writing", &tmp))
        .and_then(|()| std::fs::rename(&tmp, &file).map_err(io("replacing", &file)));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The paths among `fingerprints` whose record says viewed for that exact fingerprint
/// (`loadReviewProgress`). A missing, unreadable or malformed record is not viewed.
pub fn load_viewed(dir: &Path, fingerprints: &BTreeMap<String, String>) -> BTreeSet<String> {
    fingerprints
        .iter()
        .filter(|(path, fingerprint)| {
            let Ok(text) = std::fs::read_to_string(record_path(dir, path)) else { return false };
            let Ok(value) = serde_json::from_str::<Value>(&text) else { return false };
            let valid = value.get("v").and_then(Value::as_f64) == Some(1.0)
                && value.get("path").and_then(Value::as_str) == Some(path.as_str())
                && value.get("fingerprint").is_some_and(Value::is_string)
                && value.get("viewed").is_some_and(Value::is_boolean)
                && value.get("suppressed").is_some_and(Value::is_boolean);
            valid
                && value.get("fingerprint").and_then(Value::as_str) == Some(fingerprint.as_str())
                && value.get("viewed").and_then(Value::as_bool) == Some(true)
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// A review's viewed marks: the files' fingerprints for this capture, where their records
/// live, and which files are viewed now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// Whether marks are saved at all (the setting); kept for [`Self::reopen`].
    save: bool,
    /// The records' folder; `None` when marks are not saved (turned off, or git could not
    /// name the review).
    dir: Option<PathBuf>,
    /// `sha256(identity)` per file that has an identity.
    fingerprints: BTreeMap<String, String>,
    viewed: BTreeSet<String>,
}

impl Progress {
    /// The marks for `capture`, read from `data_dir`. `save` false keeps them in memory.
    pub fn open(capture: &Capture, data_dir: &Path, save: bool) -> Self {
        let dir = if save { Self::dir_for(capture, data_dir) } else { None };
        let fingerprints = if dir.is_some() {
            file_identities(capture).into_iter().map(|(path, identity)| (path, sha256(&identity))).collect()
        } else {
            BTreeMap::new()
        };
        let viewed = dir.as_deref().map(|d| load_viewed(d, &fingerprints)).unwrap_or_default();
        Self { save, dir, fingerprints, viewed }
    }

    /// The marks for a new capture of the same review (a reload). A file viewed for the
    /// session only stays viewed while it is still in the diff and still has no
    /// fingerprint; a file with one is viewed only if its record matches the new version.
    #[must_use]
    pub fn reopen(&self, capture: &Capture, data_dir: &Path) -> Self {
        let mut next = Self::open(capture, data_dir, self.save);
        for path in &self.viewed {
            if !next.fingerprints.contains_key(path) && capture.file(path).is_some() && !self.is_saved(path) {
                next.viewed.insert(path.clone());
            }
        }
        next
    }

    /// The records' folder for `capture`: `<data>/review-progress/<key>`, or this app's own
    /// folder for a pull request.
    fn dir_for(capture: &Capture, data_dir: &Path) -> Option<PathBuf> {
        if let Scope::PullRequest { url, .. } = &capture.scope {
            let key = review_key(&pull_identity(url));
            return Some(data_dir.join("clients").join("plannotator-tui").join(PROGRESS_DIR).join(key));
        }
        let identity = local_identity(&capture.root, &capture.scope)?;
        Some(data_dir.join(PROGRESS_DIR).join(review_key(&identity)))
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn fingerprint(&self, path: &str) -> Option<&str> {
        self.fingerprints.get(path).map(String::as_str)
    }

    /// Whether a mark on `path` is saved (it has a fingerprint and a folder to go to).
    pub fn is_saved(&self, path: &str) -> bool {
        self.dir.is_some() && self.fingerprints.contains_key(path)
    }

    pub fn is_viewed(&self, path: &str) -> bool {
        self.viewed.contains(path)
    }

    /// How many of `paths` are viewed.
    pub fn count<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> usize {
        paths.into_iter().filter(|p| self.viewed.contains(*p)).count()
    }

    /// Mark `path` viewed or not, and save the mark when it can be saved. The mark holds in
    /// memory even when the write fails; the error says why it will not outlast the session.
    pub fn set(&mut self, path: &str, viewed: bool) -> Result<()> {
        if viewed {
            self.viewed.insert(path.to_owned());
        } else {
            self.viewed.remove(path);
        }
        match (&self.dir, self.fingerprints.get(path)) {
            (Some(dir), Some(fingerprint)) => save_mark(dir, path, fingerprint, viewed),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
