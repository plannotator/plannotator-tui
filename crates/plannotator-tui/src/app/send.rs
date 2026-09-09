//! Sending feedback and the state of the Send button.

use std::fmt::Write as _;

use anyhow::Result;
use plannotator_tui_schema::Provenance;

use super::feedback::{Feedback, FeedbackPart, ReviewCounts, SendScope};
use super::{App, Mode};
use crate::delivery::{Clipboard, Delivery as _, DeliveryError};
use crate::store::{Location, Store};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SendState {
    Ready,
    Sent,
    Blocked(String),
}

impl App {
    pub(super) fn send_feedback(&mut self) -> Result<()> {
        let scope = if self.is_file_review() { SendScope::Pending } else { SendScope::All };
        self.send_feedback_scope(scope)
    }

    pub(super) fn resend_all(&mut self) -> Result<()> {
        self.send_feedback_scope(SendScope::All)
    }

    fn send_feedback_scope(&mut self, scope: SendScope) -> Result<()> {
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
            return Ok(());
        }
        let target = self.delivery.describe();
        let retry = if scope == SendScope::All && self.is_file_review() { 'R' } else { 'E' };
        match self.delivery.deliver(&feedback.text) {
            Ok(()) => {
                let files = feedback.parts.len();
                self.archive_submission(&mut feedback);
                let errors = self.remember_delivery(&mut feedback, &target);
                self.derive_send_state();
                let verb = if self.delivery.is_agent() { "sent" } else { "copied" };
                let across =
                    if self.tree.is_some() { format!(" across {files} files") } else { String::new() };
                let mut status = format!("{verb} {} annotation(s){across} → {target}", feedback.count);
                if !errors.is_empty() {
                    let _ = write!(
                        status,
                        "; could not save sent status: {} · next send may repeat it",
                        errors.join("; ")
                    );
                }
                self.status = Some(status);
            }
            Err(DeliveryError::Blocked(msg)) => {
                let copied = if self.copy_fallback(&feedback.text) { " · copied to clipboard" } else { "" };
                self.status = Some(format!("{target} is at a dialog{copied} · {retry} retry"));
                self.send_state = SendState::Blocked(msg);
            }
            Err(DeliveryError::Unavailable(msg)) => {
                let copied = if self.copy_fallback(&feedback.text) { " · copied to clipboard" } else { "" };
                self.status = Some(format!("no agent to send to ({msg}){copied} · {retry} retry"));
                self.derive_send_state();
            }
            Err(DeliveryError::Failed(err)) => {
                self.status = Some(format!("send failed: {err:#} · {retry} retry"));
                self.derive_send_state();
            }
        }
        Ok(())
    }

    fn copy_fallback(&self, text: &str) -> bool {
        self.clipboard && Clipboard.deliver(text).is_ok()
    }

    /// Keep the ids from the body we delivered. Attempt every file even when one record
    /// cannot be saved, and refresh counts from any intervening changes.
    fn remember_delivery(&mut self, feedback: &mut Feedback, target: &str) -> Vec<String> {
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
            if part.path.as_deref().is_none_or(|p| self.is_open(p)) {
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
    fn archive_submission(&self, feedback: &mut Feedback) {
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
            origin,
            target,
            feedback: &feedback.text,
            annotations,
            count: feedback.count,
            now_ms: None,
        });
    }

    pub(super) fn send_count(&self) -> usize {
        if self.is_file_review() { self.review_counts().pending } else { self.open.store.placed().len() }
    }

    pub(super) fn send_label(&self) -> String {
        let target = self.delivery.describe();
        let count = self.send_count();
        if self.is_file_review() {
            let verb = if self.delivery.is_agent() { "Send" } else { "Copy" };
            let across = if self.tree.is_some() {
                format!(" across {} files", self.pending_file_count())
            } else {
                String::new()
            };
            let to = if self.delivery.is_agent() { format!(" ▸ {target}") } else { String::new() };
            return format!("{verb} {count} new{across}{to} (E)");
        }
        if self.delivery.is_agent() {
            match &self.send_state {
                SendState::Ready => format!("Send {count} to {target} ▸"),
                SendState::Sent => format!("Sent ▸ {target}"),
                SendState::Blocked(_) => format!("{target} at a dialog · click to retry"),
            }
        } else {
            match &self.send_state {
                SendState::Sent => "Copied".to_owned(),
                SendState::Ready | SendState::Blocked(_) => format!("Copy {count} as feedback"),
            }
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

    pub(super) fn derive_send_state(&mut self) {
        let delivered = if self.is_file_review() {
            let counts = self.review_counts();
            counts.pending == 0 && counts.sent > 0
        } else {
            self.open.store.all_delivered()
        };
        self.send_state = if delivered { SendState::Sent } else { SendState::Ready };
    }

    pub(super) fn mark_unsent(&mut self) {
        self.update_open_review_counts();
        self.derive_send_state();
    }
}
