//! `plannotator-tui last`: annotate an agent's most recent message (docs/spec-last-message.md).
//!
//! `locate` is the one impure step (home directory, process table, transcript files); the
//! rest is the ordinary app over a transient document that is never written to disk.

#![allow(clippy::print_stdout, clippy::print_stderr, reason = "`--print` is a stdout contract")]

mod locate;

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
    /// An explicit transcript; skips detection.
    pub(crate) session: Option<PathBuf>,
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
        Err(err) => return Err(err),
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
