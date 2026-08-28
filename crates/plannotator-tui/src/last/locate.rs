//! Find the transcript and read its recent assistant messages. The only module that looks
//! at the home directory, the environment, or the process table.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use plannotator_tui_hosts::{Host, HostError, Message, Role, claude, codex, copilot, detect_host, droid};

use super::LastOptions;

pub(crate) struct Located {
    pub(crate) host: Host,
    /// The transcript file (Claude) or the newest thread file (Codex); for the label.
    pub(crate) transcript: PathBuf,
    /// Assistant messages, newest first, at most `options.pick`.
    pub(crate) messages: Vec<Message>,
}

pub(crate) fn locate(options: &LastOptions) -> Result<Located> {
    let host = host_for(options)?;
    let pick = options.pick.max(1);
    let (transcript, messages) = match (host, &options.session) {
        (Host::ClaudeCode, Some(path)) => (path.clone(), claude_messages(path, pick)?),
        (Host::ClaudeCode, None) => {
            let path = find_claude_transcript(options.pid)?;
            let messages = claude_messages(&path, pick)?;
            (path, messages)
        }
        (Host::Codex, Some(path)) if path.is_dir() => codex_thread(path, None, pick)?,
        (Host::Codex, Some(path)) => (path.clone(), codex_messages(std::slice::from_ref(path), pick)?),
        (Host::Codex, None) => {
            let home = std::env::var_os("CODEX_HOME").map_or_else(|| home().join(".codex"), PathBuf::from);
            let thread = std::env::var("CODEX_THREAD_ID").ok().filter(|t| !t.is_empty());
            codex_thread(&home, thread.as_deref(), pick)?
        }
        (Host::Copilot, Some(dir)) => (dir.clone(), copilot_messages(dir, pick)?),
        (Host::Copilot, None) => {
            let dir = find_copilot_session(options.pid)?;
            let messages = copilot_messages(&dir, pick)?;
            (dir, messages)
        }
        (Host::Droid, Some(path)) => (path.clone(), droid_messages(path, pick)?),
        (Host::Droid, None) => {
            let path = find_droid_transcript()?;
            let messages = droid_messages(&path, pick)?;
            (path, messages)
        }
    };
    let messages: Vec<Message> = messages.into_iter().filter(|m| m.role == Role::Assistant).collect();
    if messages.is_empty() {
        bail!("transcript {} has no assistant messages yet", transcript.display());
    }
    Ok(Located { host, transcript, messages })
}

fn host_for(options: &LastOptions) -> Result<Host> {
    let override_host = options.host.clone();
    let lookup = |key: &str| match key {
        "PLANNOTATOR_TUI_HOST" => override_host.clone().or_else(|| std::env::var(key).ok()),
        _ => std::env::var(key).ok(),
    };
    match detect_host(lookup) {
        Ok(host) => Ok(host),
        Err(HostError::Unsupported(name)) => {
            let supported: Vec<&str> = Host::ALL.iter().map(|h| h.label()).collect();
            bail!("{name} is not supported yet; supported hosts: {} (or use --stdin)", supported.join(", "))
        }
        Err(err) => Err(err.into()),
    }
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// The Claude Code ladder over the real `~/.claude`, starting from `pid` or our parent.
fn find_claude_transcript(pid: Option<u32>) -> Result<PathBuf> {
    let home = home();
    let sessions_dir = home.join(".claude").join("sessions");
    let projects_dir = home.join(".claude").join("projects");
    let cwd = std::env::current_dir().context("current directory")?;
    let start_pid = pid.unwrap_or_else(parent_pid);
    let table = process_table();
    claude::find_transcript(&sessions_dir, &projects_dir, &cwd, &table, start_pid).ok_or_else(|| {
        anyhow::anyhow!(
            "no Claude Code transcript for pid {start_pid} (looked in {} and {})",
            sessions_dir.display(),
            projects_dir.join(claude::project_slug(&cwd)).display()
        )
    })
}

/// Copilot's session directory via its lock files, from `pid` or our parent; the cwd
/// heuristic when no ancestor holds a live lock.
fn find_copilot_session(pid: Option<u32>) -> Result<PathBuf> {
    let copilot_home =
        std::env::var_os("COPILOT_HOME").map_or_else(|| home().join(".copilot"), PathBuf::from);
    let cwd = std::env::current_dir().context("current directory")?;
    let start_pid = pid.unwrap_or_else(parent_pid);
    let table = process_table();
    copilot::find_session(&copilot_home, &cwd, &table, start_pid, is_copilot_process).ok_or_else(|| {
        anyhow::anyhow!(
            "no Copilot CLI session for pid {start_pid} or {} (looked in {})",
            cwd.display(),
            copilot_home.join("session-state").display()
        )
    })
}

/// Does `pid` still name a Copilot process? Locks outlive sessions and pids get reused.
fn is_copilot_process(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .is_some_and(|name| name.rsplit('/').next().unwrap_or(&name).starts_with("copilot"))
}

fn copilot_messages(dir: &Path, pick: usize) -> Result<Vec<Message>> {
    let events = dir.join("events.jsonl");
    let text = std::fs::read_to_string(&events).with_context(|| format!("reading {}", events.display()))?;
    Ok(copilot::parse_messages(&text, pick))
}

/// Droid's current log for the cwd (`PLANNOTATOR_TUI_CWD` when the launcher set it).
fn find_droid_transcript() -> Result<PathBuf> {
    let factory_dir =
        std::env::var_os("FACTORY_CONFIG_DIR").map_or_else(|| home().join(".factory"), PathBuf::from);
    let cwd = match std::env::var_os("PLANNOTATOR_TUI_CWD") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().context("current directory")?,
    };
    droid::find_transcript(&factory_dir, &cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no Droid session for {} (looked in {})",
            cwd.display(),
            factory_dir.join("sessions").join(claude::project_slug(&cwd)).display()
        )
    })
}

fn droid_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(droid::parse_messages(&text, pick))
}

#[cfg(unix)]
fn parent_pid() -> u32 {
    std::os::unix::process::parent_id()
}

#[cfg(not(unix))]
fn parent_pid() -> u32 {
    0
}

/// One `ps` snapshot; an empty table when it cannot run (the ladder then relies on cwd).
fn process_table() -> Vec<(u32, u32)> {
    Command::new("ps")
        .args(["-eo", "pid=,ppid="])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| claude::parse_ps(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

fn claude_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(claude::parse_messages(&text, pick))
}

fn codex_thread(codex_home: &Path, thread: Option<&str>, pick: usize) -> Result<(PathBuf, Vec<Message>)> {
    let files = codex::find_transcripts(codex_home, thread);
    let Some(newest) = files.last().cloned() else {
        bail!(
            "no Codex transcript under {} (thread {})",
            codex_home.join("sessions").display(),
            thread.unwrap_or("newest")
        );
    };
    let messages = codex_messages(&files, pick)?;
    Ok((newest, messages))
}

fn codex_messages(files: &[PathBuf], pick: usize) -> Result<Vec<Message>> {
    let contents = files
        .iter()
        .map(|p| std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display())))
        .collect::<Result<Vec<String>>>()?;
    Ok(codex::parse_messages(&contents, pick))
}
