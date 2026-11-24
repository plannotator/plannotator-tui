//! Running git: one command shape for every call, output bounded where it can be large, and
//! a temporary index for showing untracked files.
//!
//! Every call runs `git --no-optional-locks -c core.quotePath=false -C <root>`, so a read never
//! refreshes the real index and paths print as UTF-8.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::children::exec;
use crate::error::{Error, Result};

/// A git command rooted at `dir`.
pub(crate) fn command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("--no-optional-locks").args(["-c", "core.quotePath=false"]).arg("-C").arg(dir);
    cmd.stdin(Stdio::null());
    cmd
}

/// Run to completion; stdout on success, git's message on failure.
pub(crate) fn run(cmd: Command, what: &str) -> Result<Vec<u8>> {
    let ran = exec(cmd, what, None, None)?;
    if ran.status.success() { Ok(ran.stdout) } else { Err(failure(what, &ran.stderr)) }
}

/// Run and return trimmed stdout as text.
pub(crate) fn run_text(cmd: Command, what: &str) -> Result<String> {
    Ok(String::from_utf8_lossy(&run(cmd, what)?).trim().to_owned())
}

/// True when the command exits 0. Output is discarded.
pub(crate) fn succeeds(cmd: Command) -> Result<bool> {
    Ok(exec(cmd, "", None, None)?.status.success())
}

/// Run, reading at most `limit` bytes of stdout. Past the limit the command is stopped and
/// the result is [`Error::TooLarge`], so a huge diff never sits in memory.
pub(crate) fn run_bounded(cmd: Command, what: &str, limit: usize) -> Result<Vec<u8>> {
    let ran = exec(cmd, what, None, Some(limit))?;
    if ran.over {
        return Err(Error::TooLarge { limit });
    }
    if ran.status.success() { Ok(ran.stdout) } else { Err(failure(what, &ran.stderr)) }
}

fn failure(what: &str, stderr: &[u8]) -> Error {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    let detail = if detail.is_empty() { "no message".to_owned() } else { detail };
    Error::Git { command: what.to_owned(), detail }
}

/// A copy of the repository's index in a directory of its own under this process's folder in
/// the system temp dir ([`crate::children::temp_root`]), removed with it on drop. Commands given `GIT_INDEX_FILE` of this path never touch the real
/// `.git/index`.
///
/// The directory is created fresh, private to this user and never reused, so nobody else can
/// plant a file there or swap the index git reads.
#[derive(Debug)]
pub(crate) struct TempIndex {
    dir: PathBuf,
    path: PathBuf,
}

impl TempIndex {
    /// Copy the real index (when there is one) and mark every untracked, non-ignored file
    /// intent-to-add in the copy, so `git diff` against the working tree shows them as new.
    pub(crate) fn with_untracked(root: &Path) -> Result<Self> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir =
            crate::children::temp_root()?.join(format!("index-{}", COUNTER.fetch_add(1, Ordering::Relaxed)));
        let builder = &mut std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(builder, 0o700);
        builder.create(&dir).map_err(|source| Error::Io {
            action: "creating a directory for the index copy",
            path: dir.clone(),
            source,
        })?;
        let index = Self { path: dir.join("index"), dir };
        let mut cmd = command(root);
        cmd.args(["rev-parse", "--git-path", "index"]);
        let real = PathBuf::from(run_text(cmd, "rev-parse --git-path index")?);
        let real = if real.is_absolute() { real } else { root.join(real) };
        match std::fs::copy(&real, &index.path) {
            // Best effort: a read-only copy keeps a fresh mtime and only misses that rare edit.
            Ok(_) => drop(keep_mtime(&real, &index.path)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // a repo with nothing staged yet
            Err(source) => return Err(Error::Io { action: "copying the git index", path: real, source }),
        }
        index.add_untracked(root)?;
        Ok(index)
    }

    fn add_untracked(&self, root: &Path) -> Result<()> {
        let mut list = self.command(root);
        list.args(["ls-files", "--others", "--exclude-standard", "-z"]);
        let listed = run(list, "ls-files --others")?;
        // A nested repository lists as `dir/`; adding it would record a gitlink.
        let paths: Vec<&[u8]> =
            listed.split(|b| *b == 0).filter(|p| !p.is_empty() && !p.ends_with(b"/")).collect();
        if paths.is_empty() {
            return Ok(());
        }
        let mut add = self.command(root);
        add.args(["add", "--intent-to-add", "--pathspec-from-file=-", "--pathspec-file-nul"])
            .env("GIT_LITERAL_PATHSPECS", "1");
        let input = paths.iter().flat_map(|p| p.iter().copied().chain([0])).collect();
        let ran = exec(add, "add --intent-to-add", Some(input), None)?;
        if ran.status.success() { Ok(()) } else { Err(failure("add --intent-to-add", &ran.stderr)) }
    }

    /// A git command that reads and writes this index instead of the real one.
    pub(crate) fn command(&self, root: &Path) -> Command {
        let mut cmd = command(root);
        cmd.env("GIT_INDEX_FILE", &self.path);
        cmd
    }
}

/// Give `copy` the mtime of `original`. Git checks an entry's content, not just its cached
/// stat, only when the entry is no older than the index file; a fresh mtime on the copy would
/// make an edit in the second the index was written, of the same size, look unchanged.
fn keep_mtime(original: &Path, copy: &Path) -> std::io::Result<()> {
    let modified = std::fs::metadata(original)?.modified()?;
    std::fs::File::options().write(true).open(copy)?.set_modified(modified)
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// Unix only: the test reads the directory's mode bits.
#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::expect_used, reason = "tests assert by panicking")]
    #![allow(clippy::print_stderr, reason = "a skipped test says why")]

    use super::*;

    #[test]
    fn the_index_copy_sits_in_a_private_directory_of_its_own_that_drop_removes() {
        use std::os::unix::fs::PermissionsExt as _;
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is not installed; skipping");
            return;
        }
        let repo = std::env::temp_dir().join(format!("pt-review-git-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).expect("repo dir");
        let mut init = command(&repo);
        init.args(["init", "-q"]);
        assert!(succeeds(init).expect("git"), "git init");
        std::fs::write(repo.join("new.txt"), "new\n").expect("write");

        let index = TempIndex::with_untracked(&repo).expect("index copy");
        let dir = index.path.parent().expect("a parent").to_path_buf();
        assert_ne!(dir, std::env::temp_dir(), "not loose in the shared temp dir");
        let mode = std::fs::metadata(&dir).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        let parent = dir.parent().expect("the process's folder");
        assert_eq!(std::fs::metadata(parent).expect("metadata").permissions().mode() & 0o777, 0o700);
        assert!(
            parent.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("plannotator-tui-"))
        );
        assert!(index.path.is_file(), "git wrote the copy there");
        drop(index);
        assert!(!dir.exists(), "dropping the copy removes its directory");
        std::fs::remove_dir_all(&repo).expect("cleanup");
    }
}
