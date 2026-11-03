//! `plannotator-tui hc last|open`: find the agent in an hc terminal, start plannotator-tui in a
//! terminal of its own, wired to send feedback back to the agent, and show it. `foreground`,
//! `argv` and `run_args` are pure; only `run` touches `hc` and herdr-app (through `show`).

#![allow(clippy::print_stdout, reason = "the launcher says where an unshown review is")]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use super::agent::{AGENT_VAR, PID_VAR, TERMINAL_VAR};
use super::command::{BIN_VAR, Hc, HcError};
use super::show::show_and_release;
use crate::config::{Placement, SplitDirection};
use crate::herdr::launch::known_host;

/// What the review terminal shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Review {
    /// The agent's recent replies; `newest` skips the picker. Without a `session`, the newest
    /// transcript in the agent's folder is used, which several agents there can share.
    Last { newest: bool, session: Option<PathBuf> },
    /// A file or folder; the terminal's directory when `None`. With a [`Gate`], the review can
    /// be approved, as `herdr open --gate` does.
    Open(Option<PathBuf>, Option<Gate>),
}

/// `hc open --gate`: Approve beside Send, and where the decision is published.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Gate {
    pub(crate) result_file: Option<PathBuf>,
}

/// The program in the foreground of a terminal, from `hc get` JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Foreground {
    pub(crate) pid: u32,
    pub(crate) cwd: Option<PathBuf>,
    /// The agent's host label when the program is one the hosts crate can read.
    pub(crate) host: Option<&'static str>,
}

/// Read the foreground program. The name it was started by (`argv[0]`) comes before the name
/// the OS gives the process, which for a wrapper can be something else.
pub(crate) fn foreground(facts: &Value) -> Option<Foreground> {
    let fg = facts.get("foreground").filter(|fg| fg.is_object())?;
    let pid = u32::try_from(fg.get("pid")?.as_u64()?).ok()?;
    let argv0 = fg.pointer("/argv/0").and_then(Value::as_str);
    let name = fg.get("name").and_then(Value::as_str);
    let host = [argv0, name].into_iter().flatten().find_map(known_host);
    let cwd = fg.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    Some(Foreground { pid, cwd, host })
}

/// The program the review terminal runs: `env` sets the delivery target, then
/// `plannotator-tui hc pane` opens the review once the launcher releases it (`--held`, see
/// [`super::show`]). Without an agent in the terminal, feedback goes to the clipboard.
pub(crate) fn argv(
    exe: &Path,
    hc: &Path,
    target: &str,
    fg: &Foreground,
    review: &Review,
) -> Result<Vec<String>> {
    let mut args = vec!["env".to_owned()];
    if let Some(host) = fg.host {
        args.push(format!("{TERMINAL_VAR}={target}"));
        args.push(format!("{AGENT_VAR}={host}"));
        args.push(format!("{PID_VAR}={}", fg.pid));
        args.push(format!("{BIN_VAR}={}", hc.display()));
    }
    args.extend([exe.display().to_string(), "hc".to_owned(), "pane".to_owned(), "--held".to_owned()]);
    match review {
        Review::Last { newest, session } => {
            let host = fg.host.with_context(|| format!("no supported agent runs in {target}"))?;
            args.extend(["last".to_owned(), "--pid".to_owned(), fg.pid.to_string()]);
            args.extend(["--host".to_owned(), host.to_owned()]);
            if let Some(session) = session {
                args.extend(["--session".to_owned(), session.display().to_string()]);
            }
            if *newest {
                args.push("--newest".to_owned());
            }
        }
        Review::Open(path, gate) => {
            let path = path.clone().or_else(|| fg.cwd.clone()).context("no file or folder to open")?;
            args.extend(["open".to_owned(), path.display().to_string()]);
            if let Some(gate) = gate {
                args.push("--gate".to_owned());
                if let Some(file) = &gate.result_file {
                    args.extend(["--result-file".to_owned(), file.display().to_string()]);
                }
            }
        }
    }
    Ok(args)
}

/// A terminal name hc accepts (1-32 of `a-z 0-9 - _`) for the review of `target`.
fn terminal_name(target: &str) -> String {
    let tail: String = target
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .take(23)
        .collect();
    format!("annotate-{tail}")
}

/// The `hc run` arguments for `program`: in the background, removed when it exits, and without
/// a note typed into the agent's prompt when it does. The review is not the agent's helper.
fn run_args(name: &str, cwd: Option<&Path>, program: &[String]) -> Vec<String> {
    let mut args: Vec<String> = ["run", "--bg", "--json", "--rm", "--no-notify", "--name", name]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if let Some(cwd) = cwd {
        args.extend(["--cwd".to_owned(), cwd.display().to_string()]);
    }
    args.push("--".to_owned());
    args.extend(program.iter().cloned());
    args
}

/// `hc` as an absolute path: the review terminal may start with a shorter `PATH` than ours.
fn absolute(hc: &Hc) -> PathBuf {
    let bin = hc.bin();
    if bin.is_absolute() {
        return bin.to_path_buf();
    }
    let file = if cfg!(windows) { "hc.exe" } else { "hc" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(file))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| bin.to_path_buf())
}

