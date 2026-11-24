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

    /// The live session of the Pi [`PID_VAR`] names, under the registry `root`; `None` when
    /// it is not set or that Pi lists no live session.
    pub(crate) fn from_lookup(env: impl Fn(&str) -> Option<String>, root: &Path) -> Option<Self> {
        let pid: u32 = env(PID_VAR)?.trim().parse().ok()?;
        super::live_pi_sessions(root).into_iter().find(|entry| entry.pid == pid).map(Self::new)
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
