//! Find a coding agent's transcript on disk and read its recent rendered messages.
//!
//! Everything here is pure over strings, slices, and injected directories: no `~`, no
//! environment reads, no spawned processes. The binary passes in `sessions_dir`,
//! `projects_dir`, a process-table snapshot, and an env lookup. The rules come from
//! Plannotator's `last` implementation (see `docs/decisions.md`, decision 9).

pub mod claude;
pub mod codex;
pub mod copilot;
pub mod droid;
pub mod pi;

use std::path::PathBuf;

/// A supported agent host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    ClaudeCode,
    Codex,
    /// GitHub Copilot CLI: `~/.copilot/session-state/<uuid>/events.jsonl`.
    Copilot,
    /// Droid (Factory): `~/.factory/sessions/<slug>/<session>.jsonl`, Claude's shape, file order.
    Droid,
    Pi,
}

impl Host {
    /// The short label Herdr and the UI use.
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Droid => "droid",
            Self::Pi => "pi",
        }
    }

    /// Every host with a transcript reader, for messages that list them.
    pub const ALL: [Host; 5] = [Host::ClaudeCode, Host::Codex, Host::Pi, Host::Copilot, Host::Droid];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Human,
    Assistant,
}

/// One rendered message: every text block that shares the host's message id, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub text: String,
    /// The host's timestamp, verbatim, when it has one.
    pub at: Option<String>,
}

/// `~/.claude/sessions/<pid>.json`: one running Claude Code session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub pid: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    pub started_at: u64,
}

#[derive(Debug)]
pub enum HostError {
    /// No transcript could be found; the message names what was searched.
    NoTranscript(String),
    /// A transcript was found but holds no renderable message.
    NoMessages(String),
    /// A host was recognised from the environment but is not supported yet.
    Unsupported(String),
    Io(std::io::Error),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTranscript(msg) | Self::NoMessages(msg) => f.write_str(msg),
            Self::Unsupported(host) => write!(f, "{host} is not supported yet"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<std::io::Error> for HostError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Which host launched us, from the environment. `PLANNOTATOR_TUI_HOST` overrides when it
/// names a known host; then the hosts' own markers, in Plannotator's order; then Claude Code.
///
/// Markers for hosts we do not support yet are reported as [`HostError::Unsupported`]
/// rather than silently treated as Claude Code.
pub fn detect_host(env: impl Fn(&str) -> Option<String>) -> Result<Host, HostError> {
    let set = |key: &str| env(key).is_some_and(|v| !v.trim().is_empty());
    if let Some(name) = env("PLANNOTATOR_TUI_HOST").map(|v| v.trim().to_ascii_lowercase()) {
        match name.as_str() {
            "claude" | "claude-code" | "claude_code" => return Ok(Host::ClaudeCode),
            "codex" => return Ok(Host::Codex),
            "copilot" | "copilot-cli" | "copilot_cli" => return Ok(Host::Copilot),
            "droid" | "factory" => return Ok(Host::Droid),
            "pi" => return Ok(Host::Pi),
            _ => {}
        }
    }
    if set("CODEX_THREAD_ID") {
        return Ok(Host::Codex);
    }
    if set("COPILOT_CLI") {
        return Ok(Host::Copilot);
    }
    // pi exports both: the generic marker names the agent, the specific one is a flag.
    if env("AI_AGENT").is_some_and(|v| v.trim().eq_ignore_ascii_case("pi")) || set("PI_CODING_AGENT") {
        return Ok(Host::Pi);
    }
    for (key, name) in [("OPENCODE", "OpenCode"), ("GEMINI_CLI", "Gemini CLI"), ("OMPCODE", "OMP")] {
        if set(key) {
            return Err(HostError::Unsupported(name.to_owned()));
        }
    }
    Ok(Host::ClaudeCode)
}
