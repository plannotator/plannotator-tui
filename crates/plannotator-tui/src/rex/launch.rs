//! `plannotator-tui rex last|open|terminal|find`: find the agent in a Rex block and open
//! plannotator-tui beside it (a split) or over it (a popup), wired to send feedback back to
//! the block. Either is an ordinary pane, so it keeps Rex's frame and theme. `foreground`,
//! `reported_host` and `argv` are pure; only `run` touches Rex.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use super::agent::{AGENT_VAR, BLOCK_VAR, PID_VAR, SUBMIT_VAR};
use super::command::Rex;
use crate::config::RexPlacement;
use crate::herdr::launch::known_host;

/// What the opened pane reviews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Review {
    /// The agent's recent replies; `newest` skips the picker.
    Last { newest: bool },
    /// A file or folder; the block's directory when `None`. With a [`Gate`], the review can
    /// be approved, as `herdr open --gate` does.
    Open(Option<PathBuf>, Option<Gate>),
    /// The block's last `lines` lines of output.
    Terminal { lines: u32 },
    /// The newest Markdown file named in the block's last `lines` lines of output. `run`
    /// turns it into an `Open`.
    Find { lines: u32 },
}

/// `rex open --gate`: Approve beside Send, and where the decision is published.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Gate {
    pub(crate) result_file: Option<PathBuf>,
}

/// How feedback reaches the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handoff {
    /// As its next message.
    Send,
    /// Into its prompt, for you to edit and submit.
    Paste,
}

/// The program in the foreground of a block, from `com.superlogical.terminal.process` JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Foreground {
    pub(crate) pid: u32,
    pub(crate) cwd: Option<PathBuf>,
    /// The agent's host label when the program is one the hosts crate can read.
    pub(crate) host: Option<&'static str>,
}

/// Read the foreground process. Rex reports the binary's file name, which for Claude Code is
/// its version number, so the name it was invoked by is checked first. An agent run by an
/// interpreter, such as pi under `node`, is named only by its `script`, if at all; `run` asks
/// the agent's own status report first.
pub(crate) fn foreground(process: &Value) -> Option<Foreground> {
    let fg = process.get("foreground")?;
    let pid = u32::try_from(fg.get("pid")?.as_u64()?).ok()?;
    let host = ["argv0", "invoked_path", "name", "path", "script"]
        .iter()
        .filter_map(|key| fg.get(*key).and_then(Value::as_str))
        .find_map(known_host);
    let cwd = fg.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    Some(Foreground { pid, cwd, host })
}

/// The agent process `pid` says it is, from `program_status` JSON: the app of a record that
/// process owns. Rex keeps records of programs that have exited, so only the owner counts.
pub(crate) fn reported_host(status: &Value, pid: u32) -> Option<&'static str> {
    let records = status.get("records")?.as_array()?;
    records
        .iter()
        .filter(|r| r.pointer("/owner/pid").and_then(Value::as_u64) == Some(u64::from(pid)))
        .flat_map(|r| ["effective_app", "app"].map(|key| r.get(key).and_then(Value::as_str)))
        .flatten()
        .find_map(|app| if app == "claude-code" { Some("claude") } else { known_host(app) })
}

