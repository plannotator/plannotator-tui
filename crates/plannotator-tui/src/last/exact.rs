//! Resolve a validated exact session id without falling through to heuristic discovery.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use plannotator_tui_hosts::{Host, claude, codex, copilot, droid, omp, opencode, pi};

use super::roots::Roots;

pub(super) enum ExactSession {
    File(PathBuf),
    Codex(Vec<PathBuf>),
    Hermes(PathBuf),
    OpenCode { database: PathBuf, schema: opencode::Schema },
}

pub(super) fn resolve(host: Host, session_id: &str, cwd: &Path, roots: &Roots) -> Result<ExactSession> {
    let id = plannotator_tui_hosts::validate_session_id(session_id)?;
    match host {
        Host::ClaudeCode => {
            let projects = roots.claude_config.join("projects");
            claude::find_transcript_by_id(&projects, cwd, id)?
                .map(ExactSession::File)
                .with_context(|| format!("no Claude Code session {id} in {}", projects.display()))
        }
        Host::Codex => {
            let files = codex::find_transcripts_by_id(&roots.codex_home, id)?;
            if files.is_empty() {
                bail!("no Codex session {id} in {}", roots.codex_home.join("sessions").display());
            }
            Ok(ExactSession::Codex(files))
        }
        Host::Copilot => {
            let root = roots.copilot_home.join("session-state");
            copilot::find_session_by_id(&roots.copilot_home, id)?
                .map(ExactSession::File)
                .with_context(|| format!("no Copilot CLI session {id} in {}", root.display()))
        }
        Host::Droid => {
            let root = roots.factory_config.join("sessions");
            droid::find_transcript_by_id(&roots.factory_config, cwd, id)?
                .map(ExactSession::File)
                .with_context(|| format!("no Droid session {id} in {}", root.display()))
        }
        Host::Pi => {
            let root = roots.pi_sessions(".pi/agent");
            pi::find_transcript_by_id(&root, cwd, id)?
                .map(ExactSession::File)
                .with_context(|| format!("no pi session {id} in {}", root.display()))
        }
        Host::Omp => {
            let root = roots.pi_sessions(omp::DEFAULT_AGENT_DIR);
            omp::find_transcript_by_id(&root, cwd, &roots.home, &std::env::temp_dir(), id)?
                .map(ExactSession::File)
                .with_context(|| format!("no omp session {id} in {}", root.display()))
        }
        Host::Hermes => Ok(ExactSession::Hermes(roots.hermes_database())),
        Host::OpenCode => {
            let databases = roots.opencode_databases();
            let found = databases
                .iter()
                .find_map(|database| {
                    opencode::schema_of(database, id).ok().map(|schema| (database.clone(), schema))
                })
                .with_context(|| format!("no OpenCode session {id} in {}", describe(&databases)))?;
            Ok(ExactSession::OpenCode { database: found.0, schema: found.1 })
        }
    }
}

pub(super) fn describe(paths: &[PathBuf]) -> String {
    paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
}
