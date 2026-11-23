//! The file list's keys and clicks. Moving onto a file shows it in the diff; `Enter` goes
//! there. `/` types a name filter that the list follows as you type and the diff takes on
//! `Enter`; `Esc` clears it.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::draw::row_of;
use super::fit;
use super::tree::{Node, Row};
use crate::keys::{self, Action};
use crate::review::{Focus, Mode, Screen};

/// Columns `narrow-files` and `widen-files` take from or give to the list.
const WIDTH_STEP: u16 = 4;

impl Screen {
    /// `toggle-files`: open the list and move to it; from the list, close it.
    pub(in crate::review) fn toggle_files(&mut self) {
        if self.files.no_room && self.focus != Focus::Files {
            self.status = Some("too narrow for the file list".into());
            return;
        }
        if self.focus == Focus::Files {
            self.files.closed = true;
            self.focus = Focus::Diff;
        } else {
            self.files.closed = false;
            self.focus = Focus::Files;
        }
    }

    /// `filter-files`: to the list, typing a name filter.
    pub(in crate::review) fn start_file_filter(&mut self) {
        self.files.filter_from = Some(self.focus);
        self.files.closed = false;
        self.focus = Focus::Files;
        self.mode = Mode::FilterFiles;
    }

    /// `narrow-files`, `widen-files`: the list's width, by four columns.
    pub(in crate::review) fn resize_files(&mut self, wider: bool) {
        // From the width last asked for, so keys pressed between two frames add up.
        let now = match (self.files.chosen_width, self.files.drawn_width) {
            (Some(chosen), _) => chosen,
            (None, 0) => fit::MIN_WIDTH,
            (None, drawn) => drawn,
        };
        let next = if wider { now.saturating_add(WIDTH_STEP) } else { now.saturating_sub(WIDTH_STEP) };
        self.files.chosen_width = Some(next.max(fit::MIN_WIDTH));
        self.files.closed = false;
    }

