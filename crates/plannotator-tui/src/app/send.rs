//! Sending feedback and the state of the Send button.

use std::fmt::Write as _;

use anyhow::Result;
use plannotator_tui_schema::Provenance;

use super::feedback::{Feedback, FeedbackPart, ReviewCounts, SendScope};
use super::{App, Mode};
use crate::delivery::{Clipboard, Delivery as _, DeliveryError};
use crate::gate::{Decision, Gate};
use crate::keys::Action;
use crate::store::{Location, Store};

#[cfg(test)]
mod tests;

pub(super) use crate::ui::send_button::SendState;

/// A send waiting on the agent picker that Send opened: what to send, and whether to close after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PickThen {
    scope: SendScope,
    close: bool,
}

impl App {
    fn default_scope(&self) -> SendScope {
        if self.is_file_review() { SendScope::Pending } else { SendScope::All }
    }

    pub(super) fn send_feedback(&mut self) -> Result<()> {
        self.send_feedback_scope(PickThen { scope: self.default_scope(), close: false }, true)
    }

    pub(super) fn resend_all(&mut self) -> Result<()> {
        self.send_feedback_scope(PickThen { scope: SendScope::All, close: false }, true)
    }

    /// Send what the picker that Send opened was waiting for, to the target now chosen.
    pub(super) fn send_picked(&mut self, then: PickThen) -> Result<()> {
        self.send_feedback_scope(then, false)
    }

    /// Send `then.scope`. With `ask`, a review that nothing named a target for opens the
    /// agent picker first when an agent is within reach, and sends once one is chosen.
    fn send_feedback_scope(&mut self, then: PickThen, ask: bool) -> Result<()> {
        let PickThen { scope, close } = then;
        let mut feedback = self.prepare_feedback(scope)?;
        if self.tree.is_some() {
            self.folder_counts = std::mem::take(&mut feedback.counts);
        }
        if feedback.count == 0 {
            self.derive_send_state();
            self.status = Some(if scope == SendScope::Pending {
                "nothing new to send".into()
            } else {
                "nothing to send".into()
            });
            self.quit |= close;
            return Ok(());
        }
        if ask {
            let folder = crate::ui::agent_picker::real(&self.review_folder());
            if let Some(picker) = self.reach.picker_for_send(self.delivery.as_ref(), &folder) {
                self.agent_picker = picker;
                self.pick_then = Some(then);
                self.mode = Mode::PickTarget;
                return Ok(());
            }
        }
        let target = self.delivery.describe();
        let retry = if scope == SendScope::All && self.is_file_review() { 'R' } else { 'E' };
        match self.delivery.deliver(&feedback.text) {
            Ok(()) => {
                let files = feedback.parts.len();
                let sent = (feedback.text.clone(), feedback.count);
                self.archive_submission(&mut feedback, "feedback");
                let errors = self.remember_delivery(&mut feedback, &target);
                self.derive_send_state();
                let verb = if self.delivery.is_agent() { "sent" } else { "copied" };
                let across = if self.tree.is_some() {
                    format!(" across {files} files")
                } else if files > 1 {
                    format!(" across {files} replies")
                } else {
                    String::new()
                };
                let mut status = format!("{verb} {} annotation(s){across} → {target}", feedback.count);
                if let Some(notice) = self.delivery.notice() {
                    let _ = write!(status, " · {notice}");
                }
                if !errors.is_empty() {
                    let _ = write!(
                        status,
                        "; could not save sent status: {} · next send may repeat it",
                        errors.join("; ")
                    );
                }
                self.status = Some(status);
                // A gated review ends with the first send: one review, one decision.
                if self.gate != Gate::Off {
                    let (feedback, count) = sent;
                    self.decision = Some(Decision::Annotated { feedback, count });
                    self.quit = true;
                }
            }
            Err(error) => {
                let copied = error.wants_copy() && self.copy_fallback(&feedback.text);
                self.status = Some(error.status(&target, copied, retry));
                self.derive_send_state();
                if let DeliveryError::Blocked(msg) = error {
                    self.send_state = SendState::Blocked(msg);
                }
            }
        }
        if close {
            self.quit = !self.has_unsent();
        }
        Ok(())
    }

    fn copy_fallback(&self, text: &str) -> bool {
        self.clipboard && Clipboard.deliver(text).is_ok()
    }

    /// Keep the ids from the body we delivered. Attempt every file even when one record
    /// cannot be saved, and refresh counts from any intervening changes.
    pub(super) fn remember_delivery(&mut self, feedback: &mut Feedback, target: &str) -> Vec<String> {
        let mut errors = Vec::new();
        for mut part in feedback.parts.drain(..) {
            if let Err(err) = self.record_feedback_delivery(&mut part, target) {
                let name = part
                    .path
                    .as_ref()
                    .map_or_else(|| self.open.source.name.clone(), |p| self.review_file_name(p));
                errors.push(format!("{name}: {err:#}"));
            }
            if self.tree.is_some()
                && let Some(path) = &part.path
            {
                self.folder_counts.insert(path.clone(), ReviewCounts::for_store(&part.store));
            }
            if let Some(index) = part.reply {
                if let Some(store) = self.reply_store_mut(index) {
                    *store = part.store;
                }
            } else if part.path.as_deref().is_none_or(|p| self.is_open(p)) {
                self.open.store = part.store;
            }
        }
        errors
    }

