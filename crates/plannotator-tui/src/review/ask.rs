//! Asking the agent about lines (`ask`): the question goes to it the way Send goes, so it is
//! part of the agent's own conversation, and its answer is read back from the agent's
//! transcript into a box under the lines, the way a note's box sits there (`inline`). More
//! questions on those lines (or on lines inside them) add to that box. Nothing here is saved:
//! the transcript is the record.
//!
//! Finding and reading the transcript take a thread (`watch`), so the screen never waits on
//! them; only the delivery itself, as quick as Send's, runs here once the thread is ready.

mod watch;

use std::hash::{Hash as _, Hasher as _};

use anyhow::Result;
use plannotator_tui_hosts::{Message, Role};
use plannotator_tui_review::Side;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};

use super::diff::Lines;
use super::{Draft, Mode, Screen};
use crate::keys::{self, Action};
use crate::last::Transcript;
use crate::theme::palette;
use crate::ui::compose::Compose;

/// How many replies a read of the transcript goes back through: an answer can be many
/// messages between tool calls, and the question must still be in reach.
const PICK: usize = 400;

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
    /// The transcript found last, and the agent it belongs to, so the next question to that
    /// agent need not find it again.
    found: Option<Found>,
}

#[derive(Debug)]
struct Found {
    host: Option<String>,
    pid: Option<u32>,
    transcript: Transcript,
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

impl Thread {
    /// `ask · L9`, `ask · L9–12`, `ask · L7 old`, as a note's box names its lines.
    pub(super) fn label(&self) -> String {
        let Lines { side, first, last, .. } = &self.lines;
        let lines = if first == last { format!("L{first}") } else { format!("L{first}\u{2013}{last}") };
        let old = if *side == Side::Old { " old" } else { "" };
        format!("ask \u{b7} {lines}{old}")
    }

