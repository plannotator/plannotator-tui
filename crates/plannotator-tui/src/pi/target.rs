//! A Pi session as the Send target.

use std::cell::Cell;
use std::path::Path;

use super::{Disposition, PiEntry, SendError};
use crate::delivery::{Delivery, DeliveryError};

pub(crate) struct PiSession {
    entry: PiEntry,
    /// How Pi took the last send, for the footer notice.
    last: Cell<Option<Disposition>>,
}

/// `PLANNOTATOR_TUI_PI_PID`: the Pi that started this review (its `/plannotator-review`
/// command), by process id. Send goes to the session that Pi lists in the registry.
pub(crate) const PID_VAR: &str = "PLANNOTATOR_TUI_PI_PID";

impl PiSession {
    pub(crate) fn new(entry: PiEntry) -> Self {
        Self { entry, last: Cell::new(None) }
    }

    /// The live session of the Pi [`PID_VAR`] names, under the registry `root`: its newest
    /// entry that answers. Only that Pi's entries are asked; the others are left alone.
    /// `None` when the variable is not set; an error when it is and that Pi lists no live
    /// session, so Send refuses ([`PiGone`]) rather than fall back to what Pi's own
    /// environment names: the pane Pi runs in, which the review has taken over.
    pub(crate) fn from_lookup(
        env: impl Fn(&str) -> Option<String>,
        root: &Path,
    ) -> Option<Result<Self, PiGone>> {
        let value = env(PID_VAR)?;
        let Ok(pid) = value.trim().parse::<u32>() else {
            return Some(Err(PiGone(format!("{PID_VAR}={value:?} names no process"))));
        };
        Some(super::live_entry_for_pid(root, pid).map(Self::new).ok_or_else(|| {
            PiGone(format!(
                "the Pi that started this review (pid {pid}) lists no live session; send from a review \
                 started again with /plannotator-review"
            ))
        }))
    }
}

/// The Send target of a review Pi started, once that Pi cannot be found: Send says why and
/// sends nothing, and the notes stay unsent.
pub(crate) struct PiGone(String);

impl Delivery for PiGone {
    fn describe(&self) -> String {
        "pi · not found".to_owned()
    }

    fn is_agent(&self) -> bool {
        true
    }

    fn agent_host(&self) -> Option<&str> {
        Some("pi")
    }

    fn deliver(&self, _feedback: &str) -> Result<(), DeliveryError> {
        Err(DeliveryError::Unavailable(self.0.clone()))
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

    fn agent_pid(&self) -> Option<u32> {
        Some(self.entry.pid)
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
