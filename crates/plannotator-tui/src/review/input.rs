//! Keys and the mouse. This app owns every key: it maps them to diff commands
//! (`Engine::execute`) by the user's keymap. Only mouse and resize events reach the viewer.

use anyhow::Result;
use diffs_rs::engine::Command;
use plannotator_tui_review::Side;
use ratatui::crossterm::event::{Event, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::{Focus, Mode, Screen};
use crate::keys::{self, Action};
use crate::ui::agent_picker::{AgentPicker, Picked, PickerKey};
use crate::ui::compose::{self, ComposeAction};

/// The Review menu's rows, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuRow {
    ResendAll,
    Finish,
    Undo,
}

pub(super) const MENU: [MenuRow; 3] = [MenuRow::ResendAll, MenuRow::Finish, MenuRow::Undo];

impl MenuRow {
    pub(super) fn binding(self) -> Action {
        match self {
            Self::ResendAll => Action::ResendAll,
            Self::Finish => Action::FinishReview,
            Self::Undo => Action::UndoFinish,
        }
    }
}

impl Screen {
    pub(crate) fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Paste(text) if matches!(self.mode, Mode::Compose(_)) => {
                self.compose.insert_text(&text);
                Ok(())
            }
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::Resize(..) => self.diff.handle_event(event).map(drop),
            _ => Ok(()),
        }
    }

    fn key(&mut self, key: KeyEvent) -> Result<()> {
        match self.mode.clone() {
            Mode::Browse => self.browse_key(key),
            Mode::Compose(draft) => {
                // Up and Down move by the rows the box shows; before its first frame, by
                // the widest it can be.
                let wrap = match self.geometry.compose_wrap {
                    0 => usize::from(super::draw::COMPOSE_MAX_WIDTH.saturating_sub(compose::CHROME)),
                    drawn => drawn,
                };
                match self.compose.key(&self.keys, key, wrap) {
                    ComposeAction::Save => self.save_draft(&draft)?,
                    ComposeAction::Cancel => self.mode = Mode::Browse,
                    ComposeAction::Edited => {}
                }
                Ok(())
            }
            Mode::ConfirmQuit => {
                match self.keys.action(&key, keys::CONFIRM_QUIT) {
                    Some(Action::Yes | Action::Confirm) => {
                        self.mode = Mode::Browse;
                        self.send_and_quit();
                    }
                    Some(Action::No) => self.quit = true,
                    Some(Action::Back) => self.mode = Mode::Browse,
                    _ => {}
                }
                Ok(())
            }
            Mode::Menu(cursor) => self.menu_key(key, cursor),
            Mode::PickTarget => {
                match self.picker.key(&self.keys, &key, self.start_delivery.is_some()) {
                    PickerKey::Stay => {}
                    PickerKey::Close => {
                        self.mode = Mode::Browse;
                        if self.pick_then.take().is_some() {
                            self.status = Some("nothing sent".into());
                        }
                    }
                    PickerKey::Chose(row) => {
                        self.mode = Mode::Browse;
                        let then = self.pick_then.take();
                        match self.picker.choose(row, &mut self.delivery, &mut self.start_delivery) {
                            Picked::Moved(status) => self.status = Some(status),
                            Picked::Copy => {}
                            Picked::Nothing => return Ok(()),
                        }
                        if let Some(then) = then {
                            self.send_picked(then);
                        }
                    }
                }
                Ok(())
            }
            Mode::Help => {
                if self.keys.action(&key, keys::HELP).is_some() {
                    self.mode = Mode::Browse;
                }
                Ok(())
            }
        }
    }

    fn browse_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.keys.action(&key, keys::REVIEW_GLOBAL) {
            Some(Action::Quit | Action::Interrupt) => {
                self.request_quit();
                return Ok(());
            }
            Some(Action::CycleFocus) => {
                self.focus = match self.focus {
                    Focus::Diff if !self.store.annotations().is_empty() => {
                        self.show_selected_note();
                        Focus::Notes
                    }
                    Focus::Diff | Focus::Notes => Focus::Diff,
                };
                return Ok(());
            }
            Some(Action::Send) => {
                self.send(false);
                return Ok(());
            }
            Some(Action::SendAndClose) => {
                self.send_and_quit();
                return Ok(());
            }
            Some(Action::ReviewMenu) => {
                self.mode = Mode::Menu(0);
                return Ok(());
            }
            Some(Action::ResendAll) => return self.run_menu(MenuRow::ResendAll),
            Some(Action::FinishReview) => return self.run_menu(MenuRow::Finish),
            Some(Action::UndoFinish) => return self.run_menu(MenuRow::Undo),
            Some(Action::Reload) => return self.reload(),
            Some(Action::PickTarget) => {
                self.picker = AgentPicker::open(&self.reach, &self.real_root());
                self.pick_then = None;
                self.mode = Mode::PickTarget;
                return Ok(());
            }
            Some(Action::Help) => {
                self.mode = Mode::Help;
                return Ok(());
            }
            _ => {}
        }
        match self.focus {
            Focus::Diff => self.diff_key(key),
            Focus::Notes => self.notes_key(key),
        }
    }

    fn diff_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(action) = self.keys.action(&key, keys::DIFF) else { return Ok(()) };
        let command = match action {
            Action::Back if self.diff.selection().is_some() => return self.diff.clear_selection(),
            Action::Back => Command::Back,
            Action::Visual | Action::VisualLine => return self.diff.begin_or_end_selection(),
            Action::Comment => return self.start_comment(),
            Action::Suggest => return self.start_suggestion(),
            Action::FileComment => return self.start_file_comment(),
            Action::Left => return self.go_to_side(Side::Old),
            Action::Right => return self.go_to_side(Side::New),
            Action::Down => Command::Down,
            Action::Up => Command::Up,
            Action::PageDown => Command::PageDown,
            Action::PageUp => Command::PageUp,
            Action::Top => Command::FirstSource,
            Action::Bottom => Command::LastSource,
            Action::NextHunk => Command::NextHunk,
            Action::PreviousHunk => Command::PreviousHunk,
            Action::NextFile => Command::NextFile,
            Action::PreviousFile => Command::PreviousFile,
            Action::ToggleLayout => Command::ToggleLayout,
            Action::ToggleWrap => Command::ToggleWrap,
            Action::ToggleFiles => Command::Files,
            Action::ToggleViewed => return self.toggle_viewed(),
            Action::NextUnviewed => return self.jump_unviewed(true),
            Action::PreviousUnviewed => return self.jump_unviewed(false),
            Action::Confirm => Command::Activate,
            _ => return Ok(()),
        };
        self.diff.run(command)
    }

    /// `h`/`l`: the old or the new side of the line under the cursor.
    fn go_to_side(&mut self, side: Side) -> Result<()> {
        if self.diff.cursor_side()?.is_some_and(|now| now != side) {
            self.diff.run(Command::SwitchSide)?;
        }
        Ok(())
    }

    fn notes_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.keys.action(&key, keys::NOTES) {
            Some(Action::Down) => self.move_rail(1),
            Some(Action::Up) => self.move_rail(-1),
            Some(Action::Confirm) => {
                self.show_selected_note();
                self.focus = Focus::Diff;
            }
            Some(Action::Edit) => self.start_edit(),
            Some(Action::Remove) => self.remove_selected()?,
            Some(Action::Back) => self.focus = Focus::Diff,
            _ => {}
        }
        Ok(())
    }

    fn menu_key(&mut self, key: KeyEvent, cursor: usize) -> Result<()> {
        match self.keys.action(&key, keys::REVIEW_MENU) {
            Some(Action::Down) => self.mode = Mode::Menu((cursor + 1).min(MENU.len() - 1)),
            Some(Action::Up) => self.mode = Mode::Menu(cursor.saturating_sub(1)),
            Some(Action::Confirm) => {
                if let Some(&row) = MENU.get(cursor) {
                    return self.run_menu(row);
                }
            }
            Some(Action::Back | Action::ReviewMenu | Action::Interrupt) => self.mode = Mode::Browse,
            Some(binding) => {
                if let Some(row) = MENU.into_iter().find(|row| row.binding() == binding) {
                    return self.run_menu(row);
                }
            }
            None => {}
        }
        Ok(())
    }

    fn run_menu(&mut self, row: MenuRow) -> Result<()> {
        self.mode = Mode::Browse;
        match row {
            MenuRow::ResendAll => self.send(true),
            MenuRow::Finish => self.finish(),
            MenuRow::Undo => return self.undo_finish(),
        }
        Ok(())
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        let at = Position::new(mouse.column, mouse.row);
        let inside = |rect: Option<Rect>| rect.is_some_and(|r| r.contains(at));
        let click = mouse.kind == MouseEventKind::Down(MouseButton::Left);
        match &self.mode {
            Mode::Menu(_) if click => {
                let row = self.geometry.menu_rows.iter().find(|(r, _)| r.contains(at)).map(|(_, i)| *i);
                if let Some(row) = row.and_then(|i| MENU.get(i).copied()) {
                    return self.run_menu(row);
                }
                self.mode = Mode::Browse;
                return Ok(());
            }
            Mode::Browse => {}
            _ => return Ok(()),
        }
        if click && inside(self.geometry.send_button) {
            self.send(false);
            return Ok(());
        }
        if click && inside(self.geometry.review_button) {
            self.mode = Mode::Menu(0);
            return Ok(());
        }
        if self.geometry.rail.contains(at) {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.rail_scroll += 3,
                MouseEventKind::ScrollUp => self.rail_scroll = self.rail_scroll.saturating_sub(3),
                MouseEventKind::Down(MouseButton::Left) => {
                    let hit = self.geometry.bubbles.iter().find(|(r, _)| r.contains(at)).map(|(_, i)| *i);
                    if let Some(position) = hit.and_then(|i| self.rail_order().iter().position(|&n| n == i)) {
                        self.rail_cursor = position;
                        self.focus = Focus::Notes;
                        self.show_selected_note();
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        if self.geometry.diff.contains(at) {
            if click {
                self.focus = Focus::Diff;
            }
            self.diff.handle_event(Event::Mouse(mouse))?;
        }
        Ok(())
    }
}
