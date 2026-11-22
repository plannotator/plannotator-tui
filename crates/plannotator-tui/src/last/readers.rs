//! Read one already-selected transcript or database session.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use plannotator_tui_hosts::{
    Host, Message, Role, antigravity, claude, codex, commandcode, copilot, droid, hermes, omp, opencode, pi,
};

use super::exact::ExactSession;

/// `pid` is the agent's process when the launcher named it; a Pi reads its place in the
/// session tree from that process's own registry entry.
pub(super) fn explicit(
    host: Host,
    path: &Path,
    session_id: Option<&str>,
    pid: Option<u32>,
    pick: usize,
) -> Result<(PathBuf, Vec<Message>)> {
    match host {
        Host::ClaudeCode => Ok((path.to_path_buf(), claude_messages(path, pick)?)),
        Host::Codex if path.is_dir() => codex_thread(path, None, pick),
        Host::Codex => {
            let files = [path.to_path_buf()];
            Ok((path.to_path_buf(), codex_messages(&files, pick)?))
        }
        Host::Copilot => Ok((path.to_path_buf(), copilot_messages(path, pick)?)),
        Host::Droid => Ok((path.to_path_buf(), droid_messages(path, pick)?)),
        Host::Pi => Ok((path.to_path_buf(), pi_messages(path, pid, pick)?)),
        Host::Omp => Ok((path.to_path_buf(), omp_messages(path, pick)?)),
        Host::CommandCode => Ok((path.to_path_buf(), commandcode_messages(path, pick)?)),
        Host::Antigravity => Ok((path.to_path_buf(), antigravity_messages(path, pick)?)),
        Host::Hermes => {
            let id = required_session_id(session_id, "hermes")?;
            Ok((path.to_path_buf(), hermes::messages_for_session(path, id, pick)?))
        }
        Host::OpenCode => {
            let id = required_session_id(session_id, "opencode")?;
            let schema = opencode::schema_of(path, id)?;
            Ok((path.to_path_buf(), opencode::messages_for_session(path, id, schema, pick)?))
        }
    }
}

pub(super) fn exact(
    host: Host,
    session_id: &str,
    exact: ExactSession,
    pid: Option<u32>,
    pick: usize,
) -> Result<(PathBuf, Vec<Message>)> {
    match (host, exact) {
        (Host::ClaudeCode, ExactSession::File(path)) => {
            let messages = claude_messages(&path, pick)?;
            Ok((path, messages))
        }
        (Host::Codex, ExactSession::Codex(files)) => codex_files(&files, session_id, pick),
        (Host::Copilot, ExactSession::File(path)) => {
            let messages = copilot_messages(&path, pick)?;
            Ok((path, messages))
        }
        (Host::Droid, ExactSession::File(path)) => {
            let messages = droid_messages(&path, pick)?;
            Ok((path, messages))
        }
        (Host::Pi, ExactSession::File(path)) => {
            let messages = pi_messages(&path, pid, pick)?;
            Ok((path, messages))
        }
        (Host::Omp, ExactSession::File(path)) => {
            let messages = omp_messages(&path, pick)?;
            Ok((path, messages))
        }
        (Host::CommandCode, ExactSession::File(path)) => {
            let messages = commandcode_messages(&path, pick)?;
            Ok((path, messages))
        }
        (Host::Antigravity, ExactSession::File(path)) => {
            let messages = antigravity_messages(&path, pick)?;
            Ok((path, messages))
        }
        (Host::Hermes, ExactSession::Hermes(database)) => {
            let messages = hermes::messages_for_session(&database, session_id, pick)?;
            Ok((database, messages))
        }
        (Host::OpenCode, ExactSession::OpenCode { database, schema }) => {
            let messages = opencode::messages_for_session(&database, session_id, schema, pick)?;
            Ok((database, messages))
        }
        _ => bail!("internal error: exact session kind does not match {}", host.label()),
    }
}

fn required_session_id<'a>(session_id: Option<&'a str>, host: &str) -> Result<&'a str> {
    let Some(id) = session_id else {
        bail!("{host} needs a session id (Herdr provides it; or pass --session-id)");
    };
    Ok(plannotator_tui_hosts::validate_session_id(id)?)
}

pub(super) fn claude_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(claude::parse_messages(&text, pick))
}

pub(super) fn codex_thread(
    codex_home: &Path,
    thread: Option<&str>,
    pick: usize,
) -> Result<(PathBuf, Vec<Message>)> {
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

fn codex_files(files: &[PathBuf], thread: &str, pick: usize) -> Result<(PathBuf, Vec<Message>)> {
    let Some(newest) = files.last().cloned() else {
        bail!("no Codex transcript for thread {thread}");
    };
    let messages = codex_messages(files, pick)?;
    Ok((newest, messages))
}

fn codex_messages(files: &[PathBuf], pick: usize) -> Result<Vec<Message>> {
    let contents = files
        .iter()
        .map(|path| std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display())))
        .collect::<Result<Vec<String>>>()?;
    Ok(codex::parse_messages(&contents, pick))
}

pub(super) fn copilot_messages(dir: &Path, pick: usize) -> Result<Vec<Message>> {
    let events = dir.join("events.jsonl");
    let text = std::fs::read_to_string(&events).with_context(|| format!("reading {}", events.display()))?;
    Ok(copilot::parse_messages(&text, pick))
}

pub(super) fn droid_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(droid::parse_messages(&text, pick))
}

pub(super) fn omp_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(omp::parse_messages(&text, pick))
}

pub(super) fn commandcode_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(commandcode::parse_messages(&text, pick))
}

pub(super) fn antigravity_messages(path: &Path, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(antigravity::parse_messages(&text, pick))
}

/// A Pi transcript's messages on the branch the session stands on. A running Pi with our
/// extension records its place in the tree in its link registry entry, since `/tree`
/// writes nothing to the transcript; `pid` picks that Pi's own entry. Without an entry,
/// the newest entry's branch.
pub(super) fn pi_messages(path: &Path, pid: Option<u32>, pick: usize) -> Result<Vec<Message>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let entry = crate::pi::live_entry_for_transcript(&crate::pi::root(), path, pid);
    let Some(at) = entry.as_ref().and_then(crate::pi::tree_position) else {
        return Ok(pi::parse_messages(&text, pick));
    };
    let messages = pi::parse_messages_at(&text, pick, &at.tip, at.leaf.as_deref());
    let replies = |messages: &[Message]| messages.iter().any(|m| m.role == Role::Assistant);
    if !replies(&messages) && replies(&pi::parse_messages(&text, pick)) {
        bail!(
            "Pi stands before the first reply of its session tree (moved there with /tree); choose a reply in /tree, or send a message, then try again"
        );
    }
    Ok(messages)
}
