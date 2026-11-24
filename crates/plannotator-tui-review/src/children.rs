//! The git processes this crate starts, kept where they can be stopped: one job's when the
//! person moves on ([`Stop`]), all of them when the app exits ([`shutdown`]). And the one
//! folder per process that every temporary index lives under ([`temp_root`]), removed at exit,
//! with what an earlier process left behind when it was killed.
//!
//! Every git command runs through [`exec`]: spawned in a process group of its own (on Unix),
//! listed in a registry while it runs, and reaped only under the registry's lock, so a kill
//! never reaches a process id that was already reused. Killing the group also ends what git
//! started itself, such as a slow clean filter.

use std::cell::RefCell;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::Duration;

use crate::error::{Error, Result};

/// Stops the git commands run under it ([`Stop::run`]): the one running is killed, and the
/// next ones fail with [`Error::Stopped`] without starting.
#[derive(Debug, Clone, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kill what runs under this handle now, and refuse what it would start next.
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
        for entry in running().iter_mut().filter(|entry| entry.stop.as_ref().is_some_and(|s| s.same(self))) {
            kill(&mut entry.child);
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Run `work` on this thread with every git command it starts under this handle.
    pub fn run<T>(&self, work: impl FnOnce() -> T) -> T {
        struct Restore(Option<Stop>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let previous = self.0.take();
                CURRENT.with(|current| *current.borrow_mut() = previous);
            }
        }
        let previous = CURRENT.with(|current| current.borrow_mut().replace(self.clone()));
        let _restore = Restore(previous);
        work()
    }

    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

thread_local! {
    /// The handle [`Stop::run`] put the commands of this thread under.
    static CURRENT: RefCell<Option<Stop>> = const { RefCell::new(None) };
}

/// One running command.
struct Entry {
    id: u64,
    child: Child,
    stop: Option<Stop>,
}

static RUNNING: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);
/// How long a stopped command's output is still read for once it has ended.
const STOPPED_GRACE: Duration = Duration::from_millis(100);
/// Set by [`shutdown`]: nothing starts any more.
static EXITING: AtomicBool = AtomicBool::new(false);

fn running() -> MutexGuard<'static, Vec<Entry>> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How many git commands this crate has running now, in this process.
pub fn running_count() -> usize {
    running().len()
}

/// Kill `child` and, on Unix, the process group it leads: what git started goes too.
fn kill(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = i32::try_from(child.id()).ok().and_then(rustix::process::Pid::from_raw) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
}

fn stopped(stop: Option<&Stop>) -> bool {
    EXITING.load(Ordering::SeqCst) || stop.is_some_and(Stop::is_stopped)
}

/// What a command printed, and how it ended.
pub(crate) struct Ran {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    /// stdout ran past the limit; the command was killed there.
    pub(crate) over: bool,
}

/// Run `cmd` to its end: `input` on its stdin, at most `limit` bytes of stdout read (past it
/// the command is killed and [`Ran::over`] set). `what` names it in errors. Under a [`Stop`]
/// that is stopped, or after [`shutdown`], it fails with [`Error::Stopped`].
pub(crate) fn exec(
    mut cmd: Command,
    what: &str,
    input: Option<Vec<u8>>,
    limit: Option<usize>,
) -> Result<Ran> {
    let stop = CURRENT.with(|current| current.borrow().clone());
    if stopped(stop.as_ref()) {
        return Err(Error::Stopped);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(Error::GitMissing)?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    {
        let mut list = running();
        list.push(Entry { id, child, stop: stop.clone() });
        // A stop that came while it started.
        if stopped(stop.as_ref())
            && let Some(entry) = list.last_mut()
        {
            kill(&mut entry.child);
        }
    }
    let writer = stdin.zip(input).map(|(mut pipe, bytes)| {
        std::thread::spawn(move || {
            let wrote = pipe.write_all(&bytes);
            drop(pipe);
            wrote
        })
    });
    // Both pipes are read on threads of their own, so a chatty git never blocks on a full
    // pipe, and a stopped one is not waited for when something else still holds its pipe
    // open: a process another thread started at that moment can inherit it.
    let (errors_sent, errors) = mpsc::channel();
    if let Some(mut pipe) = stderr {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = pipe.read_to_end(&mut text);
            let _ = errors_sent.send(text);
        });
    }
    let (output_sent, output) = mpsc::channel();
    if let Some(pipe) = stdout {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let read = if let Some(limit) = limit {
                pipe.take(limit as u64 + 1).read_to_end(&mut out)
            } else {
                let mut pipe = pipe;
                pipe.read_to_end(&mut out)
            };
            // Past the limit, git is stopped there and nothing more is read.
            if limit.is_some_and(|limit| out.len() > limit)
                && let Some(entry) = running().iter_mut().find(|entry| entry.id == id)
            {
                kill(&mut entry.child);
            }
            let _ = output_sent.send((out, read));
        });
    }
    let status = reap(id)?;
    let gave_up = stopped(stop.as_ref());
    let (out, read) = take(&output, gave_up).unwrap_or_else(|| (Vec::new(), Ok(0)));
    let stderr = take(&errors, gave_up).unwrap_or_default();
    let wrote = if gave_up { Ok(()) } else { writer.and_then(|thread| thread.join().ok()).unwrap_or(Ok(())) };
    if gave_up && !status.success() {
        return Err(Error::Stopped);
    }
    let over = limit.is_some_and(|limit| out.len() > limit);
    if !over {
        read.map_err(|e| Error::Git { command: what.to_owned(), detail: e.to_string() })?;
        // A command that failed says why on stderr; a broken pipe is only its echo.
        if status.success() {
            wrote.map_err(|e| Error::Git { command: what.to_owned(), detail: e.to_string() })?;
        }
    }
    Ok(Ran { status, stdout: out, stderr, over })
}

