//! An Ask box under its lines: asking again there (the question box opens at its end, scrolled
//! into view), removing it, and what its border says.

use anyhow::Result;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use super::Thread;
use crate::keys::{self, Action};
use crate::review::diff::Lines;
use crate::review::{Draft, Mode, Screen};
use crate::ui::compose::Compose;

impl Screen {
    /// The question box for `lines`. When they have a box already the question goes into, it
    /// opens at that box's end: the diff scrolls, as the wheel does, until the end and room
    /// for the question box under it show, which a long answer pushes below the pane.
    pub(super) fn open_question(&mut self, lines: Lines) -> Result<()> {
        if let Some(id) = self.ask.box_for(&lines).map(|thread| thread.id) {
            let pane = self.geometry.diff;
            if let Some(area) = self.diff.box_areas().iter().find(|area| area.id == id) {
                let cut = area.height.saturating_sub(area.clipped_top + area.area.height);
                let below = pane.bottom().saturating_sub(area.area.bottom());
                let rows = (cut + crate::review::compose_at::ROOM).saturating_sub(below);
                let wheel = MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: pane.x + 1,
                    row: pane.y + 1,
                    modifiers: KeyModifiers::NONE,
                };
                // The viewer scrolls three rows a wheel step.
                for _ in 0..rows.div_ceil(3) {
                    self.diff.handle_event(Event::Mouse(wheel))?;
                }
            }
        }
        self.compose = Compose::default();
        self.mode = Mode::Compose(Draft::Ask(lines));
        Ok(())
    }

    /// A key on an Ask box: Enter or `ask` asks again about its lines, the remove key removes
    /// the box. With the cursor on a box's lines, where the click that focused the pane may
    /// have put it, the remove key removes that box too. True when the key was the box's.
    pub(in crate::review) fn ask_box_key(&mut self, key: &KeyEvent) -> bool {
        let diff = self.keys.action(key, keys::DIFF);
        let remove = diff.is_none() && self.keys.is(Action::Remove, key);
        if let Some(thread) = self.cursor_thread() {
            let (id, lines) = (thread.id, thread.lines.clone());
            if matches!(diff, Some(Action::Confirm | Action::Ask)) {
                if let Err(error) = self.open_question(lines) {
                    self.status = Some(format!("{error:#}"));
                }
            } else if remove {
                self.remove_thread(id);
            } else {
                return false;
            }
            return true;
        }
        let on_lines = self.diff.cursor().ok().flatten().filter(|_| remove && self.whole.is_none());
        match on_lines.and_then(|lines| self.ask.box_for(&lines).map(|thread| thread.id)) {
            Some(id) => {
                self.remove_thread(id);
                true
            }
            None => false,
        }
    }

    fn remove_thread(&mut self, id: u64) {
        self.ask.threads.retain(|thread| thread.id != id);
        self.status = Some("questions removed from the diff; the agent's chat keeps them".into());
    }

    /// What an Ask box's border says it does, each with its key: `ask again (⏎ Enter) ·
    /// remove (⌦ Delete)`, or the keys the config binds.
    pub(in crate::review) fn ask_box_hint(&self) -> String {
        let bound = |action, code| self.keys.is(action, &KeyEvent::from(code));
        let again = if bound(Action::Confirm, KeyCode::Enter) {
            "\u{23ce} Enter".to_owned()
        } else {
            self.keys.hint(Action::Ask)
        };
        let remove = if bound(Action::Remove, KeyCode::Delete) {
            "\u{2326} Delete".to_owned()
        } else {
            self.keys.hint_besides(Action::Remove, keys::DIFF)
        };
        [("ask again", again), ("remove", remove)]
            .into_iter()
            .filter(|(_, key)| !key.is_empty())
            .map(|(what, key)| format!("{what} ({key})"))
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
    }

    /// The thread whose box the cursor is on, in the diff (not the whole file).
    fn cursor_thread(&self) -> Option<&Thread> {
        if self.whole.is_some() {
            return None;
        }
        self.ask.thread(self.diff.cursor_box()?)
    }
}
