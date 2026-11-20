//! Noticing that the diff changed on disk, typically an agent editing files, so the footer
//! can say `r` refreshes it. Nothing reloads by itself: the reviewer decides when.
//!
//! One worker thread computes the scope's [`DiskStamp`] (a `git status` and a stat per
//! changed file; see `plannotator_tui_review::stamp`) when the screen asks, so a check never
//! blocks a keypress and two checks never run at once. The screen asks at most every
//! [`EVERY`], and never while the comment box is open or within [`QUIET_AFTER_KEY`] of a
//! key, so a check cannot compete with typing. A slow repository is checked less often: the
//! next check waits [`COST_FACTOR`] times as long as the last one took, so checking stays
//! under about 2% of one core however large the repository is.
//!
//! The first stamp after a capture is the baseline; a later stamp that differs means the
//! diff changed. A reload starts over. A commit or pull request review has nothing on disk
//! to watch and starts no thread.

use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use plannotator_tui_review::{DiskStamp, Repo, Scope};

/// The shortest time between two checks.
pub(super) const EVERY: Duration = Duration::from_secs(3);
/// No check starts this soon after a key.
pub(super) const QUIET_AFTER_KEY: Duration = Duration::from_millis(1500);
/// The wait after a check, as a multiple of how long it took, when that is longer than
/// [`EVERY`].
const COST_FACTOR: u32 = 50;
/// The wait after a check that failed (no git, a broken repository).
const AFTER_FAILURE: Duration = Duration::from_secs(30);

/// One check: the stamp, which capture it is for, and how long it took.
struct Done {
    generation: u64,
    stamp: Result<Option<DiskStamp>, String>,
    took: Duration,
}

struct Ask {
    generation: u64,
    repo: Repo,
    scope: Scope,
}

pub(super) struct Watch {
    asks: Option<Sender<Ask>>,
    done: Option<Receiver<Done>>,
    /// Held by the worker while git runs, and by a reload while it captures, so the two never
    /// run git at the same time.
    git: Arc<Mutex<()>>,
    /// Which capture checks are for; a reload moves it on, and older answers are dropped.
    generation: u64,
    baseline: Option<DiskStamp>,
    changed: bool,
    in_flight: bool,
    next: Instant,
    /// The shortest time between checks; tests set it to zero.
    pub(super) every: Duration,
    /// Checks started, for tests.
    pub(super) started: usize,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch")
            .field("watching", &self.asks.is_some())
            .field("changed", &self.changed)
            .finish_non_exhaustive()
    }
}

impl Watch {
    /// Watch `scope`'s files, waking the event loop on `wake` when a check is done. A scope
    /// nothing on disk changes gets no thread.
    pub(super) fn start(scope: &Scope, wake: SyncSender<()>) -> Self {
        let git = Arc::new(Mutex::new(()));
        let (asks, done) = if scope.watches_disk() { spawn(Arc::clone(&git), wake) } else { (None, None) };
        Self {
            asks,
            done,
            git,
            generation: 0,
            baseline: None,
            changed: false,
            in_flight: false,
            next: Instant::now(),
            every: EVERY,
            started: 0,
        }
    }

    /// Whether the diff changed on disk since the last capture.
    pub(super) fn changed(&self) -> bool {
        self.changed
    }

    /// Hold while capturing, so no check runs git alongside the capture. Then call
    /// [`Self::recaptured`].
    pub(super) fn hold_git(&self) -> MutexGuard<'_, ()> {
        self.git.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new capture: forget the baseline and take a new one at once.
    pub(super) fn recaptured(&mut self) {
        self.generation += 1;
        self.baseline = None;
        self.changed = false;
        self.next = Instant::now();
    }

    /// Take a finished check, and start the next when one is due and `quiet` (nobody is
    /// typing). True when the footer's notice appeared.
    pub(super) fn poll(&mut self, repo: &Repo, scope: &Scope, quiet: bool) -> bool {
        let mut appeared = false;
        while let Some(done) = self.done.as_ref().and_then(|d| d.try_recv().ok()) {
            self.in_flight = false;
            appeared |= self.take(&done);
        }
        // Once the diff is known to have changed, nothing more is learned until a reload.
        let due = !self.in_flight && !self.changed && Instant::now() >= self.next;
        let first = self.baseline.is_none();
        if due
            && (quiet || first)
            && let Some(asks) = &self.asks
        {
            let ask = Ask { generation: self.generation, repo: repo.clone(), scope: scope.clone() };
            if asks.send(ask).is_ok() {
                self.in_flight = true;
                self.started += 1;
            } else {
                self.asks = None;
            }
        }
        appeared
    }

    fn take(&mut self, done: &Done) -> bool {
        let wait = self.every.max(done.took * COST_FACTOR);
        if done.generation != self.generation {
            return false;
        }
        let stamp = match done.stamp {
            Ok(Some(stamp)) => stamp,
            Ok(None) => return false,
            Err(_) => {
                self.next = Instant::now() + AFTER_FAILURE;
                return false;
            }
        };
        self.next = Instant::now() + wait;
        match self.baseline {
            None => {
                self.baseline = Some(stamp);
                false
            }
            Some(baseline) if baseline != stamp => {
                self.changed = true;
                true
            }
            Some(_) => false,
        }
    }
}

type Channels = (Option<Sender<Ask>>, Option<Receiver<Done>>);

/// The worker: one check at a time, in the order asked.
fn spawn(git: Arc<Mutex<()>>, wake: SyncSender<()>) -> Channels {
    let (ask_tx, ask_rx) = mpsc::channel::<Ask>();
    let (done_tx, done_rx) = mpsc::channel::<Done>();
    let spawned = std::thread::Builder::new().name("review-disk-watch".into()).spawn(move || {
        while let Ok(ask) = ask_rx.recv() {
            let started = Instant::now();
            let stamp = {
                let _git = git.lock().unwrap_or_else(PoisonError::into_inner);
                ask.repo.disk_stamp(&ask.scope).map_err(|e| e.to_string())
            };
            let done = Done { generation: ask.generation, stamp, took: started.elapsed() };
            if done_tx.send(done).is_err() {
                break;
            }
            let _ = wake.try_send(());
        }
    });
    match spawned {
        Ok(_) => (Some(ask_tx), Some(done_rx)),
        Err(_) => (None, None),
    }
}
