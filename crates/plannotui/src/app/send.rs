//! Sending feedback to the delivery target and the state the Send button shows.

use anyhow::Result;

use super::{App, Focus};
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
    /// Send feedback: the open file's, or every annotated file's when the tree has focus.
    pub(super) fn send_feedback(&mut self) -> Result<()> {
        let text = if self.focus == Focus::Tree { self.folder_feedback()? } else { self.feedback() };
        let count = self.send_count();
        let target = self.delivery.describe();
        match self.delivery.deliver(&text) {
            Ok(()) => {
                self.open.store.record_delivery(&target)?;
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

    /// Annotations the next send covers: the open file's, or the folder's when the tree
    /// has focus.
    pub(super) fn send_count(&self) -> usize {
        match (&self.tree, self.focus) {
            (Some(tree), Focus::Tree) => tree.rows.iter().filter(|r| !r.is_dir).map(|r| r.annotations).sum(),
            _ => self.open.store.placed().len(),
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
    #[allow(dead_code, reason = "wired by the quit confirmation")]
    pub(super) fn has_unsent(&self) -> bool {
        self.delivery.is_agent() && self.open.store.len() > 0 && self.send_state != SendState::Sent
    }

    /// Recompute the send state from the record (on load and file switch).
    pub(super) fn derive_send_state(&mut self) {
        self.send_state = if self.open.store.all_delivered() { SendState::Sent } else { SendState::Ready };
    }

    /// Any annotation change makes the record unsent again.
    pub(super) fn mark_unsent(&mut self) {
        self.send_state = SendState::Ready;
    }
}
