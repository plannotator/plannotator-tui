//! `plannotator-tui rex …`: the launchers a Rex action or an agent runs, and `rex pane`, the
//! entry point of the pane they open.

#![allow(clippy::print_stdout, reason = "`--print` is a stdout contract")]

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::command::Rex;
use super::launch::{self, Gate, Handoff, Review};
use crate::cli::{USAGE, gated_pane, interactive, last_command, lines_value, review_pane, wait_on_error};
use crate::config::RexPlacement;
use crate::gate::{self, GateError};

/// `plannotator-tui rex open|last|terminal|find|review`: open a review full size in a tab of
/// its own (`tab`, the default), beside a block (`split`) or over it (`popup`), as
/// `--placement` or `[rex] placement` says. The block defaults to `REX_BLOCK`, so an agent
/// running this from its own block reviews itself. `rex install` writes the Rex Annotate
/// module. `--paste` leaves the feedback in the agent's prompt instead of sending it;
/// `open --gate` adds Approve, as `herdr open --gate` does. `rex pane` is what the review pane
/// runs. A gated open with a result file fails as a [`GateError`], like the direct gate.
pub(crate) fn run(args: &[String]) -> Result<()> {
    let strict = args.first().is_some_and(|sub| sub == "open") && gate::is_strict(args);
    launch_review(args).map_err(|err| if strict { GateError(err).into() } else { err })
}

fn launch_review(args: &[String]) -> Result<()> {
    let mut review = match args.first().map(String::as_str) {
        Some("last") => Review::Last { newest: false },
        Some("open") => Review::Open(None, None),
        Some("terminal") => Review::Terminal { lines: crate::terminal::DEFAULT_LINES },
        Some("find") => Review::Find { lines: crate::terminal::DEFAULT_LINES },
        Some("review") => return code_review(args.get(1..).unwrap_or_default()),
        Some("pane") => return pane(args.get(1..).unwrap_or_default()),
        Some("install") => return super::install::run(args.get(1..).unwrap_or_default()),
        _ => anyhow::bail!(USAGE),
    };
    let mut block = None;
    let mut session = None;
    let mut handoff = Handoff::Send;
    let mut print = false;
    let mut gated = false;
    let mut result_file = None;
    let mut placement = None;
    let mut rest = args.get(1..).unwrap_or_default().iter();
    while let Some(arg) = rest.next() {
        match (arg.as_str(), &mut review) {
            ("--block", _) => block = Some(rest.next().context("--block needs a value")?.clone()),
            ("--session", _) => session = Some(rest.next().context("--session needs a value")?.clone()),
            ("--paste", _) => handoff = Handoff::Paste,
            ("--placement", _) => {
                let value = rest.next().context("--placement needs split, popup or tab")?;
                placement = Some(value.parse::<RexPlacement>()?);
            }
            ("--newest", Review::Last { newest }) => *newest = true,
            ("--lines", Review::Terminal { lines } | Review::Find { lines }) => {
                *lines = lines_value(rest.next())?;
            }
            ("--print", Review::Terminal { .. }) => print = true,
            ("--gate", Review::Open(..)) => gated = true,
            ("--result-file", Review::Open(..)) => {
                let value = rest.next().filter(|v| !v.starts_with("--"));
                let value = value.context("missing value for --result-file")?;
                result_file = Some(crate::workspace_paths::absolute(std::path::Path::new(value)));
            }
            ("--gate" | "--result-file", _) => anyhow::bail!("{arg} is only for `rex open`"),
            (flag, _) if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{USAGE}"),
            (path, Review::Open(open @ None, _)) => *open = Some(std::path::absolute(path)?),
            (extra, _) => anyhow::bail!("unexpected argument {extra:?}\n{USAGE}"),
        }
    }
    if let Some(file) = &result_file {
        anyhow::ensure!(gated, "--result-file requires --gate");
        // The pane publishes when the review ends; a name that is already taken would only
        // fail then, after the review.
        gate::check_result_file(file)?;
    }
    if let (true, Review::Open(_, gate)) = (gated, &mut review) {
        *gate = Some(Gate { result_file });
    }
    let block = target_block(block)?;
    let rex = Rex::from_lookup(|k| std::env::var(k).ok(), session)?;
    if let (true, Review::Terminal { lines }) = (print, &review) {
        print!("{}", crate::terminal::document(&super::terminal::read(&rex, &block, *lines)?));
        return Ok(());
    }
    launch::run(&rex, &block, &review, handoff, configured(placement)?)
}

/// `--block`, else `REX_BLOCK`: the block the launcher was run from.
fn target_block(block: Option<String>) -> Result<String> {
    block
        .or_else(|| std::env::var("REX_BLOCK").ok().filter(|b| !b.is_empty()))
        .context("no Rex block: pass --block or run inside Rex (REX_BLOCK is not set)")
}

/// `--placement`, else `[rex] placement`.
fn configured(placement: Option<RexPlacement>) -> Result<RexPlacement> {
    match placement {
        Some(placement) => Ok(placement),
        None => Ok(crate::config::Config::load()?.rex.placement),
    }
}

/// `plannotator-tui rex review [--staged | --unstaged | --base REF | --commit SHA | PR |
/// --pick]`: a code review of the block's repository, beside it. A folder outside a
/// repository, or a scope named with nothing to review, fails before any split opens; a bare
/// review with nothing to show, and `--pick`, open the change picker, which captures nothing
/// first.
fn code_review(args: &[String]) -> Result<()> {
    #[cfg(unix)]
    {
        review_block(args)
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        anyhow::bail!("code review is not available on Windows yet")
    }
}

