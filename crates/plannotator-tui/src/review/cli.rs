//! `plannotator-tui review [--staged | --unstaged | --base REF | --commit SHA | PR | --pick]
//! [DIR]`, and the event loop that hosts the diff viewer and the change picker. `PR` is a
//! GitHub PR link, `N` or `#N` (also `--pr PR`). `--pick` opens the picker
//! ([`super::pick`]); so does a bare `review` with nothing to show, on a terminal.
//!
//! The viewer works on its own threads and says so on a wake channel; keys arrive on a
//! reader thread that pokes the same channel. The loop waits on it, with a 100 ms tick so
//! the viewer's progressive drawing and the Pi link are never starved.

use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use anyhow::{Context as _, Result};
use plannotator_tui_review::{PrSpec, Repo, Request};
use ratatui::crossterm::event::{self, Event};

use super::pick::shown::{Opener, Shown, Start};

/// The longest the loop sleeps without work.
const TICK: Duration = Duration::from_millis(100);

/// What `review` was asked to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewArgs {
    pub(crate) target: Target,
    pub(crate) dir: Option<PathBuf>,
}

/// One review, or the change picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Review(Request),
    /// `--pick`: the folder only has to be in a repository; nothing is captured first.
    Pick,
}

impl Target {
    /// A bare `review` (no scope named): with nothing to show, it picks instead.
    pub(crate) fn picks_when_empty(&self) -> bool {
        matches!(self, Self::Review(Request::Uncommitted { .. }))
    }
}

impl ReviewArgs {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let mut request = None;
        let mut dir = None;
        let mut rest = args.iter();
        let set = |next: Target, request: &mut Option<Target>| {
            match (&*request, &next) {
                (None, _) => {}
                (Some(Target::Pick), _) | (_, Target::Pick) => anyhow::bail!(
                    "--pick chooses what to review in the picker: drop --staged, --unstaged, --base, \
                     --commit or the pull request"
                ),
                (Some(_), _) => {
                    anyhow::bail!("pick one of --staged, --unstaged, --base, --commit and a pull request")
                }
            }
            *request = Some(next);
            Ok(())
        };
        let review = |request: Request| Target::Review(request);
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--staged" => set(review(Request::Staged), &mut request)?,
                "--unstaged" => set(review(Request::Unstaged), &mut request)?,
                "--pick" => set(Target::Pick, &mut request)?,
                "--base" => {
                    let base = rest.next().filter(|v| !v.starts_with("--")).context("--base needs a ref")?;
                    set(review(Request::Branch { base: Some(base.clone()) }), &mut request)?;
                }
                "--commit" => {
                    let rev =
                        rest.next().filter(|v| !v.starts_with("--")).context("--commit needs a commit")?;
                    set(review(Request::Commit { rev: rev.clone() }), &mut request)?;
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
                    set(review(Request::PullRequest { pr: pr.clone() }), &mut request)?;
                }
                flag if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{}", crate::cli::USAGE),
                // A PR link, `#N` or a bare number is a pull request; a folder named only by
                // digits is `./123`.
                pr if PrSpec::parse(pr).is_some() => {
                    set(review(Request::PullRequest { pr: pr.to_owned() }), &mut request)?;
                }
                path if dir.is_none() => dir = Some(PathBuf::from(path)),
                extra => anyhow::bail!("unexpected argument {extra:?}\n{}", crate::cli::USAGE),
            }
        }
        Ok(Self { target: request.unwrap_or(Target::Review(Request::Uncommitted { base: None })), dir })
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
    let start = starting(&dir, &args.target, std::io::stdout().is_terminal())?;
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
        let opener = Opener {
            remembered: Some(super::settings::state::path(&data_dir)),
            data_dir,
            wake: wake.clone(),
            review: session.config.review,
            notes_rail: session.config.ui.notes_rail,
            keys: session.config.keys,
            shift_enter: session.shift_enter,
            copy_on_select: session.config.ui.copy_on_select,
            notice: session.config_notice,
            delivery: Box::new(|| crate::cli::delivery(true)),
            live: true,
        };
        let mut shown = Shown::start(start, opener)?;
        let events = read_events(wake)?;
        let result = event_loop(terminal, &mut shown, &ready, &events);
        // Join the viewer's workers either way, and report their failure only when the
        // loop itself did not fail first.
        let closed = shown.close();
        result.and(closed)
    })
}

