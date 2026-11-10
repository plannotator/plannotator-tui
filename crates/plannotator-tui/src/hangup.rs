//! End the process when its terminal hangs up.
//!
//! Once the terminal is gone, crossterm's unix reader never returns: `read` gives `Ok(0)`
//! forever and `event::poll` spins at full CPU (crossterm-rs/crossterm#793). SIGHUP would
//! normally end the process first, but not when it is ignored (`nohup`, a launcher that does
//! not forward it). A watch thread checks the terminal twice a second and exits on a hang-up.

use std::os::fd::BorrowedFd;
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};

/// How often the terminal is checked; also the longest a hung-up process keeps spinning.
const INTERVAL: Duration = Duration::from_millis(500);

/// Exit status for a hang-up: the status SIGHUP itself would have left.
const HANGUP_STATUS: i32 = 129;

/// Watch the terminal the app draws on, and exit once it hangs up.
pub(crate) fn watch() {
    let Some(terminal) = terminal([rustix::stdio::stdin(), rustix::stdio::stdout()]) else {
        return;
    };
    // Without the thread the app runs as before; a hang-up then needs SIGHUP to end it.
    let _ = std::thread::Builder::new().name("hangup".to_owned()).spawn(move || {
        while !hung_up(terminal) {
            std::thread::sleep(INTERVAL);
        }
        // Not `restore`: it reports a failure on stderr, which is the hung-up terminal too,
        // and that print panics inside ratatui's panic hook and aborts.
        let _ = ratatui::try_restore();
        std::process::exit(HANGUP_STATUS);
    });
}

/// The first descriptor that is a terminal. Piped stdin (`last`, `-`) is passed over: a pipe
/// whose writer has finished reports a hang-up too.
fn terminal(fds: [BorrowedFd<'_>; 2]) -> Option<BorrowedFd<'_>> {
    fds.into_iter().find(|fd| rustix::termios::isatty(fd))
}

/// `poll` reports the hang-up. `tcgetattr` and `isatty` cannot: on macOS both keep succeeding
/// on a terminal whose other end has closed.
fn hung_up(fd: BorrowedFd<'_>) -> bool {
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let now = Timespec { tv_sec: 0, tv_nsec: 0 };
    poll(&mut fds, Some(&now)).is_ok() && fds.iter().any(|entry| entry.revents().contains(PollFlags::HUP))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests assert by panicking")]

    use std::os::fd::{AsFd as _, OwnedFd};

    use rustix::fs::{Mode, OFlags};
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

    use super::*;

    /// A pseudo-terminal: the controller a multiplexer holds, and the terminal an app holds.
    fn pty() -> (OwnedFd, OwnedFd) {
        let controller = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("openpt");
        // Close-on-exec, so a child another test spawns meanwhile cannot hold the controller
        // open and keep the hang-up from happening.
        rustix::io::fcntl_setfd(&controller, rustix::io::FdFlags::CLOEXEC).expect("cloexec");
        grantpt(&controller).expect("grantpt");
        unlockpt(&controller).expect("unlockpt");
        let name = ptsname(&controller, Vec::new()).expect("ptsname");
        let tty =
            rustix::fs::open(name.as_c_str(), OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC, Mode::empty())
                .expect("open");
        (controller, tty)
    }

    #[test]
    fn a_terminal_reads_as_hung_up_only_after_its_other_end_closes() {
        let (controller, tty) = pty();
        assert!(!hung_up(tty.as_fd()));
        drop(controller);
        // A child that another test spawned in the moment before close-on-exec was set can
        // still hold the controller; the hang-up follows once it exits.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !hung_up(tty.as_fd()) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(hung_up(tty.as_fd()));
    }

    #[test]
    fn piped_stdin_is_passed_over_for_the_terminal() {
        let (reader, writer) = std::io::pipe().expect("pipe");
        drop(writer);
        let (_controller, tty) = pty();
        assert!(hung_up(reader.as_fd()), "a finished pipe reports a hang-up");
        let watched = terminal([reader.as_fd(), tty.as_fd()]).expect("a terminal");
        assert!(!hung_up(watched));
    }
}
