//! `plannotator-tui rex …`: the launchers a Rex action or an agent runs, and `rex pane`, the
//! entry point of the split they open.

#![allow(clippy::print_stdout, reason = "`--print` is a stdout contract")]

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::command::Rex;
use super::launch::{self, Gate, Handoff, Review};
use crate::cli::{USAGE, gated_pane, interactive, last_command, lines_value, wait_on_error};
use crate::gate::{self, GateError};

/// `plannotator-tui rex open|last|terminal|find`: open a review in a split beside a block. The block
/// defaults to `REX_BLOCK`, so an agent running this from its own block reviews itself.
/// `--paste` leaves the feedback in the agent's prompt instead of sending it; `open --gate`
/// adds Approve, as `herdr open --gate` does. `rex pane` is what the split runs. A gated open
/// with a result file fails as a [`GateError`], like the direct gate.
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
        Some("pane") => return pane(args.get(1..).unwrap_or_default()),
        _ => anyhow::bail!(USAGE),
    };
    let mut block = None;
    let mut session = None;
    let mut handoff = Handoff::Send;
    let mut print = false;
    let mut gated = false;
    let mut result_file = None;
    let mut rest = args.get(1..).unwrap_or_default().iter();
    while let Some(arg) = rest.next() {
        match (arg.as_str(), &mut review) {
            ("--block", _) => block = Some(rest.next().context("--block needs a value")?.clone()),
            ("--session", _) => session = Some(rest.next().context("--session needs a value")?.clone()),
            ("--paste", _) => handoff = Handoff::Paste,
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
    let block = block
        .or_else(|| std::env::var("REX_BLOCK").ok().filter(|b| !b.is_empty()))
        .context("no Rex block: pass --block or run inside Rex (REX_BLOCK is not set)")?;
    let rex = Rex::from_lookup(|k| std::env::var(k).ok(), session)?;
    if let (true, Review::Terminal { lines }) = (print, &review) {
        print!("{}", crate::terminal::document(&super::terminal::read(&rex, &block, *lines)?));
        return Ok(());
    }
    launch::run(&rex, &block, &review, handoff)
}

/// The split's entry point. Rex closes the split when this exits, so an error waits for
/// Enter rather than flash by unread.
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
        _ => Err(anyhow::anyhow!(USAGE)),
    };
    if let Err(err) = &result {
        wait_on_error(err);
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
    fn a_gated_rex_open_is_a_launch_not_a_direct_review_of_a_file_named_rex() {
        let launched = crate::cli::run(&args(&["rex", "open", "--result-file", "/tmp/x.json"]));
        let err = launched.expect_err("refused").to_string();
        assert_eq!(err, "--result-file requires --gate");
    }
}