    pub(in crate::review) fn files_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(action) = self.keys.action(&key, keys::FILES) else { return Ok(()) };
        let listing = self.listing();
        let rows = &listing.rows;
        let at = self.files.cursor.as_deref().and_then(|key| row_of(rows, key));
        let page = usize::from(self.geometry.files.height.saturating_sub(2)).max(1);
        let last = rows.len().saturating_sub(1);
        match action {
            Action::Down => return self.move_files_cursor(rows, at.map_or(0, |i| (i + 1).min(last))),
            Action::Up => return self.move_files_cursor(rows, at.map_or(0, |i| i.saturating_sub(1))),
            Action::PageDown => return self.move_files_cursor(rows, at.map_or(0, |i| (i + page).min(last))),
            Action::PageUp => return self.move_files_cursor(rows, at.map_or(0, |i| i.saturating_sub(page))),
            Action::Top => return self.move_files_cursor(rows, 0),
            Action::Bottom => return self.move_files_cursor(rows, last),
            Action::Confirm => match at.and_then(|i| rows.get(i)) {
                Some(Row { node: Node::Folder { .. }, key, .. }) => self.toggle_folder(key),
                Some(row) => {
                    let path = row.key.clone();
                    self.open_from_list(&path)?;
                    self.focus = Focus::Diff;
                }
                None => {}
            },
            Action::Right => {
                if let Some(Row { node: Node::Folder { expanded }, key, .. }) = at.and_then(|i| rows.get(i)) {
                    if *expanded {
                        return self.move_files_cursor(rows, at.map_or(0, |i| (i + 1).min(last)));
                    }
                    self.toggle_folder(key);
                }
            }
            Action::Left => match at.and_then(|i| rows.get(i).map(|row| (i, row))) {
                Some((_, Row { node: Node::Folder { expanded: true }, key, .. })) => self.toggle_folder(key),
                Some((i, row)) => {
                    let parent = rows.iter().take(i).rposition(|r| r.depth < row.depth);
                    if let Some(parent) = parent {
                        return self.move_files_cursor(rows, parent);
                    }
                }
                None => {}
            },
            Action::Back if !self.files.filter.query.is_empty() => {
                self.files.filter.query.clear();
                self.apply_filter(None)?;
            }
            Action::Back => self.focus = Focus::Diff,
            Action::ToggleFiles => self.toggle_files(),
            Action::FilterFiles => self.start_file_filter(),
            Action::FilterKinds => self.mode = Mode::Kinds(0),
            Action::HideViewed => self.toggle_hide_viewed()?,
            Action::ToggleViewed => {
                if let Some(Row { node: Node::File(_), key, .. }) = at.and_then(|i| rows.get(i)) {
                    let path = key.clone();
                    self.toggle_viewed_of(&path);
                }
            }
            Action::NarrowFiles => self.resize_files(false),
            Action::WidenFiles => self.resize_files(true),
            _ => {}
        }
        Ok(())
    }

    /// Put the list's cursor on row `index`; a file there shows in the diff.
    fn move_files_cursor(&mut self, rows: &[Row], index: usize) -> Result<()> {
        let Some(row) = rows.get(index) else { return Ok(()) };
        self.files.cursor = Some(row.key.clone());
        if matches!(row.node, Node::File(_)) && self.files.is_shown(&row.key) {
            let path = row.key.clone();
            self.open_from_list(&path)?;
        }
        Ok(())
    }

    /// Show a file picked in the list: in the diff, or in the whole-file view while that is
    /// open, at the same version.
    fn open_from_list(&mut self, path: &str) -> Result<()> {
        match self.whole.as_ref().map(|w| (w.path.clone(), w.version)) {
            Some((open, _)) if open == path => Ok(()),
            Some((_, version)) => {
                self.show_whole(path, version, None);
                Ok(())
            }
            None => self.goto_file(path),
        }
    }

    /// Close an open folder row, or open a closed one: every folder of its chain, which a
    /// filter may have closed under a shorter key.
    fn toggle_folder(&mut self, key: &str) {
        let listing = self.listing();
        let row = listing.rows.iter().find(|r| r.key == key);
        let Some(Row { node: Node::Folder { expanded }, label, .. }) = row else { return };
        if *expanded {
            self.files.collapsed.insert(key.to_owned());
            return;
        }
        let parent = key.len().saturating_sub(label.len());
        self.files.collapsed.retain(|closed| !(key.starts_with(closed.as_str()) && closed.len() > parent));
    }

    /// Typing the name filter. The list follows each key; `Enter` hands the filter to the
    /// diff and goes to the file under the cursor; `Esc` clears it and goes back to where
    /// `/` was pressed. Editing keys are fixed, as in the other text fields; `interrupt`
    /// quits as it does everywhere.
    pub(in crate::review) fn filter_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.keys.action(&key, keys::REVIEW_GLOBAL) == Some(Action::Interrupt) {
            self.mode = Mode::Browse;
            self.request_quit();
            return Ok(());
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.files.filter.query.clear();
                self.mode = Mode::Browse;
                self.apply_filter(None)?;
                if let Some(focus) = self.files.filter_from.take() {
                    self.focus = focus;
                }
                return Ok(());
            }
            KeyCode::Enter => {
                self.mode = Mode::Browse;
                self.files.filter_from = None;
                let listing = self.listing();
                let at = self.files.cursor.as_deref().and_then(|key| row_of(&listing.rows, key));
                let file = |row: &Row| matches!(row.node, Node::File(_)).then(|| row.key.clone());
                let target = at
                    .and_then(|i| listing.rows.get(i))
                    .and_then(file)
                    .or_else(|| listing.rows.iter().find_map(file));
                self.apply_filter(target)?;
                if !listing.rows.is_empty() {
                    self.focus = Focus::Diff;
                }
                return Ok(());
            }
            KeyCode::Down | KeyCode::Up => {
                let listing = self.listing();
                let at = self.files.cursor.as_deref().and_then(|key| row_of(&listing.rows, key));
                let next = match (key.code, at) {
                    (KeyCode::Down, Some(i)) => (i + 1).min(listing.rows.len().saturating_sub(1)),
                    (KeyCode::Up, Some(i)) => i.saturating_sub(1),
                    _ => 0,
                };
                self.files.cursor = listing.rows.get(next).map(|r| r.key.clone());
                return Ok(());
            }
            KeyCode::Backspace | KeyCode::Char('u' | 'w') if ctrl => self.files.filter.query.clear(),
            KeyCode::Backspace => {
                self.files.filter.query.pop();
            }
            KeyCode::Char(c) if !ctrl => self.files.filter.query.push(c),
            _ => return Ok(()),
        }
        self.keep_list_cursor();
        Ok(())
    }

    /// A paste while typing the filter joins it; a filter is one line, so line breaks drop
    /// out.
    pub(in crate::review) fn paste_filter(&mut self, text: &str) {
        self.files.filter.query.extend(text.chars().filter(|c| !c.is_control()));
        self.keep_list_cursor();
    }

    /// The cursor moves to the first match when its row is gone.
    fn keep_list_cursor(&mut self) {
        let listing = self.listing();
        let kept = self.files.cursor.as_deref().is_some_and(|key| listing.rows.iter().any(|r| r.key == key));
        if !kept {
            self.files.cursor =
                listing.rows.iter().find(|r| matches!(r.node, Node::File(_))).map(|r| r.key.clone());
        }
    }

    /// A click or the wheel over the list.
    pub(in crate::review) fn files_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.files.scroll += 3,
            MouseEventKind::ScrollUp => self.files.scroll = self.files.scroll.saturating_sub(3),
            MouseEventKind::Down(MouseButton::Left) => {
                let hit =
                    self.geometry.file_rows.iter().find(|(y, _)| *y == mouse.row).map(|(_, key)| key.clone());
                let Some(key) = hit else { return Ok(()) };
                self.focus = Focus::Files;
                self.files.cursor = Some(key.clone());
                if key.ends_with('/') {
                    self.toggle_folder(&key);
                } else {
                    self.open_from_list(&key)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
