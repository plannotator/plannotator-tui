//! `plannotator-tui review [--staged | --base REF | --commit SHA] [DIR]`, and the event
//! loop that hosts the diff viewer.
//!
//! The viewer works on its own threads and says so on a wake channel; keys arrive on a
//! reader thread that pokes the same channel. The loop waits on it, with a 100 ms tick so
//! the viewer's progressive drawing and the Pi link are never starved.

use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use anyhow::{Context as _, Result};
use plannotator_tui_review::Request;
use ratatui::crossterm::event::{self, Event};

use super::Screen;

/// The longest the loop sleeps without work.
const TICK: Duration = Duration::from_millis(100);

/// What `review` was asked to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewArgs {
    pub(crate) request: Request,
    pub(crate) dir: Option<PathBuf>,
}

impl ReviewArgs {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let mut request = None;
        let mut dir = None;
        let mut rest = args.iter();
        let set = |next: Request, request: &mut Option<Request>| {
            anyhow::ensure!(request.is_none(), "pick one of --staged, --base and --commit");
            *request = Some(next);
            Ok(())
        };
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--staged" => set(Request::Staged, &mut request)?,
                "--base" => {
                    let base = rest.next().filter(|v| !v.starts_with("--")).context("--base needs a ref")?;
                    set(Request::Branch { base: Some(base.clone()) }, &mut request)?;
                }
                "--commit" => {
                    let rev =
                        rest.next().filter(|v| !v.starts_with("--")).context("--commit needs a commit")?;
                    set(Request::Commit { rev: rev.clone() }, &mut request)?;
                }
                flag if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{}", crate::cli::USAGE),
                path if dir.is_none() => dir = Some(PathBuf::from(path)),
                extra => anyhow::bail!("unexpected argument {extra:?}\n{}", crate::cli::USAGE),
            }
        }
        Ok(Self { request: request.unwrap_or(Request::Uncommitted { base: None }), dir })
    }
}

/// Capture the diff first, so a folder that is no repository or has no changes fails
/// before the screen is taken; then run the review until it quits.
pub(crate) fn run(args: &[String]) -> Result<()> {
    let args = ReviewArgs::parse(args)?;
    let dir = match args.dir {
        Some(dir) => crate::workspace_paths::absolute(&dir),
        None => std::env::current_dir().context("current directory")?,
    };
    let (repo, capture) = super::capture(&dir, &args.request)?;
    // The review is drawn on stdout; redirected, there is nothing to draw on.
    anyhow::ensure!(
        std::io::stdout().is_terminal(),
        "the code review is drawn on the terminal, and stdout is not one"
    );
    let data_dir = crate::workspace_paths::data_dir();
    crate::cli::with_terminal(|terminal, session| {
        let (wake, ready) = mpsc::sync_channel(1);
        let mut screen = Screen::open(repo, capture, &data_dir, crate::cli::delivery(true), wake.clone())?;
        screen.set_keys(session.config.keys, session.shift_enter);
        screen.enable_clipboard();
        screen.start_pi_link();
        let events = read_events(wake)?;
        let result = event_loop(terminal, &mut screen, &ready, &events);
        // Join the viewer's workers either way, and report their failure only when the
        // loop itself did not fail first.
        let closed = screen.close();
        result.and(closed)
    })
}

/// Read the terminal on a thread of its own, poking `wake` after each event.
fn read_events(wake: SyncSender<()>) -> Result<Receiver<Event>> {
    let (send, events) = mpsc::channel();
    std::thread::Builder::new()
        .name("review-input".into())
        .spawn(move || {
            while let Ok(event) = event::read() {
                if send.send(event).is_err() {
                    break;
                }
                let _ = wake.try_send(());
            }
        })
        .context("starting the input reader")?;
    Ok(events)
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    screen: &mut Screen,
    ready: &Receiver<()>,
    events: &Receiver<Event>,
) -> Result<()> {
    let mut dirty = true;
    while !screen.wants_quit() {
        // Clear the wake token before draining, so work that lands after it wakes us again.
        let _ = ready.try_recv();
        dirty |= screen.tick()?;
        if dirty {
            let mut drawn = Ok(());
            terminal.draw(|frame| drawn = screen.draw(frame))?;
            drawn?;
            screen.after_draw();
            dirty = false;
        }
        let mut handled = false;
        loop {
            match events.try_recv() {
                Ok(event) => {
                    screen.handle_event(event)?;
                    handled = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => anyhow::bail!("the terminal stopped sending input"),
            }
        }
        if handled {
            dirty = true;
        } else {
            let _ = ready.recv_timeout(TICK);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ReviewArgs> {
        ReviewArgs::parse(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn scopes_parse_and_exclude_each_other() {
        assert_eq!(parse(&[]).expect("bare").request, Request::Uncommitted { base: None });
        assert_eq!(parse(&["--staged", "app"]).expect("staged").dir, Some(PathBuf::from("app")));
        assert_eq!(
            parse(&["--base", "origin/main"]).expect("base").request,
            Request::Branch { base: Some("origin/main".into()) }
        );
        assert_eq!(
            parse(&["--commit", "HEAD~1"]).expect("commit").request,
            Request::Commit { rev: "HEAD~1".into() }
        );
        assert!(parse(&["--staged", "--commit", "HEAD"]).is_err());
        assert!(parse(&["--base"]).is_err());
        assert!(parse(&["--wat"]).is_err());
    }
}
