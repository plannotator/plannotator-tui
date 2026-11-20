//! Whether the notes rail shows, in the document and in the code review.
//!
//! `[ui] notes_rail` says how a session starts: `when-notes` keeps the rail away until there
//! is a note to show, `shown` always shows it, `hidden` starts with it closed. Closing it with
//! `toggle-notes` (`z`) holds: no note, resize, file switch or reload opens it again, only the
//! key or moving the focus to it. The last choice is remembered across sessions in
//! `<data>/clients/plannotator-tui/ui-state.json` and beats the setting.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// `[ui] notes_rail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RailStart {
    /// Shown whenever the pane is wide enough, notes or not.
    Shown,
    /// Closed until `toggle-notes` or `Tab` opens it.
    Hidden,
    /// Away while there are no notes, so no empty rail takes columns; shown at the first.
    #[default]
    WhenNotes,
}

/// What the person last did to the rail, as the state file keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Choice {
    Open,
    Closed,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiState {
    notes_rail: Option<Choice>,
}

/// The rail's state for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct NotesRail {
    start: RailStart,
    /// Closed by the person (or by `hidden`): nothing but the key or the focus opens it.
    closed: bool,
    /// Opened by the person this session: shown even with no notes yet.
    opened: bool,
}

impl NotesRail {
    /// A session that starts as `start` says, with nothing remembered.
    pub(crate) fn new(start: RailStart) -> Self {
        Self { start, closed: start == RailStart::Hidden, opened: false }
    }

    /// A session that starts as the last one left the rail, else as `start` says. An
    /// unreadable state file counts as no memory: the rail is only a view.
    pub(crate) fn load(start: RailStart, data_dir: &Path) -> Self {
        let remembered = std::fs::read(state_path(data_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<UiState>(&bytes).ok())
            .and_then(|state| state.notes_rail);
        match remembered {
            Some(choice) => Self { start, closed: choice == Choice::Closed, opened: false },
            None => Self::new(start),
        }
    }

    /// Whether the rail is shown, width allowing, when the review has notes or not.
    pub(crate) fn shown(self, has_notes: bool) -> bool {
        !self.closed && (self.opened || self.start != RailStart::WhenNotes || has_notes)
    }

    /// Whether the person closed it, as the footer says: a rail away for want of notes is
    /// not worth a word.
    pub(crate) fn closed(self) -> bool {
        self.closed
    }

    /// `toggle-notes`: close a shown rail, open a hidden one, and remember the choice.
    pub(crate) fn toggle(&mut self, has_notes: bool, data_dir: &Path) -> Result<()> {
        if self.shown(has_notes) {
            self.close();
        } else {
            self.open();
        }
        self.remember(data_dir)
    }

    /// Moving the focus to the notes: open the rail if it is hidden, and remember that.
    pub(crate) fn reveal(&mut self, has_notes: bool, data_dir: &Path) -> Result<()> {
        if self.shown(has_notes) {
            return Ok(());
        }
        self.open();
        self.remember(data_dir)
    }

    fn open(&mut self) {
        (self.closed, self.opened) = (false, true);
    }

    fn close(&mut self) {
        (self.closed, self.opened) = (true, false);
    }

    fn remember(self, data_dir: &Path) -> Result<()> {
        let path = state_path(data_dir);
        let choice = if self.closed { Choice::Closed } else { Choice::Open };
        let text = serde_json::to_string(&UiState { notes_rail: Some(choice) })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, text)
            .with_context(|| format!("remembering the notes rail in {}", path.display()))
    }
}

fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("clients").join("plannotator-tui").join("ui-state.json")
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-rail-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn when_notes_waits_for_a_note_and_the_others_ignore_them() {
        let rail = NotesRail::new(RailStart::WhenNotes);
        assert_eq!((rail.shown(false), rail.shown(true)), (false, true));
        assert!(NotesRail::new(RailStart::Shown).shown(false));
        assert!(!NotesRail::new(RailStart::Hidden).shown(true));
        assert_eq!(RailStart::default(), RailStart::WhenNotes);
    }

    #[test]
    fn a_closed_rail_stays_closed_whatever_notes_come_until_toggled_or_revealed() {
        let data = scratch("closed");
        let mut rail = NotesRail::new(RailStart::WhenNotes);
        rail.toggle(true, &data).expect("toggle");
        assert!(rail.closed() && !rail.shown(true));
        rail.reveal(true, &data).expect("reveal");
        assert!(rail.shown(true));
        rail.toggle(true, &data).expect("toggle");
        rail.toggle(true, &data).expect("toggle");
        assert!(rail.shown(true));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn opening_an_empty_rail_shows_it_for_the_session() {
        let data = scratch("empty");
        let mut rail = NotesRail::new(RailStart::WhenNotes);
        rail.toggle(false, &data).expect("toggle");
        assert!(rail.shown(false));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn the_last_choice_outlives_the_session_and_beats_the_setting() {
        let data = scratch("memory");
        assert_eq!(NotesRail::load(RailStart::Shown, &data), NotesRail::new(RailStart::Shown));
        let mut rail = NotesRail::load(RailStart::Shown, &data);
        rail.toggle(true, &data).expect("toggle");
        let text = std::fs::read_to_string(state_path(&data)).expect("state file");
        assert_eq!(text, r#"{"notesRail":"closed"}"#);
        assert!(!NotesRail::load(RailStart::Shown, &data).shown(true));
        assert!(!NotesRail::load(RailStart::WhenNotes, &data).shown(true));

        rail.toggle(true, &data).expect("toggle");
        assert!(NotesRail::load(RailStart::Hidden, &data).shown(true), "an open choice beats hidden");
        let reopened = NotesRail::load(RailStart::WhenNotes, &data);
        assert!(reopened.shown(true) && !reopened.shown(false), "and when-notes still waits for a note");

        std::fs::write(state_path(&data), "not json").expect("garbage");
        assert_eq!(NotesRail::load(RailStart::Hidden, &data), NotesRail::new(RailStart::Hidden));
        let _ = std::fs::remove_dir_all(&data);
    }
}
