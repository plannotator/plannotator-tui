//! Asking the agent about lines (`ask`): the question goes to it the way Send goes, so it is
//! part of the agent's own conversation, and its answer is read back from the agent's
//! transcript into a box under the lines, the way a note's box sits there (`inline`). More
//! questions on those lines (or on lines inside them) add to that box. Nothing here is saved:
//! the transcript is the record.
//!
//! Finding and reading the transcript take a thread for each question (`watch`), so the
//! screen never waits on them; only the delivery itself, as quick as Send's, runs here once
//! that thread is ready.

mod boxes;
mod text;
mod watch;

use anyhow::Result;

use super::diff::Lines;
use super::{Mode, Screen};
use crate::keys::Action;
#[cfg(test)]
use crate::last::Transcript;

/// The first box id an Ask thread takes: far above any note's (`inline` counts those from 1).
const FIRST_ID: u64 = 1 << 40;

/// The questions of this review, by the lines they are about.
#[derive(Debug, Default)]
pub(super) struct Ask {
    threads: Vec<Thread>,
    /// The box id the next thread takes, less [`FIRST_ID`]. Ids are never reused.
    made: u64,
    /// A question waiting for the agent picker that asking opened.
    waiting: Option<(Lines, Turn)>,
    /// The transcript questions are read from, instead of finding the agent's: tests.
    #[cfg(test)]
    transcript: Option<Transcript>,
}

/// The questions on one set of lines, and their answers: one box.
#[derive(Debug)]
pub(super) struct Thread {
    /// The box's id.
    pub(super) id: u64,
    pub(super) lines: Lines,
    turns: Vec<Turn>,
}

#[derive(Debug)]
struct Turn {
    question: String,
    /// Who answers: the agent's host (`claude`, `pi`, ...), else `agent`.
    agent: String,
    /// What the agent is sent.
    prompt: String,
    /// Whether it reached the agent.
    sent: bool,
    /// The answer read so far: the agent's replies since the question, oldest first.
    answer: String,
    /// Another prompt followed the answer: nothing more comes.
    complete: bool,
    /// Why the question could not go, or its answer cannot be read.
    failed: Option<String>,
    /// The thread finding the transcript and then reading the answer, until it is complete.
    watch: Option<watch::Watch>,
}

impl Ask {
    pub(super) fn threads(&self) -> &[Thread] {
        &self.threads
    }

    /// The thread whose box is `id`.
    pub(super) fn thread(&self, id: u64) -> Option<&Thread> {
        self.threads.iter().find(|thread| thread.id == id)
    }

    /// The thread a question on `lines` goes into, when one is there already.
    pub(super) fn box_for(&self, lines: &Lines) -> Option<&Thread> {
        self.threads.iter().find(|thread| covers(&thread.lines, lines))
    }

    /// Add `turn` to the thread on `lines` or on lines around them, or start one there.
    fn add(&mut self, lines: Lines, turn: Turn) {
        if let Some(thread) = self.threads.iter_mut().find(|thread| covers(&thread.lines, &lines)) {
            thread.turns.push(turn);
            return;
        }
        let id = FIRST_ID + self.made;
        self.made += 1;
        self.threads.push(Thread { id, lines, turns: vec![turn] });
    }
}

impl Screen {
    /// Take what the questions' threads have done: send a question once its thread is ready,
    /// and show the answers read so far. True when a box changed.
    pub(super) fn poll_ask(&mut self) -> bool {
        let mut changed = false;
        for turn in self.ask.threads.iter_mut().flat_map(|thread| thread.turns.iter_mut()) {
            let Some(watch) = &turn.watch else { continue };
            let mut ended = false;
            while let Some(update) = watch.next() {
                changed = true;
                match update {
                    watch::Update::Ready => {
                        let target = self.delivery.describe();
                        match self.delivery.deliver(&turn.prompt) {
                            Ok(()) => {
                                turn.sent = true;
                                let notice = self
                                    .delivery
                                    .notice()
                                    .map(|n| format!(" \u{b7} {n}"))
                                    .unwrap_or_default();
                                self.status =
                                    Some(format!("asked {target}; the answer shows under the lines{notice}"));
                            }
                            Err(error) => {
                                let why = error.status(&target, false, 'a');
                                self.status = Some(format!("not asked: {why}"));
                                turn.failed = Some(why);
                                ended = true;
                            }
                        }
                    }
                    watch::Update::Answer { text, complete } => {
                        turn.answer = text;
                        turn.complete = complete;
                        ended |= complete;
                    }
                    watch::Update::Failed(why) => {
                        if !turn.sent {
                            self.status = Some(format!("not asked: {why}"));
                        }
                        turn.failed = Some(why);
                        ended = true;
                    }
                }
            }
            if ended {
                turn.watch = None;
            }
        }
        changed
    }

