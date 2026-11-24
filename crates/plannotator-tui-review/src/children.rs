//! The git processes this crate starts, kept where they can be stopped: one job's when the
//! person moves on ([`Stop`]), all of them when the app exits ([`shutdown`]). And the one
//! folder per process that every temporary index lives under ([`temp_root`]), removed at exit,
//! with what an earlier process left behind when it was killed.
//!
//! Every git command this crate runs goes through [`spawn`] (most through [`exec`]): listed
//! in a registry while it runs, and reaped only under the registry's lock, so a kill never
//! reaches a process id that was already reused.
//!
//! Where it runs depends on when ([`Place`]). Before the screen is taken, git runs in the
//! foreground as it always did: in the app's own process group, so it can ask on the terminal
//! (an ssh passphrase while a pull request is fetched, say) and Ctrl-C reaches it. Once the
//! screen is taken ([`screen_taken`]), or for the picker's jobs ([`Stop::run`]), it runs in
//! the background: in a process group of its own (on Unix), so killing the group also ends
//! what git started itself, such as a slow clean filter; with no stdin and
//! `GIT_TERMINAL_PROMPT=0`. A background command that still reads the terminal is stopped
//! by it (SIGTTIN) and would wait forever: it is found that way and killed
//! ([`Error::NeedsTerminal`]).

use std::cell::RefCell;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

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
            entry.kill();
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

/// Where a git command runs: see the module's notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// In the app's process group, as a command run before the screen opens.
    Foreground,
    /// In a group of its own, with no stdin and no terminal prompts.
    Background,
}

/// Whether the app has taken the terminal for its screen: from then on git runs in the
/// background. The app says so as it takes it and gives it back.
pub fn screen_taken(taken: bool) {
    SCREEN.store(taken, Ordering::SeqCst);
}

/// Whether [`screen_taken`] said the terminal is the app's now.
pub fn screen_is_taken() -> bool {
    SCREEN.load(Ordering::SeqCst)
}

static SCREEN: AtomicBool = AtomicBool::new(false);

/// Where a command started now, on this thread, runs.
pub(crate) fn place() -> Place {
    if SCREEN.load(Ordering::SeqCst) || CURRENT.with(|current| current.borrow().is_some()) {
        Place::Background
    } else {
        Place::Foreground
    }
}