/// What a pipe's reader sent: all of it, or for a command that was stopped what came within
/// [`STOPPED_GRACE`].
fn take<T>(done: &mpsc::Receiver<T>, stopped: bool) -> Option<T> {
    if stopped { done.recv_timeout(STOPPED_GRACE).ok() } else { done.recv().ok() }
}

/// Wait for command `id` to end, checking under the lock so it is never reaped while a kill
/// could still reach its id: a short sleep between checks, growing to 20 ms.
fn reap(id: u64) -> Result<ExitStatus> {
    let mut pause = Duration::from_micros(200);
    loop {
        {
            let mut list = running();
            let Some(at) = list.iter().position(|entry| entry.id == id) else {
                return Err(Error::Stopped);
            };
            let Some(entry) = list.get_mut(at) else { return Err(Error::Stopped) };
            match entry.child.try_wait() {
                Ok(Some(status)) => {
                    list.swap_remove(at);
                    return Ok(status);
                }
                Ok(None) => {}
                Err(error) => {
                    let mut entry = list.swap_remove(at);
                    drop(list);
                    kill(&mut entry.child);
                    let _ = entry.child.wait();
                    return Err(Error::GitMissing(error));
                }
            }
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(20));
    }
}

/// The folder this process keeps its temporary index copies in, created private (0700) on
/// first use. Folders an earlier process left, once it is gone, are removed then.
pub(crate) fn temp_root() -> Result<PathBuf> {
    static ROOT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    let root = ROOT.get_or_init(|| {
        sweep_stale();
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("{PREFIX}{}-{nanos}", std::process::id()));
        let builder = &mut std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(builder, 0o700);
        builder.create(&dir).map(|()| dir).map_err(|e| e.to_string())
    });
    let _ = ROOT_MADE.set(root.as_ref().ok().cloned());
    root.clone().map_err(|detail| Error::Io {
        action: "creating the folder for index copies in",
        path: std::env::temp_dir(),
        source: std::io::Error::other(detail),
    })
}

/// The folder [`temp_root`] made, for [`shutdown`] to remove.
static ROOT_MADE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// `plannotator-tui-<pid>-<nanos>`; 0.11.6 and earlier made `plannotator-tui-index-<pid>-…`
/// directly in the temp folder.
const PREFIX: &str = "plannotator-tui-";

/// Remove the index folders of processes that are gone: killed before they could.
fn sweep_stale() {
    #[cfg(unix)]
    {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(stale_pid) else { continue };
            let gone = rustix::process::Pid::from_raw(pid)
                .is_some_and(|pid| rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH));
            if gone && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// The process id in an index folder's name.
#[cfg(any(unix, test))]
fn stale_pid(name: &str) -> Option<i32> {
    let rest = name.strip_prefix(PREFIX)?;
    let rest = rest.strip_prefix("index-").unwrap_or(rest);
    let (pid, tail) = rest.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    // `<pid>-<nanos>`, or the old `<pid>-<n>-<nanos>`.
    (digits(pid) && tail.split('-').all(digits)).then(|| pid.parse().ok()).flatten()
}

/// At exit: kill every git command still running, start no more, and remove the folder of
/// index copies.
pub fn shutdown() {
    EXITING.store(true, Ordering::SeqCst);
    for entry in running().iter_mut() {
        kill(&mut entry.child);
    }
    if let Some(Some(root)) = ROOT_MADE.get() {
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_index_folders_named_by_a_process_id_are_taken_for_stale_ones() {
        assert_eq!(stale_pid("plannotator-tui-123-456789"), Some(123));
        assert_eq!(stale_pid("plannotator-tui-index-123-4-456789"), Some(123));
        for other in [
            "plannotator-tui-123",
            "plannotator-tui-abc-1",
            "plannotator-tui-123-",
            "plannotator-tui-config-1",
            "plannotator-tui-12-3x",
            "pt-review-1-2",
        ] {
            assert_eq!(stale_pid(other), None, "{other}");
        }
    }
}