/// The argv the pane runs: `env` sets the delivery target, then `plannotator-tui rex pane`
/// opens the review. Without an agent in the block, feedback goes to the clipboard.
pub(crate) fn argv(
    exe: &Path,
    block: &str,
    fg: &Foreground,
    review: &Review,
    handoff: Handoff,
) -> Result<Vec<String>> {
    let mut args = vec!["/usr/bin/env".to_owned()];
    if let Some(host) = fg.host {
        args.push(format!("{BLOCK_VAR}={block}"));
        args.push(format!("{AGENT_VAR}={host}"));
        args.push(format!("{PID_VAR}={}", fg.pid));
        if handoff == Handoff::Paste {
            args.push(format!("{SUBMIT_VAR}=0"));
        }
    }
    args.extend([exe.display().to_string(), "rex".to_owned(), "pane".to_owned()]);
    match review {
        Review::Last { newest } => {
            let host = fg.host.context("no supported agent runs in this block")?;
            args.extend(["last".to_owned(), "--pid".to_owned(), fg.pid.to_string()]);
            args.extend(["--host".to_owned(), host.to_owned()]);
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
        Review::Terminal { lines } => {
            args.extend(["terminal", "--block", block, "--lines"].map(str::to_owned));
            args.push(lines.to_string());
        }
        Review::Find { .. } => anyhow::bail!("a find review is resolved to an open before the split"),
    }
    Ok(args)
}

/// The Markdown file a `Find` names: the newest one in the block's last `lines` lines.
fn newest_markdown(rex: &Rex, block: &str, fg: &Foreground, lines: u32) -> Result<PathBuf> {
    let text = super::terminal::read(rex, block, lines)?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    super::paths::markdown_files(&text, fg.cwd.as_deref(), home.as_deref())
        .into_iter()
        .next()
        .with_context(|| format!("no Markdown file named in the last {lines} lines of block {block}"))
}

/// Open the review beside or over `block`. The agent is the one its status report names, else
/// the one its process names; a status Rex cannot give leaves the process to decide.
pub(crate) fn run(
    rex: &Rex,
    block: &str,
    review: &Review,
    handoff: Handoff,
    placement: RexPlacement,
) -> Result<()> {
    let process = rex.terminal_method(block, "process")?;
    let mut fg = foreground(&process).with_context(|| format!("block {block} has no foreground process"))?;
    if let Ok(status) = rex.terminal_method(block, "program_status") {
        fg.host = reported_host(&status, fg.pid).or(fg.host);
    }
    let exe = std::env::current_exe().context("finding the plannotator-tui binary")?;
    let review = match review {
        Review::Find { lines } => Review::Open(Some(newest_markdown(rex, block, &fg, *lines)?), None),
        other => other.clone(),
    };
    let args = argv(&exe, block, &fg, &review, handoff)?;
    super::place::open(rex, block, placement, fg.cwd.as_deref(), &args)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    /// `com.superlogical.terminal.process` for a block running Claude Code, as Rex reports it.
    const CLAUDE: &str = r#"{"last_exit":null,"child":{"pid":59515,"name":"login"},"foreground":{"pid":60223,"name":"2.1.308","path":"/Users/me/.local/share/claude/versions/2.1.308","invoked_path":"/Users/me/.local/bin/claude","argv0":"claude","cwd":"/Users/me/project"}}"#;
    const SHELL: &str = r#"{"foreground":{"pid":700,"name":"fish","argv0":"-fish","cwd":"/Users/me"}}"#;
    /// pi as installed by npm, run by `node` (Rex 1913): nothing but the status names it.
    const PI_NODE: &str = r#"{"last_exit":null,"foreground":{"pid":73139,"user_id":501,"user":"me","name":"node","path":"/Users/me/.local/share/fnm/node-versions/v24.15.0/installation/bin/node","invoked_path":"/Users/me/.local/share/fnm/node-versions/v24.15.0/installation/bin/node","argv0":"node","script":"/Users/me/.local/share/fnm/node-versions/v24.15.0/installation/lib/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js","cwd":"/Users/me/project"},"home":"/Users/me"}"#;
    const PI_STATUS: &str = r#"{"records":[{"state":"idle","app":"pi","effective_app":"pi","updated_at":"2026-11-10T00:25:21.924601-05:00","owner":{"pid":73139,"name":"node"}}]}"#;

    fn parse(json: &str) -> Foreground {
        foreground(&serde_json::from_str(json).expect("json")).expect("a foreground process")
    }

    #[test]
    fn claude_code_is_found_by_its_invoked_name_not_its_versioned_binary() {
        let fg = parse(CLAUDE);
        assert_eq!(fg.pid, 60223);
        assert_eq!(fg.host, Some("claude"));
        assert_eq!(fg.cwd.as_deref(), Some(Path::new("/Users/me/project")));
        assert_eq!(parse(SHELL).host, None);
    }

    fn status(json: &str) -> Value {
        serde_json::from_str(json).expect("json")
    }

    #[test]
    fn an_agent_under_an_interpreter_is_named_by_its_own_status_report() {
        let fg = parse(PI_NODE);
        assert_eq!(fg.host, None, "node and cli.js name no agent");
        assert_eq!(reported_host(&status(PI_STATUS), fg.pid), Some("pi"));
        let claude = r#"{"records":[{"state":"working","app":"claude-code","owner":{"pid":60223}}]}"#;
        assert_eq!(reported_host(&status(claude), 60223), Some("claude"));
    }

    #[test]
    fn a_status_left_by_another_process_names_no_agent() {
        assert_eq!(reported_host(&status(PI_STATUS), 700), None);
        let unknown = r#"{"records":[{"state":"working","app":"terraform","owner":{"pid":700}}]}"#;
        assert_eq!(reported_host(&status(unknown), 700), None);
        assert_eq!(reported_host(&status(r#"{"records":[]}"#), 700), None);
    }

    #[test]
    fn a_script_names_the_agent_when_its_file_is_called_after_it() {
        let script = r#"{"foreground":{"pid":9,"name":"node","argv0":"node","script":"/opt/bin/pi"}}"#;
        assert_eq!(parse(script).host, Some("pi"));
    }

    #[test]
    fn last_reads_the_agents_pid_and_sends_back_to_its_block() {
        let args = argv(
            Path::new("/bin/pt"),
            "block:b",
            &parse(CLAUDE),
            &Review::Last { newest: true },
            Handoff::Send,
        )
        .expect("argv");
        assert_eq!(
            args,
            [
                "/usr/bin/env",
                "PLANNOTATOR_TUI_REX_BLOCK=block:b",
                "PLANNOTATOR_TUI_DELIVER_AGENT=claude",
                "PLANNOTATOR_TUI_REX_PID=60223",
                "/bin/pt",
                "rex",
                "pane",
                "last",
                "--pid",
                "60223",
                "--host",
                "claude",
                "--newest"
            ]
        );
        assert!(
            argv(
                Path::new("/bin/pt"),
                "block:b",
                &parse(SHELL),
                &Review::Last { newest: false },
                Handoff::Send
            )
            .is_err()
        );
    }

    #[test]
    fn open_without_an_agent_reviews_the_blocks_folder_and_copies_to_the_clipboard() {
        let args =
            argv(Path::new("/bin/pt"), "block:b", &parse(SHELL), &Review::Open(None, None), Handoff::Send)
                .expect("argv");
        assert_eq!(args, ["/usr/bin/env", "/bin/pt", "rex", "pane", "open", "/Users/me"]);
    }

    #[test]
    fn pasting_is_passed_on_only_where_there_is_an_agent_to_paste_into() {
        let paste = |json| {
            argv(Path::new("/bin/pt"), "block:b", &parse(json), &Review::Open(None, None), Handoff::Paste)
                .expect("argv")
        };
        assert!(paste(CLAUDE).contains(&"PLANNOTATOR_TUI_REX_SUBMIT=0".to_owned()));
        assert!(!paste(SHELL).iter().any(|arg| arg.starts_with("PLANNOTATOR_TUI_REX")));
    }

    #[test]
    fn a_gated_open_hands_the_gate_and_its_result_file_to_the_pane() {
        let gated = |gate| {
            let review = Review::Open(Some(PathBuf::from("/p/plan.md")), Some(gate));
            argv(Path::new("/bin/pt"), "block:b", &parse(CLAUDE), &review, Handoff::Send).expect("argv")
        };
        let tail = |args: &[String]| args.iter().skip_while(|a| *a != "open").cloned().collect::<Vec<_>>();
        assert_eq!(tail(&gated(Gate::default())), ["open", "/p/plan.md", "--gate"]);
        let published = gated(Gate { result_file: Some(PathBuf::from("/p/decision.json")) });
        assert_eq!(tail(&published), ["open", "/p/plan.md", "--gate", "--result-file", "/p/decision.json"]);
        assert!(published.contains(&"PLANNOTATOR_TUI_REX_BLOCK=block:b".to_owned()), "the agent hears it");
    }

    #[test]
    fn a_terminal_review_reads_the_block_from_inside_the_split() {
        let args = argv(
            Path::new("/bin/pt"),
            "block:b",
            &parse(SHELL),
            &Review::Terminal { lines: 80 },
            Handoff::Send,
        )
        .expect("argv");
        assert_eq!(
            args,
            ["/usr/bin/env", "/bin/pt", "rex", "pane", "terminal", "--block", "block:b", "--lines", "80"]
        );
    }
}
