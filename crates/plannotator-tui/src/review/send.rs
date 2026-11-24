//! Sending notes, the way a file review sends them: `E` sends what is new or edited, `R`
//! resends everything, a failed send keeps the notes pending and copies the text, and
//! finishing archives what was sent.

use std::fmt::Write as _;

use anyhow::Result;
use plannotator_tui_review::{CodeAnnotation, NoteScope, agent_feedback, pull_agent_feedback};

use super::{Mode, Screen};
use crate::archive::{self, AnnotationRecord, ReviewTarget, Submission, Target};
use crate::delivery::{Clipboard, Delivery as _, DeliveryError};
use crate::ui::send_button::{LastSend, SendState};

/// The largest feedback a send carries. Highlighted code is capped per note, so normal
/// reviews stay far below this; a larger one is refused rather than cut.
pub(super) const MAX_SEND_BYTES: usize = 256 * 1024;

/// A send waiting on the agent picker that Send opened: every note or the new ones, and
/// whether to close after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PickThen {
    all: bool,
    close: bool,
}

impl Screen {
    /// The feedback text for `notes`, as the agent receives it.
    pub(super) fn feedback(&self, notes: &[&CodeAnnotation]) -> String {
        if let Some(pull) = &self.capture.pull {
            let clone = pull.clone.as_ref().map(|root| root.display().to_string());
            return pull_agent_feedback(&pull.view, clone.as_deref(), notes);
        }
        let diff = self.capture.scope.describe(self.capture.commit_subject.as_deref());
        agent_feedback(&self.root().display().to_string(), &diff, notes)
    }

    /// Send new and edited notes, or with `all` every note. When nothing named a target and
    /// an agent is within reach, ask which one first.
    pub(super) fn send(&mut self, all: bool) {
        self.send_asking(PickThen { all, close: false });
    }

    /// Send what the picker that Send opened was waiting for, to the target now chosen.
    pub(super) fn send_picked(&mut self, then: PickThen) {
        self.deliver_notes(then.all);
        if then.close {
            self.quit = !self.has_unsent();
        }
    }

    /// Open the agent picker when Send should ask first and there is something to send;
    /// else send at once. True when the picker opened.
    fn send_asking(&mut self, then: PickThen) -> bool {
        let empty =
            if then.all { self.store.annotations().is_empty() } else { self.store.pending().is_empty() };
        if !empty && let Some(picker) = self.reach.picker_for_send(self.delivery.as_ref(), &self.real_root())
        {
            self.picker = picker;
            self.pick_then = Some(then);
            self.mode = Mode::PickTarget;
            return true;
        }
        self.deliver_notes(then.all);
        false
    }

    fn deliver_notes(&mut self, all: bool) {
        let notes: Vec<&CodeAnnotation> =
            if all { self.store.annotations().iter().collect() } else { self.store.pending() };
        if notes.is_empty() {
            self.status = Some(if all { "nothing to send" } else { "nothing new to send" }.into());
            return;
        }
        let text = self.feedback(&notes);
        if text.len() > MAX_SEND_BYTES {
            self.status = Some(format!(
                "the feedback is {} KiB, over the {} KiB one send carries; send fewer notes",
                text.len().div_ceil(1024),
                MAX_SEND_BYTES / 1024
            ));
            return;
        }
        let ids: Vec<String> = notes.iter().map(|n| n.id.clone()).collect();
        let records = notes.iter().map(|n| record(n)).collect();
        let target = self.delivery.describe();
        let retry = if all { 'R' } else { 'E' };
        match self.delivery.deliver(&text) {
            Ok(()) => {
                self.archive(&text, records, ids.len());
                let saved = self.store.record_delivery(&target, &ids);
                self.last_send = LastSend::to(self.delivery.is_agent());
                let verb = if self.delivery.is_agent() { "sent" } else { "copied" };
                let mut status = format!("{verb} {} note(s) → {target}", ids.len());
                if let Some(notice) = self.delivery.notice() {
                    let _ = write!(status, " · {notice}");
                }
                if let Err(err) = saved {
                    let _ = write!(status, "; could not save sent status: {err} · next send may repeat it");
                }
                self.status = Some(status);
                self.sync_markers();
                self.derive_send_state();
            }
            Err(error) => {
                let copied = error.wants_copy() && self.clipboard && Clipboard.deliver(&text).is_ok();
                self.status = Some(error.status(&target, copied, retry));
                self.derive_send_state();
                if let DeliveryError::Blocked(msg) = error {
                    self.send_state = SendState::Blocked(msg);
                }
            }
        }
    }

