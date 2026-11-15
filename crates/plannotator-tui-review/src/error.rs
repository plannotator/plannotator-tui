//! The crate's one error type. Every message names what was being done, so the app can show
//! it to the person as is.

use std::fmt;
use std::path::PathBuf;

/// Something capture, side reading or the review store could not do.
#[derive(Debug)]
pub enum Error {
    /// `git` could not be started at all (not installed, not on `PATH`).
    GitMissing(std::io::Error),
    /// The folder is not inside a git work tree.
    NotARepo { dir: PathBuf, detail: String },
    /// A git command failed. `command` is the subcommand line, `detail` git's own message.
    Git { command: String, detail: String },
    /// The patch is over the size limit, so the review does not open.
    TooLarge { limit: usize },
    /// A branch review needs a base, and this one does not resolve to a commit.
    NoBase { base: String },
    /// A file in the review store could not be read or written.
    Io { action: &'static str, path: PathBuf, source: std::io::Error },
    /// `review.json` is not valid.
    Json { path: PathBuf, source: serde_json::Error },
    /// Archiving would overwrite an archived note with the same id.
    ArchiveConflict { id: String },
    /// A pull request review needs `gh`, and it is not installed.
    GhMissing,
    /// `gh` is installed but not logged in. `detail` is gh's message.
    GhNotLoggedIn { detail: String },
    /// `gh` found no such pull request. `what` names it (`PR #12 in o/r`).
    PullNotFound { what: String, detail: String },
    /// A PR number was given in a folder whose repository has no GitHub remote.
    NoGitHubRemote { detail: String },
    /// `gh` failed some other way.
    Gh { detail: String },
    /// The text is no PR link or number.
    NotAPull { text: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitMissing(source) => write!(f, "could not run git: {source}"),
            Self::NotARepo { dir, detail } => {
                write!(f, "{} is not inside a git repository ({detail})", dir.display())
            }
            Self::Git { command, detail } => write!(f, "git {command} failed: {detail}"),
            Self::TooLarge { limit } => write!(
                f,
                "the diff is larger than {} MiB, which is too large to review here (a diff that size \
                 needs hundreds of MiB of memory to show); review a smaller scope, such as staged \
                 changes or one commit",
                limit / (1024 * 1024)
            ),
            Self::NoBase { base } => {
                write!(f, "the branch base {base} does not name a commit; pass --base REF")
            }
            Self::Io { action, path, source } => write!(f, "{action} {}: {source}", path.display()),
            Self::Json { path, source } => write!(f, "parsing {}: {source}", path.display()),
            Self::ArchiveConflict { id } => {
                write!(f, "cannot archive note {id}: an archived note has the same id")
            }
            Self::GhMissing => write!(
                f,
                "reviewing a pull request needs the GitHub CLI (gh), and it is not installed; install it \
                 from https://cli.github.com and run `gh auth login`"
            ),
            Self::GhNotLoggedIn { detail } => {
                write!(f, "gh is not logged in to GitHub; run `gh auth login` ({detail})")
            }
            Self::PullNotFound { what, detail } => write!(f, "{what} not found ({detail})"),
            Self::NoGitHubRemote { detail } => write!(
                f,
                "a PR number is looked up in this folder's GitHub repository, and there is none here \
                 ({detail}); pass the PR's link instead"
            ),
            Self::Gh { detail } => write!(f, "gh pr view failed: {detail}"),
            Self::NotAPull { text } => write!(
                f,
                "{text:?} is not a pull request; give a link (https://github.com/OWNER/REPO/pull/N), N or #N"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::GitMissing(source) | Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
