//! Gate mode in the app: the Approve action, and the decision a gated review ends with.
//! Send and quit keep their meaning; in a gated review each also ends it (decision 17).

use anyhow::Result;
use plannotator_tui_schema::Provenance;

use super::App;
use super::feedback::{Feedback, SendScope};
use super::send::SendState;
use crate::delivery::DeliveryError;
use crate::gate::{self, Decision, Gate};
use crate::keys::Action;

#[cfg(test)]
mod tests;

impl App {
    /// Approve and end the review. Notes not yet sent ride along as approval notes, which
    /// Plannotator calls approving with notes. An agent receives the approval message; a
    /// refused delivery keeps the review open, as a refused send does.
    pub(super) fn approve(&mut self) -> Result<()> {
        if self.gate != Gate::Approve {
            return Ok(());
        }
        let scope = if self.is_file_review() { SendScope::Pending } else { SendScope::All };
        let mut feedback = self.prepare_feedback(scope)?;
        if self.tree.is_some() {
            self.folder_counts = std::mem::take(&mut feedback.counts);
        }
        let target = self.delivery.describe();
        let retry =
            self.keys.labelled(Action::Approve, "retry").unwrap_or_else(|| "click to retry".to_owned());
        match self.delivery.deliver(&gate::approval_message(&feedback.text, &self.context_line())) {
            Ok(()) => {
                let decision = Decision::Approved { feedback: feedback.text.clone(), count: feedback.count };
                self.archive_submission(&mut feedback, decision.archive_decision());
                // The notes reached the agent with the approval, so they count as sent. The
                // review is ending; a record that cannot say so only re-offers them next time.
                let _ = self.remember_delivery(&mut feedback, &target);
                self.derive_send_state();
                self.decision = Some(decision);
                self.quit = true;
            }
            Err(DeliveryError::Blocked(msg)) => {
                self.status = Some(format!("{target} is at a dialog · {retry}"));
                self.send_state = SendState::Blocked(msg);
            }
            Err(DeliveryError::Unavailable(msg)) => {
                self.status = Some(format!("no agent to send the approval to ({msg}) · {retry}"));
            }
            Err(DeliveryError::Failed(err)) => {
                self.status = Some(format!("approve failed: {err:#} · {retry}"));
            }
        }
        Ok(())
    }

    /// `Approve (A)`, or `Approve with 2 notes (A)` when unsent notes would ride along.
    pub(super) fn approve_label(&self) -> String {
        let notes = match self.send_count() {
            0 => String::new(),
            1 => " with 1 note".to_owned(),
            n => format!(" with {n} notes"),
        };
        let key = match self.keys.hint(Action::Approve) {
            key if key.is_empty() => String::new(),
            key => format!(" ({key})"),
        };
        format!("Approve{notes}{key}")
    }

    /// The `File:` or `Folder:` line an approval with notes names its target by.
    fn context_line(&self) -> String {
        match (&self.tree, &self.open.source.provenance) {
            (Some(tree), _) => gate::context_line(tree.root(), true),
            (None, Provenance::File { path }) => gate::context_line(path, false),
            _ => String::new(),
        }
    }

    /// How a gated review ended, for whoever ran it; `None` for an ordinary review.
    /// Quitting without approving or sending is a dismissal, archived as one.
    pub(crate) fn take_decision(&mut self) -> Option<Decision> {
        if self.gate == Gate::Off {
            return None;
        }
        Some(self.decision.take().unwrap_or_else(|| {
            self.archive_submission(&mut Feedback::default(), Decision::Dismissed.archive_decision());
            Decision::Dismissed
        }))
    }
}
