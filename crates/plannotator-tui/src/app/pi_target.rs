//! The app's half of the Pi link (`docs/spec-pi-link.md`): the picker that makes a running
//! Pi session the Send target, and notes a Pi session leaves on the open document.

use std::path::{Path, PathBuf};

use plannotator_tui_schema::{Kind, Provenance};
use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr as _;

use super::draw::picker_help;
use super::{App, Mode};
use crate::keys::{self, Action};
use crate::pi::link::{self, Link, Note};
use crate::pi::target::PiSession;
use crate::pi::{self, PiEntry};
use crate::theme::palette;

#[cfg(test)]
mod tests;

/// The picker's state while it is open.
#[derive(Debug, Default)]
pub(super) struct PiPicker {
    sessions: Vec<PiEntry>,
    cursor: usize,
}

impl App {
    /// List this review in the registry and listen for notes. A review that cannot (no
    /// writable data folder) works as before, without the link.
    pub(crate) fn start_pi_link(&mut self) {
        let cwd = std::fs::canonicalize(self.review_folder()).unwrap_or_else(|_| self.review_folder());
        self.pi_link = Link::start(&pi::root(), cwd, self.pi_files()).ok();
    }

    /// The folder this review is about: the tree's root, the file's folder, or the cwd.
    fn review_folder(&self) -> PathBuf {
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

    /// Open the Pi picker on the live sessions, the ones in this review's folder first.
    pub(super) fn open_pi_picker(&mut self) {
        let folder = std::fs::canonicalize(self.review_folder()).unwrap_or_else(|_| self.review_folder());
        let mut sessions = pi::live_pi_sessions(&pi::root());
        // Pi records its cwd as it was given (on macOS `/var/…` for `/private/var/…`).
        sessions.sort_by_key(|s| {
            let cwd = std::fs::canonicalize(&s.cwd).unwrap_or_else(|_| s.cwd.clone());
            (!cwd.starts_with(&folder) && !folder.starts_with(&cwd), std::cmp::Reverse(s.updated))
        });
        self.pi_picker = PiPicker { sessions, cursor: 0 };
        self.mode = Mode::PickTarget;
    }

    /// Rows: each live session, then the target the review started with when Send has
    /// moved off it.
    fn pi_rows(&self) -> usize {
        self.pi_picker.sessions.len() + usize::from(self.start_delivery.is_some())
    }

    pub(super) fn pick_target_key(&mut self, key: KeyEvent) {
        match self.keys.action(&key, keys::PICK_TARGET) {
            Some(Action::Down) => {
                self.pi_picker.cursor = (self.pi_picker.cursor + 1).min(self.pi_rows().saturating_sub(1));
            }
            Some(Action::Up) => self.pi_picker.cursor = self.pi_picker.cursor.saturating_sub(1),
            Some(Action::Confirm) => self.choose_target(self.pi_picker.cursor),
            Some(Action::Back | Action::PickTarget | Action::Interrupt) => self.mode = Mode::Browse,
            _ => {}
        }
    }

    fn choose_target(&mut self, row: usize) {
        self.mode = Mode::Browse;
        if let Some(entry) = self.pi_picker.sessions.get(row).cloned() {
            let session: Box<dyn crate::delivery::Delivery> = Box::new(PiSession::new(entry));
            let previous = std::mem::replace(&mut self.delivery, session);
            // Keep the first target only: picking Pi after Pi drops the earlier Pi.
            self.start_delivery.get_or_insert(previous);
        } else if let Some(start) = self.start_delivery.take() {
            self.delivery = start;
        } else {
            return;
        }
        self.status = Some(format!("Send now goes to {}", self.delivery.describe()));
    }

    pub(super) fn draw_pi_picker(&mut self, frame: &mut Frame) {
        let area = self.viewport(frame.area());
        let folder = std::fs::canonicalize(self.review_folder()).unwrap_or_else(|_| self.review_folder());
        let mut rows: Vec<String> = self
            .pi_picker
            .sessions
            .iter()
            .map(|s| {
                let state = if s.busy { "busy" } else { "idle" };
                format!(" {}  {}  {state}  pid {} ", s.label(), short_path(&s.cwd, &folder), s.pid)
            })
            .collect();
        if let Some(start) = &self.start_delivery {
            rows.push(format!(" back to {} ", start.describe()));
        }
        let empty = rows.is_empty();
        if empty {
            rows.push(
                " No Pi sessions found. Run `plannotator-tui pi install`, then restart Pi. ".to_owned(),
            );
        }
        let help = picker_help(
            "move",
            [self.keys.labelled(Action::Confirm, "send here"), self.keys.labelled(Action::Back, "close")],
        );
        let content = rows.iter().map(|row| row.width()).max().unwrap_or(0).max(help.width());
        let width = (content as u16 + 2).min(area.width);
        let height = (rows.len() as u16 + 2).min(area.height);
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
            .title(Span::styled(" send to which Pi? ", Style::new().dim()))
            .title_bottom(Span::styled(help, Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .enumerate()
            .take(usize::from(inner.height))
            .map(|(index, text)| {
                let style = if empty {
                    Style::new().dim()
                } else if index == self.pi_picker.cursor {
                    palette().selection
                } else {
                    Style::new()
                };
                Line::from(Span::styled(format!("{text:<pad$}", pad = usize::from(inner.width)), style))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// The byte offset where 1-based `line` starts, or `None` past the end.
fn line_start(source: &str, line: usize) -> Option<usize> {
    if line <= 1 {
        return Some(0);
    }
    source.match_indices('\n').nth(line - 2).map(|(at, _)| at + 1)
}

/// `path` relative to `folder` when it is inside it, else under `~`, else as is.
fn short_path(path: &Path, folder: &Path) -> String {
    if path == folder {
        return ".".to_owned();
    }
    let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if path == folder {
        return ".".to_owned();
    }
    if let Ok(rest) = path.strip_prefix(folder) {
        return format!("./{}", rest.display());
    }
    match std::env::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
