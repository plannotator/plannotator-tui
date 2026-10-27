//! Local images on notes: `attach-image` asks for a path, `remove-image` drops the last one.
//!
//! An image is a reference to a file on this machine. Nothing is copied or uploaded; the
//! feedback names the absolute path and the agent opens it from there.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use plannotator_tui_schema::Provenance;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::compose::{Compose, ComposeAction};
use super::{App, Focus, Mode};
use crate::workspace_paths;

/// Extensions accepted as images, lowercase.
const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

impl App {
    /// The note `attach-image` and `remove-image` act on: the rail's selected note, or in
    /// the document the selected block's note (the rail's selection when it is in the block).
    pub(super) fn current_note(&self) -> Option<String> {
        let placed = self.open.store.placed();
        if self.focus == Focus::Rail {
            return placed.get(self.rail_cursor).map(|p| p.annotation.id.clone());
        }
        let in_block = |index: &usize| {
            placed
                .get(*index)
                .is_some_and(|p| self.open.doc.block_containing(p.range.start) == Some(self.selected))
        };
        let index = Some(self.rail_cursor).filter(in_block).or_else(|| (0..placed.len()).find(in_block))?;
        placed.get(index).map(|p| p.annotation.id.clone())
    }

    /// Open the one-line path prompt for the current note.
    pub(super) fn begin_attach_image(&mut self) {
        let Some(id) = self.current_note() else {
            self.status = Some("no note here to attach an image to".into());
            return;
        };
        self.compose = Compose::default();
        self.status = None;
        self.mode = Mode::AttachImage(id);
    }

    pub(super) fn remove_image(&mut self) -> Result<()> {
        let Some(id) = self.current_note() else { return Ok(()) };
        match self.open.store.remove_last_image(&id)? {
            Some(path) => {
                self.mark_unsent();
                self.status = Some(format!("image removed: {}", file_name(&path)));
            }
            None => self.status = Some("this note has no images".into()),
        }
        Ok(())
    }

    /// Keys in the path prompt: Enter attaches, Esc cancels, the rest edit the one line.
    pub(super) fn image_prompt_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter => return self.submit_image_prompt(),
            // The comment box's new-line key; a path has one line.
            KeyCode::Char('j' | 'J') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            _ => {
                if self.compose.handle_key(key) == ComposeAction::Cancel {
                    self.mode = Mode::Browse;
                }
            }
        }
        Ok(())
    }

    /// A paste or a dropped file joins the line; line breaks are dropped.
    pub(super) fn image_prompt_paste(&mut self, text: &str) {
        self.compose.insert_text(&text.replace(['\r', '\n'], ""));
    }

    /// Attach the typed path. A path that is not an image file keeps the prompt open with
    /// the reason, so it can be corrected; an empty line closes it.
    fn submit_image_prompt(&mut self) -> Result<()> {
        let Mode::AttachImage(id) = &self.mode else { return Ok(()) };
        let id = id.clone();
        let input = self.compose.value();
        if input.trim().is_empty() {
            self.mode = Mode::Browse;
            return Ok(());
        }
        let path = match image_path(&input, &self.document_folder()) {
            Ok(path) => path,
            Err(err) => {
                self.status = Some(format!("{err:#}"));
                return Ok(());
            }
        };
        self.mode = Mode::Browse;
        let name = file_name(&path);
        if self.open.store.add_image(&id, &path)? {
            self.mark_unsent();
            self.status = Some(format!("image attached: {name}"));
        } else {
            self.status = Some(format!("already attached: {name}"));
        }
        Ok(())
    }

    /// Where a relative path starts: the open file's folder, else the working directory.
    fn document_folder(&self) -> PathBuf {
        match &self.open.source.provenance {
            Provenance::File { path } => path.parent().map(Path::to_path_buf),
            _ => None,
        }
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

/// The absolute path of the image file `input` names. Accepts what a terminal types, pastes
/// or drops: surrounding quotes, `file://` URLs, `~`, backslash-escaped spaces, and paths
/// relative to `folder`.
fn image_path(input: &str, folder: &Path) -> Result<String> {
    let text = unquote(input.trim());
    let text = text.strip_prefix("file://").map_or_else(
        || text.to_owned(),
        |rest| percent_decode(rest.strip_prefix("localhost").unwrap_or(rest)),
    );
    // A dropped file arrives shell-escaped (`Screen\ Shot.png`) in many terminals. The
    // literal spelling is tried first, so a Windows path keeps its separators.
    let mut path = resolve(&text, folder);
    if !path.exists() {
        let unescaped = resolve(&unescape(&text), folder);
        if unescaped.exists() {
            path = unescaped;
        }
    }
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !extension.is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str())) {
        bail!("not an image ({}): {}", IMAGE_EXTENSIONS.join(", "), path.display());
    }
    if !path.is_file() {
        bail!("no such file: {}", path.display());
    }
    match path.to_str() {
        Some(path) => Ok(path.to_owned()),
        None => bail!("path is not valid UTF-8: {}", path.display()),
    }
}

fn unquote(text: &str) -> &str {
    ['"', '\''].iter().find_map(|&q| text.strip_prefix(q).and_then(|t| t.strip_suffix(q))).unwrap_or(text)
}

/// `a\ b` → `a b`: drop each backslash and keep the character it escapes.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let hex = (byte == b'%').then(|| text.get(i + 1..i + 3)).flatten();
        if let Some(decoded) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            out.push(decoded);
            i += 3;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `~` and `~/…` from the home directory, other relative paths from `folder`.
fn resolve(text: &str, folder: &Path) -> PathBuf {
    let home = || std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let path = if text == "~" {
        home()
    } else if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        home().join(rest)
    } else {
        folder.join(text)
    };
    workspace_paths::absolute(&path)
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests;
