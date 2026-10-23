//! A small archive picker: identify a note by file, quote and body, then restore it.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use super::draw::picker_help;
use super::{App, Mode, label};
use crate::keys::{self, Action};
use crate::theme::palette;

impl App {
    pub(super) fn archive_key(&mut self, key: KeyEvent) {
        match self.keys.action(&key, keys::ARCHIVE) {
            Some(Action::Down) => self.move_archive_cursor(1),
            Some(Action::Up) => self.move_archive_cursor(-1),
            Some(Action::Confirm) => self.restore_selected_archived(),
            Some(Action::Back | Action::Archive | Action::Quit | Action::Interrupt) => {
                self.mode = Mode::Browse;
            }
            _ => {}
        }
    }

    fn move_archive_cursor(&mut self, delta: i64) {
        let last = self.archive_items.len().saturating_sub(1);
        self.archive_cursor = (self.archive_cursor as i64 + delta).clamp(0, last as i64) as usize;
    }

    pub(super) fn archive_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.move_archive_cursor(1),
            MouseEventKind::ScrollUp => self.move_archive_cursor(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self.geometry.archive_rows.iter().find(|(rect, _)| {
                    mouse.column >= rect.x
                        && mouse.column < rect.right()
                        && mouse.row >= rect.y
                        && mouse.row < rect.bottom()
                });
                if let Some((_, index)) = hit {
                    self.archive_cursor = *index;
                    self.restore_selected_archived();
                }
            }
            _ => {}
        }
    }

    pub(super) fn draw_archive(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let width = area.width.saturating_sub(4).clamp(1, 100).min(area.width);
        let height = area.height.saturating_sub(4).clamp(3, 22).min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(format!(" Archived annotations ({}) ", self.archive_items.len()))
            .title_bottom(picker_help(
                "select",
                [
                    // A click restores too, so it is named even with `confirm` unbound.
                    Some(match self.keys.hint(Action::Confirm) {
                        key if key.is_empty() => "click restore".to_owned(),
                        key => format!("{key}/click restore"),
                    }),
                    self.keys.labelled(Action::Back, "close"),
                ],
            ));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if self.archive_items.is_empty() {
            frame.render_widget(
                Paragraph::new(match self.keys.hint(Action::FinishReview) {
                    key if key.is_empty() => "No archived annotations.".to_owned(),
                    key => format!("No archived annotations. {key} finishes sent annotations."),
                }),
                inner,
            );
            return;
        }
        let visible = (usize::from(inner.height) / 3).max(1);
        let start = self.archive_cursor.saturating_sub(visible - 1);
        for (index, item) in self.archive_items.iter().enumerate().skip(start).take(visible) {
            let y = inner.y + ((index - start) * 3) as u16;
            if y >= inner.bottom() {
                break;
            }
            let path = self.review_file_name(&item.path);
            let quote = item.annotation.anchor.original_text.split_whitespace().collect::<Vec<_>>().join(" ");
            let body = item.annotation.body.split_whitespace().collect::<Vec<_>>().join(" ");
            let style = if index == self.archive_cursor {
                // Reversed accent: the row text takes the terminal background, which an accent
                // picked to read as text on that background contrasts with.
                Style::new().fg(palette().accent).add_modifier(Modifier::REVERSED)
            } else {
                Style::new()
            };
            let rows = vec![
                Line::from(Span::styled(
                    format!(" {path} · {}", label(item.annotation.anchor.kind())),
                    style,
                )),
                Line::from(Span::styled(format!(" “{quote}”"), style)),
                Line::from(Span::styled(format!(" {body}"), style)),
            ];
            let row_rect = Rect { x: inner.x, y, width: inner.width, height: 3.min(inner.bottom() - y) };
            frame.render_widget(Paragraph::new(rows).style(style), row_rect);
            self.geometry.archive_rows.push((row_rect, index));
        }
    }
}
