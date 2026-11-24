//! Where the change picker's rows come from, and the threads that ask.
//!
//! [`Source`] is the seam to git: the real one runs it ([`GitSource`]); a test stands in a
//! slow one to show the picker never waits for it. Every call after the picker's first frame
//! runs on a thread of its own ([`Work`]) and pokes the event loop's wake channel when done,
//! as the disk watch does, so no git command ever runs on the thread that draws. Each job's
//! git runs under a [`Stop`], so a reload, a choice or the picker closing kills it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::time::{Duration, Instant};

use plannotator_tui_review::{Capture, Commit, DiffStat, Repo, ReviewStore, Scope, Stop, project_for};

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
        let stat = self.repo.diff_stat(scope).map_err(|e| match e {
            // A filter that asks on the terminal: the row says so, briefly.
            plannotator_tui_review::Error::NeedsTerminal { .. } => super::rows::NEEDS_TERMINAL.to_owned(),
            e => e.to_string(),
        })?;
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

/// Jobs running at once, whatever they are: each runs one git command at a time, so this is
/// also the most git commands the picker ever has running.
pub(super) const AT_ONCE: usize = 2;

/// The longest dropping the picker waits for its jobs to stop: they are killed first, so
/// they end at once; this only bounds a git that ignores SIGKILL's group.
const STOP_WAIT: Duration = Duration::from_secs(3);

/// A job started, and the handle that stops its git.
struct Started {
    capture: bool,
    stop: Stop,
    ended: Arc<AtomicBool>,
}

/// The threads asking the source. At most [`AT_ONCE`] jobs run, counting those stopped that
/// have not ended yet, so asking again and again (`r`) never piles git commands up; the
/// rest wait their turn here, a capture first. Dropping it stops every job, waits for them
/// to end (their temporary index copies go with them) and drops what waits.
pub(super) struct Work {
    source: Arc<dyn Source>,
    send: Sender<Done>,
    done: Receiver<Done>,
    wake: SyncSender<()>,
    queue: VecDeque<(u64, Job)>,
    started: Vec<Started>,
    /// Jobs whose thread has not ended, stopped or not.
    running: Arc<AtomicUsize>,
}

impl Work {
    pub(super) fn new(source: Arc<dyn Source>, wake: SyncSender<()>) -> Self {
        let (send, done) = mpsc::channel();
        Self {
            source,
            send,
            done,
            wake,
            queue: VecDeque::new(),
            started: Vec::new(),
            running: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Queue `job`, a capture ahead of the rest, and start what there is room for. False when
    /// no thread could be started.
    pub(super) fn ask(&mut self, generation: u64, job: Job) -> bool {
        if matches!(job, Job::Capture { .. }) {
            self.queue.push_front((generation, job));
        } else {
            self.queue.push_back((generation, job));
        }
        self.pump()
    }

    /// Room for another size: nothing waits and fewer than [`AT_ONCE`] jobs run.
    pub(super) fn has_room(&self) -> bool {
        self.queue.is_empty() && self.running.load(Ordering::SeqCst) < AT_ONCE
    }

    /// Start waiting jobs while there is room. False when a thread could not be started.
    pub(super) fn pump(&mut self) -> bool {
        self.started.retain(|job| !job.ended.load(Ordering::SeqCst));
        while self.running.load(Ordering::SeqCst) < AT_ONCE {
            let Some((generation, job)) = self.queue.pop_front() else { return true };
            if !self.start(generation, job) {
                return false;
            }
        }
        true
    }

    fn start(&mut self, generation: u64, job: Job) -> bool {
        let stop = Stop::new();
        let capture = matches!(job, Job::Capture { .. });
        let (source, send, wake) = (Arc::clone(&self.source), self.send.clone(), self.wake.clone());
        let ended = Arc::new(AtomicBool::new(false));
        self.running.fetch_add(1, Ordering::SeqCst);
        let guard = Ended { running: Arc::clone(&self.running), ended: Arc::clone(&ended) };
        let handle = stop.clone();
        let started = std::thread::Builder::new().name("pick-changes".into()).spawn(move || {
            let done = handle.run(|| match job {
                Job::List { skip, count } => {
                    Done::List { generation, skip, listing: source.list(skip, count) }
                }
                Job::Size { row, scope } => Done::Size { generation, row, size: source.size(&scope) },
                Job::Capture { row, scope } => {
                    Done::Capture { generation, row, captured: Box::new(source.capture(&scope)) }
                }
            });
            // Ended before the answer, so the picker that wakes finds the room.
            drop(guard);
            if !handle.is_stopped() {
                let _ = send.send(done);
            }
            let _ = wake.try_send(());
        });
        match started {
            Ok(_) => {
                self.started.push(Started { capture, stop, ended });
                true
            }
            // The guard went with the closure, counting the job out.
            Err(_) => false,
        }
    }

    /// Stop every job and drop what waits: a reload.
    pub(super) fn stop_all(&mut self) {
        self.queue.clear();
        for job in self.started.drain(..) {
            job.stop.stop();
        }
    }

    /// Stop the jobs that are not captures, and drop the sizes waiting: Enter wants the room.
    pub(super) fn stop_sizes(&mut self) {
        self.queue.retain(|(_, job)| !matches!(job, Job::Size { .. }));
        for job in self.started.iter().filter(|job| !job.capture) {
            job.stop.stop();
        }
    }

    /// Stop the capture running, for another.
    pub(super) fn stop_captures(&mut self) {
        self.queue.retain(|(_, job)| !matches!(job, Job::Capture { .. }));
        for job in self.started.iter().filter(|job| job.capture) {
            job.stop.stop();
        }
    }

    /// Jobs whose thread has not ended.
    #[cfg(test)]
    pub(super) fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// The answers that arrived.
    pub(super) fn take(&self) -> Vec<Done> {
        self.done.try_iter().collect()
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        self.stop_all();
        let deadline = Instant::now() + STOP_WAIT;
        while self.running.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Counts a job's thread out when it ends, a panic included.
struct Ended {
    running: Arc<AtomicUsize>,
    ended: Arc<AtomicBool>,
}

impl Drop for Ended {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::SeqCst);
        self.running.fetch_sub(1, Ordering::SeqCst);
    }
}
