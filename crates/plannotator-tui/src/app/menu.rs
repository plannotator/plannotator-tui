//! The Review menu: the file and folder review actions behind one header button. `m` or
//! a click opens it under the header's right edge; it is drawn like the message picker.

use anyhow::Result;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use super::{App, Mode};

#[cfg(test)]
mod tests;

/// Menu rows, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewAction {
    ResendAll,
    Finish,
    Undo,
    Archive,
}

const REVIEW_ACTIONS: [ReviewAction; 4] =
    [ReviewAction::ResendAll, ReviewAction::Finish, ReviewAction::Undo, ReviewAction::Archive];

const MENU_HELP: &str = " \u{2191}\u{2193} move \u{b7} enter run \u{b7} esc close ";

impl ReviewAction {
    /// The key that runs the action, from the menu and directly in the review.
    fn key(self) -> char {
        match self {
            Self::ResendAll => 'R',
            Self::Finish => 'F',
            Self::Undo => 'U',
            Self::Archive => 'H',
        }
    }

    fn from_key(key: char) -> Option<Self> {
        REVIEW_ACTIONS.into_iter().find(|action| action.key() == key)
    }
}

impl App {
    fn action_label(&self, action: ReviewAction) -> String {
        let counts = self.review_counts();
        match action {
            ReviewAction::ResendAll => format!("Resend all \u{b7} {} sent", counts.sent),
            ReviewAction::Finish => format!("Finish review \u{b7} archive {} sent", counts.sent),
            ReviewAction::Undo => "Undo finish".to_owned(),
            ReviewAction::Archive => format!("Archive \u{b7} {} notes", counts.archived),
        }
    }

    /// Whether the action has anything to act on. Other rows are dimmed and skipped.
    fn action_applies(&self, action: ReviewAction) -> bool {
        let counts = self.review_counts();
        match action {
            ReviewAction::ResendAll | ReviewAction::Finish => counts.sent > 0,
            ReviewAction::Undo => !self.undo_archive.is_empty(),
            ReviewAction::Archive => counts.archived > 0,
        }
    }

    /// Open the menu on the first row that applies. Reply reviews have no menu.
    pub(crate) fn open_review_menu(&mut self) {
        if !self.is_file_review() {
            return;
        }
        self.menu_cursor = REVIEW_ACTIONS.iter().position(|&action| self.action_applies(action)).unwrap_or(0);
        self.mode = Mode::ReviewMenu;
    }

    /// Leave the menu, then act exactly as the direct key does.
    pub(super) fn run_review_action(&mut self, action: ReviewAction) -> Result<()> {
        self.mode = Mode::Browse;
        match action {
            ReviewAction::ResendAll => self.resend_all(),
            ReviewAction::Finish => {
                self.finish_review();
                Ok(())
            }
            ReviewAction::Undo => {
                self.undo_finish_review();
                Ok(())
            }
            ReviewAction::Archive => {
                self.open_archive();
                Ok(())
            }
        }
    }

    /// Move to the next row that applies in the given direction; stay when there is none.
    fn move_menu_cursor(&mut self, down: bool) {
        let applies = |(index, action): (usize, &ReviewAction)| self.action_applies(*action).then_some(index);
        let rows = REVIEW_ACTIONS.iter().enumerate();
        let next = if down {
            rows.skip(self.menu_cursor + 1).find_map(applies)
        } else {
            rows.take(self.menu_cursor).filter_map(applies).next_back()
        };
        if let Some(index) = next {
            self.menu_cursor = index;
        }
    }

    pub(super) fn menu_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_menu_cursor(true),
            KeyCode::Char('k') | KeyCode::Up => self.move_menu_cursor(false),
            KeyCode::Enter => {
                if let Some(&action) = REVIEW_ACTIONS.get(self.menu_cursor)
                    && self.action_applies(action)
                {
                    return self.run_review_action(action);
                }
            }
            KeyCode::Esc | KeyCode::Char('m') => self.mode = Mode::Browse,
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => self.mode = Mode::Browse,
            KeyCode::Char(ch) => {
                if let Some(action) = ReviewAction::from_key(ch) {
                    return self.run_review_action(action);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// A row that applies runs on click; a click outside the menu closes it.
    pub(super) fn menu_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(());
        }
        let inside = |rect: Rect| {
            mouse.column >= rect.x
                && mouse.column < rect.right()
                && mouse.row >= rect.y
                && mouse.row < rect.bottom()
        };
        let hit = self.geometry.menu_rows.iter().find(|(rect, _)| inside(*rect)).map(|(_, index)| *index);
        match hit.and_then(|index| REVIEW_ACTIONS.get(index).copied()) {
            Some(action) if self.action_applies(action) => self.run_review_action(action),
            Some(_) => Ok(()),
            None => {
                if !self.geometry.menu.is_some_and(inside) {
                    self.mode = Mode::Browse;
                }
                Ok(())
            }
        }
    }

    pub(super) fn draw_review_menu(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let rows: Vec<(ReviewAction, String)> = REVIEW_ACTIONS
            .iter()
            .map(|&action| (action, format!(" {}  {} ", action.key(), self.action_label(action))))
            .collect();
        let content = rows.iter().map(|(_, text)| text.width()).max().unwrap_or(0).max(MENU_HELP.width());
        let width = (content as u16 + 2).min(area.width);
        let height = (REVIEW_ACTIONS.len() as u16 + 2).min(area.height);
        // Under the Review button, flush with its right edge, like a dropdown.
        let button = self.geometry.review_button;
        let x = button.map_or(area.right(), Rect::right).saturating_sub(width).max(area.x);
        let y = button.map_or(area.y, Rect::bottom).min(area.bottom().saturating_sub(height));
        let rect = Rect { x, y, width, height };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Cyan))
            .title(Span::styled(" review ", Style::new().dim()))
            .title_bottom(Span::styled(MENU_HELP, Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let mut menu_rows = Vec::new();
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .enumerate()
            .take(usize::from(inner.height))
            .map(|(index, (action, text))| {
                let row = Rect { x: inner.x, y: inner.y + index as u16, width: inner.width, height: 1 };
                menu_rows.push((row, index));
                let style = if !self.action_applies(action) {
                    Style::new().dim()
                } else if index == self.menu_cursor {
                    Style::new().reversed()
                } else {
                    Style::new()
                };
                Line::from(Span::styled(format!("{text:<pad$}", pad = usize::from(inner.width)), style))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        self.geometry.menu_rows = menu_rows;
        self.geometry.menu = Some(rect);
    }
}