    /// The shared feedback archive, as the browser's code review writes it.
    fn archive(&self, text: &str, annotations: Vec<AnnotationRecord>, count: usize) {
        if !archive::enabled(|key| std::env::var(key).ok(), &self.data_dir) {
            return;
        }
        let pull = self.capture.pull.as_ref();
        let review = ReviewTarget {
            vcs_type: "git",
            diff_type: self.capture.scope.diff_type(),
            // A PR's diff is merge-base(base, head)..head: its base commit, and `gitRef` and
            // `pr` as the browser's PR review names them.
            base: pull
                .map(|p| p.view.base_sha.clone())
                .or_else(|| self.capture.scope.base().map(str::to_owned)),
            git_ref: pull.map(|p| format!("PR #{}", p.view.number)),
            pr: pull.map(|p| archive::PrTarget {
                provider: "github",
                repo: p.view.repo.slug(),
                number: p.view.number,
            }),
            cwd: self.root().display().to_string(),
            changed_files: self.capture.files.len(),
            patch_bytes: self.capture.patch_bytes(),
        };
        let origin = self.delivery.agent_host().map(|host| archive::origin_label(host).to_owned());
        archive::append(&Submission {
            data_dir: &self.data_dir,
            project: &self.project,
            surface: "review",
            decision: "feedback",
            origin,
            target: Target::review(review),
            feedback: text,
            annotations,
            count,
            images: 0,
            now_ms: None,
        });
    }

    /// Whether quitting now would leave notes the agent has not had.
    pub(super) fn has_unsent(&self) -> bool {
        self.delivery.is_agent() && !self.store.pending().is_empty()
    }

    /// Quit, or ask first when notes are unsent here or on a review this session left
    /// ([`super::pick::shown::Switched`]).
    pub(super) fn request_quit(&mut self) {
        self.count_unsent_elsewhere();
        if self.has_unsent() || !self.switched.unsent.is_empty() {
            self.mode = Mode::ConfirmQuit;
        } else {
            self.quit = true;
        }
    }

    /// Send, then quit unless something is still unsent. A refused send keeps the review
    /// open so the footer can say why.
    /// When Send asks which agent first, the quit waits for that choice.
    pub(super) fn send_and_quit(&mut self) {
        if !self.store.pending().is_empty() && self.send_asking(PickThen { all: false, close: true }) {
            return;
        }
        self.quit = !self.has_unsent();
    }

    /// Finish: archive every sent, unchanged note. Pending notes stay.
    pub(super) fn finish(&mut self) {
        let ids = match self.store.archive_sent() {
            Ok(ids) => ids,
            Err(err) => {
                self.status = Some(format!("could not finish: {err}"));
                return;
            }
        };
        self.status = Some(if ids.is_empty() {
            "no sent notes to archive".to_owned()
        } else {
            format!("archived {} note(s)", ids.len())
        });
        if !ids.is_empty() {
            self.undo = ids;
        }
        self.after_note_change();
    }

    pub(super) fn undo_finish(&mut self) -> Result<()> {
        if self.undo.is_empty() {
            self.status = Some("nothing to undo".into());
            return Ok(());
        }
        let restored = self.store.restore_archived(&std::mem::take(&mut self.undo))?;
        self.place_notes()?;
        self.status = Some(format!("restored {restored} note(s)"));
        self.after_note_change();
        Ok(())
    }

    /// The Send button's words.
    pub(super) fn send_label(&self) -> String {
        let count = self.store.pending().len();
        // Nothing named a target, but Send asks which agent first.
        let asks = self.reach.asks(self.delivery.as_ref());
        let verb = if self.delivery.is_agent() || asks { "Send" } else { "Copy" };
        let to = if self.delivery.is_agent() {
            format!(" ▸ {}", self.delivery.describe())
        } else if asks {
            " ▸ choose agent".to_owned()
        } else {
            String::new()
        };
        let key = match self.keys.hint(crate::keys::Action::Send) {
            key if key.is_empty() => String::new(),
            key => format!(" ({key})"),
        };
        match &self.send_state {
            SendState::Blocked(_) => format!("{} at a dialog · retry{key}", self.delivery.describe()),
            SendState::Sent if self.delivery.is_agent() => format!("Sent ✓{to}"),
            SendState::Sent if self.last_send == LastSend::Clipboard => "Copied ✓".to_owned(),
            SendState::Sent => "Sent ✓".to_owned(),
            SendState::Ready => format!("{verb} {count} new{to}{key}"),
        }
    }
}

/// A note as the archive records it.
fn record(note: &CodeAnnotation) -> AnnotationRecord {
    let line = note.scope == NoteScope::Line;
    AnnotationRecord {
        id: Some(note.id.clone()),
        kind: Some(
            serde_json::to_value(note.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
        ),
        text: note.text.clone().filter(|t| !t.is_empty()),
        original_text: note.original_code.clone().or_else(|| note.anchor_text.clone()),
        file: Some(note.file_path.clone()),
        line_start: line.then_some(note.line_start),
        line_end: line.then_some(note.line_end),
        side: line.then(|| note.side.as_str().to_owned()),
        scope: Some(
            serde_json::to_value(note.scope)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
        ),
        author: note.author_name.clone(),
    }
}
