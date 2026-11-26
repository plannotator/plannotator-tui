//! An agent's transcript, found the way `last` finds it, then read again as it grows: the
//! code review's Ask reads its answers from there.

#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Result, bail};
use plannotator_tui_hosts::{Host, Message};

use super::fallback::Discovery;
use super::{LastOptions, locate, readers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Transcript {
    host: Host,
    path: PathBuf,
    /// The host's session id, for hosts whose transcript holds several sessions.
    session_id: Option<String>,
    /// The agent process, for hosts that read their place in the session from it (Pi).
    pid: Option<u32>,
}

impl Transcript {
    /// The transcript of agent process `pid`, of host `host` when the target names one
    /// (`claude`, `pi`, ...), else as detected. Only the agent's own session: the newest one
    /// in its folder is a guess, and another session's answer would show as this one's.
    pub(crate) fn find(host: Option<&str>, pid: Option<u32>) -> Result<Self> {
        let options = LastOptions { host: host.map(str::to_owned), pid, pick: 1, ..LastOptions::default() };
        let located = locate::locate(&options)?;
        if located.discovery != Discovery::Exact {
            bail!(
                "only a guess ({}) for {}: asking needs the agent's own session, which a target that names its \
                 process gives for Claude Code, Copilot and Pi",
                located.transcript.display(),
                located.host.label()
            );
        }
        Ok(Self { host: located.host, path: located.transcript, session_id: located.session_id, pid })
    }

    /// A transcript of `host` at `path`, without looking anything up.
    #[cfg(test)]
    pub(crate) fn at(host: Host, path: &Path) -> Self {
        Self { host, path: path.to_path_buf(), session_id: None, pid: None }
    }

    pub(crate) fn host(&self) -> Host {
        self.host
    }

    /// The file that grows as the agent writes: Copilot's session is a folder whose
    /// `events.jsonl` it appends to.
    pub(crate) fn file(&self) -> PathBuf {
        match self.host {
            Host::Copilot => self.path.join("events.jsonl"),
            _ => self.path.clone(),
        }
    }

    /// Prompts and replies, newest first, through the `pick`th reply.
    pub(crate) fn conversation(&self, pick: usize) -> Result<Vec<Message>> {
        let (_, messages) =
            readers::explicit(self.host, &self.path, self.session_id.as_deref(), self.pid, pick)?;
        Ok(messages)
    }
}
