//! `plannotator-tui hc …`: the launchers an agent or a person runs in an hc terminal, and
//! `hc pane`, the entry point of the terminal they start.

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::command::Hc;
use super::launch::{self, Gate, Review};
use crate::cli::{USAGE, gated_pane, interactive, last_command, review_pane, wait_on_error};
use crate::config::{Config, Placement};
use crate::gate::{self, GateError};

/// `plannotator-tui hc open|last|review`: start a review of the agent in an hc terminal and show
/// it. The terminal defaults to `HC_TERMINAL_ID`, so an agent running this from its own terminal
/// reviews itself; that is how it hands you a plan. `open --gate` adds Approve, as
/// `herdr open --gate` does. `hc pane` is what the review terminal runs. A gated open with a
/// result file fails as a [`GateError`], like the direct gate.
pub(crate) fn run(args: &[String]) -> Result<()> {
    let strict = args.first().is_some_and(|sub| sub == "open") && gate::is_strict(args);
    launch_review(args).map_err(|err| if strict { GateError(err).into() } else { err })
}

fn launch_review(args: &[String]) -> Result<()> {
    let mut review = match args.first().map(String::as_str) {
        Some("last") => Review::Last { newest: false, session: None },
        Some("open") => Review::Open(None, None),
        Some("review") => return code_review(args.get(1..).unwrap_or_default()),
        Some("pane") => return pane(args.get(1..).unwrap_or_default()),
        _ => anyhow::bail!(USAGE),
    };
    let mut terminal = None;
    let mut placement = None;
    let mut gated = false;
    let mut result_file = None;
    let mut rest = args.get(1..).unwrap_or_default().iter();
    while let Some(arg) = rest.next() {
        match (arg.as_str(), &mut review) {
            ("--terminal", _) => terminal = Some(rest.next().context("--terminal needs a value")?.clone()),
            ("--placement", _) => {
                let value = rest.next().context("--placement needs a value")?;
                placement = Some(value.parse::<Placement>()?);
            }
            ("--newest", Review::Last { newest, .. }) => *newest = true,
            ("--session", Review::Last { session, .. }) => {
                // An unset variable expands to nothing, and an empty path would mean the folder.
                let value = rest.next().filter(|v| !v.is_empty()).context("--session needs a path")?;
                let path = crate::workspace_paths::absolute(std::path::Path::new(value));
                anyhow::ensure!(path.is_file(), "--session {} is not a file", path.display());
                *session = Some(path);
            }
            ("--gate", Review::Open(..)) => gated = true,
            ("--result-file", Review::Open(..)) => {
                let value = rest.next().filter(|v| !v.starts_with("--"));
                let value = value.context("missing value for --result-file")?;
                result_file = Some(crate::workspace_paths::absolute(std::path::Path::new(value)));
            }
            ("--gate" | "--result-file", _) => anyhow::bail!("{arg} is only for `hc open`"),
            ("--newest" | "--session", _) => anyhow::bail!("{arg} is only for `hc last`"),
            (flag, _) if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{USAGE}"),
            (path, Review::Open(open @ None, _)) => {
                *open = Some(crate::workspace_paths::absolute(path.as_ref()));
            }
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
    let target = target_terminal(terminal)?;
    let config = Config::load()?;
    let placement = placement.unwrap_or(config.herdr.placement);
    let hc = Hc::from_lookup(|k| std::env::var(k).ok());
    launch::run(&hc, &target, &review, placement, config.herdr.split_direction)
}

/// `--terminal`, else `HC_TERMINAL_ID`: the terminal the launcher was run from.
fn target_terminal(terminal: Option<String>) -> Result<String> {
    terminal
        .or_else(|| std::env::var("HC_TERMINAL_ID").ok().filter(|t| !t.is_empty()))
        .context("no hc terminal: pass --terminal or run inside hc (HC_TERMINAL_ID is not set)")
}

/// `plannotator-tui hc review [--staged | --base REF | --commit SHA | PR]`: a code review of the
/// terminal's repository. A folder outside a repository, or with nothing to review, fails
/// before any terminal starts.
fn code_review(args: &[String]) -> Result<()> {
    #[cfg(unix)]
    {
        review_terminal(args)
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        anyhow::bail!("code review is not available on Windows yet")
    }
}

#[cfg(unix)]
fn review_terminal(args: &[String]) -> Result<()> {
    let mut terminal = None;
    let mut placement = None;
    let mut scope = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--terminal" => terminal = Some(rest.next().context("--terminal needs a value")?.clone()),
            "--placement" => {
                let value = rest.next().context("--placement needs a value")?;
                placement = Some(value.parse::<Placement>()?);
            }
            _ => scope.push(arg.clone()),
        }
    }
    let refusal = "hc review takes no folder: it reviews the terminal's repository";
    let request = crate::review::scope::request(&scope, refusal)?;
    let target = target_terminal(terminal)?;
    let config = Config::load()?;
    let hc = Hc::from_lookup(|k| std::env::var(k).ok());
    let fg = launch::agent(&hc, &target)?;
    let folder = fg.cwd.clone().with_context(|| format!("{target} reports no folder"))?;
    let (repo, scope) = crate::review::scope::changes(&folder, &request)?;
    let placement = placement.unwrap_or(config.herdr.placement);
    launch::start(&hc, &target, &fg, &Review::Code { scope, repo }, placement, config.herdr.split_direction)
}

