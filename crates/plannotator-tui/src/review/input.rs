//! Keys and the mouse. This app owns every key: it maps them to diff commands
//! (`Engine::execute`) by the user's keymap. Only mouse and resize events reach the viewer.

use anyhow::Result;
use diffs_rs::engine::Command;
use plannotator_tui_review::Side;
use ratatui::crossterm::event::{Event, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::diff::Diff;
use super::whole::Version;
use super::{Focus, Mode, Screen};
use crate::keys::{self, Action};
use crate::ui::agent_picker::{AgentPicker, Picked, PickerKey};
use crate::ui::compose::{self, ComposeAction};

/// The Review menu's rows, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuRow {
    ChangeComment,
    ResendAll,
    Finish,
    Undo,
}

pub(super) const MENU: [MenuRow; 4] =
    [MenuRow::ChangeComment, MenuRow::ResendAll, MenuRow::Finish, MenuRow::Undo];

impl MenuRow {
    pub(super) fn binding(self) -> Action {
        match self {
            Self::ChangeComment => Action::ChangeComment,
            Self::ResendAll => Action::ResendAll,
            Self::Finish => Action::FinishReview,
            Self::Undo => Action::UndoFinish,
        }
    }
}

impl Screen {
    pub(crate) fn handle_event(&mut self, event: Event) -> Result<()> {
        let event = match event {
            Event::Mouse(mouse) => match crate::ui::drag::on_screen(mouse, self.geometry.screen) {
                Some(mouse) => Event::Mouse(mouse),
                None => return Ok(()),
            },
            event => event,
        };
        if matches!(event, Event::Key(_) | Event::Paste(_)) {
            self.last_key = Some(std::time::Instant::now());
            self.toolbar = None;
        }
        // A drag ends with its release over the review. Anything else first (a key, a paste,
        // the terminal resized or left, a second press, the pointer moving with no button
        // held, which says the release went elsewhere, any mouse event over a box or the
        // help) drops it unapplied, its highlight with it, so what is lit is what `c` uses.
        let drops = match &event {
            Event::Key(key) => key.kind != KeyEventKind::Release,
            Event::Paste(_) | Event::Resize(..) | Event::FocusLost => true,
            Event::Mouse(mouse) => {
                self.mode != Mode::Browse
                    || matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Moved)
            }
            Event::FocusGained => false,
        };
        if drops {
            self.cancel_drags();
        }
        if event == Event::FocusLost {
            self.dividers.unlight();
        }
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key)?,
            Event::Paste(text) if matches!(self.mode, Mode::Compose(_)) => self.compose.insert_text(&text),
            Event::Paste(text) if self.mode == Mode::FilterFiles => self.paste_filter(&text),
            Event::Mouse(mouse) => {
                // Nothing the mouse does may end the review: what fails is said in the footer.
                if let Err(error) = self.mouse(mouse) {
                    self.cancel_drags();
                    self.status = Some(format!("{error:#}"));
                }
            }
            Event::Resize(..) => {
                if let Some(whole) = &mut self.whole {
                    whole.handle_event(event.clone())?;
                }
                self.diff.handle_event(event)?;
            }
            _ => {}
        }
        if let Some(notice) = self.active_diff_mut().and_then(Diff::take_notice) {
            self.status = Some(notice.into());
        }
        // A key that opened a box, the help or a menu mid-drag dropped the drag above; one
        // that did so by the mouse (a toolbar button) leaves none behind either.
        if self.mode != Mode::Browse {
            self.cancel_drags();
        }
        // With a draft open no line between panes can be dragged, so none is lit.
        if matches!(self.mode, Mode::Compose(_)) {
            self.dividers.unlight();
        }
        // Whatever moved the diff (a key, a note picked in the rail, a jump, the mouse), the
        // file list and the footer follow the file its cursor is in; the frame asks again.
        self.track_current();
        self.remember_settings();
        Ok(())
    }

    /// Drop a drag in progress, of lines or of a pane's edge, without applying it.
    pub(super) fn cancel_drags(&mut self) {
        if self.drag.take().is_some() {
            self.sync_markers();
        }
        self.dividers.cancel();
    }

    fn key(&mut self, key: KeyEvent) -> Result<()> {
        match self.mode.clone() {
            Mode::Browse => self.browse_key(key),
            Mode::Compose(draft) => {
                // Up and Down move by the rows the box shows; before its first frame, by
                // the widest it can be.
                let wrap = match self.geometry.compose_wrap {
                    0 => usize::from(super::compose_at::MAX_WIDTH.saturating_sub(compose::CHROME)),
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
            Mode::Settings(cursor) => {
                self.settings_key(key, cursor);
                Ok(())
            }
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
            Mode::Pick(pick) => self.pick_key(key, pick),
            Mode::FilterFiles => self.filter_key(key),
            Mode::Kinds(cursor) => self.kinds_key(key, cursor),
            Mode::Help => {
                let room = self.help_room.get();
                match self.keys.action(&key, keys::HELP) {
                    Some(Action::Down) => self.help_scroll = (self.help_scroll + 1).min(room),
                    Some(Action::Up) => self.help_scroll = self.help_scroll.saturating_sub(1),
                    Some(Action::PageDown) => self.help_scroll = (self.help_scroll + 10).min(room),
                    Some(Action::PageUp) => self.help_scroll = self.help_scroll.saturating_sub(10),
                    Some(_) => self.mode = Mode::Browse,
                    None => {}
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
                    // Moving to the notes shows them again when `toggle-notes` hid them.
                    Focus::Diff if !self.store.annotations().is_empty() => {
                        self.reveal_notes();
                        self.show_selected_note();
                        Focus::Notes
                    }
                    Focus::Diff | Focus::Notes | Focus::Files => Focus::Diff,
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
            Some(Action::ChangeComment) => return self.run_menu(MenuRow::ChangeComment),
            Some(Action::Reload) => return self.reload(),
            Some(Action::PickTarget) => {
                self.picker = AgentPicker::open(&self.reach, &self.real_root());
                self.pick_then = None;
                self.mode = Mode::PickTarget;
                return Ok(());
            }
            Some(Action::Help) => {
                self.help_scroll = 0;
                self.mode = Mode::Help;
                return Ok(());
            }
            Some(action @ (Action::NarrowNotes | Action::WidenNotes)) => {
                self.resize_notes(action == Action::WidenNotes);
                return Ok(());
            }
            Some(Action::ToggleNotes) => {
                self.toggle_notes();
                return Ok(());
            }
            Some(Action::DiffSettings) => {
                self.open_settings();
                return Ok(());
            }
            _ => {}
        }
        match self.focus {
            Focus::Diff => self.diff_key(key),
            Focus::Notes => self.notes_key(key),
            Focus::Files => self.files_key(key),
        }
    }

    fn diff_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.keys.action(&key, keys::DIFF) {
            Some(action) => self.diff_action(action),
            None => Ok(()),
        }
    }

    /// A diff key's action, or the toolbar button for it.
    fn diff_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::CopyLines => {
                self.copy_selection();
                return Ok(());
            }
            Action::GoToDefinition => return self.go_to_definition(),
            Action::JumpBack => return self.jump_history(false),
            Action::JumpForward => return self.jump_history(true),
            // The file list's keys work the same over the whole-file view.
            Action::ToggleFiles => {
                self.toggle_files();
                return Ok(());
            }
            Action::FilterFiles => {
                self.start_file_filter();
                return Ok(());
            }
            Action::FilterKinds => {
                self.mode = Mode::Kinds(0);
                return Ok(());
            }
            Action::HideViewed => return self.toggle_hide_viewed(),
            Action::NarrowFiles | Action::WidenFiles => {
                self.resize_files(action == Action::WidenFiles);
                return Ok(());
            }
            _ => {}
        }
        if self.whole.is_some() {
            return self.whole_key(action);
        }
        let command = match action {
            Action::Back if self.diff.selection().is_some() => return self.diff.clear_selection(),
            Action::Back => Command::Back,
            Action::Visual | Action::VisualLine => return self.diff.begin_or_end_selection(),
            Action::Comment => return self.start_comment(),
            Action::Suggest => return self.start_suggestion(),
            Action::FileComment => {
                self.start_file_comment();
                return Ok(());
            }
            Action::WholeFile => return self.open_whole(Version::Current),
            Action::WholeBase => return self.open_whole(Version::Base),
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
            Action::NextFile => return self.step_file(true),
            Action::PreviousFile => return self.step_file(false),
            Action::ToggleLayout => {
                self.toggle_layout();
                return Ok(());
            }
            Action::ToggleWrap => {
                self.toggle_wrap();
                return Ok(());
            }
            Action::SplitLeft | Action::SplitRight => {
                self.step_split(action == Action::SplitRight);
                return Ok(());
            }
            Action::ToggleViewed => {
                self.toggle_viewed();
                return Ok(());
            }
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
                self.open_selected_note()?;
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
            MenuRow::ChangeComment => self.start_change_comment(),
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
            Mode::Settings(_) => {
                self.settings_mouse(mouse);
                return Ok(());
            }
            Mode::Help => {
                let room = self.help_room.get();
                match mouse.kind {
                    MouseEventKind::ScrollDown => self.help_scroll = (self.help_scroll + 3).min(room),
                    MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(3),
                    _ => {}
                }
                return Ok(());
            }
            // While the box is open a click on its text moves the text cursor; nothing else
            // the mouse does changes anything: a click elsewhere keeps the draft, and the
            // wheel leaves the diff, and the lines the box is next to, where they are.
            Mode::Compose(_) => {
                if click
                    && let Some((text, first)) = self.geometry.compose_text
                    && text.contains(at)
                {
                    let (row, col) = (first + usize::from(at.y - text.y), usize::from(at.x - text.x));
                    self.compose.click(row, col, self.geometry.compose_wrap);
                }
                return Ok(());
            }
            Mode::Browse => {}
            _ => return Ok(()),
        }
        // A line between panes takes its own presses and drags; a drag that selects lines
        // goes on wherever the pointer is, over the list, the notes or past the pane.
        if self.divider_mouse(mouse) {
            self.toolbar = None;
            return Ok(());
        }
        if self.drag.is_some() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => return self.drag_to(mouse),
                MouseEventKind::Up(MouseButton::Left) => return self.release(mouse),
                _ => {}
            }
        }
        if self.geometry.files.contains(at) {
            if click {
                self.toolbar = None;
            }
            return self.files_mouse(mouse);
        }
        let toolbar = self.toolbar.is_some();
        if click
            && toolbar
            && let Some(action) = self.toolbar_hit(at)
        {
            self.toolbar = None;
            self.focus = Focus::Diff;
            return self.diff_action(action);
        }
        // Between and beside its buttons the toolbar takes the click and does nothing.
        if click && toolbar && inside(self.geometry.toolbar_bar) {
            return Ok(());
        }
        if click || matches!(mouse.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
            self.toolbar = None;
        }
        if click && inside(self.geometry.send_button) {
            self.send(false);
            return Ok(());
        }
        if click && inside(self.geometry.review_button) {
            self.mode = Mode::Menu(0);
            return Ok(());
        }
        if click && inside(self.geometry.settings_button) {
            self.open_settings();
            return Ok(());
        }
        if click && inside(self.geometry.wrap_switch) {
            self.toggle_wrap();
            return Ok(());
        }
        if let Some(&(_, split)) =
            self.geometry.layout_switch.iter().find(|(rect, _)| click && rect.contains(at))
        {
            self.pick_layout(split);
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
                return self.press(mouse);
            }
            if let Some(whole) = &mut self.whole {
                whole.handle_event(Event::Mouse(mouse))?;
            } else {
                self.diff.handle_event(Event::Mouse(mouse))?;
            }
        }
        Ok(())
    }
}
