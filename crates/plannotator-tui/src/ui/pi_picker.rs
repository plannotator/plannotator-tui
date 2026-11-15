//! The Pi picker (`docs/spec-pi-link.md`): choose which running Pi session Send goes to.
//! Document and code reviews open the same picker; each keeps its own Send target and
//! hands it here to be swapped.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use crate::delivery::Delivery;
use crate::keys::{self, Action, Keymap};
use crate::pi::target::PiSession;
use crate::pi::{self, PiEntry};
use crate::theme::palette;
use crate::ui::hints;

/// The picker's state while it is open.
#[derive(Debug, Default)]
pub(crate) struct PiPicker {
    pub(crate) sessions: Vec<PiEntry>,
    pub(crate) cursor: usize,
}

/// What a key did in the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerKey {
    Stay,
    Close,
    /// The row under the cursor was confirmed.
    Chose(usize),
}

impl PiPicker {
    /// The live sessions, the ones in `folder` (a real path) first, then the newest.
    pub(crate) fn open(folder: &Path) -> Self {
        let mut sessions = pi::live_pi_sessions(&pi::root());
        // Pi records its cwd as it was given (on macOS `/var/…` for `/private/var/…`).
        sessions.sort_by_key(|s| {
            let cwd = std::fs::canonicalize(&s.cwd).unwrap_or_else(|_| s.cwd.clone());
            (!cwd.starts_with(folder) && !folder.starts_with(&cwd), std::cmp::Reverse(s.updated))
        });
        Self { sessions, cursor: 0 }
    }

    /// Rows: each live session, then the target the review started with when Send has
    /// moved off it.
    pub(crate) fn rows(&self, has_start: bool) -> usize {
        self.sessions.len() + usize::from(has_start)
    }

    pub(crate) fn key(&mut self, keymap: &Keymap, key: &KeyEvent, has_start: bool) -> PickerKey {
        match keymap.action(key, keys::PICK_TARGET) {
            Some(Action::Down) => {
                self.cursor = (self.cursor + 1).min(self.rows(has_start).saturating_sub(1));
                PickerKey::Stay
            }
            Some(Action::Up) => {
                self.cursor = self.cursor.saturating_sub(1);
                PickerKey::Stay
            }
            Some(Action::Confirm) => PickerKey::Chose(self.cursor),
            Some(Action::Back | Action::PickTarget | Action::Interrupt) => PickerKey::Close,
            _ => PickerKey::Stay,
        }
    }

    /// Make `row` the Send target: a session, or the way back to where the review started.
    /// Returns the status to show, or `None` when nothing changed.
    pub(crate) fn choose(
        &self,
        row: usize,
        delivery: &mut Box<dyn Delivery>,
        start: &mut Option<Box<dyn Delivery>>,
    ) -> Option<String> {
        if let Some(entry) = self.sessions.get(row).cloned() {
            let session: Box<dyn Delivery> = Box::new(PiSession::new(entry));
            let previous = std::mem::replace(delivery, session);
            // Keep the first target only: picking Pi after Pi drops the earlier Pi.
            start.get_or_insert(previous);
        } else {
            *delivery = start.take()?;
        }
        Some(format!("Send now goes to {}", delivery.describe()))
    }

    /// Draw the picker centred in `area`. `folder` shortens the sessions' folders.
    pub(crate) fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        keymap: &Keymap,
        folder: &Path,
        start: Option<&dyn Delivery>,
    ) {
        let mut rows: Vec<String> = self
            .sessions
            .iter()
            .map(|s| {
                let state = if s.busy { "busy" } else { "idle" };
                format!(" {}  {}  {state}  pid {} ", s.label(), short_path(&s.cwd, folder), s.pid)
            })
            .collect();
        if let Some(start) = start {
            rows.push(format!(" back to {} ", start.describe()));
        }
        let empty = rows.is_empty();
        if empty {
            rows.push(
                " No Pi sessions found. Run `plannotator-tui pi install`, then restart Pi. ".to_owned(),
            );
        }
        let help = hints::picker(
            "move",
            [keymap.labelled(Action::Confirm, "send here"), keymap.labelled(Action::Back, "close")],
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
                } else if index == self.cursor {
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

/// `folder` as a real path, for comparing with the folders Pi sessions report.
pub(crate) fn real(folder: &Path) -> PathBuf {
    std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
}