/// Set `cmd` up to run at `place`; `input` says whether something is written to its stdin.
fn prepare(cmd: &mut Command, place: Place, input: bool) {
    if input {
        cmd.stdin(Stdio::piped());
    }
    if place == Place::Background {
        if !input {
            cmd.stdin(Stdio::null());
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(cmd, 0);
    }
}

/// One running command.
struct Entry {
    id: u64,
    child: Child,
    stop: Option<Stop>,
    place: Place,
    started: Instant,
}

impl Entry {
    /// Kill the command and, in the background on Unix, the process group it leads: what git
    /// started goes too. A command in the app's own group is killed alone.
    fn kill(&mut self) {
        #[cfg(unix)]
        if self.place == Place::Background
            && let Some(pid) = i32::try_from(self.child.id()).ok().and_then(rustix::process::Pid::from_raw)
        {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = self.child.kill();
    }
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

/// How long a picker job ([`Stop::run`]) may run before it is killed.
const JOB_LIMIT: Duration = Duration::from_mins(1);
/// How long a background command runs before it is checked for waiting on the terminal, and
/// how often after that.
const TERMINAL_CHECK: Duration = Duration::from_millis(500);

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

/// A command [`spawn`] started: its pipes, and the handle to wait for it or kill it.
pub(crate) struct Spawned {
    id: u64,
    stop: Option<Stop>,
    what: String,
    pub(crate) stdin: Option<std::process::ChildStdin>,
    pub(crate) stdout: Option<ChildStdout>,
    pub(crate) stderr: Option<ChildStderr>,
}

impl Spawned {
    /// Kill it (and its group, in the background), if it still runs.
    pub(crate) fn kill(&self) {
        if let Some(entry) = running().iter_mut().find(|entry| entry.id == self.id) {
            entry.kill();
        }
    }

    /// Wait for it to end. A background command found waiting on the terminal, or a picker
    /// job past [`JOB_LIMIT`], is killed and fails.
    pub(crate) fn wait(&self) -> Result<ExitStatus> {
        reap(self.id, &self.what)
    }

    fn gave_up(&self) -> bool {
        stopped(self.stop.as_ref())
    }
}

/// Start `cmd` with stdout and stderr piped, and stdin piped when `input` is set, at the
/// [`place`] for now. Under a [`Stop`] that is stopped, or after [`shutdown`], it fails with
/// [`Error::Stopped`] without starting.
pub(crate) fn spawn(mut cmd: Command, what: &str, input: bool) -> Result<Spawned> {
    let stop = CURRENT.with(|current| current.borrow().clone());
    if stopped(stop.as_ref()) {
        return Err(Error::Stopped);
    }
    let place = place();
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    prepare(&mut cmd, place, input);
    let mut child = cmd.spawn().map_err(Error::GitMissing)?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    {
        let mut list = running();
        list.push(Entry { id, child, stop: stop.clone(), place, started: Instant::now() });
        // A stop that came while it started.
        if stopped(stop.as_ref())
            && let Some(entry) = list.last_mut()
        {
            entry.kill();
        }
    }
    Ok(Spawned { id, stop, what: what.to_owned(), stdin, stdout, stderr })
}

/// Run `cmd` to its end: `input` on its stdin, at most `limit` bytes of stdout read (past it
/// the command is killed and [`Ran::over`] set). `what` names it in errors. Under a [`Stop`]
/// that is stopped, or after [`shutdown`], it fails with [`Error::Stopped`].
pub(crate) fn exec(cmd: Command, what: &str, input: Option<Vec<u8>>, limit: Option<usize>) -> Result<Ran> {
    let mut spawned = spawn(cmd, what, input.is_some())?;
    let id = spawned.id;
    let (stdin, stdout, stderr) = (spawned.stdin.take(), spawned.stdout.take(), spawned.stderr.take());
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
                entry.kill();
            }
            let _ = output_sent.send((out, read));
        });
    }
    let status = spawned.wait()?;
    let gave_up = spawned.gave_up();
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
/// could still reach its id: a short sleep between checks, growing to 20 ms. In the
/// background it is also checked every [`TERMINAL_CHECK`] for waiting on the terminal, and a
/// picker job for running past [`JOB_LIMIT`]; either is killed, and fails.
fn reap(id: u64, what: &str) -> Result<ExitStatus> {
    let mut pause = Duration::from_micros(200);
    let mut next_check = TERMINAL_CHECK;
    loop {
        let (group, ran, job) = {
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
                    entry.kill();
                    let _ = entry.child.wait();
                    return Err(Error::GitMissing(error));
                }
            }
            let group = (entry.place == Place::Background).then(|| entry.child.id());
            (group, entry.started.elapsed(), entry.stop.is_some())
        };
        if let Some(group) = group {
            let failed = if job && ran > JOB_LIMIT {
                Some(Error::TimedOut { command: what.to_owned(), after: JOB_LIMIT })
            } else if ran >= next_check {
                next_check = ran + TERMINAL_CHECK;
                waits_on_terminal(group).then(|| Error::NeedsTerminal { command: what.to_owned() })
            } else {
                None
            };
            if let Some(error) = failed {
                if let Some(entry) = running().iter_mut().find(|entry| entry.id == id) {
                    entry.kill();
                }
                // Reaped once it has ended, which a kill makes quick.
                let _ = reap_quietly(id);
                return Err(error);
            }
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(20));
    }
}

