//! An agent's transcript, found once the way `last` finds it, then read again as it grows:
//! the code review's Ask reads its answers from there.

use std::path::{Path, PathBuf};

use anyhow::Result;
use plannotator_tui_hosts::{Host, Message};

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
    /// The transcript of agent process `pid` (else this folder's newest session), of host
    /// `host` when the target names one (`claude`, `codex`, ...), else as detected.
    pub(crate) fn find(host: Option<&str>, pid: Option<u32>) -> Result<Self> {
        let options = LastOptions { host: host.map(str::to_owned), pid, pick: 1, ..LastOptions::default() };
        let located = locate::locate(&options)?;
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

    /// Whether its reader gives the person's prompts too, which Ask finds its question by.
    /// Codex's gives only the replies of the newest turn.
    pub(crate) fn has_prompts(&self) -> bool {
        self.host != Host::Codex
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Prompts and replies, newest first, through the `pick`th reply.
    pub(crate) fn conversation(&self, pick: usize) -> Result<Vec<Message>> {
        let (_, messages) =
            readers::explicit(self.host, &self.path, self.session_id.as_deref(), self.pid, pick)?;
        Ok(messages)
    }
}
