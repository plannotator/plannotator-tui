//! Re-finding a note after the code changed.
//!
//! Each line note is checked against its side's full file content (the working file, the
//! index blob, or the file at a commit), using the text and context it recorded:
//!
//! 1. **Still there**: the anchor text and context read the same at the same lines.
//! 2. **Moved**: exactly one other place in the file reads the same, text and context.
//!    The browser never moves a note; a local review shifts lines on every edit above a
//!    note, and a unique full-context match is not a guess.
//! 3. **Not in the diff**: found (in place or moved), but the current patch does not show
//!    all of its lines, so no marker can be drawn.
//! 4. **Outdated**: anything else. The note keeps its lines and is never dropped.
//!
//! Context follows the browser's record: a `null` context line was not in the patch when
//! the note was made, so it says nothing and matches any line. The hunk's function context
//! is compared only when both the note and the current patch have one. Lines compare with
//! a trailing `\r` ignored, so a CRLF working file matches the LF text git diffs.

use crate::annotation::{AnchorContext, CodeAnnotation, NoteScope, Side, Status};
use crate::patch::FilePatch;

/// What the diff looks like now, for one note's file and side.
#[derive(Debug, Clone, Copy)]
pub struct Current<'a> {
    /// The side's full file content; `None` when the file does not exist on that side.
    pub content: Option<&'a str>,
    /// This file's section of the current patch; `None` when the diff no longer touches it.
    pub patch: Option<&'a FilePatch>,
    /// The current patch's key. A note with no recorded anchor text is trusted only on the
    /// patch it was made on.
    pub patch_key: &'a str,
}

/// Where a note is now. Lines are 1-based and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    StillThere,
    Moved { line_start: u32, line_end: u32 },
    NotInDiff { line_start: u32, line_end: u32 },
    Outdated,
}

impl Resolution {
    pub fn status(self) -> Status {
        match self {
            Self::StillThere => Status::StillThere,
            Self::Moved { .. } => Status::Moved,
            Self::NotInDiff { .. } => Status::NotInDiff,
            Self::Outdated => Status::Outdated,
        }
    }
}

/// Resolve `note` against the current state of its file.
pub fn resolve(note: &CodeAnnotation, current: &Current<'_>) -> Resolution {
    let (start, end) = (note.line_start, note.line_end);
    match note.scope {
        NoteScope::General => Resolution::StillThere,
        NoteScope::File if current.patch.is_some() => Resolution::StillThere,
        NoteScope::File => Resolution::NotInDiff { line_start: start, line_end: end },
        NoteScope::Line => resolve_lines(note, current),
    }
}

fn resolve_lines(note: &CodeAnnotation, current: &Current<'_>) -> Resolution {
    let (start, end) = (note.line_start, note.line_end);
    let place = |first: u32, last: u32| {
        let shown = current.patch.is_some_and(|p| p.shows(note.side, first, last));
        match (shown, first == start) {
            (true, true) => Resolution::StillThere,
            (true, false) => Resolution::Moved { line_start: first, line_end: last },
            (false, _) => Resolution::NotInDiff { line_start: first, line_end: last },
        }
    };
    let Some(anchor) = note.anchor_text.as_deref() else {
        let same_patch = note.plannotator_tui.patch_key.as_deref() == Some(current.patch_key);
        return if same_patch { place(start, end) } else { Resolution::Outdated };
    };
    let Some(content) = current.content else { return Resolution::Outdated };
    let wanted: Vec<&str> = anchor.split('\n').collect();
    let span = wanted.len() as u32;
    if start < 1 || end.checked_sub(start).map(|d| d + 1) != Some(span) {
        return Resolution::Outdated;
    }
    let file = file_lines(content);
    let probe = Probe { file: &file, wanted: &wanted, side: note.side, patch: current.patch };
    let recorded = note.anchor_context.as_ref();
    if probe.matches(start, recorded) {
        return place(start, end);
    }
    // A move needs the recorded context: without one, only the stored lines count.
    if recorded.is_none() {
        return Resolution::Outdated;
    }
    let last_start = (file.len() as u32 + 1).saturating_sub(span);
    let mut found = (1..=last_start).filter(|&at| probe.matches(at, recorded));
    match (found.next(), found.next()) {
        (Some(at), None) => place(at, at + span - 1),
        _ => Resolution::Outdated,
    }
}

/// Move `note` to where `resolution` found it and record the outcome. Returns whether the
/// note changed. Moving is not an edit: `updatedAt` stays, so a sent note stays sent.
pub fn apply(note: &mut CodeAnnotation, resolution: Resolution) -> bool {
    let before = note.clone();
    if let Resolution::Moved { line_start, line_end } | Resolution::NotInDiff { line_start, line_end } =
        resolution
    {
        note.line_start = line_start;
        note.line_end = line_end;
    }
    note.outdated = resolution == Resolution::Outdated;
    note.plannotator_tui.resolution = Some(resolution.status());
    *note != before
}

/// One candidate check: the anchor text at a position, and the context around it.
struct Probe<'a> {
    file: &'a [&'a str],
    wanted: &'a [&'a str],
    side: Side,
    patch: Option<&'a FilePatch>,
}

impl Probe<'_> {
    fn matches(&self, at: u32, context: Option<&AnchorContext>) -> bool {
        let text_matches =
            self.wanted.iter().enumerate().all(|(i, want)| {
                self.line(u64::from(at) + i as u64).is_some_and(|have| same_line(have, want))
            });
        text_matches && context.is_none_or(|c| self.context_matches(at, c))
    }

    fn context_matches(&self, at: u32, context: &AnchorContext) -> bool {
        let first = u64::from(at);
        let after_first = first + self.wanted.len() as u64;
        let before_first = first.checked_sub(context.before.len() as u64);
        let before = context.before.iter().enumerate().all(|(k, want)| {
            want.as_deref().is_none_or(|want| {
                before_first
                    .is_some_and(|b| self.line(b + k as u64).is_some_and(|have| same_line(have, want)))
            })
        });
        let after = context.after.iter().enumerate().all(|(k, want)| {
            want.as_deref()
                .is_none_or(|want| self.line(after_first + k as u64).is_some_and(|h| same_line(h, want)))
        });
        let hunk = match (context.hunk.as_deref(), self.patch.and_then(|p| p.hunk_context(self.side, at))) {
            (Some(recorded), Some(now)) if !recorded.is_empty() && !now.is_empty() => recorded == now,
            _ => true,
        };
        before && after && hunk
    }

    /// 1-based line `n` of the file.
    fn line(&self, n: u64) -> Option<&str> {
        let index = usize::try_from(n.checked_sub(1)?).ok()?;
        self.file.get(index).copied()
    }
}

/// The file's lines; a final newline does not start another line.
fn file_lines(content: &str) -> Vec<&str> {
    let body = content.strip_suffix('\n').unwrap_or(content);
    if content.is_empty() { Vec::new() } else { body.split('\n').collect() }
}

fn same_line(a: &str, b: &str) -> bool {
    a.strip_suffix('\r').unwrap_or(a) == b.strip_suffix('\r').unwrap_or(b)
}

#[cfg(test)]
mod tests;
