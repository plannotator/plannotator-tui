//! Go to definition: `d` on a line of the diff or the whole-file view searches the repository
//! for the identifier on it (`plannotator_tui_review::codenav`) and goes there; `ctrl+o` comes
//! back and `ctrl+i` goes forward again.
//!
//! The line's identifiers come left to right; with more than one, a small list asks which.
//! The search runs on a thread of its own, with the core's time limit, on the side the cursor
//! is on: the diff's new side searches the reviewed version, its old side and the base view
//! search the base. One strong answer (a single definition elsewhere, or with no definition a
//! single use) is jumped to; several are listed in the core's order. A place the diff shows
//! opens in the diff; any other opens in the whole-file view, on the side that was searched.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use plannotator_tui_review::Side;
use plannotator_tui_review::codenav::{self, Candidate, Found, Kind, Language, Query};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize as _};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use super::whole::Version;
use super::{Mode, Screen};
use crate::keys::{self, Action};
use crate::theme::palette;
use crate::ui::hints;

/// Places kept to go back to (and forward again).
pub(super) const HISTORY: usize = 30;
/// Rows the list of answers shows at once.
const LIST_ROWS: usize = 12;

/// Where the person was: a line of a file, in the diff or the whole-file view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Place {
    /// The capture's path of the file (its new path), or the searched path for a file the
    /// diff does not have.
    pub(super) path: String,
    pub(super) whole: bool,
    pub(super) side: Side,
    pub(super) line: u32,
}

/// What `d` was pressed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Ask {
    pub(super) from: Place,
    /// The file's path on the searched side (the old path of a renamed file on the base).
    file: String,
    words: Vec<String>,
}

/// The list open over the review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pick {
    /// Which identifier of the line.
    Word { ask: Ask, cursor: usize },
    /// Which of the answers.
    Answer { word: String, side: Side, from: Place, answers: Vec<Candidate>, cursor: usize },
}

/// A search on its thread.
struct Searching {
    word: String,
    side: Side,
    from: Place,
    file: String,
    answer: Receiver<Result<Found, String>>,
}

/// Go-to-definition state: the search under way and the places to go back and forward to.
#[derive(Default)]
pub(super) struct Nav {
    searching: Option<Searching>,
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl Nav {
    /// `searching… word`, for the footer.
    pub(super) fn notice(&self) -> Option<String> {
        self.searching.as_ref().map(|s| format!("searching for {}\u{2026}", s.word))
    }
}

impl Screen {
    /// Where the cursor is now, as a place to come back to.
    fn here(&self) -> anyhow::Result<Option<Place>> {
        if let Some(whole) = &self.whole {
            let line = whole.target()?.map_or(1, |(first, _)| first);
            return Ok(Some(Place {
                path: whole.path.clone(),
                whole: true,
                side: whole.version.side(),
                line,
            }));
        }
        let at = match self.diff.selection() {
            Some(lines) => Some(lines),
            None => self.diff.cursor()?,
        };
        Ok(at.map(|lines| Place { path: lines.path, whole: false, side: lines.side, line: lines.first }))
    }

    /// The capture's file a searched path names on `side`.
    fn capture_path(&self, path: &str, side: Side) -> Option<String> {
        self.capture
            .files
            .iter()
            .find(|f| match side {
                Side::New => f.path == path,
                Side::Old => f.old_path.as_deref().unwrap_or(&f.path) == path,
            })
            .map(|f| f.path.clone())
    }

