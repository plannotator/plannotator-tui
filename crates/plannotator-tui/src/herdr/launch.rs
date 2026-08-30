//! `plannotator-tui herdr open`: resolve what to open, where feedback goes, and how the pane is
//! placed, then run `herdr plugin pane open`. One command for humans (manifest actions)
//! and agents (the skill). `plan` and `argv` are pure; only `run` touches a process.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use plannotator_tui_hosts::Host;

use super::context::{HerdrEnv, Target};
use crate::config::{Config, Placement, SplitDirection};

/// Command-line inputs to the launcher.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OpenArgs {
    pub(crate) path: Option<PathBuf>,
    pub(crate) placement: Option<Placement>,
    pub(crate) deliver_to: Option<String>,
}

/// A fully resolved launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Launch {
    /// File or folder plannotator-tui opens.
    pub(crate) file: PathBuf,
    /// The pane's working directory: the file's folder, or the folder itself.
    pub(crate) cwd: PathBuf,
    pub(crate) placement: Placement,
    pub(crate) direction: SplitDirection,
    /// Popup `(width, height)`.
    pub(crate) popup: (String, String),
    /// The pane a split opens next to.
    pub(crate) target_pane: Option<String>,
    pub(crate) deliver: Option<Target>,
    /// The plugin id to open under: whatever plugin ships this binary.
    pub(crate) plugin: String,
    /// Open an agent's last message instead of `file`: (pid, host label).
    pub(crate) message: Option<(u32, String)>,
    /// The agent's session as Herdr reports it: a transcript path or a host-specific id.
    pub(crate) session: Option<AgentSession>,
}

/// `agent_session` from `herdr agent get`: the exact transcript when Herdr knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSession {
    Path(String),
    Id(String),
}

/// Parse `herdr agent get <pane>` JSON for its `agent_session`, when present.
pub(crate) fn agent_session(agent_get_json: &str) -> Option<AgentSession> {
    let json: serde_json::Value = serde_json::from_str(agent_get_json).ok()?;
    let session = json.pointer("/result/agent/agent_session")?;
    let value = session.get("value")?.as_str()?.to_owned();
    match session.get("kind")?.as_str()? {
        "path" => Some(AgentSession::Path(value)),
        "id" => Some(AgentSession::Id(value)),
        _ => None,
    }
}

/// Parse `herdr agent get <pane>` JSON for a host with a transcript reader.
fn agent_host(agent_get_json: &str) -> Option<&'static str> {
    let json: serde_json::Value = serde_json::from_str(agent_get_json).ok()?;
    let label = json.pointer("/result/agent/agent")?.as_str()?;
    Host::ALL.into_iter().find(|host| host.label() == label).map(Host::label)
}

/// The agent process behind a pane, from `herdr pane process-info --pane <id>` JSON: the
/// foreground process whose name is a known agent, else the group leader. Returns
/// `(pid, host)` where host is the label `plannotator-tui last --host` accepts.
pub(crate) fn agent_pid(process_info_json: &str) -> Option<(u32, String)> {
    let json: serde_json::Value = serde_json::from_str(process_info_json).ok()?;
    let info = json.pointer("/result/process_info")?;
    let processes = info.get("foreground_processes")?.as_array()?;
    let name_of = |p: &serde_json::Value| p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_owned();
    let pid_of = |p: &serde_json::Value| p.get("pid").and_then(serde_json::Value::as_u64).map(|n| n as u32);
    for process in processes {
        let name = name_of(process);
        if let Some(host) = known_host(&name) {
            return pid_of(process).map(|pid| (pid, host.to_owned()));
        }
    }
    // No process we can read: report the group leader under its own name, so the pane can
    // say "<name> is not supported" and fall back to the screen.
    let leader = info.get("foreground_process_group_id").and_then(serde_json::Value::as_u64)?;
    let process = processes.iter().find(|p| pid_of(p) == Some(leader as u32))?;
    let name = name_of(process);
    let host = name.rsplit('/').next().unwrap_or(&name).to_owned();
    pid_of(process).map(|pid| (pid, if host.is_empty() { "claude".to_owned() } else { host }))
}

/// Agents whose transcripts the hosts crate can read, by process name.
fn known_host(name: &str) -> Option<&'static str> {
    match name.rsplit('/').next().unwrap_or(name) {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "pi" => Some("pi"),
        "opencode" => Some("opencode"),
        _ => None,
    }
}

/// Resolve a `last` launch: the target pane as for `open`, the folder from the context, and
/// the agent process from the pane's process info (fetched by the caller).
pub(crate) fn plan_last(
    env: &HerdrEnv,
    config: &Config,
    args: OpenArgs,
    cwd: &Path,
    process_info_json: &str,
    agent_get_json: Option<&str>,
) -> Result<Launch> {
    let mut launch = plan(env, config, OpenArgs { path: None, ..args }, cwd)?;
    let pane = launch.deliver.as_ref().map(|t| t.pane.clone()).or_else(|| launch.target_pane.clone());
    let Some(pane) = pane else {
        anyhow::bail!("no agent pane to read: not focused on one and no --deliver-to")
    };
    let Some((pid, process_host)) = agent_pid(process_info_json) else {
        anyhow::bail!("no agent process found in pane {pane}");
    };
    let host = agent_get_json.and_then(agent_host).unwrap_or(&process_host).to_owned();
    launch.file.clone_from(&launch.cwd);
    launch.message = Some((pid, host));
    launch.session = agent_get_json.and_then(agent_session);
    Ok(launch)
}