/// Start the review of `target` and show it. Without herdr-app to show it, the review keeps
/// running and the launcher says how to attach.
pub(crate) fn run(
    hc: &Hc,
    target: &str,
    review: &Review,
    placement: Placement,
    direction: SplitDirection,
) -> Result<()> {
    anyhow::ensure!(
        !target.contains('/'),
        "{target} is on another machine; a review reads the transcripts and files on this one"
    );
    let facts = hc.get(target)?;
    let fg = foreground(&facts).with_context(|| format!("{target} has no foreground program"))?;
    let exe = std::env::current_exe().context("finding the plannotator-tui binary")?;
    let program = argv(&exe, &absolute(hc), target, &fg, review)?;
    let cwd = match review {
        Review::Open(Some(path), _) if path.is_file() => path.parent().map(Path::to_path_buf),
        Review::Open(Some(path), _) => Some(path.clone()),
        _ => fg.cwd.clone(),
    };
    let name = terminal_name(target);
    let started = hc.run(&run_args(&name, cwd.as_deref(), &program)).map_err(|err| match err {
        HcError::Taken(_) => anyhow::anyhow!(
            "a review of {target} is already open: quit it, or attach to it with `hc attach {name}`"
        ),
        other => other.into(),
    })?;
    let started: Value = serde_json::from_str(&started).context("reading hc run's reply")?;
    let address = started.get("address").and_then(Value::as_str).context("hc run named no terminal")?;
    if !show_and_release(hc, address, placement, direction)? {
        println!("review running as {address}; attach with `hc attach {address}`");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use serde_json::json;

    use super::*;

    fn pi() -> Foreground {
        Foreground { pid: 42, cwd: Some(PathBuf::from("/work/app")), host: Some("pi") }
    }

    fn strs(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn an_agent_is_found_by_the_name_it_was_started_with_before_the_name_the_os_gives_it() {
        let facts =
            json!({"foreground":{"pid":42,"cwd":"/work/app","name":"2.1.312","argv":["/usr/bin/claude"]}});
        assert_eq!(foreground(&facts).and_then(|fg| fg.host), Some("claude"));
        let facts = json!({"foreground":{"pid":42,"name":"pi","argv":["node","/opt/pi/cli.js"]}});
        assert_eq!(foreground(&facts).and_then(|fg| fg.host), Some("pi"));
        let shell = json!({"foreground":{"pid":9,"name":"zsh","argv":["/usr/bin/zsh"]}});
        assert_eq!(foreground(&shell).map(|fg| fg.host), Some(None));
        assert_eq!(foreground(&json!({"foreground":null})), None, "a finished terminal has none");
    }

    #[test]
    fn the_review_terminal_is_told_who_to_send_to_and_which_hc_to_use() {
        let exe = Path::new("/bin/plannotator-tui");
        let review = Review::Last { newest: true, session: None };
        let args = argv(exe, Path::new("/opt/hc"), "t7", &pi(), &review).expect("an argv");
        assert_eq!(
            args,
            strs(&[
                "env",
                "PLANNOTATOR_TUI_HC_TERMINAL=t7",
                "PLANNOTATOR_TUI_DELIVER_AGENT=pi",
                "PLANNOTATOR_TUI_HC_PID=42",
                "PLANNOTATOR_TUI_HC_BIN=/opt/hc",
                "/bin/plannotator-tui",
                "hc",
                "pane",
                "--held",
                "last",
                "--pid",
                "42",
                "--host",
                "pi",
                "--newest",
            ])
        );
    }

    #[test]
    fn a_named_transcript_goes_to_the_reader_instead_of_the_newest_one_in_the_folder() {
        let exe = Path::new("/bin/plannotator-tui");
        let session = Some(PathBuf::from("/s/a.jsonl"));
        let args = argv(exe, Path::new("hc"), "t7", &pi(), &Review::Last { newest: false, session })
            .expect("an argv");
        assert!(args.ends_with(&strs(&["--host", "pi", "--session", "/s/a.jsonl"])));
    }

    #[test]
    fn a_terminal_without_an_agent_reviews_a_document_and_copies_instead_of_sending() {
        let shell = Foreground { host: None, ..pi() };
        let exe = Path::new("/bin/plannotator-tui");
        let open = Review::Open(None, None);
        let args = argv(exe, Path::new("/opt/hc"), "t7", &shell, &open).expect("an argv");
        assert_eq!(args, strs(&["env", "/bin/plannotator-tui", "hc", "pane", "--held", "open", "/work/app"]));
        let last =
            argv(exe, Path::new("/opt/hc"), "t7", &shell, &Review::Last { newest: false, session: None });
        assert!(last.is_err(), "there is no transcript to read");
    }

    #[test]
    fn a_gate_carries_its_result_file_into_the_pane() {
        let exe = Path::new("/bin/plannotator-tui");
        let gate = Gate { result_file: Some(PathBuf::from("/tmp/decision.json")) };
        let open = Review::Open(Some(PathBuf::from("/plans/a.md")), Some(gate));
        let args = argv(exe, Path::new("hc"), "t7", &pi(), &open).expect("an argv");
        assert!(args.ends_with(&strs(&["/plans/a.md", "--gate", "--result-file", "/tmp/decision.json"])));
    }

    #[test]
    fn the_review_terminal_neither_notifies_the_agent_nor_outlives_its_exit() {
        let args = run_args("annotate-t7", Some(Path::new("/work/app")), &strs(&["env", "x"]));
        assert_eq!(
            args,
            strs(&[
                "run",
                "--bg",
                "--json",
                "--rm",
                "--no-notify",
                "--name",
                "annotate-t7",
                "--cwd",
                "/work/app",
                "--",
                "env",
                "x"
            ])
        );
    }

    #[test]
    fn a_terminal_name_always_fits_what_hc_accepts() {
        assert_eq!(terminal_name("t1h"), "annotate-t1h");
        assert_eq!(terminal_name("My Agent"), "annotate-my-agent");
        let long = terminal_name(&"x".repeat(80));
        assert!(long.len() <= 32, "{long}");
    }
}
