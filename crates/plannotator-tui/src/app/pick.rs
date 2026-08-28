//! The message picker: which of the agent's recent messages to review. Newest first, the
//! newest already open behind it.

use anyhow::Result;
use plannotator_tui_hosts::Message;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use super::{App, Mode, Open};
use crate::last::message_source;

const PICK_MAX_WIDTH: u16 = 90;

impl App {
    /// Open with `messages` (newest first) as candidates; the picker shows when there is a
    /// choice to make.
    pub(crate) fn open_message(
        host: &str,
        transcript: &str,
        messages: Vec<Message>,
        width: usize,
        delivery: Box<dyn crate::delivery::Delivery>,
    ) -> Result<Self> {
        let Some(newest) = messages.first() else { anyhow::bail!("no message to open") };
        let mut app = Self::open(message_source(host, transcript, newest), width, delivery)?;
        host.clone_into(&mut app.message_host);
        transcript.clone_into(&mut app.message_transcript);
        app.candidates = messages;
        if app.candidates.len() > 1 {
            app.mode = Mode::Pick;
        }
        Ok(app)
    }

    /// Swap the open document for candidate `index`.
    fn open_candidate(&mut self, index: usize) -> Result<()> {
        let Some(message) = self.candidates.get(index) else { return Ok(()) };
        let source = message_source(&self.message_host, &self.message_transcript, message);
        self.open = Open::new(source, self.open.layout.width, &self.data_dir, &self.project)?;
        self.scroll = 0;
        self.selected = 0;
        self.cursor = (0, 0);
        self.rail_cursor = 0;
        self.clear_selection();
        self.derive_send_state();
        self.mode = Mode::Browse;
        self.status = Some(format!("message {} of {}", index + 1, self.candidates.len()));
        Ok(())
    }

    pub(super) fn reopen_picker(&mut self) {
        if self.candidates.len() > 1 {
            self.mode = Mode::Pick;
        }
    }

    pub(super) fn pick_key(&mut self, key: KeyEvent) -> Result<()> {
        let last = self.candidates.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.pick_cursor = (self.pick_cursor + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.pick_cursor = self.pick_cursor.saturating_sub(1),
            KeyCode::Enter => return self.open_candidate(self.pick_cursor),
            KeyCode::Esc => self.mode = Mode::Browse,
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
        Ok(())
    }

    pub(super) fn pick_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(());
        }
        let hit =
            self.geometry.pick_rows.iter().find(|(rect, _)| {
                mouse.row == rect.y && mouse.column >= rect.x && mouse.column < rect.right()
            });
        match hit.map(|(_, index)| *index) {
            Some(index) => self.open_candidate(index),
            None => Ok(()),
        }
    }

    pub(super) fn draw_pick(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let width = PICK_MAX_WIDTH.min(area.width.saturating_sub(4)).max(20);
        let rows = self.candidates.len() as u16;
        let height = (rows + 2).min(area.height.saturating_sub(2)).max(3);
        let rect = Rect { x: (area.width - width) / 2, y: (area.height - height) / 2, width, height };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Cyan))
            .title(Span::styled(" which message? ", Style::new().dim()))
            .title_bottom(Span::styled(" ↑↓ choose · enter open · esc newest · q quit ", Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let mut pick_rows = Vec::new();
        let lines: Vec<Line<'static>> = self
            .candidates
            .iter()
            .enumerate()
            .take(usize::from(inner.height))
            .map(|(index, message)| {
                let row = Rect { x: inner.x, y: inner.y + index as u16, width: inner.width, height: 1 };
                pick_rows.push((row, index));
                let text = fit(&pick_label(message), usize::from(inner.width).saturating_sub(1));
                let style = if index == self.pick_cursor { Style::new().reversed() } else { Style::new() };
                Line::from(Span::styled(format!(" {text}"), style))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        self.geometry.pick_rows = pick_rows;
    }
}

/// `HH:MM  first line of the message`.
fn pick_label(message: &Message) -> String {
    let time = message.at.as_deref().and_then(clock).unwrap_or_else(|| "     ".to_owned());
    let first = message.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    format!("{time}  {first}")
}

/// `HH:MM` out of an RFC 3339 timestamp; anything else is left blank.
fn clock(at: &str) -> Option<String> {
    let time = at.get(11..16)?;
    (time.len() == 5 && time.as_bytes().get(2) == Some(&b':')).then(|| time.to_owned())
}

fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if out.width() + 1 >= width {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}
