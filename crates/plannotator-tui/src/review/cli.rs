//! `plannotator-tui review [--staged | --base REF | --commit SHA | PR] [DIR]`, and the event
//! loop that hosts the diff viewer. `PR` is a GitHub PR link, `N` or `#N` (also `--pr PR`).
//!
//! The viewer works on its own threads and says so on a wake channel; keys arrive on a
//! reader thread that pokes the same channel. The loop waits on it, with a 100 ms tick so
//! the viewer's progressive drawing and the Pi link are never starved.

use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use anyhow::{Context as _, Result};
use plannotator_tui_review::{PrSpec, Request};
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
            anyhow::ensure!(request.is_none(), "pick one of --staged, --base, --commit and a pull request");
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
                "--pr" => {
                    let pr = rest
                        .next()
                        .filter(|v| !v.starts_with("--"))
                        .context("--pr needs a PR link or number")?;
                    anyhow::ensure!(
                        PrSpec::parse(pr).is_some(),
                        "--pr takes a PR link (https://github.com/OWNER/REPO/pull/N), N or #N, not {pr:?}"
                    );
                    set(Request::PullRequest { pr: pr.clone() }, &mut request)?;
                }
                flag if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{}", crate::cli::USAGE),
                // A PR link, `#N` or a bare number is a pull request; a folder named only by
                // digits is `./123`.
                pr if PrSpec::parse(pr).is_some() => {
                    set(Request::PullRequest { pr: pr.to_owned() }, &mut request)?;
                }
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
    open(ReviewArgs::parse(args)?)
}

/// [`run`] for arguments already parsed: from the command line, or from a Herdr pane.
pub(super) fn open(args: ReviewArgs) -> Result<()> {
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
        // The diff draws as `[review]` says, with the choices remembered from the last review
        // over it; the review remembers its own in the data folder.
        let remembered = super::settings::state::path(&data_dir);
        let settings = super::settings::Settings::load(session.config.review, Some(remembered));
        let mut screen =
            Screen::open_with(repo, capture, &data_dir, crate::cli::delivery(true), wake.clone(), settings)?;
        screen.start_notes_rail(session.config.ui.notes_rail);
        screen.set_keys(session.config.keys, session.shift_enter);
        screen.enable_clipboard();
        screen.set_copy_on_select(session.config.ui.copy_on_select);
        screen.start_pi_link();
        if let Some(notice) = session.config_notice {
            screen.add_notice(notice);
        }
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
            // A drag held past the diff's edge scrolls on the tick.
            let wait = if screen.held_at_edge() { crate::ui::drag::STEP } else { TICK };
            let _ = ready.recv_timeout(wait);
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
        let link = "https://github.com/o/r/pull/12";
        for (args, pr, dir) in [
            (vec!["12"], "12", None),
            (vec!["#12", "app"], "#12", Some("app")),
            (vec!["app", link], link, Some("app")),
            (vec!["--pr", "12"], "12", None),
        ] {
            let parsed = parse(&args).expect("a PR");
            assert_eq!(parsed.request, Request::PullRequest { pr: pr.into() });
            assert_eq!(parsed.dir, dir.map(PathBuf::from));
        }
        assert_eq!(parse(&["./12"]).expect("a folder").dir, Some(PathBuf::from("./12")));
        assert!(parse(&["12", "--staged"]).is_err());
        assert!(parse(&["--pr", "main"]).is_err());
        assert!(parse(&["12", "13"]).is_err());
        assert!(parse(&["--base"]).is_err());
        assert!(parse(&["--wat"]).is_err());
    }
}
