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
pub mod hermes;
pub mod omp;
pub mod opencode;
pub mod pi;
pub(crate) mod sqlite;
pub(crate) mod time;

use std::path::{Path, PathBuf};

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
    /// Oh My Pi: pi's format and layout under `~/.omp/agent/sessions`.
    Omp,
    /// Hermes CLI: conversations in `SQLite` (`~/.hermes/state.db`), addressed by session id.
    Hermes,
    /// `OpenCode`: sessions, messages and parts in `SQLite` (`<xdg data>/opencode/opencode.db`),
    /// addressed by session id or found by the directory `OpenCode` was started in.
    OpenCode,
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
            Self::Omp => "omp",
            Self::Hermes => "hermes",
            Self::OpenCode => "opencode",
        }
    }

    /// Every host with a transcript reader, for messages that list them.
    pub const ALL: [Host; 8] = [
        Host::ClaudeCode,
        Host::Codex,
        Host::Pi,
        Host::Omp,
        Host::Copilot,
        Host::Droid,
        Host::Hermes,
        Host::OpenCode,
    ];
}

/// Which reader a transcript file wants, from its first lines. For a path handed to us
/// without a host name (Herdr's `agent_session`, a user's `--session`).
pub fn sniff(head: &str) -> Option<Host> {
    // Hand-written or pretty-printed JSON puts a space after the colon; compact writers don't.
    let compact = head.replace(": ", ":");
    let lines: Vec<&str> = compact.lines().filter(|l| !l.trim().is_empty()).take(50).collect();
    let any = |needle: &str| lines.iter().any(|l| l.contains(needle));
    if any(r#""type":"session""#) && (any(r#""parentId""#) || any(r#""version""#)) {
        return Some(Host::Pi);
    }
    if any(r#""type":"assistant.message""#) || any(r#""type":"session.start""#) {
        return Some(Host::Copilot);
    }
    if any(r#""type":"response_item""#) || any(r#""type":"event_msg""#) || any(r#""type":"session_meta""#) {
        return Some(Host::Codex);
    }
    // Droid: Claude's message shape keyed by `id`/`parentId`, no pi session header.
    if any(r#""parentId""#) && any(r#""type":"message""#) {
        return Some(Host::Droid);
    }
    if any(r#""parentUuid""#) || any(r#""uuid""#) {
        return Some(Host::ClaudeCode);
    }
    None
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

/// Which rung of a discovery ladder chose a session, so a caller can say whether the
/// result identifies the agent's own session or is the best guess for its folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// The session the agent process (or one of its ancestors) registered: not a guess.
    Session,
    /// A session whose recorded working directory is the agent's, newest first.
    Cwd,
    /// The newest session filed under the agent's folder or one of its parents.
    Folder,
    /// The newest session the host has at all, not scoped to any directory.
    Newest,
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
    /// An exact session id is not one safe path component.
    InvalidSessionId(String),
    Io(std::io::Error),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTranscript(msg) | Self::NoMessages(msg) | Self::InvalidSessionId(msg) => f.write_str(msg),
            Self::Unsupported(host) => write!(f, "{host} is not supported yet"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

/// Reject an exact session id before a resolver touches the filesystem or a database.
pub fn validate_session_id(id: &str) -> Result<&str, HostError> {
    if id.trim().is_empty() || id.contains(['/', '\\', '\0']) || id.contains("..") {
        return Err(HostError::InvalidSessionId(format!(
            "invalid session id {id:?}: expected one non-empty path component"
        )));
    }
    Ok(id)
}

impl std::error::Error for HostError {}

impl From<std::io::Error> for HostError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// The host-assigned session id a transcript's name carries, when the host's naming
/// scheme makes it unambiguous: Claude Code and Droid file a session as `<uuid>.jsonl`,
/// a Codex rollout ends in its thread uuid, and a Copilot session is a directory named by
/// its uuid. Anything that is not uuid-shaped yields `None`: an arbitrary path handed in
/// with `--session` must never be mistaken for an id, and a path is never one.
pub fn session_id_of(host: Host, transcript: &Path) -> Option<String> {
    let candidate = match host {
        Host::ClaudeCode | Host::Droid => transcript.file_stem()?.to_str()?.to_owned(),
        Host::Codex => codex::thread_of(transcript)?,
        Host::Copilot => transcript.file_name()?.to_str()?.to_owned(),
        Host::Pi | Host::Omp | Host::Hermes | Host::OpenCode => return None,
    };
    is_uuid(&candidate).then_some(candidate)
}

/// `8-4-4-4-12` hex groups, any case.
fn is_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
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
            "omp" | "oh-my-pi" | "ohmypi" => return Ok(Host::Omp),
            "hermes" | "hermes-cli" | "hermes_cli" => return Ok(Host::Hermes),
            "opencode" | "open-code" | "open_code" => return Ok(Host::OpenCode),
            _ => {}
        }
    }
    if set("CODEX_THREAD_ID") {
        return Ok(Host::Codex);
    }
    if set("COPILOT_CLI") {
        return Ok(Host::Copilot);
    }
    let ai_agent = env("AI_AGENT").map(|v| v.trim().to_ascii_lowercase());
    // Oh My Pi exports `OMPCODE=1` (and `CLAUDECODE=1`) into the shells it spawns and no
    // pi marker (verified in oh-my-pi `packages/utils/src/procmgr.ts`), so it is found by
    // `OMPCODE` below; `AI_AGENT=omp` is accepted in case a future version adds it.
    if ai_agent.as_deref() == Some("omp") {
        return Ok(Host::Omp);
    }
    // pi exports both: the generic marker names the agent, the specific one is a flag.
    if ai_agent.as_deref() == Some("pi") || set("PI_CODING_AGENT") {
        return Ok(Host::Pi);
    }
    if set("OPENCODE") {
        return Ok(Host::OpenCode);
    }
    if set("GEMINI_CLI") {
        return Err(HostError::Unsupported("Gemini CLI".to_owned()));
    }
    if set("OMPCODE") {
        return Ok(Host::Omp);
    }
    Ok(Host::ClaudeCode)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01a04583-a848-7b21-a890-f3ed0c9fef05";

    #[test]
    fn a_uuid_named_transcript_yields_its_session_id() {
        let claude = Path::new("/h/.claude/projects/-w-repo").join(format!("{ID}.jsonl"));
        assert_eq!(session_id_of(Host::ClaudeCode, &claude).as_deref(), Some(ID));
        let droid = Path::new("/h/.factory/sessions/-w-repo").join(format!("{ID}.jsonl"));
        assert_eq!(session_id_of(Host::Droid, &droid).as_deref(), Some(ID));
        let codex = Path::new("/h/.codex/sessions/2026/08/27")
            .join(format!("rollout-2026-08-27T16-17-33-{ID}.jsonl"));
        assert_eq!(session_id_of(Host::Codex, &codex).as_deref(), Some(ID));
        let copilot = Path::new("/h/.copilot/session-state").join(ID);
        assert_eq!(session_id_of(Host::Copilot, &copilot).as_deref(), Some(ID));
    }

    #[test]
    fn anything_that_is_not_uuid_shaped_is_not_an_id() {
        assert_eq!(session_id_of(Host::ClaudeCode, Path::new("/tmp/session.jsonl")), None);
        assert_eq!(
            session_id_of(Host::Codex, Path::new("/tmp/rollout-2026-08-27T16-17-33-not-a-uuid.jsonl")),
            None
        );
        assert_eq!(session_id_of(Host::Copilot, Path::new("/h/.copilot/session-state/near")), None);
        let pi = Path::new("/h/.pi/agent/sessions/x").join(format!("{ID}.jsonl"));
        assert_eq!(session_id_of(Host::Pi, &pi), None, "pi names sessions by pattern, not by id alone");
        assert_eq!(session_id_of(Host::OpenCode, Path::new("/h/opencode.db")), None);
    }

    #[test]
    fn uuid_shape_is_checked_strictly() {
        assert!(is_uuid(ID));
        assert!(is_uuid("AAAA1111-0000-4000-8000-000000000001"));
        assert!(!is_uuid("01a04583a8487b21a890f3ed0c9fef05"));
        assert!(!is_uuid("01a04583-a848-7b21-a890-f3ed0c9fef0g"));
        assert!(!is_uuid(""));
    }
}
