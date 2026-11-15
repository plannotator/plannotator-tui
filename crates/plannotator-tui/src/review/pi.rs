//! The Pi link in a code review (`docs/spec-pi-link.md`): the registry lists the changed
//! files, and a Pi session can leave a note on one.
//!
//! A note names an absolute path in the repository. `line` is a line of the new file (the
//! working file, the index, or the commit); `quote` is text on or near it. The note becomes
//! a line comment on the new side when the diff shows that line, else a file comment when
//! neither is given. A line the diff does not show is `not_in_diff`, so Pi can say why.

use std::path::{Path, PathBuf};

use plannotator_tui_review::{CodeAnnotation, FilePatch, Side};
use serde_json::{Value, json};

use super::{Mode, Screen};
use crate::pi::link::{self, Link, Note};

/// What `author` is for every Pi note: the agent, by id.
const AUTHOR: &str = "pi";

impl Screen {
    /// List this review in the registry and listen for notes. Without a writable data
    /// folder the review works as before, without the link.
    pub(crate) fn start_pi_link(&mut self) {
        self.pi_link = Link::start(&crate::pi::root(), self.real_root(), self.pi_files()).ok();
    }

    fn real_root(&self) -> PathBuf {
        std::fs::canonicalize(self.root()).unwrap_or_else(|_| self.root().to_path_buf())
    }

    /// The changed files, as absolute paths.
    pub(super) fn pi_files(&self) -> Vec<PathBuf> {
        let root = self.real_root();
        self.capture.files.iter().map(|file| root.join(&file.path)).collect()
    }

    /// Take the notes that arrived since the last poll. True when the screen changed.
    pub(super) fn poll_pi(&mut self) -> bool {
        let Some(link) = &self.pi_link else { return false };
        link.set_files(self.pi_files());
        let notes = link.take_notes();
        let changed = !notes.is_empty();
        for note in notes {
            let reply = self.take_pi_note(&note);
            note.answer(reply);
        }
        changed
    }

    pub(super) fn take_pi_note(&mut self, note: &Note) -> Value {
        if matches!(self.mode, Mode::Compose(_)) {
            return link::error("busy", "the person is typing a note; try again shortly");
        }
        let Some(file) = self.changed_file(&note.file) else {
            return link::error("not_open", "that file has no changes in this code review");
        };
        let key = self.capture.patch_key.clone();
        let made = match place(file, note.quote.as_deref(), note.line) {
            Ok(Some(line)) => {
                CodeAnnotation::line_comment(file, Side::New, line, line, note.body.clone(), &key)
            }
            Ok(None) => CodeAnnotation::file_comment(file, note.body.clone(), &key),
            Err(reply) => return reply,
        };
        let mut made = made;
        made.author = Some(AUTHOR.to_owned());
        made.author_name = Some(note.author.clone());
        match self.add_note(made) {
            Ok(()) => {
                self.status = Some(format!("note from {}", note.author));
                json!({"ok": true})
            }
            Err(err) => link::error("failed", &format!("{err:#}")),
        }
    }

    /// The changed file at absolute `path`, comparing real paths.
    fn changed_file(&self, path: &Path) -> Option<&FilePatch> {
        let wanted = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let root = self.real_root();
        // A deleted file has no real path any more; its parent folder still does.
        let wanted = match (wanted.parent(), wanted.file_name()) {
            (Some(parent), Some(name)) if !wanted.exists() => {
                std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf()).join(name)
            }
            _ => wanted,
        };
        let relative = wanted.strip_prefix(&root).ok()?;
        let relative = relative.to_str()?.replace('\\', "/");
        self.capture.file(&relative)
    }
}

/// The new-side line a note goes on (`None`: the whole file), or the reply that says why
/// it cannot go anywhere.
fn place(file: &FilePatch, quote: Option<&str>, line: Option<usize>) -> Result<Option<u32>, Value> {
    let line = line.and_then(|n| u32::try_from(n).ok());
    let not_in_diff = |line: u32| {
        link::error(
            "not_in_diff",
            &format!(
                "line {line} of the new {} is not in the diff; comment on a changed line or the lines around one, or leave out line for a note on the whole file",
                file.path
            ),
        )
    };
    match (quote, line) {
        (None, None) => Ok(None),
        (None, Some(line)) if file.shows(Side::New, line, line) => Ok(Some(line)),
        (None, Some(line)) => Err(not_in_diff(line)),
        (Some(quote), line) => {
            let found = file.lines(Side::New).filter(|(_, text)| text.contains(quote)).map(|(n, _)| n);
            let best = match line {
                Some(line) => found.min_by_key(|n| n.abs_diff(line)),
                None => found.into_iter().next(),
            };
            best.map(Some).ok_or_else(|| {
                link::error(
                    "quote_not_found",
                    "the quote is not on any new-side line the diff shows; copy it exactly from one changed or nearby line",
                )
            })
        }
    }
}
