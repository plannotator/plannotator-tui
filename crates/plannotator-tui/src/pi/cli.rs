//! `plannotator-tui pi list|send|install`.

#![allow(clippy::print_stdout, reason = "list and send report on stdout")]

use std::io::Read as _;

use anyhow::{Context, Result};

use super::{Disposition, PiEntry, SendError};

const PI_USAGE: &str = "usage: plannotator-tui pi list [--json]
       plannotator-tui pi send --session <id or name> < feedback.md
       plannotator-tui pi install [--dry-run | --print]";

pub(crate) fn run(args: &[String]) -> Result<()> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("list") => list(rest),
        Some("send") => send(rest),
        Some("install") => super::install::run(rest),
        _ => anyhow::bail!(PI_USAGE),
    }
}

fn list(args: &[String]) -> Result<()> {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => anyhow::bail!(PI_USAGE),
    };
    let sessions = super::live_pi_sessions(&super::root());
    if json {
        let rows: Vec<_> = sessions
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id, "pid": s.pid, "name": s.name, "cwd": s.cwd,
                    "transcript": s.transcript, "busy": s.busy,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("No Pi sessions found. Run `plannotator-tui pi install`, then restart Pi.");
    }
    for s in &sessions {
        let state = if s.busy { "busy" } else { "idle" };
        println!("{}  {}  {state}  {}  pid {}", s.id, s.label(), s.cwd.display(), s.pid);
    }
    Ok(())
}

fn send(args: &[String]) -> Result<()> {
    let session = match args {
        [flag, session] if flag == "--session" => session,
        _ => anyhow::bail!(PI_USAGE),
    };
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).context("reading the feedback from stdin")?;
    anyhow::ensure!(!text.trim().is_empty(), "nothing to send: stdin was empty");
    let entry = pick(&super::live_pi_sessions(&super::root()), session)?;
    match super::send(&entry, &text) {
        Ok(Disposition::Started) => println!("sent to pi · {}", entry.label()),
        Ok(Disposition::Queued) => {
            println!("queued for pi · {}: it is busy and reads this next", entry.label());
        }
        Err(SendError::Gone) => anyhow::bail!("pi · {} has closed", entry.label()),
        Err(SendError::Failed(message)) => anyhow::bail!("pi · {}: {message}", entry.label()),
    }
    Ok(())
}

/// The one live session whose id, id prefix or name is `wanted`.
fn pick(sessions: &[PiEntry], wanted: &str) -> Result<PiEntry> {
    let exact: Vec<&PiEntry> = sessions.iter().filter(|s| s.id == wanted).collect();
    let matches: Vec<&PiEntry> = if exact.is_empty() {
        sessions.iter().filter(|s| s.id.starts_with(wanted) || s.name.as_deref() == Some(wanted)).collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => anyhow::bail!("no live Pi session matches {wanted:?}; see `plannotator-tui pi list`"),
        many => anyhow::bail!(
            "{} live Pi sessions match {wanted:?}; use the full id:\n{}",
            many.len(),
            many.iter().map(|s| format!("  {}  {}", s.id, s.label())).collect::<Vec<_>>().join("\n")
        ),
    }
}