/// The review terminal's entry point. hc removes the terminal when this exits, so an error
/// waits for Enter rather than flash by unread. `--held` makes it wait for Enter first: the
/// launcher presses it once herdr-app shows the terminal, so the review asks for the terminal's
/// colours after herdr-app has told it what they are.
fn pane(args: &[String]) -> Result<()> {
    let args = match args.split_first() {
        Some((flag, rest)) if flag == "--held" => {
            let _ = std::io::stdin().read_line(&mut String::new());
            rest
        }
        _ => args,
    };
    let result = match args.first().map(String::as_str) {
        Some("last") => last_command(args.get(1..).unwrap_or_default()),
        Some("open") => match args.get(1..).unwrap_or_default() {
            [path] => interactive(&PathBuf::from(path)),
            [path, gate] if gate == "--gate" => gated_pane(&PathBuf::from(path), None),
            [path, gate, flag, file] if gate == "--gate" && flag == "--result-file" => {
                gated_pane(&PathBuf::from(path), Some(std::path::Path::new(file)))
            }
            _ => Err(anyhow::anyhow!(
                "hc pane open takes one path, then --gate [--result-file <path>]\n{USAGE}"
            )),
        },
        Some("review") => match args.get(1..).unwrap_or_default() {
            [scope, repo] => review_pane(scope, Some(std::path::Path::new(repo))),
            _ => Err(anyhow::anyhow!("hc pane review takes a scope and a repository\n{USAGE}")),
        },
        _ => Err(anyhow::anyhow!(USAGE)),
    };
    if let Err(err) = &result {
        wait_on_error(err);
    }
    result
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    fn refusal(list: &[&str]) -> String {
        run(&args(list)).map(drop).err().map(|e| e.to_string()).unwrap_or_default()
    }

    #[test]
    fn the_gate_belongs_to_open_and_a_result_file_needs_it() {
        assert_eq!(refusal(&["last", "--gate"]), "--gate is only for `hc open`");
        assert_eq!(refusal(&["last", "--result-file", "/tmp/x.json"]), "--result-file is only for `hc open`");
        assert_eq!(refusal(&["open", "--result-file", "/tmp/x.json"]), "--result-file requires --gate");
        let strict = run(&args(&["open", "--result-file", "/tmp/x.json"])).expect_err("refused");
        assert!(strict.is::<GateError>(), "a result file makes every failure a gate error");
    }

    #[test]
    fn a_gated_hc_open_is_a_launch_not_a_direct_review_of_a_file_named_hc() {
        let launched = crate::cli::run(&args(&["hc", "open", "--result-file", "/tmp/x.json"]));
        assert_eq!(launched.expect_err("refused").to_string(), "--result-file requires --gate");
    }

    #[test]
    fn a_transcript_that_is_missing_empty_or_a_folder_is_refused_by_the_launcher() {
        assert_eq!(refusal(&["last", "--session", ""]), "--session needs a path");
        assert_eq!(refusal(&["last", "--session"]), "--session needs a path");
        let folder = std::env::temp_dir().display().to_string();
        let message = refusal(&["last", "--session", &folder]);
        assert!(message.starts_with("--session ") && message.ends_with(" is not a file"), "{message}");
        assert!(refusal(&["last", "--session", "/no/such/transcript.jsonl"]).ends_with("is not a file"));
    }

    #[cfg(unix)]
    #[test]
    fn a_code_review_refuses_a_folder_and_a_mistyped_flag_before_asking_hc() {
        let folder = "hc review takes no folder: it reviews the terminal's repository";
        assert_eq!(refusal(&["review", "../other"]), folder);
        assert!(refusal(&["review", "--newest"]).starts_with("unknown flag --newest"));
        assert_eq!(refusal(&["review", "--base"]), "--base needs a ref");
        assert!(refusal(&["review", "--placement", "sideways"]).contains("sideways"));
    }

    #[cfg(not(unix))]
    #[test]
    fn a_code_review_says_it_is_not_on_windows_yet() {
        assert_eq!(refusal(&["review"]), "code review is not available on Windows yet");
    }

    #[test]
    fn a_mistyped_flag_is_refused_before_anything_starts() {
        assert_eq!(refusal(&["open", "--session", "/s/a.jsonl"]), "--session is only for `hc last`");
        assert_eq!(refusal(&["open", "--newest"]), "--newest is only for `hc last`");
        assert!(refusal(&["last", "--bogus"]).starts_with("unknown flag --bogus"));
        assert!(refusal(&["open", "a.md", "b.md"]).starts_with("unexpected argument \"b.md\""));
        assert!(refusal(&["last", "--placement", "sideways"]).contains("sideways"));
    }
}
