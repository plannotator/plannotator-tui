//! Where the change picker's rows come from, and the threads that ask.
//!
//! [`Source`] is the seam to git: the real one runs it ([`GitSource`]); a test stands in a
//! slow one to show the picker never waits for it. Every call after the picker's first frame
//! runs on a thread of its own ([`Work`]) and pokes the event loop's wake channel when done,
//! as the disk watch does, so no git command ever runs on the thread that draws.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};

use plannotator_tui_review::{Capture, Commit, DiffStat, Repo, ReviewStore, Scope, project_for};

/// Commits listed at a time.
pub(crate) const PAGE: usize = 20;

/// The rows a listing adds: the branch (on the first page) and a page of commits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Listing {
    pub(crate) branch: Option<Scope>,
    pub(crate) commits: Vec<Commit>,
    /// Older commits are left to list.
    pub(crate) more: bool,
}

/// How large a row's diff is, and the notes saved on it that were never sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Size {
    pub(crate) stat: DiffStat,
    pub(crate) unsent: usize,
}

/// What the picker asks of a repository. Errors are the text the footer shows.
pub(crate) trait Source: Send + Sync {
    /// The folder's name, for the title.
    fn name(&self) -> String;
    /// `count` commits after the first `skip`, and on the first page the branch: quick calls.
    fn list(&self, skip: usize, count: usize) -> Result<Listing, String>;
    /// How large `scope`'s diff is: as slow as capturing it.
    fn size(&self, scope: &Scope) -> Result<Size, String>;
    /// Capture `scope` exactly, with no fallback to another.
    fn capture(&self, scope: &Scope) -> Result<(Repo, Capture), String>;
}

/// The repository itself.
pub(crate) struct GitSource {
    pub(crate) repo: Repo,
    /// Where the notes are saved, for the unsent counts.
    pub(crate) data_dir: PathBuf,
}

impl Source for GitSource {
    fn name(&self) -> String {
        let root = self.repo.root();
        root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into())
    }

    fn list(&self, skip: usize, count: usize) -> Result<Listing, String> {
        let mut commits = self.repo.commits(skip, count + 1).map_err(|e| e.to_string())?;
        let more = commits.len() > count;
        commits.truncate(count);
        let branch = if skip == 0 { self.repo.branch_choice() } else { None };
        Ok(Listing { branch, commits, more })
    }

    fn size(&self, scope: &Scope) -> Result<Size, String> {
        let stat = self.repo.diff_stat(scope).map_err(|e| e.to_string())?;
        let root = self.repo.root();
        // The same record the review opens (`Capture::record_owner` for a local scope).
        let unsent =
            ReviewStore::load(&self.data_dir, &project_for(root), &root.display().to_string(), scope)
                .map_or(0, |store| store.pending().len());
        Ok(Size { stat, unsent })
    }

    fn capture(&self, scope: &Scope) -> Result<(Repo, Capture), String> {
        self.repo.capture(scope).map(|capture| (self.repo.clone(), capture)).map_err(|e| e.to_string())
    }
}

/// A question for the source.
#[derive(Debug, Clone)]
pub(super) enum Job {
    List { skip: usize, count: usize },
    Size { row: usize, scope: Scope },
    Capture { row: usize, scope: Scope },
}

/// Its answer, with the round of asking (`generation`) it belongs to.
pub(super) enum Done {
    List { generation: u64, skip: usize, listing: Result<Listing, String> },
    Size { generation: u64, row: usize, size: Result<Size, String> },
    Capture { generation: u64, row: usize, captured: Box<Result<(Repo, Capture), String>> },
}

/// The threads asking the source. Each job gets a thread of its own; the picker keeps few in
/// flight. A thread whose answer nobody takes any more ends when its git does.
pub(super) struct Work {
    source: Arc<dyn Source>,
    send: Sender<Done>,
    done: Receiver<Done>,
    wake: SyncSender<()>,
}

impl Work {
    pub(super) fn new(source: Arc<dyn Source>, wake: SyncSender<()>) -> Self {
        let (send, done) = mpsc::channel();
        Self { source, send, done, wake }
    }

    /// Start `job`. False when no thread could be started.
    pub(super) fn ask(&self, generation: u64, job: Job) -> bool {
        let (source, send, wake) = (Arc::clone(&self.source), self.send.clone(), self.wake.clone());
        std::thread::Builder::new()
            .name("pick-changes".into())
            .spawn(move || {
                let done = match job {
                    Job::List { skip, count } => {
                        Done::List { generation, skip, listing: source.list(skip, count) }
                    }
                    Job::Size { row, scope } => Done::Size { generation, row, size: source.size(&scope) },
                    Job::Capture { row, scope } => {
                        Done::Capture { generation, row, captured: Box::new(source.capture(&scope)) }
                    }
                };
                let _ = send.send(done);
                let _ = wake.try_send(());
            })
            .is_ok()
    }

    /// The answers that arrived.
    pub(super) fn take(&self) -> Vec<Done> {
        self.done.try_iter().collect()
    }
}
