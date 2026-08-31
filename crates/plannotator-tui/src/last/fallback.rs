//! PID metadata and cwd/mtime discovery after no exact path or id was supplied.

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

use anyhow::{Context, Result, bail};
use plannotator_tui_hosts::{Host, Message, claude, copilot, droid, opencode, pi};

use super::LastOptions;
use super::exact;
use super::readers;
use super::roots::Roots;

pub(super) fn read(
    host: Host,
    options: &LastOptions,
    cwd: &Path,
    roots: &Roots,
    pick: usize,
) -> Result<(PathBuf, Vec<Message>)> {
    match host {
        Host::ClaudeCode => {
            let path = find_claude_transcript(options.pid, cwd, roots)?;
            let messages = readers::claude_messages(&path, pick)?;
            Ok((path, messages))
        }
        Host::Codex => {
            let thread = std::env::var("CODEX_THREAD_ID").ok().filter(|thread| !thread.is_empty());
            readers::codex_thread(&roots.codex_home, thread.as_deref(), pick)
        }
        Host::Copilot => {
            let path = find_copilot_session(options.pid, cwd, roots)?;
            let messages = readers::copilot_messages(&path, pick)?;
            Ok((path, messages))
        }
        Host::Droid => {
            let path = find_droid_transcript(cwd, roots)?;
            let messages = readers::droid_messages(&path, pick)?;
            Ok((path, messages))
        }
        Host::Pi => {
            let path = find_pi_transcript(cwd, roots, ".pi/agent", "pi")?;
            let messages = readers::pi_messages(&path, pick)?;
            Ok((path, messages))
        }
        Host::Omp => bail!(
            "OMP session discovery without an exact path or id is unsupported; pass --session or --session-id"
        ),
        Host::Hermes => bail!("hermes needs a session id (Herdr provides it; or pass --session-id)"),
        Host::OpenCode => opencode_for_cwd(cwd, roots, pick),
    }
}

fn find_claude_transcript(pid: Option<u32>, cwd: &Path, roots: &Roots) -> Result<PathBuf> {
    let sessions_dir = roots.claude_config.join("sessions");
    let projects_dir = roots.claude_config.join("projects");
    let (start_pid, table) = process_context(pid);
    claude::find_transcript(&sessions_dir, &projects_dir, cwd, &table, start_pid).ok_or_else(|| {
        anyhow::anyhow!(
            "no Claude Code transcript for pid {start_pid} (looked in {} and {})",
            sessions_dir.display(),
            projects_dir.join(claude::project_slug(cwd)).display()
        )
    })
}

#[cfg(unix)]
fn find_copilot_session(pid: Option<u32>, cwd: &Path, roots: &Roots) -> Result<PathBuf> {
    let (start_pid, table) = process_context(pid);
    copilot::find_session(&roots.copilot_home, cwd, &table, start_pid, is_copilot_process).ok_or_else(|| {
        anyhow::anyhow!(
            "no Copilot CLI session for pid {start_pid} or {} (looked in {})",
            cwd.display(),
            roots.copilot_home.join("session-state").display()
        )
    })
}

#[cfg(not(unix))]
fn find_copilot_session(_pid: Option<u32>, _cwd: &Path, _roots: &Roots) -> Result<PathBuf> {
    bail!("Copilot session discovery without --session-id is unsupported on Windows; pass --session-id")
}

/// Does `pid` still name a Copilot process? Locks outlive sessions and pids get reused.
#[cfg(unix)]
fn is_copilot_process(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .is_some_and(|name| name.rsplit('/').next().unwrap_or(&name).starts_with("copilot"))
}

fn find_droid_transcript(cwd: &Path, roots: &Roots) -> Result<PathBuf> {
    droid::find_transcript(&roots.factory_config, cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no Droid session for {} (looked in {})",
            cwd.display(),
            roots.factory_config.join("sessions").join(claude::project_slug(cwd)).display()
        )
    })
}

fn find_pi_transcript(cwd: &Path, roots: &Roots, default_agent_dir: &str, label: &str) -> Result<PathBuf> {
    let sessions_dir = roots.pi_sessions(default_agent_dir);
    pi::find_transcript(&sessions_dir, cwd).ok_or_else(|| {
        anyhow::anyhow!("no {label} session for {} (looked in {})", cwd.display(), sessions_dir.display())
    })
}

#[cfg(unix)]
fn process_context(pid: Option<u32>) -> (u32, Vec<(u32, u32)>) {
    (pid.unwrap_or_else(std::os::unix::process::parent_id), process_table())
}

#[cfg(not(unix))]
fn process_context(pid: Option<u32>) -> (u32, Vec<(u32, u32)>) {
    (pid.unwrap_or(0), Vec::new())
}

/// One POSIX `ps` snapshot; an empty table when it cannot run.
#[cfg(unix)]
fn process_table() -> Vec<(u32, u32)> {
    Command::new("ps")
        .args(["-eo", "pid=,ppid="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| claude::parse_ps(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

fn opencode_for_cwd(cwd: &Path, roots: &Roots, pick: usize) -> Result<(PathBuf, Vec<Message>)> {
    let databases = roots.opencode_databases();
    let mut best: Option<(PathBuf, opencode::Found)> = None;
    for database in &databases {
        if let Ok(found) = opencode::find_session(database, cwd)
            && best.as_ref().is_none_or(|(_, current)| found.updated > current.updated)
        {
            best = Some((database.clone(), found));
        }
    }
    let (database, found) = best.with_context(|| {
        format!("no OpenCode session for {} in {}", cwd.display(), exact::describe(&databases))
    })?;
    let messages = opencode::messages_for_session(&database, &found.id, found.schema, pick)?;
    Ok((database, messages))
}