    /// A number that changes whenever what the box shows does.
    pub(super) fn revision(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for turn in &self.turns {
            (&turn.question, turn.sent, &turn.answer, &turn.failed).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The box's text at `width` columns: each question, then its answer as Markdown, all of it.
    pub(super) fn body(&self, width: usize) -> Vec<Line<'static>> {
        let speaker =
            |who: &str, colour| Line::from(Span::styled(who.to_owned(), Style::new().fg(colour).bold()));
        let mut rows = Vec::new();
        for (index, turn) in self.turns.iter().enumerate() {
            if index > 0 {
                let rule = "\u{254c}".repeat(width);
                rows.push(Line::from(Span::styled(rule, Style::new().fg(palette().idle_fg))));
            }
            rows.push(speaker("You", palette().accent));
            for line in turn.question.lines() {
                rows.extend(wrapped(&Line::from(line.to_owned()), width));
            }
            rows.push(Line::default());
            rows.push(speaker(&turn.agent, palette().heading));
            for line in crate::layout::styled(&turn.answer).lines {
                rows.extend(wrapped(&line, width));
            }
            let waiting = if turn.sent { "waiting for the answer\u{2026}" } else { "sending\u{2026}" };
            match &turn.failed {
                Some(why) => {
                    let why = if turn.sent {
                        format!("cannot read the answer: {why}")
                    } else {
                        format!("not sent: {why}")
                    };
                    rows.extend(wrapped(&Line::from(Span::styled(why, palette().delete)), width));
                }
                None if turn.answer.is_empty() => rows.push(Line::from(Span::raw(waiting).italic())),
                None => {}
            }
        }
        rows
    }
}

fn wrapped(line: &Line<'_>, width: usize) -> impl Iterator<Item = Line<'static>> {
    let chars: usize = line.spans.iter().map(|span| span.content.chars().count()).sum();
    crate::wrap::wrap_line(line, &vec![None; chars], width).into_iter().map(|row| row.line)
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
                    watch::Update::Ready(transcript) => {
                        let host = self.delivery.agent_host().map(str::to_owned);
                        self.ask.found = Some(Found { host, pid: self.delivery.agent_pid(), transcript });
                        let target = self.delivery.describe();
                        match self.delivery.deliver(&turn.prompt) {
                            Ok(()) => {
                                turn.sent = true;
                                self.status =
                                    Some(format!("asked {target}; the answer shows under the lines"));
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

    /// The question box for `lines`. When they have a box already the question goes into, it
    /// opens at that box's end: the diff scrolls, as the wheel does, until the end and room
    /// for the question box under it show, which a long answer pushes below the pane.
    fn open_question(&mut self, lines: Lines) -> Result<()> {
        if let Some(id) = self.ask.box_for(&lines).map(|thread| thread.id) {
            let pane = self.geometry.diff;
            if let Some(area) = self.diff.box_areas().iter().find(|area| area.id == id) {
                let cut = area.height.saturating_sub(area.clipped_top + area.area.height);
                let below = pane.bottom().saturating_sub(area.area.bottom());
                let rows = (cut + super::compose_at::ROOM).saturating_sub(below);
                let wheel = MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: pane.x + 1,
                    row: pane.y + 1,
                    modifiers: KeyModifiers::NONE,
                };
                // The wheel scrolls three rows a step.
                for _ in 0..rows.div_ceil(3) {
                    self.diff.handle_event(Event::Mouse(wheel))?;
                }
            }
        }
        self.compose = Compose::default();
        self.mode = Mode::Compose(Draft::Ask(lines));
        Ok(())
    }

    /// A key on an Ask box: Enter or `ask` asks again about its lines, the panel's remove key
    /// removes the box. True when the key was the box's.
    pub(super) fn ask_box_key(&mut self, key: &KeyEvent) -> bool {
        let diff = self.keys.action(key, keys::DIFF);
        let remove = diff.is_none() && self.keys.is(Action::Remove, key);
        if let Some(thread) = self.cursor_thread() {
            let (id, lines) = (thread.id, thread.lines.clone());
            if matches!(diff, Some(Action::Confirm | Action::Ask)) {
                if let Err(error) = self.open_question(lines) {
                    self.status = Some(format!("{error:#}"));
                }
            } else if remove {
                self.remove_thread(id);
            } else {
                return false;
            }
            return true;
        }
        // On the lines a box is about, where the click that focused the pane may have put the
        // cursor, the remove key removes that box too.
        let on_lines = self.diff.cursor().ok().flatten().filter(|_| remove && self.whole.is_none());
        match on_lines.and_then(|lines| self.ask.threads.iter().find(|thread| covers(&thread.lines, &lines)))
        {
            Some(thread) => {
                self.remove_thread(thread.id);
                true
            }
            None => false,
        }
    }

    fn remove_thread(&mut self, id: u64) {
        self.ask.threads.retain(|thread| thread.id != id);
        self.status = Some("questions removed from the diff; the agent's chat keeps them".into());
    }

    /// What an Ask box's border says it does, each with its key as the keyboard shows it:
    /// `ask again (⏎ Enter) · remove (⌫ Delete)`. Backspace is the Mac's delete key.
    pub(super) fn ask_box_hint(&self) -> String {
        let bound = |action, code| self.keys.is(action, &KeyEvent::from(code));
        let again = if bound(Action::Confirm, KeyCode::Enter) {
            "\u{23ce} Enter".to_owned()
        } else {
            self.keys.hint(Action::Ask)
        };
        let remove = if bound(Action::Remove, KeyCode::Backspace) {
            "\u{232b} Delete".to_owned()
        } else if bound(Action::Remove, KeyCode::Delete) {
            "\u{2326} Delete".to_owned()
        } else {
            self.keys.hint_besides(Action::Remove, keys::DIFF)
        };
        [("ask again", again), ("remove", remove)]
            .into_iter()
            .filter(|(_, key)| !key.is_empty())
            .map(|(what, key)| format!("{what} ({key})"))
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
    }

    /// The thread whose box the cursor is on, in the diff (not the whole file).
    fn cursor_thread(&self) -> Option<&Thread> {
        if self.whole.is_some() {
            return None;
        }
        self.ask.thread(self.diff.cursor_box()?)
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
        let agent = self.delivery.agent_host().unwrap_or("agent").to_owned();
        let turn =
            Turn { question, agent, prompt, sent: false, answer: String::new(), failed: None, watch: None };
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
        let pid = self.delivery.agent_pid();
        let found = self
            .ask
            .found
            .as_ref()
            .filter(|found| found.host == host && found.pid == pid)
            .map(|found| found.transcript.clone());
        turn.watch = Some(watch::Watch::start(watch::Source { found, host, pid }, self.wake.clone()));
        self.ask.add(lines, turn);
        self.status = Some(format!("asking {}\u{2026}", self.delivery.describe()));
        let _ = self.diff.clear_selection();
    }

    /// Read answers from `transcript` instead of finding the agent's: tests.
    #[cfg(test)]
    pub(super) fn answer_from(&mut self, transcript: Transcript) {
        let host = self.delivery.agent_host().map(str::to_owned);
        self.ask.found = Some(Found { host, pid: self.delivery.agent_pid(), transcript });
    }
}

/// Whether a box on `on` is about `lines` too: the same file and side, and `lines` inside.
fn covers(on: &Lines, lines: &Lines) -> bool {
    on.path == lines.path && on.side == lines.side && on.first <= lines.first && lines.last <= on.last
}

/// The id of the newest prompt in `newest_first`, if it has one.
fn newest_prompt(newest_first: &[Message]) -> Option<String> {
    newest_first.iter().find(|message| message.role == Role::Human).map(|message| message.id.clone())
}

/// The answer to the first prompt after prompt `after` (with `None`, the first prompt of all):
/// the replies after it up to the next prompt, oldest first, and whether that next prompt has
/// come, which ends the answer. `None` until the question is in the transcript, or when `after`
/// is out of reach.
fn answer(newest_first: &[Message], after: Option<&str>) -> Option<(String, bool)> {
    let newer = match after {
        Some(id) => newest_first.get(..newest_first.iter().position(|message| message.id == id)?)?,
        None => newest_first,
    };
    // Newest first, so the oldest prompt in `newer` is the question.
    let asked = newer.iter().rposition(|message| message.role == Role::Human)?;
    let since = newer.get(..asked)?;
    let next = since.iter().rposition(|message| message.role == Role::Human);
    let replies = match next {
        Some(next) => since.get(next + 1..)?,
        None => since,
    };
    let text: Vec<&str> = replies
        .iter()
        .rev()
        .filter(|message| message.role == Role::Assistant)
        .map(|message| message.text.as_str())
        .collect();
    Some((text.join("\n\n"), next.is_some()))
}

#[cfg(test)]
mod tests;
