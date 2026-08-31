//! `plannotator-tui last`: annotate an agent's most recent message (docs/spec-last-message.md).
//!
//! The modules here isolate home-directory, process-table, and transcript access from the
//! ordinary app over a transient document that is never written to disk.

#![allow(clippy::print_stdout, clippy::print_stderr, reason = "`--print` is a stdout contract")]

mod exact;
mod fallback;
mod locate;
mod readers;
mod roots;

use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context, Result};
use plannotator_tui_hosts::Message;
use plannotator_tui_schema::{DocumentSource, Provenance};

use crate::app::App;
use crate::cli;

/// What `last` was asked to open. Every field optional; detection fills the gaps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LastOptions {
    /// `claude` | `codex`; else detected from the environment.
    pub(crate) host: Option<String>,
    /// The agent process to start the transcript search from.
    pub(crate) pid: Option<u32>,
    /// An explicit transcript; skips detection. Its format is sniffed when no host is named.
    pub(crate) session: Option<PathBuf>,
    /// An exact host-specific session id; takes precedence over pid and cwd discovery.
    pub(crate) session_id: Option<String>,
    /// Read the document from stdin instead of a transcript.
    pub(crate) stdin: bool,
    /// Print the newest message and exit instead of opening the UI.
    pub(crate) print: bool,
    /// How many recent messages the picker offers.
    pub(crate) pick: usize,
}

pub(crate) fn run(options: &LastOptions) -> Result<()> {
    if options.stdin {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).context("reading stdin")?;
        if options.print {
            print!("{text}");
            return Ok(());
        }
        let source = DocumentSource::new(text, "stdin · message", true, Provenance::Stdin);
        return cli::run_ui(|width| App::open(source, width, cli::delivery(true)));
    }
    let located = match locate::locate(options) {
        Ok(located) => located,
        // The delivery contract: a script or hook must never be aborted by us.
        Err(err) if options.print => {
            eprintln!("plannotator-tui last: {err:#}");
            return Ok(());
        }
        // Inside Herdr we can still show what the agent printed, whatever it is.
        Err(err) => {
            let Some(screen) = locate::screen_fallback(&crate::herdr::context::HerdrEnv::from_env()) else {
                return Err(err);
            };
            let note = format!("{err:#} — showing the pane's recent output instead");
            return cli::run_ui(|width| {
                let mut app = App::open(screen, width, cli::delivery(true))?;
                app.set_status(note.clone());
                Ok(app)
            });
        }
    };
    if options.print {
        if let Some(newest) = located.messages.first() {
            println!("{}", newest.text);
        }
        return Ok(());
    }
    let label = located.host.label();
    let transcript = located.transcript.display().to_string();
    let messages = located.messages;
    cli::run_ui(|width| App::open_message(label, &transcript, messages, width, cli::delivery(true)))
}

/// A message as a document: transient, provenance names the host, transcript and message.
pub(crate) fn message_source(host: &str, transcript: &str, message: &Message) -> DocumentSource {
    DocumentSource::new(
        message.text.clone(),
        format!("{host} · last message"),
        true,
        Provenance::AgentMessage {
            host: host.to_owned(),
            session: Some(transcript.to_owned()),
            message_id: Some(message.id.clone()),
        },
    )
}
