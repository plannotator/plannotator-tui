//! Handing the review back to a host that delivers the feedback itself.
//!
//! A host that runs plannotator-tui as a modal step (an editor or agent plugin, say) sets
//! `PLANNOTATOR_TUI_SEND_INTENT` to a file path. A send then writes `send` there and quits in
//! one step; the host exports the notes with `--export` and delivers them. Nothing is
//! delivered or archived from here, so the notes stay pending and the Review menu is off.

use std::path::PathBuf;

use super::App;
use crate::keys::Action;

/// The intent file a host asked for, if any; an empty value means none.
pub(crate) fn send_intent() -> Option<PathBuf> {
    std::env::var_os("PLANNOTATOR_TUI_SEND_INTENT").filter(|path| !path.is_empty()).map(PathBuf::from)
}

impl App {
    pub(super) fn is_hosted(&self) -> bool {
        self.send_intent.is_some()
    }

    /// Resend, finish, undo and archive track deliveries this app made itself.
    pub(super) fn has_review_menu(&self) -> bool {
        self.is_file_review() && !self.is_hosted()
    }

    /// Record the send for the host and quit; with nothing to send, stay and say so.
    pub(super) fn hand_back(&mut self) {
        if self.send_count() == 0 {
            self.status = Some("nothing to send".into());
            return;
        }
        let Some(path) = &self.send_intent else { return };
        match std::fs::write(path, "send\n") {
            Ok(()) => self.quit = true,
            Err(err) => self.status = Some(format!("could not hand the notes back: {err}")),
        }
    }

    /// `quit` in a hosted review: leaving with notes is sending them. `interrupt` leaves
    /// without, and the host decides what to do with them.
    pub(super) fn leave_hosted(&mut self) {
        if self.send_count() == 0 {
            self.quit = true;
        } else {
            self.hand_back();
        }
    }

    /// The header button's label: the key that hands the notes back.
    pub(super) fn hosted_send_label(&self) -> String {
        let key = [Action::Quit, Action::Send].map(|action| self.keys.hint(action));
        match key.iter().find(|key| !key.is_empty()) {
            Some(key) => format!("Send {} back ({key})", self.send_count()),
            None => format!("Send {} back", self.send_count()),
        }
    }
}

#[cfg(test)]
mod tests;