    fn record_feedback_delivery(&self, part: &mut FeedbackPart, target: &str) -> Result<()> {
        if self.is_file_review()
            && let Some(path) = &part.path
        {
            let latest = if self.is_open(path) {
                let location = Location::for_file(&self.data_dir, &self.project, path);
                Store::load(&location, &self.open.doc)?
            } else {
                self.load_review_file(path)?.1
            };
            let unchanged = part.store.same_review(&latest);
            part.store = latest;
            // The transport can block while another writer changes the shared record.
            // Keep that record intact; its newer notes were not in the delivered body.
            anyhow::ensure!(unchanged, "annotations changed while sending; kept the newer record");
        }
        part.store.record_delivery(target, &part.ids)
    }

    /// The shared submission history records only the selected feedback. Finishing a
    /// review has its own complete copies in the annotation record and does not rely on it.
    /// `decision` is the archive's word for it: `feedback` for a send, or a gate's decision.
    pub(super) fn archive_submission(&self, feedback: &mut Feedback, decision: &str) {
        use crate::archive::{self, Submission, Target};
        if !archive::enabled(|key| std::env::var(key).ok(), &self.data_dir) {
            return;
        }
        let (surface, target, annotations) = if let Some(tree) = &self.tree {
            ("annotate-folder", Target::file(tree.root()), Vec::new())
        } else {
            let annotations = std::mem::take(&mut feedback.annotations);
            match &self.open.source.provenance {
                Provenance::File { path } => ("annotate", Target::file(path), annotations),
                Provenance::AgentMessage { host, session, .. } => (
                    "annotate-last",
                    Target::agent(
                        archive::origin_label(host),
                        session.clone(),
                        (!self.message_transcript.is_empty()).then(|| self.message_transcript.clone()),
                    ),
                    annotations,
                ),
                _ => ("annotate", Target::default(), annotations),
            }
        };
        let origin = self.delivery.agent_host().map(|host| archive::origin_label(host).to_owned());
        archive::append(&Submission {
            data_dir: &self.data_dir,
            project: &self.project,
            surface,
            decision,
            origin,
            target,
            feedback: &feedback.text,
            annotations,
            count: feedback.count,
            images: feedback.images,
            now_ms: None,
        });
    }

    pub(super) fn send_count(&self) -> usize {
        if self.is_file_review() {
            self.review_counts().pending
        } else {
            self.replies().iter().map(|(_, open)| open.store.placed().len()).sum()
        }
    }

    pub(super) fn send_label(&self) -> String {
        let target = self.delivery.describe();
        let count = self.send_count();
        // Nothing named a target, but Send asks which agent first.
        let asks = self.reach.asks(self.delivery.as_ref());
        let key = match self.keys.hint(Action::Send) {
            key if key.is_empty() => String::new(),
            key => format!(" ({key})"),
        };
        match &self.send_state {
            SendState::Blocked(_) => return format!("{target} at a dialog · retry{key}"),
            SendState::Sent if self.delivery.is_agent() => return format!("Sent ✓ ▸ {target}"),
            SendState::Sent if asks => return "Sent ✓".to_owned(),
            SendState::Sent => return "Copied ✓".to_owned(),
            SendState::Ready => {}
        }
        if self.is_file_review() {
            let verb = if self.delivery.is_agent() || asks { "Send" } else { "Copy" };
            let across = if self.tree.is_some() {
                format!(" across {} files", self.pending_file_count())
            } else {
                String::new()
            };
            let to = if self.delivery.is_agent() {
                format!(" ▸ {target}")
            } else if asks {
                " ▸ choose agent".to_owned()
            } else {
                String::new()
            };
            return format!("{verb} {count} new{across}{to}{key}");
        }
        if self.delivery.is_agent() {
            format!("Send {count} to {target} ▸")
        } else if asks {
            format!("Send {count} ▸ choose agent")
        } else {
            format!("Copy {count} as feedback")
        }
    }

    pub(super) fn has_unsent(&self) -> bool {
        self.delivery.is_agent()
            && self.send_count() > 0
            && (self.is_file_review() || self.send_state != SendState::Sent)
    }

    pub(super) fn request_quit(&mut self) {
        if self.has_unsent() {
            self.mode = Mode::ConfirmQuit;
        } else {
            self.quit = true;
        }
    }

    /// Send, then quit unless something is still unsent. A refused send keeps the app open
    /// so the footer can say why; an empty review or a clipboard copy has nothing to wait for.
    /// When Send asks which agent first, the quit waits for that choice.
    pub(super) fn send_and_quit(&mut self) -> Result<()> {
        self.send_feedback_scope(PickThen { scope: self.default_scope(), close: true }, true)
    }

    pub(super) fn derive_send_state(&mut self) {
        let delivered = if self.is_file_review() {
            let counts = self.review_counts();
            counts.pending == 0 && counts.sent > 0
        } else {
            // Every reply with notes has had them delivered; a reply without notes has
            // nothing to send and does not hold the review back.
            let annotated: Vec<&Store> = self
                .replies()
                .into_iter()
                .map(|(_, open)| &open.store)
                .filter(|store| store.len() > 0)
                .collect();
            !annotated.is_empty() && annotated.iter().all(|store| store.all_delivered())
        };
        self.send_state = if delivered { SendState::Sent } else { SendState::Ready };
    }

    pub(super) fn mark_unsent(&mut self) {
        self.update_open_review_counts();
        self.derive_send_state();
    }
}