    /// `d`: the identifiers on the cursor's line; one is searched for at once, several are
    /// offered in a list.
    pub(super) fn go_to_definition(&mut self) -> anyhow::Result<()> {
        let Some(from) = self.here()? else {
            self.status = Some("move the cursor onto a line first".into());
            return Ok(());
        };
        let file = match (self.file(&from.path), from.side) {
            (Some(patch), Side::Old) => patch.old_path.clone().unwrap_or_else(|| from.path.clone()),
            _ => from.path.clone(),
        };
        let text = match &self.whole {
            Some(whole) => whole.code(from.line, from.line),
            None => self.file(&from.path).and_then(|f| f.line(from.side, from.line)).map(str::to_owned),
        };
        let Some(text) = text else {
            self.status = Some("move the cursor onto a line first".into());
            return Ok(());
        };
        // A set beside the list: a long line from anyone's diff can hold many thousand names.
        let mut seen = std::collections::HashSet::new();
        let words: Vec<String> = codenav::identifiers_on_line(&text, Language::from_path(&file))
            .into_iter()
            .filter(|ident| seen.insert(ident.text))
            .map(|ident| ident.text.to_owned())
            .collect();
        match words.as_slice() {
            [] => self.status = Some("no identifier on this line".into()),
            [word] => {
                let word = word.clone();
                self.search(&word, from, file);
            }
            _ => self.mode = Mode::Pick(Pick::Word { ask: Ask { from, file, words }, cursor: 0 }),
        }
        Ok(())
    }

    /// Start searching for `word` from `from`; a search already running is dropped.
    fn search(&mut self, word: &str, from: Place, file: String) {
        let (tell, answer) = mpsc::sync_channel(1);
        let root = self.capture.root.clone();
        let source = self.capture.source(from.side).clone();
        let changed = codenav::changed_paths(&self.capture.files);
        let wake = self.wake.clone();
        let (thread_word, thread_file, line) = (word.to_owned(), file.clone(), from.line);
        let spawned = std::thread::Builder::new().name("codenav".into()).spawn(move || {
            let query = Query { file: &thread_file, line, word: &thread_word, changed: &changed };
            let found = codenav::search(&root, &source, &query).map_err(|err| format!("{err:#}"));
            let _ = tell.send(found);
            let _ = wake.try_send(());
        });
        if let Err(err) = spawned {
            self.status = Some(format!("could not search: {err}"));
            return;
        }
        self.status = None;
        let side = from.side;
        self.nav.searching = Some(Searching { word: word.to_owned(), side, from, file, answer });
    }

    /// Take a finished search's answer. Returns whether the screen changed.
    pub(super) fn poll_search(&mut self) -> anyhow::Result<bool> {
        let Some(searching) = &self.nav.searching else { return Ok(false) };
        let found = match searching.answer.try_recv() {
            Ok(found) => found,
            Err(TryRecvError::Empty) => return Ok(false),
            Err(TryRecvError::Disconnected) => Err("the search stopped".to_owned()),
        };
        let Some(Searching { word, side, from, file, .. }) = self.nav.searching.take() else {
            return Ok(false);
        };
        let found = match found {
            Ok(found) => found,
            Err(err) => {
                self.status = Some(format!("could not search for {word}: {err}"));
                return Ok(true);
            }
        };
        let query = Query { file: &file, line: from.line, word: &word, changed: &[] };
        let elsewhere = |c: &&Candidate| !(c.path == file && c.line == from.line);
        let definitions: Vec<&Candidate> = found.definitions.iter().filter(elsewhere).collect();
        let answers: Vec<Candidate> = found.candidates().filter(elsewhere).cloned().collect();
        let strong = definitions.len() == 1 || (definitions.is_empty() && answers.len() == 1);
        let base = if side == Side::Old { " in the base" } else { "" };
        if answers.is_empty() {
            let stopped = if found.complete { "" } else { " (the search stopped early)" };
            self.status = Some(format!("no definition or use of {word} found{base}{stopped}"));
        } else if strong && let Some(best) = found.best(&query).cloned() {
            self.jump(&best, side, from)?;
        } else if self.mode == Mode::Browse {
            self.mode = Mode::Pick(Pick::Answer { word, side, from, answers, cursor: 0 });
        } else {
            self.status = Some(format!("{} places for {word}{base}: d again to list them", answers.len()));
        }
        Ok(true)
    }

