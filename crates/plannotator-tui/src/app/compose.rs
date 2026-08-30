//! The comment box: a small multi-line editor.
//!
//! Enter saves; Shift+Enter (where the terminal's keyboard protocol distinguishes it),
//! Alt+Enter and Ctrl+J insert a new line; Esc cancels. The buffer is a char vec with a
//! cursor, wrapped to the box width for display only — the saved body keeps exactly the
//! newlines the user typed.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

/// What a keystroke did to the compose box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComposeAction {
    Edited,
    Save,
    Cancel,
}

#[derive(Debug, Default)]
pub(super) struct Compose {
    chars: Vec<char>,
    cursor: usize,
}

fn width_of(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

impl Compose {
    pub(super) fn with_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Self { chars, cursor }
    }

    pub(super) fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> ComposeAction {
        match key.code {
            KeyCode::Esc => return ComposeAction::Cancel,
            KeyCode::Enter if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => {
                self.insert('\n');
            }
            KeyCode::Enter => return ComposeAction::Save,
            KeyCode::Char('j' | 'J') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert('\n');
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            KeyCode::Up => self.move_vertical(-1),
            KeyCode::Down => self.move_vertical(1),
            KeyCode::Home => {
                while self.cursor > 0 && self.chars.get(self.cursor - 1) != Some(&'\n') {
                    self.cursor -= 1;
                }
            }
            KeyCode::End => {
                while self.cursor < self.chars.len() && self.chars.get(self.cursor) != Some(&'\n') {
                    self.cursor += 1;
                }
            }
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.insert(c);
            }
            _ => {}
        }
        ComposeAction::Edited
    }

    /// Insert pasted text as typed characters, newlines included.
    pub(super) fn insert_text(&mut self, text: &str) {
        for c in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
            self.insert(c);
        }
    }

    fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Move to the logical line above or below, landing on the nearest display column.
    fn move_vertical(&mut self, delta: isize) {
        let text: String = self.chars.iter().take(self.cursor).collect();
        let row = text.split('\n').count().saturating_sub(1);
        let col: usize = text.rsplit('\n').next().unwrap_or("").chars().map(width_of).sum();
        let all: String = self.chars.iter().collect();
        let lines: Vec<&str> = all.split('\n').collect();
        let target = row.saturating_add_signed(delta).min(lines.len().saturating_sub(1));
        let mut cursor: usize = lines.iter().take(target).map(|l| l.chars().count() + 1).sum();
        let mut used = 0;
        for c in lines.get(target).copied().unwrap_or_default().chars() {
            let w = width_of(c);
            if used + w > col {
                break;
            }
            used += w;
            cursor += 1;
        }
        self.cursor = cursor;
    }

    /// The buffer wrapped to `width` display cells, with the cursor's (row, col) in that
    /// wrapping. Explicit newlines always break; a full row wraps to the next.
    pub(super) fn wrapped(&self, width: usize) -> (Vec<String>, usize, usize) {
        let width = width.max(1);
        let mut lines = vec![String::new()];
        let mut used = 0;
        let (mut cursor_row, mut cursor_col) = (0, 0);
        for (i, &c) in self.chars.iter().enumerate() {
            if i == self.cursor {
                (cursor_row, cursor_col) = (lines.len() - 1, used);
            }
            if c == '\n' {
                lines.push(String::new());
                used = 0;
                continue;
            }
            let w = width_of(c);
            if used + w > width {
                lines.push(String::new());
                used = 0;
            }
            if let Some(last) = lines.last_mut() {
                last.push(c);
            }
            used += w;
        }
        if self.cursor == self.chars.len() {
            (cursor_row, cursor_col) = (lines.len() - 1, used);
        }
        (lines, cursor_row, cursor_col)
    }
}