    /// `ask`: a question about the selection or the cursor's line.
    pub(super) fn start_ask(&mut self) -> Result<()> {
        let Some(lines) = self.target_lines()? else {
            self.status = Some("move the cursor onto a line of code first".into());
            return Ok(());
        };
        self.open_question(lines)
    }

    /// Ask what the comment box holds. When Send would ask which agent first, so does this.
    pub(super) fn save_question(&mut self, lines: &Lines) {
        self.mode = Mode::Browse;
        let question = self.compose.value().trim().to_owned();
        if question.is_empty() {
            self.status = Some("nothing asked".into());
            return;
        }
        let prompt = self.question_prompt(lines, &question);
        if prompt.len() > super::send::MAX_SEND_BYTES {
            self.status = Some(format!(
                "the question and its code are {} KiB, over the {} KiB one send carries; select fewer lines",
                prompt.len().div_ceil(1024),
                super::send::MAX_SEND_BYTES / 1024
            ));
            return;
        }
        let turn = Turn {
            question,
            agent: String::new(),
            prompt,
            sent: false,
            answer: String::new(),
            complete: false,
            failed: None,
            watch: None,
        };
        if let Some(picker) = self.reach.picker_for_send(self.delivery.as_ref(), &self.real_root()) {
            self.picker = picker;
            self.pick_then = None;
            self.ask.waiting = Some((lines.clone(), turn));
            self.mode = Mode::PickTarget;
            return;
        }
        self.start_question(lines.clone(), turn);
    }

    /// The agent picker that asking opened closed: ask now if it chose a target.
    pub(super) fn after_ask_picker(&mut self, chose: bool) {
        if let Some((lines, turn)) = self.ask.waiting.take() {
            if chose {
                self.start_question(lines, turn);
            } else {
                self.status = Some("nothing asked".into());
            }
        }
    }

    /// The question as the agent reads it: where it comes from, the lines and their code, and
    /// the question itself.
    fn question_prompt(&self, lines: &Lines, question: &str) -> String {
        let diff = self.capture.scope.describe(self.capture.commit_subject.as_deref());
        let about = super::draw::lines_label(lines);
        let from = format!(
            "A question from the code review of {diff} in {}, about `{about}`",
            self.root().display()
        );
        match self.selected_code(lines) {
            Some(code) => format!("{from}:\n\n{}\n{question}", crate::export::fenced(&code)),
            None => format!("{from}:\n\n{question}"),
        }
    }

    /// Put the question in its box and start its thread; it goes to the agent once the thread
    /// has noted where the answer will start ([`Self::poll_ask`]).
    fn start_question(&mut self, lines: Lines, mut turn: Turn) {
        if !self.delivery.is_agent() {
            self.status = Some(match self.keys.hint(Action::PickTarget) {
                key if key.is_empty() => "asking needs an agent to answer".to_owned(),
                key => format!("asking needs an agent to answer; {key} picks one"),
            });
            return;
        }
        let host = self.delivery.agent_host().map(str::to_owned);
        turn.agent = host.clone().unwrap_or_else(|| "agent".to_owned());
        let first = turn.prompt.lines().next().unwrap_or_default().to_owned();
        let question = watch::Question {
            host,
            pid: self.delivery.agent_pid(),
            marks: [first, turn.question.clone()],
            #[cfg(test)]
            transcript: self.ask.transcript.clone(),
        };
        match watch::Watch::start(question, self.wake.clone()) {
            Ok(watch) => turn.watch = Some(watch),
            Err(error) => {
                self.status = Some(format!("not asked: could not start reading the answer: {error}"));
                return;
            }
        }
        self.ask.add(lines, turn);
        self.status = Some(format!("asking {}\u{2026}", self.delivery.describe()));
        let _ = self.diff.clear_selection();
    }

    /// Read answers from `transcript` instead of finding the agent's: tests.
    #[cfg(test)]
    pub(super) fn answer_from(&mut self, transcript: Transcript) {
        self.ask.transcript = Some(transcript);
    }
}

/// Whether a box on `on` is about `lines` too: the same file and side, and `lines` inside.
fn covers(on: &Lines, lines: &Lines) -> bool {
    on.path == lines.path && on.side == lines.side && on.first <= lines.first && lines.last <= on.last
}