    /// Go to `to` on `side`, remembering `from` to come back to.
    fn jump(&mut self, to: &Candidate, side: Side, from: Place) -> anyhow::Result<()> {
        let path = self.capture_path(&to.path, side);
        let in_diff =
            path.as_deref().and_then(|p| self.file(p)).is_some_and(|f| f.shows(side, to.line, to.line));
        let place =
            Place { path: path.unwrap_or_else(|| to.path.clone()), whole: !in_diff, side, line: to.line };
        remember(&mut self.nav.back, from);
        self.nav.forward.clear();
        self.go(&place)?;
        let kind = match to.kind {
            Kind::Definition => "definition",
            Kind::Reference => "use",
        };
        let back =
            self.keys.labelled(Action::JumpBack, "back").map(|k| format!(" \u{b7} {k}")).unwrap_or_default();
        self.status = Some(format!("{kind}: {}:{}{back}", to.path, to.line));
        Ok(())
    }

    /// Show `place`: a line of the diff, or of the whole file on its side.
    fn go(&mut self, place: &Place) -> anyhow::Result<()> {
        if place.whole {
            self.show_whole(&place.path, Version::of(place.side), Some(place.line));
            return Ok(());
        }
        self.close_whole()?;
        if let Err(err) = self.diff.reveal(&place.path, place.side, place.line) {
            self.status = Some(format!("{err:#}"));
        }
        Ok(())
    }

    /// `ctrl+o` (`forward: false`) and `ctrl+i`: the place before, or after, in the history.
    pub(super) fn jump_history(&mut self, forward: bool) -> anyhow::Result<()> {
        let here = self.here()?;
        let nav = &mut self.nav;
        let (from, to) =
            if forward { (&mut nav.forward, &mut nav.back) } else { (&mut nav.back, &mut nav.forward) };
        let Some(place) = from.pop() else {
            self.status = Some(if forward { "no later place" } else { "no earlier place" }.into());
            return Ok(());
        };
        if let Some(here) = here {
            remember(to, here);
        }
        self.go(&place)?;
        self.status = Some(format!("{}:{}", place.path, place.line));
        Ok(())
    }

    pub(super) fn pick_key(&mut self, key: KeyEvent, pick: Pick) -> anyhow::Result<()> {
        let count = match &pick {
            Pick::Word { ask, .. } => ask.words.len(),
            Pick::Answer { answers, .. } => answers.len(),
        };
        let cursor = match &pick {
            Pick::Word { cursor, .. } | Pick::Answer { cursor, .. } => *cursor,
        };
        // 1-9 picks a word at once.
        let digit = match key.code {
            KeyCode::Char(c @ '1'..='9') if matches!(pick, Pick::Word { .. }) => {
                Some(c as usize - '1' as usize).filter(|&i| i < count)
            }
            _ => None,
        };
        let last = count.saturating_sub(1);
        let to = match (digit, self.keys.action(&key, keys::PICK)) {
            (Some(index), _) => Err(index),
            (None, Some(Action::Confirm)) => Err(cursor),
            (None, Some(Action::Back | Action::Quit)) => {
                self.mode = Mode::Browse;
                return Ok(());
            }
            (None, Some(Action::Down)) => Ok((cursor + 1).min(last)),
            (None, Some(Action::Up)) => Ok(cursor.saturating_sub(1)),
            (None, Some(Action::PageDown)) => Ok((cursor + LIST_ROWS).min(last)),
            (None, Some(Action::PageUp)) => Ok(cursor.saturating_sub(LIST_ROWS)),
            (None, Some(Action::Top)) => Ok(0),
            (None, Some(Action::Bottom)) => Ok(last),
            _ => return Ok(()),
        };
        // `Ok` moves the highlight; `Err` is the row chosen.
        let index = match to {
            Ok(to) => {
                self.set_pick_cursor(pick, to);
                return Ok(());
            }
            Err(index) => index,
        };
        self.mode = Mode::Browse;
        match pick {
            Pick::Word { ask, .. } => {
                if let Some(word) = ask.words.get(index) {
                    let word = word.clone();
                    self.search(&word, ask.from, ask.file);
                }
            }
            Pick::Answer { side, from, answers, .. } => {
                if let Some(to) = answers.get(index) {
                    let to = to.clone();
                    self.jump(&to, side, from)?;
                }
            }
        }
        Ok(())
    }

