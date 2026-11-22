//! Putting the review terminal on screen with herdr-app, and starting it once it is there.
//! herdr-app is driven only through `herdr-app do`; `show_args`, `refusal` and `retried` are
//! pure.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

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

/// The refusal that passes once herdr-app knows the terminal.
const NOT_YET: &str = "no running terminal";

/// How one attempt to show the terminal went.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Try {
    Shown,
    /// herdr-app does not know the terminal yet, in its words.
    Again(String),
    /// A refusal that will not pass, or no herdr-app to ask, and why.
    GiveUp(String),
}

fn show(bin: &Path, args: &[String]) -> Try {
    let output = Command::new(bin).args(args).stdin(Stdio::null()).output();
    match output {
        Ok(output) if output.status.success() => Try::Shown,
        Ok(output) => {
            refusal(&String::from_utf8_lossy(&output.stdout), &String::from_utf8_lossy(&output.stderr))
        }
        Err(err) => Try::GiveUp(format!("cannot run {}: {err}", bin.display())),
    }
}

/// What a failed `herdr-app do` means. The window's refusals come back as
/// `{"error": "..."}` on stdout (exit 1); failures before the window is reached, such as no
/// window at all, are a line on stderr (exit 2). Only a terminal the window has not seen yet
/// is worth asking about again.
fn refusal(stdout: &str, stderr: &str) -> Try {
    let reply: Option<Value> = serde_json::from_str(stdout.trim()).ok();
    let error = reply.as_ref().and_then(|r| r.get("error")).and_then(Value::as_str);
    let stderr = stderr.trim();
    let reason = match error {
        Some(error) => error.to_owned(),
        None if stderr.is_empty() => "herdr-app failed with no output".to_owned(),
        None => stderr.trim_start_matches("herdr-app: ").to_owned(),
    };
    if error.is_some_and(|e| e.starts_with(NOT_YET)) { Try::Again(reason) } else { Try::GiveUp(reason) }
}

/// Run `attempt` until it shows the terminal, up to `tries` times, `pause` apart. On failure,
/// why the last attempt did not show it.
fn retried(tries: u32, pause: Duration, mut attempt: impl FnMut() -> Try) -> Result<(), String> {
    let mut last = String::new();
    for n in 0..tries {
        match attempt() {
            Try::Shown => return Ok(()),
            Try::GiveUp(reason) => return Err(reason),
            Try::Again(reason) => last = reason,
        }
        if n + 1 < tries {
            std::thread::sleep(pause);
        }
    }
    Err(last)
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

/// Show the terminal `address` and let its held review start. When herdr-app did not show it,
/// returns why; the review still starts, and the caller says how to attach.
pub(crate) fn show_and_release(
    hc: &Hc,
    address: &str,
    placement: Placement,
    direction: SplitDirection,
) -> Result<Option<String>> {
    let args = show_args(placement, direction, address);
    let shown = retried(SHOW_TRIES, SHOW_PAUSE, || show(Path::new("herdr-app"), &args));
    release(hc, address, shown.is_ok())?;
    Ok(shown.err())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn strs(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    /// How showing ended and how many attempts it took, when each attempt answers the next of
    /// `answers` (and says "not yet" once they run out).
    fn showing(tries: u32, answers: &[Try]) -> (Result<(), String>, usize) {
        let mut calls = 0;
        let shown = retried(tries, Duration::ZERO, || {
            calls += 1;
            answers
                .get(calls - 1)
                .cloned()
                .unwrap_or_else(|| Try::Again("no running terminal \"t9\"".to_owned()))
        });
        (shown, calls)
    }

    #[test]
    fn showing_waits_for_herdr_app_to_know_the_terminal_but_not_past_a_refusal_that_stays() {
        let not_yet = Try::Again("no running terminal \"t9\"".to_owned());
        assert_eq!(
            showing(5, &[not_yet.clone(), not_yet, Try::Shown]),
            (Ok(()), 3),
            "stops at the first success"
        );
        assert_eq!(
            showing(5, &[]),
            (Err("no running terminal \"t9\"".to_owned()), 5),
            "a refusal that never passes ends after the last try, and says why"
        );
        let gone = Try::GiveUp("no Herdr window at /tmp/herdr-app.sock".to_owned());
        assert_eq!(showing(5, &[gone]), (Err("no Herdr window at /tmp/herdr-app.sock".to_owned()), 1));
    }

    #[test]
    fn only_an_unknown_terminal_is_asked_about_again_and_every_refusal_keeps_its_reason() {
        let not_yet = refusal("{\n  \"error\": \"no running terminal \\\"t9\\\"\"\n}\n", "");
        assert_eq!(not_yet, Try::Again("no running terminal \"t9\"".to_owned()));
        let unknown = refusal(r#"{"error":"unknown command \"terminal.peek\""}"#, "");
        assert_eq!(unknown, Try::GiveUp("unknown command \"terminal.peek\"".to_owned()));
        let no_window = refusal("", "herdr-app: no Herdr window at /tmp/h.sock: Connection refused\n");
        assert_eq!(no_window, Try::GiveUp("no Herdr window at /tmp/h.sock: Connection refused".to_owned()));
        assert_eq!(refusal("", ""), Try::GiveUp("herdr-app failed with no output".to_owned()));
    }

    /// A `herdr-app` that refuses the first `not-yet` attempts the way the window does for a
    /// terminal it has not seen, then shows it, or fails before reaching a window when `fail`
    /// is set. Returns the script.
    #[cfg(unix)]
    fn fake_app(tag: &str, not_yet: u32, fail: bool) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("plannotator-tui-herdr-app-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = format!(
            r#"#!/bin/sh
echo "$*" >> {dir}/log
if [ {fail} = 1 ]; then echo "herdr-app: no Herdr window at {dir}/sock: No such file" >&2; exit 2; fi
if [ "$(wc -l < {dir}/log)" -le {not_yet} ]; then printf '{{\n  "error": "no running terminal \\"%s\\""\n}}\n' "$4"; exit 1; fi
echo '{{}}'
"#,
            dir = dir.display(),
            fail = u8::from(fail),
        );
        let bin = dir.join("herdr-app");
        crate::test_support::write_script(&bin, &script);
        bin
    }

    #[cfg(unix)]
    #[test]
    fn a_real_herdr_app_answer_is_retried_while_the_window_catches_up_and_not_without_a_window() {
        let args = show_args(Placement::Overlay, SplitDirection::Right, "t9");
        let catching_up = fake_app("late", 2, false);
        assert_eq!(retried(5, Duration::ZERO, || show(&catching_up, &args)), Ok(()));
        let calls =
            |bin: &Path| std::fs::read_to_string(bin.with_file_name("log")).map(|l| l.lines().count());
        assert_eq!(calls(&catching_up).ok(), Some(3));
        let late = fake_app("never", 99, false);
        assert_eq!(
            retried(3, Duration::ZERO, || show(&late, &args)),
            Err("no running terminal \"t9\"".to_owned())
        );
        let windowless = fake_app("windowless", 0, true);
        let err = retried(5, Duration::ZERO, || show(&windowless, &args)).expect_err("no window");
        assert!(err.starts_with("no Herdr window at "), "{err}");
        assert_eq!(calls(&windowless).ok(), Some(1), "a missing window is not asked again");
        for bin in [catching_up, late, windowless] {
            bin.parent().map(std::fs::remove_dir_all);
        }
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