/// What the screen opens on: the review `target` names, or the picker. A folder outside a
/// repository fails here, as a review with nothing to show does, unless it is a bare
/// `review` on a `terminal`, which picks instead.
fn starting(dir: &std::path::Path, target: &Target, terminal: bool) -> Result<Start> {
    let request = match target {
        Target::Pick => return Ok(Start::Pick(Repo::discover(dir)?)),
        Target::Review(request) => request,
    };
    let (repo, capture) = super::capture_any(dir, request)?;
    match super::nothing_to_review(&repo, &capture) {
        None => Ok(Start::Review(repo, Box::new(capture))),
        // The picker says `Nothing to review in app` itself once it has looked.
        Some(_) if terminal && target.picks_when_empty() => Ok(Start::Pick(repo)),
        Some(nothing) => anyhow::bail!(nothing),
    }
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
    screen: &mut Shown,
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
        let review = |request: Request| Target::Review(request);
        assert_eq!(parse(&[]).expect("bare").target, review(Request::Uncommitted { base: None }));
        assert_eq!(parse(&["--unstaged"]).expect("unstaged").target, review(Request::Unstaged));
        assert_eq!(parse(&["--staged", "app"]).expect("staged").dir, Some(PathBuf::from("app")));
        assert_eq!(
            parse(&["--base", "origin/main"]).expect("base").target,
            review(Request::Branch { base: Some("origin/main".into()) })
        );
        assert_eq!(
            parse(&["--commit", "HEAD~1"]).expect("commit").target,
            review(Request::Commit { rev: "HEAD~1".into() })
        );
        assert!(parse(&["--staged", "--commit", "HEAD"]).is_err());
        assert!(parse(&["--staged", "--unstaged"]).is_err());
        let link = "https://github.com/o/r/pull/12";
        for (args, pr, dir) in [
            (vec!["12"], "12", None),
            (vec!["#12", "app"], "#12", Some("app")),
            (vec!["app", link], link, Some("app")),
            (vec!["--pr", "12"], "12", None),
        ] {
            let parsed = parse(&args).expect("a PR");
            assert_eq!(parsed.target, review(Request::PullRequest { pr: pr.into() }));
            assert_eq!(parsed.dir, dir.map(PathBuf::from));
        }
        assert_eq!(parse(&["./12"]).expect("a folder").dir, Some(PathBuf::from("./12")));
        assert!(parse(&["12", "--staged"]).is_err());
        assert!(parse(&["--pr", "main"]).is_err());
        assert!(parse(&["12", "13"]).is_err());
        assert!(parse(&["--base"]).is_err());
        assert!(parse(&["--wat"]).is_err());
    }

    #[test]
    fn pick_takes_a_folder_and_refuses_every_scope_beside_it() {
        assert_eq!(parse(&["--pick"]).expect("pick").target, Target::Pick);
        let both = parse(&["app", "--pick"]).expect("pick a folder");
        assert_eq!((both.target, both.dir), (Target::Pick, Some(PathBuf::from("app"))));
        for args in [
            &["--pick", "--staged"][..],
            &["--unstaged", "--pick"],
            &["--pick", "--base", "main"],
            &["--commit", "HEAD", "--pick"],
            &["--pick", "12"],
            &["--pick", "--pick"],
        ] {
            let err = parse(args).expect_err("refused").to_string();
            assert!(err.starts_with("--pick chooses what to review in the picker"), "{args:?}: {err}");
        }
    }

    /// A repository with one commit and a clean tree, in a folder of its own.
    fn clean_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pt-review-start-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git").arg("-C").arg(&dir).args(args).status().expect("git");
            assert!(ok.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").expect("write");
        git(&["add", "."]);
        git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn a_bare_review_with_nothing_to_show_picks_on_a_terminal_and_fails_as_before_elsewhere() {
        let dir = clean_repo("bare");
        let bare = Target::Review(Request::Uncommitted { base: None });
        assert!(
            matches!(starting(&dir, &bare, true), Ok(Start::Pick(_))),
            "a bare review on a terminal picks"
        );
        let err = starting(&dir, &bare, false).err().expect("not a terminal").to_string();
        assert!(err.starts_with("No uncommitted changes in "), "{err}");
        // A scope named is that scope: empty, it fails on a terminal too.
        for named in [Request::Staged, Request::Unstaged] {
            assert!(starting(&dir, &Target::Review(named.clone()), true).is_err(), "{named:?}");
        }
        assert!(matches!(starting(&dir, &Target::Pick, false), Ok(Start::Pick(_))));
        std::fs::write(dir.join("a.txt"), "two\n").expect("edit");
        assert!(matches!(starting(&dir, &bare, true), Ok(Start::Review(..))), "changes open the review");
        let outside = std::env::temp_dir().join(format!("pt-review-start-none-{}", std::process::id()));
        std::fs::create_dir_all(&outside).expect("dir");
        assert!(starting(&outside, &Target::Pick, true).is_err(), "not a repository fails before any screen");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
