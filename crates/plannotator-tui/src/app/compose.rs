//! The comment box: a small multi-line editor.
//!
//! Enter saves; Shift+Enter (where the terminal's keyboard protocol distinguishes it),
//! Alt+Enter and Ctrl+J insert a new line; Esc cancels. The usual readline keys edit by word
//! and by line, and Up and Down move between display rows. The buffer is a char vec with a
//! cursor, wrapped at word boundaries to the box width for display only — the saved body
//! keeps exactly the newlines the user typed.

use std::ops::Range;

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

    /// Apply one keystroke. `wrap_width` is the width the box last wrapped to, which Up and
    /// Down move within.
    pub(super) fn handle_key(&mut self, key: KeyEvent, wrap_width: usize) -> ComposeAction {
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
            KeyCode::Up => self.move_vertical(-1, wrap_width),
            KeyCode::Down => self.move_vertical(1, wrap_width),
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
    fn move_vertical(&mut self, delta: isize, wrap_width: usize) {
        let rows = self.rows(wrap_width);
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

    /// The range of `chars` each display row shows at `width`. Explicit newlines always
    /// break; a word that does not fit moves to the next row, and one wider than a whole row
    /// splits. Every char stays on a row so the cursor can reach it: spaces at a break hang
    /// past the edge rather than opening the next row.
    fn rows(&self, width: usize) -> Vec<Range<usize>> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let (mut start, mut used) = (0, 0);
        let mut word_start = None;
        for (i, &c) in self.chars.iter().enumerate() {
            if c == '\n' {
                rows.push(start..i);
                start = i + 1;
                used = 0;
                word_start = None;
                continue;
            }
            let w = width_of(c);
            if c.is_whitespace() {
                word_start = Some(i + 1);
            } else if used + w > width {
                let at = word_start.take().unwrap_or(i);
                if at > start {
                    rows.push(start..at);
                    start = at;
                    used = self.chars.get(at..i).unwrap_or_default().iter().copied().map(width_of).sum();
                }
                // A wide char can overflow even a row holding only its own word.
                if used + w > width && i > start {
                    rows.push(start..i);
                    start = i;
                    used = 0;
                }
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

    /// The buffer as display rows at `width`, with the cursor's (row, col) in them.
    pub(super) fn wrapped(&self, width: usize) -> (Vec<String>, usize, usize) {
        let rows = self.rows(width);
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

    const WRAP_WIDTH: usize = 45;

    fn press(compose: &mut Compose, code: KeyCode) {
        compose.handle_key(KeyEvent::new(code, KeyModifiers::CONTROL), WRAP_WIDTH);
    }

    #[test]
    fn a_word_that_does_not_fit_moves_whole_to_the_next_row() {
        let (lines, _, _) = Compose::with_text("the quick brown fox").wrapped(10);
        assert_eq!(lines, ["the quick ", "brown fox"]);
    }

    #[test]
    fn a_word_wider_than_a_row_splits() {
        let (lines, _, _) = Compose::with_text("hi aaaaaaaaaaaaaaa").wrapped(10);
        assert_eq!(lines, ["hi ", "aaaaaaaaaa", "aaaaa"]);
    }

    #[test]
    fn wrapping_keeps_every_character_of_the_buffer() {
        let text = "spaces   at   a   break and 日本語の長い行 keep their place";
        let (lines, _, _) = Compose::with_text(text).wrapped(7);
        assert_eq!(lines.concat(), text);
    }

    #[test]
    fn wide_characters_never_overflow_a_row() {
        let (lines, _, _) = Compose::with_text("a 日本語日本語").wrapped(5);
        assert_eq!(lines, ["a ", "日本", "語日", "本語"]);
    }

    #[test]
    fn spaces_past_the_edge_stay_on_the_row_they_follow() {
        let (lines, row, col) = Compose::with_text("0123456789  ").wrapped(10);
        assert_eq!(lines, ["0123456789  "]);
        assert_eq!((row, col), (0, 12));
    }

    #[test]
    fn the_cursor_follows_a_word_as_it_wraps() {
        let mut compose = Compose::with_text("0123456 ");
        compose.insert_text("abcdef");
        assert_eq!(compose.wrapped(10), (vec!["0123456 ".to_owned(), "abcdef".to_owned()], 1, 6));
    }

    #[test]
    fn up_and_down_cross_a_word_wrap_without_sticking() {
        let mut compose = Compose::with_text("the quick brown fox");
        compose.cursor = 0;
        compose.handle_key(KeyEvent::from(KeyCode::Down), 10);
        assert_eq!(compose.wrapped(10).1, 1);
        compose.handle_key(KeyEvent::from(KeyCode::End), 10);
        compose.handle_key(KeyEvent::from(KeyCode::Up), 10);
        assert_eq!(compose.wrapped(10).1, 0, "the end of the long row maps back onto the first");
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
        compose.handle_key(KeyEvent::new(code, modifiers), WRAP_WIDTH);
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
        let (_, row, col) = compose.wrapped(WRAP_WIDTH);
        assert_eq!((row, col), (1, 0));
    }

    #[test]
    fn up_from_past_a_full_row_stays_on_that_row() {
        let text = "a".repeat(WRAP_WIDTH * 2);
        let mut compose = Compose::with_text(&text);
        send(&mut compose, KeyCode::Up, KeyModifiers::NONE);
        let (_, row, col) = compose.wrapped(WRAP_WIDTH);
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
