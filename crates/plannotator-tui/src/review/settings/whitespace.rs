//! Hiding changes that only touch whitespace, and showing them again: `W` (`toggle-whitespace`),
//! the header's `no ws` chip, which shows while they are hidden, and the menu's Whitespace row.
//!
//! Hiding rebuilds the viewer's rows, tens of milliseconds on a huge diff, so a change waits
//! until a frame or another key needs it: a burst of `W` costs one change, or none, and ends
//! where a single change to the same setting ends. When it is taken, a selection whose lines
//! the viewer no longer shows (a hunk of whitespace changes only is drawn as its heading) is
//! cleared, so what `c` comments on is always what is lit.

use anyhow::Result;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::Position;

use super::{ROWS, Row};
use crate::config::review::Whitespace;
use crate::keys::{self, Action};
use crate::review::{Focus, Mode, Screen};

impl Screen {
    /// `W` or a click on the header's chip: hide whitespace changes, as they were last hidden
    /// (`git diff -w` unless `[review]` or the menu said `-b`), or show them again.
    pub(in crate::review) fn toggle_whitespace(&mut self) {
        let now = &mut self.settings.now.whitespace;
        *now = if *now == Whitespace::Show { self.settings.hide_as } else { Whitespace::Show };
        self.status = Some(self.whitespace_status());
    }

    /// What the footer says after a change of the whitespace setting.
    pub(super) fn whitespace_status(&self) -> String {
        match self.settings.now.whitespace {
            Whitespace::Show => "whitespace changes shown".to_owned(),
            hidden => {
                let back = self.keys.labelled(Action::ToggleWhitespace, "shows them").unwrap_or_default();
                let back = if back.is_empty() { back } else { format!(" \u{b7} {back}") };
                let what = match hidden {
                    Whitespace::IgnoreChange => "changes in the amount of whitespace hidden (git -b)",
                    _ => "whitespace changes hidden (like git -w)",
                };
                format!("{what}{back}")
            }
        }
    }

    /// Whether `key`, in the review, is `W`: the diff's key, which the file list and the notes
    /// take too (whitespace is the whole review's, as the settings are) unless they give the
    /// key to one of their own.
    pub(in crate::review) fn whitespace_key(&self, key: &KeyEvent) -> bool {
        let pane = match self.focus {
            Focus::Diff => None,
            Focus::Files => Some(keys::FILES),
            Focus::Notes => Some(keys::NOTES),
        };
        self.keys.action(key, keys::REVIEW_GLOBAL).is_none()
            && self.keys.action(key, keys::DIFF) == Some(Action::ToggleWhitespace)
            && pane.is_none_or(|pane| self.keys.action(key, pane).is_none())
    }

    /// Whether `event` changes the whitespace setting and nothing else, so a waiting change
    /// can wait on: `W` over the diff, a step of the menu's Whitespace row, a click on the
    /// chip or on that row. A key's release does nothing either.
    pub(in crate::review) fn changes_whitespace_only(&self, event: &Event) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Release => true,
            Event::Key(key) => match self.mode {
                Mode::Browse => self.whitespace_key(key),
                Mode::Settings(cursor) => {
                    let space =
                        key.code == KeyCode::Char(' ') && !key.modifiers.contains(KeyModifiers::CONTROL);
                    let step = matches!(
                        self.keys.action(key, keys::SETTINGS),
                        Some(Action::Left | Action::Right | Action::Confirm)
                    );
                    ROWS.get(cursor) == Some(&Row::Whitespace) && (space || step)
                }
                _ => false,
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                match self.mode {
                    Mode::Browse => self.geometry.whitespace_chip.is_some_and(|rect| rect.contains(at)),
                    Mode::Settings(_) => {
                        self.geometry.settings_rows.iter().any(|(rect, index)| {
                            rect.contains(at) && ROWS.get(*index) == Some(&Row::Whitespace)
                        })
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Bring the viewers to the whitespace setting when a change is waiting. A selection the
    /// diff no longer shows is cleared, and said so; one it shows keeps its lines. When the
    /// viewer cannot take the change, the setting goes back to what it shows.
    pub(in crate::review) fn take_whitespace(&mut self) -> Result<()> {
        let (shown, wanted) = (self.settings.shown_whitespace, self.settings.now.whitespace);
        if shown == wanted {
            return Ok(());
        }
        let selection = self.diff.selection();
        self.settings.shown_whitespace = wanted;
        if let Err(error) = self.apply_settings() {
            self.settings.shown_whitespace = shown;
            self.settings.now.whitespace = shown;
            self.apply_settings()?;
            self.status = Some(format!("{error:#}"));
            return Ok(());
        }
        if wanted != Whitespace::Show {
            self.settings.hide_as = wanted;
        }
        // A shift-click grows from a line that may be hidden now.
        self.grown = None;
        if let Some(lines) = selection
            && self.diff.selection().is_some()
            && self.diff.hides(&lines)
        {
            self.diff.clear_selection()?;
            self.status = Some("selection cleared: its lines changed only whitespace, now hidden".into());
        }
        Ok(())
    }

    /// Why the diff cannot show `line` of `path` on `side`, when whitespace is hidden and the
    /// line is in a hunk of whitespace changes only.
    pub(in crate::review) fn hidden_whitespace_reason(
        &self,
        path: &str,
        side: plannotator_tui_review::Side,
        line: u32,
    ) -> Option<String> {
        let lines = crate::review::diff::Lines { path: path.to_owned(), side, first: line, last: line };
        if !self.diff.hides(&lines) {
            return None;
        }
        let key = self.keys.labelled(Action::ToggleWhitespace, "shows them").unwrap_or_default();
        let key = if key.is_empty() { key } else { format!(" \u{b7} {key}") };
        Some(format!("its lines changed only whitespace, hidden now{key}"))
    }
}
