//! Putting the review terminal on screen with herdr-app, and starting it once it is there.
//! herdr-app is driven only through `herdr-app do`; `show_args` and `retried` are pure.

use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

use super::command::Hc;
use crate::config::{Placement, SplitDirection};

/// The `herdr-app do` arguments that put the review terminal on screen: over the tab for an
/// overlay or popup, in a pane beside the focused one for a split.
fn show_args(placement: Placement, direction: SplitDirection, address: &str) -> Vec<String> {
    let mut args: Vec<String> = ["do"].into_iter().map(str::to_owned).collect();
    match placement {
        Placement::Overlay | Placement::Popup => {
            args.extend(["terminal.peek", "--terminal", address].map(str::to_owned));
        }
        Placement::Split => {
            args.extend(["terminal.keep", "--terminal", address, "--place"].map(str::to_owned));
            args.push(direction.as_str().to_owned());
        }
    }
    args
}

/// herdr-app learns of a new terminal a moment after hc starts it (about 0.4 s measured), and
/// until then refuses to show it with "no running terminal". Retry for a few seconds.
const SHOW_TRIES: u32 = 30;
const SHOW_PAUSE: Duration = Duration::from_millis(100);

/// How one attempt to show the terminal went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Try {
    Shown,
    /// herdr-app answered with a refusal that may pass.
    Again,
    /// There is no herdr-app to ask.
    GiveUp,
}

fn show(args: &[String]) -> Try {
    let status = Command::new("herdr-app")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => Try::Shown,
        Ok(_) => Try::Again,
        Err(_) => Try::GiveUp,
    }
}

/// Run `attempt` until it shows the terminal, up to `tries` times, `pause` apart.
fn retried(tries: u32, pause: Duration, mut attempt: impl FnMut() -> Try) -> bool {
    for n in 0..tries {
        match attempt() {
            Try::Shown => return true,
            Try::GiveUp => return false,
            Try::Again if n + 1 < tries => std::thread::sleep(pause),
            Try::Again => {}
        }
    }
    false
}

/// After herdr-app shows a terminal it still has to tell it its colours. A terminal answers a
/// colour query with the core's dark default until then, so a review that asks as it starts
/// draws a dark palette on a light window. 0.1 s was enough in 20 of 20 runs; this leaves margin.
const COLOURS_SETTLE: Duration = Duration::from_millis(250);

/// Let the held review start: press Enter in it, once herdr-app has shown it and settled.
/// Without herdr-app there is nothing to wait for.
fn release(hc: &Hc, address: &str, shown: bool) -> Result<()> {
    if shown {
        std::thread::sleep(COLOURS_SETTLE);
    }
    hc.run(&["keys", address, "enter"]).map(drop).with_context(|| {
        format!("releasing the review {address}; attach with `hc attach {address}` and press Enter")
    })
}

/// Show the terminal `address` and let its held review start. Returns whether herdr-app showed
/// it; without herdr-app the review still starts, and the caller says how to attach.
pub(crate) fn show_and_release(
    hc: &Hc,
    address: &str,
    placement: Placement,
    direction: SplitDirection,
) -> Result<bool> {
    let args = show_args(placement, direction, address);
    let shown = retried(SHOW_TRIES, SHOW_PAUSE, || show(&args));
    release(hc, address, shown)?;
    Ok(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    /// Whether showing succeeded and how many attempts it took, when each attempt answers
    /// the next of `answers` (and refuses once they run out).
    fn showing(tries: u32, answers: &[Try]) -> (bool, usize) {
        let mut calls = 0;
        let shown = retried(tries, Duration::ZERO, || {
            calls += 1;
            answers.get(calls - 1).copied().unwrap_or(Try::Again)
        });
        (shown, calls)
    }

    #[test]
    fn showing_waits_for_herdr_app_to_know_the_terminal_but_not_for_a_missing_herdr_app() {
        assert_eq!(
            showing(5, &[Try::Again, Try::Again, Try::Shown]),
            (true, 3),
            "stops at the first success"
        );
        assert_eq!(showing(5, &[]), (false, 5), "a refusal that never passes ends after the last try");
        assert_eq!(showing(5, &[Try::GiveUp]), (false, 1), "no herdr-app means no second try");
    }

    #[test]
    fn an_overlay_peeks_and_a_split_takes_a_pane() {
        let peek = strs(&["do", "terminal.peek", "--terminal", "t9"]);
        assert_eq!(show_args(Placement::Overlay, SplitDirection::Right, "t9"), peek);
        assert_eq!(show_args(Placement::Popup, SplitDirection::Down, "t9"), peek);
        assert_eq!(
            show_args(Placement::Split, SplitDirection::Down, "t9"),
            strs(&["do", "terminal.keep", "--terminal", "t9", "--place", "down"])
        );
    }
}