#[cfg(unix)]
fn review_block(args: &[String]) -> Result<()> {
    let mut block = None;
    let mut session = None;
    let mut handoff = Handoff::Send;
    let mut placement = None;
    let mut scope = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--block" => block = Some(rest.next().context("--block needs a value")?.clone()),
            "--session" => session = Some(rest.next().context("--session needs a value")?.clone()),
            "--paste" => handoff = Handoff::Paste,
            "--placement" => {
                let value = rest.next().context("--placement needs split, popup or tab")?;
                placement = Some(value.parse::<RexPlacement>()?);
            }
            _ => scope.push(arg.clone()),
        }
    }
    let refusal = "rex review takes no folder: it reviews the block's repository";
    let request = crate::review::scope::request(&scope, refusal)?;
    let block = target_block(block)?;
    let rex = Rex::from_lookup(|k| std::env::var(k).ok(), session)?;
    let fg = launch::agent(&rex, &block)?;
    let folder = fg.cwd.clone().with_context(|| format!("block {block} reports no folder"))?;
    let (repo, scope) = crate::review::scope::changes(&folder, &request)?;
    let review = Review::Code { scope, repo };
    launch::open(&rex, &block, &fg, &review, handoff, configured(placement)?)
}

/// The review pane's entry point. Rex closes the pane when this exits, so an error waits for
/// Enter rather than flash by unread. A review in a tab of its own then shows the pane it was
/// opened from again.
fn pane(args: &[String]) -> Result<()> {
    let result = match args.first().map(String::as_str) {
        Some("last") => last_command(args.get(1..).unwrap_or_default()),
        Some("open") => match args.get(1..).unwrap_or_default() {
            [path] => interactive(&PathBuf::from(path)),
            [path, gate] if gate == "--gate" => gated_pane(&PathBuf::from(path), None),
            [path, gate, flag, file] if gate == "--gate" && flag == "--result-file" => {
                gated_pane(&PathBuf::from(path), Some(std::path::Path::new(file)))
            }
            _ => Err(anyhow::anyhow!(
                "rex pane open takes one path, then --gate [--result-file <path>]\n{USAGE}"
            )),
        },
        Some("terminal") => pane_terminal(args.get(1..).unwrap_or_default()),
        Some("review") => match args.get(1..).unwrap_or_default() {
            [scope, repo] => review_pane(scope, Some(std::path::Path::new(repo))),
            _ => Err(anyhow::anyhow!("rex pane review takes a scope and a repository\n{USAGE}")),
        },
        _ => Err(anyhow::anyhow!(USAGE)),
    };
    if let Err(err) = &result {
        wait_on_error(err);
    }
    if let Some(back) = std::env::var(launch::RETURN_VAR).ok().filter(|b| !b.is_empty())
        && let Some(own) = std::env::var("REX_BLOCK").ok().filter(|b| !b.is_empty())
        && let Ok(rex) = Rex::from_lookup(|k| std::env::var(k).ok(), None)
    {
        super::place::show_again(&rex, &own, &back);
    }
    result
}

fn pane_terminal(args: &[String]) -> Result<()> {
    let mut block = None;
    let mut lines = crate::terminal::DEFAULT_LINES;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--block" => block = Some(rest.next().context("--block needs a value")?.clone()),
            "--lines" => lines = lines_value(rest.next())?,
            other => anyhow::bail!("unexpected argument {other:?}\n{USAGE}"),
        }
    }
    let block = block.context("rex pane terminal needs --block")?;
    let rex = Rex::from_lookup(|k| std::env::var(k).ok(), None)?;
    super::terminal::run(&rex, &block, lines)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn the_gate_belongs_to_open_and_a_result_file_needs_it() {
        let err = |list: &[&str]| run(&args(list)).map(drop).err().map(|e| e.to_string()).unwrap_or_default();
        assert_eq!(err(&["last", "--gate"]), "--gate is only for `rex open`");
        assert_eq!(
            err(&["terminal", "--result-file", "/tmp/x.json"]),
            "--result-file is only for `rex open`"
        );
        assert_eq!(err(&["open", "--result-file", "/tmp/x.json"]), "--result-file requires --gate");
        let strict = run(&args(&["open", "--result-file", "/tmp/x.json"])).expect_err("refused");
        assert!(strict.is::<GateError>(), "a result file makes every failure a gate error");
    }

    #[test]
    fn a_placement_is_split_popup_or_tab_and_named_when_wrong() {
        let err = run(&args(&["open", "--placement", "overlay"])).expect_err("refused").to_string();
        assert!(err.contains("\"overlay\"") && err.contains("split, popup or tab"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_code_review_refuses_a_folder_and_a_mistyped_flag_before_asking_rex() {
        let err = |list: &[&str]| run(&args(list)).map(drop).err().map(|e| e.to_string()).unwrap_or_default();
        assert_eq!(
            err(&["review", "../other"]),
            "rex review takes no folder: it reviews the block's repository"
        );
        assert!(err(&["review", "--gate"]).starts_with("unknown flag --gate"));
        assert_eq!(
            err(&["review", "--staged", "--commit", "HEAD"]),
            "pick one of --staged, --unstaged, --base, --commit and a pull request"
        );
        assert!(err(&["review", "--placement", "overlay"]).contains("split, popup or tab"));
    }

    #[cfg(not(unix))]
    #[test]
    fn a_code_review_says_it_is_not_on_windows_yet() {
        let err = run(&args(&["review"])).expect_err("refused").to_string();
        assert_eq!(err, "code review is not available on Windows yet");
    }

    #[test]
    fn a_gated_rex_open_is_a_launch_not_a_direct_review_of_a_file_named_rex() {
        let launched = crate::cli::run(&args(&["rex", "open", "--result-file", "/tmp/x.json"]));
        let err = launched.expect_err("refused").to_string();
        assert_eq!(err, "--result-file requires --gate");
    }
}
