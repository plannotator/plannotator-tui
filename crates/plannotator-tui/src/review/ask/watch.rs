//! The work an Ask question waits on, on a thread of its own so the screen never does: find
//! the agent's transcript (unless it was found before), note the newest prompt in it, say the
//! question can go, then read the transcript again each time it grows until the answer is
//! complete: the agent got another prompt after it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::time::{Duration, SystemTime};

use crate::last::Transcript;

/// How often the transcript's size and time are looked at; it is read only when they change.
const POLL: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Update {
    /// The transcript was found and its newest prompt noted: the question can go now.
    Ready(Transcript),
    /// The answer as far as the transcript has it; `complete` once another prompt followed.
    Answer { text: String, complete: bool },
    /// The transcript cannot be found or read; nothing more comes.
    Failed(String),
}

/// Where the answer will be: the transcript found before for this agent, else the agent's
/// host and process to find it from.
#[derive(Debug)]
pub(super) struct Source {
    pub(super) found: Option<Transcript>,
    pub(super) host: Option<String>,
    pub(super) pid: Option<u32>,
}

#[derive(Debug)]
pub(super) struct Watch {
    updates: Receiver<Update>,
    stop: Arc<AtomicBool>,
}

impl Watch {
    /// Start the work for one question, sending each step and poking `wake`.
    pub(super) fn start(source: Source, wake: SyncSender<()>) -> Self {
        let (sender, updates) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        std::thread::spawn(move || run(source, &sender, &wake, &stopped));
        Self { updates, stop }
    }

    pub(super) fn next(&self) -> Option<Update> {
        self.updates.try_recv().ok()
    }
}

impl Drop for Watch {
    /// The thread ends at its next look, with the review or the question.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(source: Source, sender: &Sender<Update>, wake: &SyncSender<()>, stop: &AtomicBool) {
    let send = |update: Update| {
        let sent = sender.send(update).is_ok();
        let _ = wake.try_send(());
        sent
    };
    let found = match source.found {
        Some(transcript) => Ok(transcript),
        None => Transcript::find(source.host.as_deref(), source.pid)
            .map_err(|error| format!("cannot find the agent's transcript: {error:#}")),
    };
    let found = found.and_then(|transcript| {
        if transcript.has_prompts() {
            return Ok(transcript);
        }
        Err(format!(
            "asking is not supported for {} yet: its transcript reader gives only the agent's replies",
            transcript.host().label()
        ))
    });
    let noted = found.and_then(|transcript| match transcript.conversation(super::PICK) {
        Ok(messages) => Ok((super::newest_prompt(&messages), transcript)),
        Err(error) => Err(format!("cannot read the agent's transcript: {error:#}")),
    });
    let (after, transcript) = match noted {
        Ok(noted) => noted,
        Err(why) => {
            send(Update::Failed(why));
            return;
        }
    };
    if send(Update::Ready(transcript.clone())) {
        follow(&transcript, after.as_deref(), &send, stop);
    }
}

/// Read `transcript` each time it changes for the answer to the first prompt after `after`.
fn follow(transcript: &Transcript, after: Option<&str>, send: &impl Fn(Update) -> bool, stop: &AtomicBool) {
    let mut seen: Option<(u64, Option<SystemTime>)> = None;
    let mut last: Option<Update> = None;
    while !stop.load(Ordering::Relaxed) {
        let now = std::fs::metadata(transcript.path()).ok().map(|meta| (meta.len(), meta.modified().ok()));
        if now.is_none() || now != seen {
            seen = now;
            // shortcut: the whole transcript is read again on each change (a few MB, a few
            // hundred ms on this thread), not just what was added; read only the tail if a
            // large transcript makes answers lag.
            let update = match transcript.conversation(super::PICK) {
                Ok(messages) => {
                    super::answer(&messages, after).map(|(text, complete)| Update::Answer { text, complete })
                }
                Err(error) => Some(Update::Failed(format!("cannot read the agent's transcript: {error:#}"))),
            };
            if let Some(update) = update
                && last.as_ref() != Some(&update)
            {
                let ends = matches!(update, Update::Answer { complete: true, .. } | Update::Failed(_));
                if !send(update.clone()) || ends {
                    return;
                }
                last = Some(update);
            }
        }
        std::thread::sleep(POLL);
    }
}
