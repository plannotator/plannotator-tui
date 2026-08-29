//! Find the transcript and read its recent assistant messages. The only module that looks
//! at the home directory, the environment, or the process table.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use plannotator_tui_hosts::{
    Host, HostError, Message, Role, claude, codex, copilot, detect_host, droid, hermes, omp, opencode, pi,
    sniff,
};
use plannotator_tui_schema::{DocumentSource, Provenance};

use super::LastOptions;

pub(crate) struct Located {
    pub(crate) host: Host,
    /// The transcript file (Claude) or the newest thread file (Codex); for the label.
    pub(crate) transcript: PathBuf,
    /// Assistant messages, newest first, at most `options.pick`.
    pub(crate) messages: Vec<Message>,
}

pub(crate) fn locate(options: &LastOptions) -> Result<Located> {
    let session = options.session.as_deref().map(expand_home);
    let host = match &session {
        // A path without a host name: Herdr hands us the exact transcript for agents it
        // integrates, whatever their format. Its first lines say which reader applies.
        Some(path) if !host_named(options) => sniff(&head(path)?).map_or_else(|| host_for(options), Ok)?,
        _ => host_for(options)?,
    };
    let pick = options.pick.max(1);
    let (transcript, messages) = match (host, &session) {
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
        (Host::Pi, Some(path)) => (path.clone(), pi_messages(path, pick)?),
        (Host::Pi, None) => {
            let path = find_pi_transcript(".pi/agent", "pi")?;
            let messages = pi_messages(&path, pick)?;
            (path, messages)
        }
        (Host::Omp, Some(path)) => (path.clone(), omp_messages(path, pick)?),
        (Host::Omp, None) => {
            let path = find_pi_transcript(omp::DEFAULT_AGENT_DIR, "omp")?;
            let messages = omp_messages(&path, pick)?;
            (path, messages)
        }
        (Host::Hermes, _) => {
            let Some(id) = options.session_id.as_deref().filter(|s| !s.trim().is_empty()) else {
                bail!("hermes needs a session id (Herdr provides it; or pass --session-id)");
            };
            let db =
                std::env::var_os("HERMES_HOME").map_or_else(hermes_home, PathBuf::from).join(hermes::DB_FILE);
            let messages = hermes::messages_for_session(&db, id, pick)?;
            (db, messages)
        }
        (Host::OpenCode, _) => {
            let db = opencode_db();
            let id = match options.session_id.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(id) => id.to_owned(),
                None => opencode::find_session(&db, &agent_cwd()?)?,
            };
            let messages = opencode::messages_for_session(&db, &id, pick)?;
            (db, messages)
        }
    };
    let messages: Vec<Message> = messages.into_iter().filter(|m| m.role == Role::Assistant).collect();
    if messages.is_empty() {
        bail!("transcript {} has no assistant messages yet", transcript.display());
    }
    Ok(Located { host, transcript, messages })
}

/// Was a host named explicitly, by flag or by the launcher?
fn host_named(options: &LastOptions) -> bool {
    options.host.as_deref().is_some_and(|h| !h.trim().is_empty())
        || std::env::var("PLANNOTATOR_TUI_HOST").is_ok_and(|h| !h.trim().is_empty())
}

/// The first 64 KiB of a transcript, enough for `sniff`.
fn head(path: &Path) -> Result<String> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut bytes = vec![0u8; 64 * 1024];
    let n = file.read(&mut bytes).with_context(|| format!("reading {}", path.display()))?;
    bytes.truncate(n);
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// `~/x` as Herdr reports some session paths.
fn expand_home(path: &Path) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix("~/")) {
        Some(rest) => home().join(rest),
        None => path.to_path_buf(),
    }
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

/// The agent pane's recent output as a document, when Herdr and a target pane are known.
/// Lossy (no markdown structure survives a terminal), so only a fallback.
pub(crate) fn screen_fallback(env: &crate::herdr::context::HerdrEnv) -> Option<DocumentSource> {
    let target = env.delivery_target()?;
    if !env.in_herdr {
        return None;
    }
    let output = Command::new(&env.bin)
        .args(["agent", "read", &target.pane, "--source", "recent-unwrapped", "--format", "text"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout).trim_end().to_owned();
    if text.is_empty() {
        return None;
    }
    let host = env.host.clone().or(target.agent).unwrap_or_else(|| "agent".to_owned());
    Some(DocumentSource::new(
        text,
        format!("{host} · screen"),
        true,
        Provenance::AgentMessage { host, session: None, message_id: None },
    ))
}

/// Hermes' platform default (`hermes_constants.py`): `%LOCALAPPDATA%\hermes` on Windows,
/// `~/.hermes` elsewhere.
#[cfg(windows)]
fn hermes_home() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home().join("AppData").join("Local"), PathBuf::from)
        .join("hermes")
}

#[cfg(not(windows))]
fn hermes_home() -> PathBuf {
    home().join(".hermes")
}

/// `OpenCode`'s database: `$OPENCODE_DB`, else `<xdg data>/opencode/opencode.db` where the
/// xdg data dir is `$XDG_DATA_HOME` or `~/.local/share` (opencode `packages/core/src/global.ts`
/// uses `xdg-basedir`, which applies the same rule on every platform).
fn opencode_db() -> PathBuf {
    if let Some(db) = std::env::var_os("OPENCODE_DB").filter(|v| !v.is_empty()) {
        return PathBuf::from(db);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home().join(".local").join("share"), PathBuf::from);
    data.join(opencode::DATA_DIR).join(opencode::DB_FILE)
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

/// The agent pane's cwd when the Herdr launcher set it, else our own.
fn agent_cwd() -> Result<PathBuf> {
    match std::env::var_os("PLANNOTATOR_TUI_CWD") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => std::env::current_dir().context("current directory"),
    }
}

/// Droid's current log for the cwd: no pid registry, so the newest log for the cwd's slug.
fn find_droid_transcript() -> Result<PathBuf> {
    let factory_dir =
        std::env::var_os("FACTORY_CONFIG_DIR").map_or_else(|| home().join(".factory"), PathBuf::from);
    let cwd = agent_cwd()?;
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

/// Pi and OMP have no pid registry: the newest session for the agent's cwd, under the
/// agent dir (`default_agent_dir` relative to `$HOME`; OMP reuses pi's override variables).
fn find_pi_transcript(default_agent_dir: &str, label: &str) -> Result<PathBuf> {
    let sessions_dir = match std::env::var_os("PI_CODING_AGENT_SESSION_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::var_os("PI_CODING_AGENT_DIR")
            .map_or_else(|| home().join(default_agent_dir), PathBuf::from)
            .join("sessions"),
    };
    let cwd = agent_cwd()?;
    pi::find_transcript(&sessions_dir, &cwd).ok_or_else(|| {
        anyhow::anyhow!("no {label} session for {} (looked in {})", cwd.display(), sessions_dir.display())
    })
}

fn omp_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(omp::parse_messages(&text, pick))
}

fn pi_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(pi::parse_messages(&text, pick))
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
