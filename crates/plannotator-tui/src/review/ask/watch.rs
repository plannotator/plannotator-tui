//! The work one Ask question waits on, on a thread of its own so the screen never does: find
//! the agent's transcript, note the newest prompt in it, say the question can go, then read
//! the transcript again each time it grows until the answer is complete.
//!
//! The question is the first prompt after the noted one that holds the question's first line
//! and its text, so a prompt typed into the agent meanwhile, or another question still
//! waiting in its queue, is not taken for it. The answer is the replies after it up to the
//! next prompt.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, SystemTime};

use plannotator_tui_hosts::{Host, Message, Role};

use crate::last::Transcript;

/// How often the transcript's size and time are looked at; it is read only when they change.
const POLL: Duration = Duration::from_millis(300);

/// How many replies a read of the transcript goes back through: an answer can be many
/// messages between tool calls, and the noted prompt must still be in reach.
const PICK: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Update {
    /// The transcript was found and its newest prompt noted: the question can go now.
    Ready,
    /// The answer as far as the transcript has it; `complete` once another prompt followed.
    Answer { text: String, complete: bool },
    /// The question cannot be asked, or its answer cannot be read; nothing more comes.
    Failed(String),
}

/// One question, as its thread looks for it.
#[derive(Debug)]
pub(super) struct Question {
    /// The agent's host and process, as the target names them.
    pub(super) host: Option<String>,
    pub(super) pid: Option<u32>,
    /// What the prompt holds that tells it apart: its first line (the file and lines) and the
    /// question as typed.
    pub(super) marks: [String; 2],
    /// The transcript to read instead of finding the agent's: tests.
    #[cfg(test)]
    pub(super) transcript: Option<Transcript>,
}

#[derive(Debug)]
pub(super) struct Watch {
    updates: Receiver<Update>,
    stop: Arc<AtomicBool>,
}

impl Watch {
    /// Start the thread for `question`, which sends each step and pokes `wake`.
    pub(super) fn start(question: Question, wake: SyncSender<()>) -> std::io::Result<Self> {
        let (sender, updates) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        std::thread::Builder::new().name("review-ask".into()).spawn(move || {
            let send = |update: Update| {
                let sent = sender.send(update).is_ok();
                let _ = wake.try_send(());
                sent
            };
            run(&question, &send, &stopped);
        })?;
        Ok(Self { updates, stop })
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

fn run(question: &Question, send: &impl Fn(Update) -> bool, stop: &AtomicBool) {
    let noted = transcript(question).and_then(|transcript| {
        // Codex's reader gives only the replies of the newest turn: no prompt to find.
        if transcript.host() == Host::Codex {
            return Err(
                "asking is not supported for codex yet: its transcript reader gives only the agent's replies"
                    .to_owned(),
            );
        }
        let messages = transcript
            .conversation(PICK)
            .map_err(|error| format!("cannot read the agent's transcript: {error:#}"))?;
        Ok((newest_prompt(&messages), transcript))
    });
    match noted {
        Ok((after, transcript)) if send(Update::Ready) => {
            follow(&transcript, after.as_deref(), question, send, stop);
        }
        Ok(_) => {}
        Err(why) => {
            send(Update::Failed(why));
        }
    }
}

fn transcript(question: &Question) -> Result<Transcript, String> {
    #[cfg(test)]
    if let Some(transcript) = &question.transcript {
        return Ok(transcript.clone());
    }
    Transcript::find(question.host.as_deref(), question.pid)
        .map_err(|error| format!("cannot find the agent's session: {error:#}"))
}

/// Read `transcript` each time it changes for the answer to `question` after prompt `after`.
/// A read that fails is tried again at the next change: the file may be mid-write.
fn follow(
    transcript: &Transcript,
    after: Option<&str>,
    question: &Question,
    send: &impl Fn(Update) -> bool,
    stop: &AtomicBool,
) {
    let marks = question.marks.each_ref().map(String::as_str);
    let file = transcript.file();
    let mut seen: Option<(u64, Option<SystemTime>)> = None;
    let mut last: Option<Update> = None;
    while !stop.load(Ordering::Relaxed) {
        let now = std::fs::metadata(&file).ok().map(|meta| (meta.len(), meta.modified().ok()));
        if now.is_none() || now != seen {
            seen = now;
            // shortcut: the whole transcript is read again on each change (a few MB, a few
            // hundred ms on this thread), not just what was added; read only the tail if a
            // large transcript makes answers lag.
            let update = match transcript.conversation(PICK).map(|messages| answer(&messages, after, &marks))
            {
                Ok(Found::Answer { text, complete }) => Some(Update::Answer { text, complete }),
                Ok(Found::Lost) => {
                    Some(Update::Failed("the question is out of reach in the transcript".to_owned()))
                }
                Ok(Found::Waiting) | Err(_) => None,
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

/// The id of the newest prompt in `newest_first`, if it has one.
fn newest_prompt(newest_first: &[Message]) -> Option<String> {
    newest_first.iter().find(|message| message.role == Role::Human).map(|message| message.id.clone())
}

/// What the transcript says of a question.
#[derive(Debug, PartialEq, Eq)]
enum Found {
    /// The question is not in it yet.
    Waiting,
    /// The replies to it so far, oldest first; `complete` once another prompt followed.
    Answer { text: String, complete: bool },
    /// The noted prompt is out of reach, so the question cannot be told from older ones.
    Lost,
}

/// The answer, in `newest_first`, to the first prompt after prompt `after` (with `None`, after
/// none) that holds every one of `marks`.
fn answer(newest_first: &[Message], after: Option<&str>, marks: &[&str]) -> Found {
    let newer = match after {
        Some(id) => match newest_first.iter().position(|message| message.id == id) {
            Some(at) => newest_first.get(..at).unwrap_or_default(),
            None => return Found::Lost,
        },
        None => newest_first,
    };
    // Newest first, so the last match in `newer` is the oldest: the question.
    let holds = |message: &Message| {
        message.role == Role::Human && marks.iter().all(|mark| message.text.contains(mark))
    };
    let Some(asked) = newer.iter().rposition(holds) else { return Found::Waiting };
    let since = newer.get(..asked).unwrap_or_default();
    let next = since.iter().rposition(|message| message.role == Role::Human);
    let replies = next.map_or(since, |next| since.get(next + 1..).unwrap_or_default());
    let text: Vec<&str> = replies
        .iter()
        .rev()
        .filter(|message| message.role == Role::Assistant)
        .map(|message| message.text.as_str())
        .collect();
    Found::Answer { text: text.join("\n\n"), complete: next.is_some() }
}

#[cfg(test)]
mod tests;
