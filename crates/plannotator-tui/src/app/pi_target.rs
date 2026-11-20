//! The app's half of the Pi link (`docs/spec-pi-link.md`): the agent picker that makes a
//! running Pi session, Herdr agent or Rex agent the Send target, and notes a Pi session
//! leaves on the open document.

use std::path::{Path, PathBuf};

use plannotator_tui_schema::{Kind, Provenance};
use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use serde_json::{Value, json};

use super::{App, Mode};
use crate::pi;
use crate::pi::link::{self, Link, Note};
pub(super) use crate::ui::agent_picker::AgentPicker;
use crate::ui::agent_picker::{self, Picked, PickerKey, Reach};

#[cfg(all(test, unix))]
mod blocked_tests;
#[cfg(all(test, unix))]
mod first_look_tests;
#[cfg(all(test, unix))]
mod rex_tests;
#[cfg(test)]
mod tests;

impl App {
    /// List this review in the registry and listen for notes, and start looking for the
    /// agents Send can reach. A review that cannot list itself (no writable data folder)
    /// works as before, without the link.
    pub(crate) fn start_pi_link(&mut self) {
        let cwd = agent_picker::real(&self.review_folder());
        self.pi_link = Link::start(&pi::root(), cwd.clone(), self.pi_files()).ok();
        // The document loop cannot be woken; it checks often while the look runs.
        self.reach = Reach::start(cwd, None);
    }

    /// The folder this review is about: the tree's root, the file's folder, or the cwd.
    pub(super) fn review_folder(&self) -> PathBuf {
        if let Some(tree) = &self.tree {
            return tree.root().to_path_buf();
        }
        match &self.open.source.provenance {
            Provenance::File { path } => path.parent().map(Path::to_path_buf),
            _ => None,
        }
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
    }

    /// The documents a Pi note can go on: the open file, when it is one.
    fn pi_files(&self) -> Vec<PathBuf> {
        self.open_file().into_iter().collect()
    }

    fn open_file(&self) -> Option<PathBuf> {
        match &self.open.source.provenance {
            Provenance::File { path } if !self.open.source.transient => {
                Some(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            }
            _ => None,
        }
    }

    /// Take the notes that arrived since the last poll. True when the screen changed.
    pub(crate) fn poll_pi(&mut self) -> bool {
        let Some(link) = &self.pi_link else { return false };
        link.set_files(self.pi_files());
        let notes = link.take_notes();
        let changed = !notes.is_empty();
        for note in notes {
            let reply = self.take_pi_note(&note);
            note.answer(reply);
        }
        changed
    }

    fn take_pi_note(&mut self, note: &Note) -> Value {
        if matches!(self.mode, Mode::Compose | Mode::Edit(_) | Mode::AttachImage(_)) {
            return link::error("busy", "the person is typing a note; try again shortly");
        }
        let wanted = std::fs::canonicalize(&note.file).unwrap_or_else(|_| note.file.clone());
        if self.open_file().as_deref() != Some(wanted.as_path()) {
            return link::error("not_open", "that file is not the document open in plannotator-tui");
        }
        let source = &self.open.doc.source;
        let range = match (&note.quote, note.line) {
            (Some(quote), line) => {
                let starts = source.match_indices(quote.as_str()).map(|(start, _)| start);
                let start = match line.and_then(|line| line_start(source, line)) {
                    Some(at) => starts.min_by_key(|&start| start.abs_diff(at)),
                    None => starts.into_iter().next(),
                };
                match start {
                    Some(start) => start..start + quote.len(),
                    None => {
                        return link::error(
                            "quote_not_found",
                            "the quote is not in the file; copy the text exactly as it appears in the source",
                        );
                    }
                }
            }
            (None, line) => {
                let at = line.and_then(|line| line_start(source, line)).unwrap_or(0);
                let block = self
                    .open
                    .doc
                    .block_containing(at)
                    .or_else(|| (!self.open.doc.blocks.is_empty()).then_some(0));
                match block.and_then(|block| self.open.doc.blocks.get(block)) {
                    Some(block) => block.range.clone(),
                    None => return link::error("quote_not_found", "the document is empty"),
                }
            }
        };
        let added = self.annotate_by(range, note);
        match added {
            Ok(()) => {
                self.status = Some(format!("note from {}", note.author));
                json!({"ok": true})
            }
            Err(err) => link::error("failed", &format!("{err:#}")),
        }
    }

    fn annotate_by(&mut self, range: std::ops::Range<usize>, note: &Note) -> anyhow::Result<()> {
        self.annotate(range, Kind::Comment, note.body.clone())?;
        self.open.store.credit_last("pi", &note.author)
    }

    /// Open the agent picker on the agents within reach, the ones in this review's folder first.
    pub(super) fn open_pi_picker(&mut self) {
        self.agent_picker = AgentPicker::open(&self.reach, &agent_picker::real(&self.review_folder()));
        self.pick_then = None;
        self.mode = Mode::PickTarget;
    }

    #[cfg(test)]
    fn pi_rows(&self) -> usize {
        self.agent_picker.rows(self.start_delivery.is_some())
    }

    pub(super) fn pick_target_key(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        match self.agent_picker.key(&self.keys, &key, self.start_delivery.is_some()) {
            PickerKey::Stay => Ok(()),
            PickerKey::Close => {
                self.mode = Mode::Browse;
                if self.pick_then.take().is_some() {
                    self.status = Some("nothing sent".into());
                }
                Ok(())
            }
            PickerKey::Chose(row) => self.choose_target(row),
        }
    }

    /// Act on the picker's `row`, then send when Send opened the picker.
    fn choose_target(&mut self, row: usize) -> anyhow::Result<()> {
        self.mode = Mode::Browse;
        let then = self.pick_then.take();
        match self.agent_picker.choose(row, &mut self.delivery, &mut self.start_delivery) {
            Picked::Moved(status) => self.status = Some(status),
            Picked::Copy => {}
            Picked::Nothing => return Ok(()),
        }
        match then {
            Some(then) => self.send_picked(then),
            None => Ok(()),
        }
    }

    pub(super) fn draw_pi_picker(&mut self, frame: &mut Frame) {
        let area = self.viewport(frame.area());
        let folder = agent_picker::real(&self.review_folder());
        self.agent_picker.draw(frame, area, &self.keys, &folder, self.start_delivery.as_deref());
    }
}

/// The byte offset where 1-based `line` starts, or `None` past the end.
fn line_start(source: &str, line: usize) -> Option<usize> {
    if line <= 1 {
        return Some(0);
    }
    source.match_indices('\n').nth(line - 2).map(|(at, _)| at + 1)
}
