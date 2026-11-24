//! A code review takes its git commands and their index copies with it when it ends: a
//! normal quit, a panic on the main thread, SIGHUP, SIGTERM or SIGINT, or the terminal
//! hanging up (`crate::hangup`). Once the screen is up, git runs in process groups of its own
//! ([`plannotator_tui_review::children`]), so a signal to the review's group no longer
//! reaches it; the review kills it instead. Ctrl-C on the screen is a key (raw mode), not
//! SIGINT; SIGINT comes from Ctrl-C before the screen opens, or from `kill -INT`.

use std::sync::Once;

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// Kills the review's git commands and removes its index copies when dropped.
pub(super) struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        plannotator_tui_review::shutdown();
    }
}

/// Set up the panic hook and the signal thread, once per process, and return the guard for
/// a normal end. Called before the first git command, so what starts early is covered too.
pub(super) fn guard() -> Guard {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // A worker's panic is reported and survived; only the main thread's ends the app.
            if std::thread::current().name() == Some("main") {
                plannotator_tui_review::shutdown();
            }
            previous(info);
        }));
        // Without the thread a signal ends the review as before, leaving git to finish alone.
        if let Ok(mut signals) = Signals::new([SIGHUP, SIGTERM, SIGINT]) {
            let _ = std::thread::Builder::new().name("review-signals".into()).spawn(move || {
                if let Some(signal) = signals.forever().next() {
                    plannotator_tui_review::shutdown();
                    // Before the screen opens the terminal is as the shell left it.
                    if plannotator_tui_review::children::screen_is_taken() {
                        crate::cli::release_input(true);
                        let _ = ratatui::try_restore();
                    }
                    crate::pi::link::remove_entry();
                    std::process::exit(128 + signal);
                }
            });
        }
    });
    Guard
}
