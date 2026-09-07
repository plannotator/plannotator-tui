//! PID metadata and cwd/mtime discovery after no exact path or id was supplied.

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

use anyhow::{Context, Result, bail};
#[cfg(unix)]
use plannotator_tui_hosts::copilot;
use plannotator_tui_hosts::{Host, Message, claude, droid, opencode, pi};

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
            #[cfg(target_os = "linux")]
            if let Some(pid) = options.pid {
                let path = find_codex_transcript(pid)?;
                return readers::explicit(host, &path, None, pick);
            }
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

/// A selected Linux process is a stronger hint than an inherited thread id or the newest
/// rollout. Missing or ambiguous descriptors must not silently select another session.
/// Codex keeps its subagents' rollouts (reviews, guardians) open too; those are ignored.
#[cfg(target_os = "linux")]
fn find_codex_transcript(pid: u32) -> Result<PathBuf> {
    let directory = PathBuf::from(format!("/proc/{pid}/fd"));
    let entries = std::fs::read_dir(&directory)
        .with_context(|| format!("reading Codex process {pid} descriptors in {}", directory.display()))?;
    let mut transcripts = std::collections::BTreeSet::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", directory.display()))?;
        // Descriptors can close between readdir and readlink.
        let Ok(path) = std::fs::read_link(entry.path()) else { continue };
        if path.extension().is_some_and(|extension| extension == "jsonl")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-"))
            && path.is_file()
            && !plannotator_tui_hosts::codex::is_subagent(&path)
        {
            transcripts.insert(path);
        }
    }
    if transcripts.len() != 1 {
        bail!(
            "Codex process {pid} has {} open transcripts; pass --session or --session-id to select one",
            transcripts.len()
        );
    }
    transcripts.into_iter().next().with_context(|| format!("no transcript for Codex process {pid}"))
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
