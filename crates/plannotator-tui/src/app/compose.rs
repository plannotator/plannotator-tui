//! The comment box: a small multi-line editor.
//!
//! Enter saves; Shift+Enter (where the terminal's keyboard protocol distinguishes it),
//! Alt+Enter and Ctrl+J insert a new line; Esc cancels. The usual readline keys edit by word
//! and by line, and Up and Down move between display rows. The buffer is a char vec with a
//! cursor, wrapped to the box width for display only — the saved body keeps exactly the
//! newlines the user typed.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

pub(super) const COMPOSE_WIDTH: u16 = 48;
/// The box width less its two borders and one column of left padding.
const WRAP_WIDTH: usize = COMPOSE_WIDTH as usize - 3;

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
    /// The text the box opened with, to tell whether cancelling would lose anything.
    initial: String,
}

fn width_of(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

impl Compose {
    pub(super) fn with_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Self { chars, cursor, initial: text.to_owned() }
    }

    pub(super) fn value(&self) -> String {
        self.chars.iter().collect()
    }

    /// Whether saving would store something other than what the box opened with. Saving
    /// trims, so whitespace at the edges does not count.
    pub(super) fn has_changes(&self) -> bool {
        self.value().trim() != self.initial.trim()
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> ComposeAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return ComposeAction::Cancel,
            KeyCode::Enter if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => {
                self.insert('\n');
            }
            KeyCode::Enter => return ComposeAction::Save,
            KeyCode::Char('j' | 'J') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert('\n');
            }
            KeyCode::Char('a' | 'A') if ctrl => self.cursor = self.line_start(),
            KeyCode::Char('e' | 'E') if ctrl => self.cursor = self.line_end(),
            KeyCode::Char('u' | 'U') if ctrl => self.delete_to(self.line_start()),
            KeyCode::Char('k' | 'K') if ctrl => self.delete_to(self.line_end()),
            // Many terminals send Ctrl+H for Ctrl+Backspace.
            KeyCode::Char('w' | 'W' | 'h' | 'H') if ctrl => self.delete_to(self.previous_word_start()),
            KeyCode::Backspace if ctrl || alt => self.delete_to(self.previous_word_start()),
            KeyCode::Char('d' | 'D') if alt => self.delete_to(self.next_word_end()),
            KeyCode::Delete if ctrl => self.delete_to(self.next_word_end()),
            KeyCode::Char('b' | 'B') if alt => self.cursor = self.previous_word_start(),
            KeyCode::Char('f' | 'F') if alt => self.cursor = self.next_word_end(),
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
            KeyCode::Left if ctrl || alt => self.cursor = self.previous_word_start(),
            KeyCode::Right if ctrl || alt => self.cursor = self.next_word_end(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            KeyCode::Up => self.move_vertical(-1),
            KeyCode::Down => self.move_vertical(1),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Char(c) if !ctrl && !alt => self.insert(c),
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

    /// Remove the text between the cursor and `index`, leaving the cursor at the gap.
    fn delete_to(&mut self, index: usize) {
        let range = self.cursor.min(index)..self.cursor.max(index);
        self.cursor = range.start;
        self.chars.drain(range);
    }

    /// Where the logical line under the cursor starts.
    fn line_start(&self) -> usize {
        let mut index = self.cursor;
        while index > 0 && self.chars.get(index - 1) != Some(&'\n') {
            index -= 1;
        }
        index
    }

    /// Where the logical line under the cursor ends, before its newline.
    fn line_end(&self) -> usize {
        let mut index = self.cursor;
        while index < self.chars.len() && self.chars.get(index) != Some(&'\n') {
            index += 1;
        }
        index
    }

    fn is_word_char_at(&self, index: usize) -> bool {
        self.chars.get(index).is_some_and(|c| c.is_alphanumeric() || *c == '_')
    }

    /// Where the word before the cursor starts, skipping any separators in between.
    fn previous_word_start(&self) -> usize {
        let mut index = self.cursor;
        while index > 0 && !self.is_word_char_at(index - 1) {
            index -= 1;
        }
        while index > 0 && self.is_word_char_at(index - 1) {
            index -= 1;
        }
        index
    }

    /// Where the word after the cursor ends, skipping any separators in between.
    fn next_word_end(&self) -> usize {
        let mut index = self.cursor;
        while index < self.chars.len() && !self.is_word_char_at(index) {
            index += 1;
        }
        while index < self.chars.len() && self.is_word_char_at(index) {
            index += 1;
        }
        index
    }

    /// Move to the display row above or below, landing on the nearest display column. The
    /// first and last rows have nowhere further to go, so the cursor stays put.
    fn move_vertical(&mut self, delta: isize) {
        let rows = self.rows();
        let (row, col) = self.cursor_position(&rows);
        let Some(target_row) = row.checked_add_signed(delta) else { return };
        let Some(target) = rows.get(target_row) else { return };
        let mut cursor = target.start;
        let mut used = 0;
        for &c in self.chars.get(target.clone()).unwrap_or_default() {
            let w = width_of(c);
            if used + w > col {
                break;
            }
            used += w;
            cursor += 1;
        }
        // A wrapped row's end is the next row's start, which displays on that next row.
        let wraps = rows.get(target_row + 1).is_some_and(|next| next.start == target.end);
        self.cursor = if wraps && cursor == target.end { cursor.saturating_sub(1) } else { cursor };
    }

    /// The range of `chars` each display row shows. Explicit newlines always break; a full
    /// row wraps to the next.
    fn rows(&self) -> Vec<Range<usize>> {
        let mut rows = Vec::new();
        let (mut start, mut used) = (0, 0);
        for (i, &c) in self.chars.iter().enumerate() {
            if c == '\n' {
                rows.push(start..i);
                start = i + 1;
                used = 0;
                continue;
            }
            let w = width_of(c);
            if used + w > WRAP_WIDTH {
                rows.push(start..i);
                start = i;
                used = 0;
            }
            used += w;
        }
        rows.push(start..self.chars.len());
        rows
    }

    /// The cursor's (row, col) in `rows`. At a wrap the cursor shows on the later row, next
    /// to the character it sits before.
    fn cursor_position(&self, rows: &[Range<usize>]) -> (usize, usize) {
        let row = rows.iter().rposition(|range| range.start <= self.cursor).unwrap_or(0);
        let start = rows.get(row).map_or(0, |range| range.start);
        let before = self.chars.get(start..self.cursor).unwrap_or_default();
        (row, before.iter().copied().map(width_of).sum())
    }

    /// The buffer as display rows, with the cursor's (row, col) in them.
    pub(super) fn wrapped(&self) -> (Vec<String>, usize, usize) {
        let rows = self.rows();
        let (cursor_row, cursor_col) = self.cursor_position(&rows);
        let lines = rows
            .iter()
            .map(|range| self.chars.get(range.clone()).unwrap_or_default().iter().collect())
            .collect();
        (lines, cursor_row, cursor_col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(compose: &mut Compose, code: KeyCode) {
        compose.handle_key(KeyEvent::new(code, KeyModifiers::CONTROL));
    }

    #[test]
    fn ctrl_w_deletes_the_word_before_the_cursor_and_its_trailing_separators() {
        let mut compose = Compose::with_text("one two, ");
        press(&mut compose, KeyCode::Char('w'));
        assert_eq!(compose.value(), "one ");
        assert_eq!(compose.cursor, 4);
    }

    #[test]
    fn ctrl_w_keeps_the_text_after_the_cursor() {
        let mut compose = Compose::with_text("one two");
        compose.cursor = 5;
        press(&mut compose, KeyCode::Char('w'));
        assert_eq!(compose.value(), "one wo");
        assert_eq!(compose.cursor, 4);
    }

    #[test]
    fn ctrl_w_at_the_start_of_the_buffer_changes_nothing() {
        let mut compose = Compose::with_text("one");
        compose.cursor = 0;
        press(&mut compose, KeyCode::Char('w'));
        assert_eq!(compose.value(), "one");
        assert_eq!(compose.cursor, 0);
    }

    #[test]
    fn ctrl_left_lands_on_each_word_start_and_stops_at_the_buffer_start() {
        let mut compose = Compose::with_text("foo_1 bar-baz\nqux");
        let mut stops = Vec::new();
        for _ in 0..5 {
            press(&mut compose, KeyCode::Left);
            stops.push(compose.cursor);
        }
        assert_eq!(stops, [14, 10, 6, 0, 0]);
    }

    #[test]
    fn ctrl_right_lands_on_each_word_end_and_stops_at_the_buffer_end() {
        let mut compose = Compose::with_text("foo_1 bar-baz\nqux");
        compose.cursor = 0;
        let mut stops = Vec::new();
        for _ in 0..5 {
            press(&mut compose, KeyCode::Right);
            stops.push(compose.cursor);
        }
        assert_eq!(stops, [5, 9, 13, 17, 17]);
    }

    fn send(compose: &mut Compose, code: KeyCode, modifiers: KeyModifiers) {
        compose.handle_key(KeyEvent::new(code, modifiers));
    }

    #[test]
    fn ctrl_a_and_ctrl_e_stay_on_the_logical_line() {
        let mut compose = Compose::with_text("one\ntwo three\nfour");
        compose.cursor = 6;
        press(&mut compose, KeyCode::Char('a'));
        assert_eq!(compose.cursor, 4);
        press(&mut compose, KeyCode::Char('e'));
        assert_eq!(compose.cursor, 13);
    }

    #[test]
    fn ctrl_u_and_ctrl_k_delete_to_the_line_edges_and_keep_other_lines() {
        let mut compose = Compose::with_text("one\ntwo three\nfour");
        compose.cursor = 7;
        press(&mut compose, KeyCode::Char('u'));
        assert_eq!(compose.value(), "one\n three\nfour");
        assert_eq!(compose.cursor, 4);
        press(&mut compose, KeyCode::Char('k'));
        assert_eq!(compose.value(), "one\n\nfour");
        assert_eq!(compose.cursor, 4);
    }

    #[test]
    fn every_previous_word_delete_key_deletes_the_word() {
        let keys = [
            (KeyCode::Char('w'), KeyModifiers::CONTROL),
            (KeyCode::Char('h'), KeyModifiers::CONTROL),
            (KeyCode::Backspace, KeyModifiers::ALT),
            (KeyCode::Backspace, KeyModifiers::CONTROL),
        ];
        for (code, modifiers) in keys {
            let mut compose = Compose::with_text("one two");
            send(&mut compose, code, modifiers);
            assert_eq!(compose.value(), "one ", "{code:?} {modifiers:?}");
        }
    }

    #[test]
    fn every_next_word_delete_key_deletes_the_word() {
        let keys = [(KeyCode::Char('d'), KeyModifiers::ALT), (KeyCode::Delete, KeyModifiers::CONTROL)];
        for (code, modifiers) in keys {
            let mut compose = Compose::with_text("one two three");
            compose.cursor = 3;
            send(&mut compose, code, modifiers);
            assert_eq!(compose.value(), "one three", "{code:?} {modifiers:?}");
            assert_eq!(compose.cursor, 3);
        }
    }

    #[test]
    fn plain_backspace_and_delete_still_remove_one_character() {
        let mut compose = Compose::with_text("one two");
        send(&mut compose, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(compose.value(), "one tw");
        compose.cursor = 0;
        send(&mut compose, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(compose.value(), "ne tw");
    }

    #[test]
    fn alt_keys_jump_by_word_like_ctrl_arrows() {
        for (code, modifiers) in [(KeyCode::Left, KeyModifiers::ALT), (KeyCode::Char('b'), KeyModifiers::ALT)]
        {
            let mut compose = Compose::with_text("one two");
            send(&mut compose, code, modifiers);
            assert_eq!(compose.cursor, 4, "{code:?}");
        }
        for (code, modifiers) in
            [(KeyCode::Right, KeyModifiers::ALT), (KeyCode::Char('f'), KeyModifiers::ALT)]
        {
            let mut compose = Compose::with_text("one two");
            compose.cursor = 0;
            send(&mut compose, code, modifiers);
            assert_eq!(compose.cursor, 3, "{code:?}");
        }
    }

    #[test]
    fn up_and_down_move_between_wrapped_rows_of_one_line() {
        let text = "a".repeat(WRAP_WIDTH * 2 + 10);
        let mut compose = Compose::with_text(&text);
        compose.cursor = WRAP_WIDTH + 5;
        send(&mut compose, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(compose.cursor, 5);
        send(&mut compose, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(compose.cursor, WRAP_WIDTH + 5);
        send(&mut compose, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(compose.cursor, WRAP_WIDTH * 2 + 5);
    }

    #[test]
    fn up_on_the_first_row_and_down_on_the_last_stay_put() {
        let mut compose = Compose::with_text("one\ntwo");
        compose.cursor = 1;
        send(&mut compose, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(compose.cursor, 1);
        compose.cursor = 5;
        send(&mut compose, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(compose.cursor, 5);
    }

    #[test]
    fn down_from_column_zero_lands_visibly_on_the_next_wrapped_row() {
        let text = "a".repeat(WRAP_WIDTH * 2);
        let mut compose = Compose::with_text(&text);
        compose.cursor = 0;
        send(&mut compose, KeyCode::Down, KeyModifiers::NONE);
        let (_, row, col) = compose.wrapped();
        assert_eq!((row, col), (1, 0));
    }

    #[test]
    fn up_from_past_a_full_row_stays_on_that_row() {
        let text = "a".repeat(WRAP_WIDTH * 2);
        let mut compose = Compose::with_text(&text);
        send(&mut compose, KeyCode::Up, KeyModifiers::NONE);
        let (_, row, col) = compose.wrapped();
        assert_eq!((row, col), (0, WRAP_WIDTH - 1));
    }

    #[test]
    fn vertical_moves_keep_the_display_column_across_wide_characters() {
        let mut compose = Compose::with_text("日本語\nabcdef");
        compose.cursor = 7;
        send(&mut compose, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(compose.cursor, 1);
    }

    #[test]
    fn only_a_real_change_counts_as_unsaved() {
        assert!(!Compose::default().has_changes());
        assert!(!Compose::with_text("body").has_changes());
        let mut blank = Compose::default();
        blank.insert_text("  \n ");
        assert!(!blank.has_changes(), "whitespace alone would never be saved");
        let mut edited = Compose::with_text("body");
        edited.insert_text("!");
        assert!(edited.has_changes());
        send(&mut edited, KeyCode::Backspace, KeyModifiers::NONE);
        assert!(!edited.has_changes(), "typing it back to the original loses nothing");
    }

    #[test]
    fn non_ascii_letters_count_as_word_characters() {
        let mut compose = Compose::with_text("über größe");
        press(&mut compose, KeyCode::Char('w'));
        assert_eq!(compose.value(), "über ");
    }
}