/// A `file://` URL as a local path; anything else is not ours to open.
fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // `file:///abs` → `/abs`; `file://host/abs` → `/abs` for localhost only.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) if matches!(&rest[..slash], "" | "localhost") => &rest[slash..],
        _ => return None,
    };
    let decoded = percent_decode(path);
    // Windows: `file:///C:/x` carries a leading slash before the drive letter.
    #[cfg(windows)]
    let decoded = match decoded.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => {
            decoded.get(1..).unwrap_or_default().to_owned()
        }
        _ => decoded,
    };
    Some(PathBuf::from(decoded))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes.get(i) == Some(&b'%'))
            .then(|| s.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = decoded {
            out.push(byte);
            i += 3;
        } else {
            out.extend(bytes.get(i..=i).unwrap_or_default());
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve every input per the contract. `cwd` is the launcher process's directory.
pub(crate) fn plan(env: &HerdrEnv, config: &Config, args: OpenArgs, cwd: &Path) -> Result<Launch> {
    let context = env.context.as_ref();
    let placement = match (args.placement, env.placement.as_deref()) {
        (Some(p), _) => p,
        (None, Some(text)) => text.parse().context("PLANNOTATOR_TUI_PLACEMENT")?,
        (None, None) => config.herdr.placement,
    };

    let file = args
        .path
        .or_else(|| context.and_then(|c| c.clicked_url.as_deref()).and_then(file_url_path))
        .or_else(|| context.and_then(|c| c.focused_pane_cwd.clone()).map(PathBuf::from))
        .or_else(|| context.and_then(|c| c.workspace_cwd.clone()).map(PathBuf::from))
        .unwrap_or_else(|| cwd.to_path_buf());
    let file = if file.is_absolute() { file } else { cwd.join(file) };
    let dir = if file.is_dir() {
        file.clone()
    } else {
        file.parent().map_or_else(|| cwd.to_path_buf(), Path::to_path_buf)
    };

    let deliver = match args.deliver_to {
        Some(pane) => Some(Target { pane, agent: None }),
        // With a context (a manifest action) the focused pane counts only when an agent
        // runs there. Without one, the caller is an agent running the skill from its own
        // pane, and HERDR_PANE_ID is that pane.
        None if env.context.is_some() => env.focused_agent_pane(),
        None => env.pane_id.clone().map(|pane| Target { pane, agent: None }),
    };
    let target_pane = deliver
        .as_ref()
        .map(|t| t.pane.clone())
        .or_else(|| context.and_then(|c| c.focused_pane_id.clone()))
        .or_else(|| env.pane_id.clone());

    Ok(Launch {
        file,
        cwd: dir,
        placement,
        direction: config.herdr.split_direction,
        popup: (config.herdr.popup_width.clone(), config.herdr.popup_height.clone()),
        target_pane,
        deliver,
        plugin: env.plugin_id.clone().unwrap_or_else(|| "plannotator-tui".to_owned()),
        message: None,
        session: None,
    })
}

/// The `herdr` arguments for a launch.
pub(crate) fn argv(launch: &Launch) -> Vec<String> {
    let mut out: Vec<String> = ["plugin", "pane", "open", "--plugin", &launch.plugin, "--entrypoint", "doc"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    out.extend(["--placement".to_owned(), launch.placement.to_string()]);
    match launch.placement {
        Placement::Split => {
            out.extend(["--direction".to_owned(), launch.direction.to_string()]);
            if let Some(pane) = &launch.target_pane {
                out.extend(["--target-pane".to_owned(), pane.clone()]);
            }
        }
        Placement::Popup => {
            out.extend(["--width".to_owned(), launch.popup.0.clone()]);
            out.extend(["--height".to_owned(), launch.popup.1.clone()]);
        }
        Placement::Overlay => {}
    }
    out.push("--focus".to_owned());
    out.extend(["--cwd".to_owned(), launch.cwd.display().to_string()]);
    match &launch.message {
        Some((pid, host)) => {
            out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_MESSAGE_PID={pid}")]);
            out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_HOST={host}")]);
            out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_CWD={}", launch.cwd.display())]);
            match &launch.session {
                Some(AgentSession::Path(p)) => {
                    out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_SESSION={p}")]);
                }
                Some(AgentSession::Id(id)) => {
                    out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_SESSION_ID={id}")]);
                }
                None => {}
            }
        }
        None => out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_FILE={}", launch.file.display())]),
    }
    if let Some(target) = &launch.deliver {
        out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_DELIVER_TO={}", target.pane)]);
        if let Some(agent) = &target.agent {
            out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_DELIVER_AGENT={agent}")]);
        }
    }
    out
}

/// `herdr pane process-info --pane <pane>`, raw JSON.
pub(crate) fn process_info(env: &HerdrEnv, pane: &str) -> Result<String> {
    let output = Command::new(&env.bin)
        .args(["pane", "process-info", "--pane", pane])
        .output()
        .with_context(|| format!("running {} pane process-info", env.bin.display()))?;
    if !output.status.success() {
        anyhow::bail!("herdr pane process-info {pane}: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `herdr agent get <pane>`, raw JSON; `None` when the pane has no agent Herdr can describe.
pub(crate) fn agent_get(env: &HerdrEnv, pane: &str) -> Option<String> {
    let output = Command::new(&env.bin).args(["agent", "get", pane]).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run the launch through `bin`. Herdr's own stdout/stderr pass through.
pub(crate) fn run(env: &HerdrEnv, launch: &Launch) -> Result<()> {
    if !env.in_herdr {
        anyhow::bail!("not inside Herdr (HERDR_ENV is not set)");
    }
    let status = Command::new(&env.bin)
        .args(argv(launch))
        .status()
        .with_context(|| format!("running {}", env.bin.display()))?;
    if !status.success() {
        anyhow::bail!("herdr plugin pane open failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