    fn set_pick_cursor(&mut self, mut pick: Pick, to: usize) {
        match &mut pick {
            Pick::Word { cursor, .. } | Pick::Answer { cursor, .. } => *cursor = to,
        }
        self.mode = Mode::Pick(pick);
    }

    pub(super) fn draw_pick(&self, frame: &mut Frame, area: Rect, pick: &Pick) {
        let (title, rows, cursor): (String, Vec<Line<'static>>, usize) = match pick {
            Pick::Word { ask, cursor } => {
                let rows = ask
                    .words
                    .iter()
                    .enumerate()
                    .map(|(i, word)| {
                        let number = if i < 9 { format!("{}", i + 1) } else { " ".to_owned() };
                        Line::from(vec![
                            Span::raw(format!(" {number}  ")).dim(),
                            Span::raw(format!("{word} ")),
                        ])
                    })
                    .collect();
                (" go to the definition of ".to_owned(), rows, *cursor)
            }
            Pick::Answer { word, side, answers, cursor, .. } => {
                let base = if *side == Side::Old { " in the base" } else { "" };
                let places = |n: usize| if n == 1 { "1 place".to_owned() } else { format!("{n} places") };
                let title = format!(" {word}: {}{base} ", places(answers.len()));
                let at_width =
                    answers.iter().map(|a| format!("{}:{}", a.path, a.line).width()).max().unwrap_or(0);
                let rows = answers
                    .iter()
                    .map(|a| {
                        let at = format!("{}:{}", a.path, a.line);
                        let (kind, colour) = match a.kind {
                            Kind::Definition => ("definition", palette().accent),
                            Kind::Reference => ("use       ", Color::DarkGray),
                        };
                        Line::from(vec![
                            Span::raw(format!(" {at:<at_width$}  ")),
                            Span::styled(kind, Style::new().fg(colour)),
                            Span::raw(format!("  {} ", a.text.trim())).dim(),
                        ])
                    })
                    .collect();
                (title, rows, *cursor)
            }
        };
        let help = hints::picker(
            "move",
            [self.keys.labelled(Action::Confirm, "go"), self.keys.labelled(Action::Back, "close")],
        );
        let widest = rows.iter().map(Line::width).max().unwrap_or(0).max(help.width()).max(title.width());
        let width = (widest as u16 + 2).min(area.width.saturating_sub(4)).max(20.min(area.width));
        let shown = rows.len().min(LIST_ROWS);
        let height = (shown as u16 + 2).min(area.height);
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(title, Style::new().dim()))
            .title_bottom(Span::styled(help, Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let top = cursor.saturating_sub(shown.saturating_sub(1));
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .enumerate()
            .skip(top)
            .take(usize::from(inner.height))
            .map(|(index, line)| if index == cursor { line.style(palette().selection) } else { line })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// Add `place` to a history, unless it is the last one there; the oldest goes past [`HISTORY`].
fn remember(history: &mut Vec<Place>, place: Place) {
    if history.last() != Some(&place) {
        history.push(place);
    }
    if history.len() > HISTORY {
        history.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(line: u32) -> Place {
        Place { path: "a.rs".into(), whole: false, side: Side::New, line }
    }

    #[test]
    fn the_history_skips_repeats_and_keeps_the_latest_places() {
        let mut history = Vec::new();
        remember(&mut history, place(1));
        remember(&mut history, place(1));
        assert_eq!(history, [place(1)]);
        for line in 2..=40 {
            remember(&mut history, place(line));
        }
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history.first(), Some(&place(11)));
        assert_eq!(history.last(), Some(&place(40)));
    }
}
