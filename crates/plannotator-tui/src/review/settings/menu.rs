//! The settings menu (`,`, or the header's Settings button): one row per setting with its
//! value, under its button. `↑`/`↓` pick a row, `←`/`→` change its value back or on, `Enter`
//! and `Space` move it on (and make the split ratio even), a click on a row does what `Enter`
//! does. Each change applies at once and is written to the config file. `Esc`, `,` or a
//! click outside close it.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use super::{ROWS, Step};
use crate::keys::{self, Action};
use crate::review::{Mode, Screen};
use crate::theme::palette;
use crate::ui::hints;

/// The label column's width: the longest label and a gap.
const LABEL_WIDTH: usize = 14;

impl Screen {
    pub(in crate::review) fn settings_key(&mut self, key: KeyEvent, cursor: usize) {
        let row = ROWS.get(cursor).copied();
        if key.code == KeyCode::Char(' ') && !key.modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(row) = row {
                self.change_setting(row, Step::Enter);
            }
            return;
        }
        match self.keys.action(&key, keys::SETTINGS) {
            Some(Action::Down) => self.mode = Mode::Settings((cursor + 1).min(ROWS.len() - 1)),
            Some(Action::Up) => self.mode = Mode::Settings(cursor.saturating_sub(1)),
            Some(action @ (Action::Left | Action::Right | Action::Confirm)) => {
                let step = match action {
                    Action::Left => Step::Back,
                    Action::Right => Step::On,
                    _ => Step::Enter,
                };
                if let Some(row) = row {
                    self.change_setting(row, step);
                }
            }
            Some(Action::Back | Action::DiffSettings | Action::Interrupt) => self.mode = Mode::Browse,
            _ => {}
        }
    }

    /// A press on a row changes it as `Enter` does; a press outside the menu closes it.
    /// Nothing else the mouse does reaches the review while the menu is open.
    pub(in crate::review) fn settings_mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        let hit = self.geometry.settings_rows.iter().find(|(rect, _)| rect.contains(at)).map(|(_, i)| *i);
        match hit.and_then(|index| ROWS.get(index).map(|row| (index, *row))) {
            Some((index, row)) => {
                self.mode = Mode::Settings(index);
                self.change_setting(row, Step::Enter);
            }
            None if self.geometry.settings_box.is_some_and(|rect| rect.contains(at)) => {}
            None => self.mode = Mode::Browse,
        }
    }

    pub(in crate::review) fn draw_settings(&mut self, frame: &mut Frame, cursor: usize) {
        let area = frame.area();
        let rows: Vec<String> = ROWS
            .iter()
            .map(|row| format!(" {:<LABEL_WIDTH$}{} ", row.label(), self.setting_value(*row)))
            .collect();
        let saved = self.settings.file_name().map(|name| format!(" saved to {name} [review] "));
        let help = hints::picker(
            "row",
            [Some("\u{2190}\u{2192} change".to_owned()), self.keys.labelled(Action::Back, "close")],
        );
        let content = rows
            .iter()
            .map(|r| r.width())
            .chain(saved.as_ref().map(|s| s.width()))
            .max()
            .unwrap_or(0)
            .max(help.width() + 2);
        let width = (content as u16 + 2).min(area.width);
        let lines = rows.len() + usize::from(saved.is_some());
        let height = (lines as u16 + 2).min(area.height);
        let button = self.geometry.settings_button.or(self.geometry.send_button);
        let x = button.map_or(area.right(), Rect::right).min(area.right()).saturating_sub(width).max(area.x);
        let y = button.map_or(area.y, Rect::bottom).min(area.bottom().saturating_sub(height));
        let rect = Rect { x, y, width, height };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(" diff settings ", Style::new().dim()))
            .title_bottom(Span::styled(help, Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let pad = usize::from(inner.width);
        let mut hits = Vec::new();
        let mut text: Vec<Line<'static>> = rows
            .into_iter()
            .enumerate()
            .take(usize::from(inner.height))
            .map(|(index, row)| {
                hits.push((
                    Rect { x: inner.x, y: inner.y + index as u16, width: inner.width, height: 1 },
                    index,
                ));
                let style = if index == cursor { palette().selection } else { Style::new() };
                Line::from(Span::styled(format!("{row:<pad$}"), style))
            })
            .collect();
        if let Some(saved) = saved {
            text.push(Line::from(Span::styled(saved, Style::new().dim())));
        }
        frame.render_widget(Paragraph::new(text), inner);
        self.geometry.settings_rows = hits;
        self.geometry.settings_box = Some(rect);
    }
}
