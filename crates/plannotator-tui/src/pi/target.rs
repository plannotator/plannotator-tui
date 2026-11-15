//! A Pi session as the Send target.

use std::cell::Cell;

use super::{Disposition, PiEntry, SendError};
use crate::delivery::{Delivery, DeliveryError};

pub(crate) struct PiSession {
    entry: PiEntry,
    /// How Pi took the last send, for the footer notice.
    last: Cell<Option<Disposition>>,
}

impl PiSession {
    pub(crate) fn new(entry: PiEntry) -> Self {
        Self { entry, last: Cell::new(None) }
    }
}

impl Delivery for PiSession {
    fn describe(&self) -> String {
        format!("pi · {}", self.entry.label())
    }

    fn is_agent(&self) -> bool {
        true
    }

    fn agent_host(&self) -> Option<&str> {
        Some("pi")
    }

    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError> {
        self.last.set(None);
        match super::send(&self.entry, feedback) {
            Ok(disposition) => {
                self.last.set(Some(disposition));
                Ok(())
            }
            Err(SendError::Gone) => Err(DeliveryError::Unavailable(format!(
                "pi · {} has closed; pick another Pi",
                self.entry.label()
            ))),
            Err(SendError::Failed(message)) => Err(DeliveryError::Failed(anyhow::anyhow!(message))),
        }
    }

    fn notice(&self) -> Option<String> {
        (self.last.get() == Some(Disposition::Queued)).then(|| "Pi is busy; it reads this next".to_owned())
    }
}