/// Wait for a killed command to end, and take it off the list.
fn reap_quietly(id: u64) -> Option<ExitStatus> {
    loop {
        {
            let mut list = running();
            let at = list.iter().position(|entry| entry.id == id)?;
            if let Some(status) = list.get_mut(at).and_then(|entry| entry.child.try_wait().ok()).flatten() {
                list.swap_remove(at);
                return Some(status);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Whether a process of group `group` is stopped: in the background, that is a read from
/// (or a mode change of) the terminal, which no one will ever answer. Asked of `ps`; without
/// it, nothing is found and the job limit still applies.
fn waits_on_terminal(group: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(out) = Command::new("ps")
            .args(["-A", "-o", "pgid=,stat="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout).lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next().and_then(|pgid| pgid.parse::<u32>().ok()) == Some(group)
                && fields.next().is_some_and(|stat| stat.starts_with('T'))
        })
    }
    #[cfg(not(unix))]
    {
        let _ = group;
        false
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
        entry.kill();
    }
    if let Some(Some(root)) = ROOT_MADE.get() {
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests assert by panicking")]
    use super::*;

    #[cfg(unix)]
    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", script]);
        cmd
    }

    /// `<pid> <process group> <GIT_TERMINAL_PROMPT or unset> <read or eof>`.
    #[cfg(unix)]
    const WHERE: &str = "printf '%s %s %s ' $$ $(ps -o pgid= -p $$) \"${GIT_TERMINAL_PROMPT-unset}\"; \
                         if read line; then echo read; else echo eof; fi";

    #[cfg(unix)]
    fn where_it_ran(ran: &Ran) -> Vec<String> {
        String::from_utf8_lossy(&ran.stdout).split_whitespace().map(str::to_owned).collect()
    }

    #[cfg(unix)]
    #[test]
    fn before_the_screen_opens_a_command_runs_in_the_apps_own_process_group() {
        // Its stdin is what the caller set, as `git::command` does: here nothing.
        let mut cmd = sh(WHERE);
        cmd.stdin(Stdio::null());
        let ran = exec(cmd, "sh", None, None).expect("runs");
        let seen = where_it_ran(&ran);
        assert_eq!(seen.get(1), Some(&rustix::process::getpgrp().as_raw_nonzero().to_string()), "{seen:?}");
        let prompt = std::env::var("GIT_TERMINAL_PROMPT").unwrap_or_else(|_| "unset".into());
        assert_eq!(seen.get(2), Some(&prompt), "terminal prompts as the person has them");
    }

    #[cfg(unix)]
    #[test]
    fn a_picker_job_runs_in_a_group_of_its_own_with_no_stdin_and_no_prompts() {
        let ran = Stop::new().run(|| exec(sh(WHERE), "sh", None, None)).expect("runs");
        let seen = where_it_ran(&ran);
        assert_eq!(seen.first(), seen.get(1), "it leads its own group: {seen:?}");
        assert_ne!(seen.get(1), Some(&rustix::process::getpgrp().as_raw_nonzero().to_string()));
        assert_eq!(seen.get(2).map(String::as_str), Some("0"));
        assert_eq!(seen.get(3).map(String::as_str), Some("eof"), "stdin is empty");
        // Input given still arrives.
        let ran = Stop::new().run(|| exec(sh(WHERE), "sh", Some(b"x\n".to_vec()), None)).expect("runs");
        assert_eq!(where_it_ran(&ran).get(3).map(String::as_str), Some("read"));
    }

    /// A read from the terminal stops a process in a background group (SIGTTIN) for good;
    /// stopping itself does the same without a terminal.
    #[cfg(unix)]
    #[test]
    fn a_background_command_stopped_waiting_on_the_terminal_is_killed_and_says_so() {
        let started = Instant::now();
        let error = Stop::new()
            .run(|| exec(sh("kill -STOP $$; echo never"), "sh", None, None))
            .map(|_| ())
            .expect_err("it does not end on its own");
        assert!(matches!(error, Error::NeedsTerminal { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    }

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
