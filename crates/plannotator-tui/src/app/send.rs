//! Sending feedback to the delivery target and the state the Send button shows.

use anyhow::Result;

use super::{App, Mode};
use crate::delivery::{Clipboard, Delivery as _, DeliveryError};

/// What the Send button says. Re-derived from the store on load and file switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SendState {
    /// Something to send (or nothing yet, in which case the button is dimmed).
    Ready,
    /// Everything on record has been sent; nothing changed since.
    Sent,
    /// The last send was refused because the agent is at a dialog.
    Blocked(String),
}

impl App {
    /// Send feedback: every annotated file's in folder mode, else the open file's.
    pub(super) fn send_feedback(&mut self) -> Result<()> {
        let text = if self.tree.is_some() { self.folder_feedback()? } else { self.feedback() };
        let count = self.send_count();
        let target = self.delivery.describe();
        match self.delivery.deliver(&text) {
            Ok(()) => {
                self.record_delivery(&target)?;
                self.send_state = SendState::Sent;
                self.status = Some(format!("sent {count} annotation(s) → {target}"));
            }
            Err(DeliveryError::Blocked(msg)) => {
                self.copy_fallback(&text);
                self.status =
                    Some(format!("{target} is at a dialog — copied to clipboard instead; E retries"));
                self.send_state = SendState::Blocked(msg);
            }
            Err(DeliveryError::Unavailable(msg)) => {
                self.copy_fallback(&text);
                self.status = Some(format!("no agent to send to ({msg}) — copied to clipboard"));
            }
            Err(DeliveryError::Failed(err)) => {
                self.status = Some(format!("send failed: {err:#}"));
            }
        }
        Ok(())
    }

    fn copy_fallback(&self, text: &str) {
        if self.clipboard {
            let _ = Clipboard.deliver(text);
        }
    }

    /// Annotations the next send covers: the whole folder's, or the open file's.
    pub(super) fn send_count(&self) -> usize {
        match &self.tree {
            Some(tree) => tree.rows.iter().filter(|r| !r.is_dir).map(|r| r.annotations).sum(),
            None => self.open.store.placed().len(),
        }
    }

    /// Text for the Send button.
    pub(super) fn send_label(&self) -> String {
        let target = self.delivery.describe();
        let count = self.send_count();
        if self.delivery.is_agent() {
            match &self.send_state {
                SendState::Ready => format!("Send {count} to {target} ▸"),
                SendState::Sent => format!("Sent ▸ {target}"),
                SendState::Blocked(_) => format!("{target} at a dialog · copied · click to retry"),
            }
        } else {
            match &self.send_state {
                SendState::Sent => "Copied".to_owned(),
                SendState::Ready | SendState::Blocked(_) => format!("Copy {count} as feedback"),
            }
        }
    }

    /// True when an agent is waiting on feedback that has not been sent since it changed.
    pub(super) fn has_unsent(&self) -> bool {
        self.delivery.is_agent() && self.send_count() > 0 && self.send_state != SendState::Sent
    }

    /// Quit, unless an agent is still waiting on feedback: then ask in the footer first.
    pub(super) fn request_quit(&mut self) {
        if self.has_unsent() {
            self.mode = Mode::ConfirmQuit;
        } else {
            self.quit = true;
        }
    }

    /// Recompute the send state from the record (on load and file switch).
    pub(super) fn derive_send_state(&mut self) {
        let delivered = match &self.tree {
            Some(_) => self.folder_all_delivered().unwrap_or(false),
            None => self.open.store.all_delivered(),
        };
        self.send_state = if delivered { SendState::Sent } else { SendState::Ready };
    }

    /// Any annotation change makes the record unsent again.
    pub(super) fn mark_unsent(&mut self) {
        self.send_state = SendState::Ready;
    }
}
