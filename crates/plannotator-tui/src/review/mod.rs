//! `plannotator-tui review`: a git diff in the terminal, with notes the person sends to a
//! coding agent, the way a document review sends them.
//!
//! The data side (capture, notes, anchoring, the record, the feedback text) is the
//! `plannotator-tui-review` crate; the diff itself is drawn by diffs-rs (`diff`). This
//! module tree is the screen between them, parallel to the document review in `app/`, and
//! it reuses that review's pieces from `ui/`, `delivery`, `archive` and `keys`.
//!
//! - `mod.rs`: the screen's state and the operations that change notes.
//! - [`diff`]: the diffs-rs engine, in this app's terms.
//! - [`notes`]: the rail's order and wording.
//! - [`input`]: keys and the mouse. [`draw`] and [`rail`]: the frame.
//! - [`send`]: send, resend, finish and undo. [`pi`]: the Pi link.
//! - [`cli`]: `plannotator-tui review [--staged | --base REF | --commit SHA] [DIR]`.

pub(crate) mod cli;
mod diff;
mod draw;
mod input;
mod notes;
mod pi;
mod rail;
mod send;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;

use anyhow::{Context as _, Result};
use plannotator_tui_review::{
    Capture, CodeAnnotation, FilePatch, NoteScope, NoteType, Repo, ReviewStore, Side, Status,
};
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::delivery::Delivery;
use crate::keys::Keymap;
use crate::theme::palette;
use crate::ui::compose::Compose;
use crate::ui::pi_picker::PiPicker;
use crate::ui::send_button::SendState;
use diff::{Diff, Lines};

/// What the comment box is writing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Draft {
    /// A comment on lines.
    Comment(Lines),
    /// Replacement code for new-side lines.
    Suggestion(Lines),
    /// A comment on a whole file.
    File(String),
    /// A new body for an existing note, by id.
    Edit(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Browse,
    Compose(Draft),
    /// Quit was asked for while notes are unsent; the footer asks first.
    ConfirmQuit,
    /// The Review menu, with its highlighted row.
    Menu(usize),
    PickTarget,
    Help,
}

/// Which pane keys go to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Diff,
    Notes,
}

/// Where things were drawn last frame, for the mouse.
#[derive(Debug, Default, Clone)]
struct Geometry {
    diff: Rect,
    rail: Rect,
    send_button: Option<Rect>,
    review_button: Option<Rect>,
    /// Rail bubbles drawn last frame, with their note's index.
    bubbles: Vec<(Rect, usize)>,
    menu_rows: Vec<(Rect, usize)>,
    /// The columns the comment box wrapped to last frame; 0 before its first.
    compose_wrap: usize,
}

/// The review screen.
pub(crate) struct Screen {
    repo: Repo,
    capture: Capture,
    diff: Diff,
    store: ReviewStore,
    data_dir: PathBuf,
    project: String,
    delivery: Box<dyn Delivery>,
    /// The target the review started with, while Send goes to a Pi picked instead.
    start_delivery: Option<Box<dyn Delivery>>,
    picker: PiPicker,
    pi_link: Option<crate::pi::link::Link>,
    keys: Keymap,
    /// Whether the terminal reports Shift+Enter distinctly; the comment box's hint says so.
    shift_enter: bool,
    /// Copy to the terminal clipboard when a send cannot reach the agent (off headless).
    clipboard: bool,
    mode: Mode,
    focus: Focus,
    /// The selected note, by its position in the rail's order.
    rail_cursor: usize,
    /// First rail row drawn.
    rail_scroll: usize,
    compose: Compose,
    send_state: SendState,
    /// Notes the last finish archived, for undo.
    undo: Vec<String>,
    status: Option<String>,
    geometry: Geometry,
    quit: bool,
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Screen")
            .field("root", &self.capture.root)
            .field("scope", &self.capture.scope)
            .finish_non_exhaustive()
    }
}

impl Screen {
    /// Open the review of `capture`: load its notes, place them on this diff, and start the
    /// viewer. `wake` is the event loop's: the viewer sends on it when it has work ready.
    pub(crate) fn open(
        repo: Repo,
        capture: Capture,
        data_dir: &Path,
        delivery: Box<dyn Delivery>,
        wake: SyncSender<()>,
    ) -> Result<Self> {
        let project = plannotator_tui_review::project_for(&capture.root);
        let root = capture.root.display().to_string();
        let store = ReviewStore::load(data_dir, &project, &root, &capture.scope)?;
        let light = crate::theme::theme() == crate::theme::Theme::Light;
        let diff = Diff::open(&capture, light, wake)?;
        let mut screen = Self {
            repo,
            capture,
            diff,
            store,
            data_dir: data_dir.to_path_buf(),
            project,
            delivery,
            start_delivery: None,
            picker: PiPicker::default(),
            pi_link: None,
            keys: Keymap::default(),
            shift_enter: false,
            clipboard: false,
            mode: Mode::Browse,
            focus: Focus::Diff,
            rail_cursor: 0,
            rail_scroll: 0,
            compose: Compose::default(),
            send_state: SendState::Ready,
            undo: Vec::new(),
            status: None,
            geometry: Geometry::default(),
            quit: false,
        };
        let placed = screen.place_notes()?;
        screen.status = placed;
        screen.derive_send_state();
        Ok(screen)
    }

    pub(crate) fn set_keys(&mut self, keys: Keymap, shift_enter: bool) {
        self.keys = keys;
        self.shift_enter = shift_enter;
    }

    pub(crate) fn enable_clipboard(&mut self) {
        self.clipboard = true;
    }

    pub(crate) fn wants_quit(&self) -> bool {
        self.quit
    }

    /// Take what the viewer and the Pi link have ready. True when the screen changed.
    pub(crate) fn tick(&mut self) -> Result<bool> {
        let mut changed = false;
        if self.diff.poll(&self.capture)? {
            self.sync_markers();
            changed = true;
        }
        changed |= self.poll_pi();
        Ok(changed || self.diff.needs_draw())
    }

    pub(crate) fn after_draw(&mut self) {
        self.diff.after_draw();
    }

    /// Stop the viewer's workers. Call it once the loop ends, so their errors are reported.
    pub(crate) fn close(&mut self) -> Result<()> {
        self.diff.close()
    }

    /// Resolve every note against the current capture and save where they are now. Returns
    /// what moved or went outdated, for the footer.
    fn place_notes(&mut self) -> Result<Option<String>> {
        let resolutions = self.capture.resolve_all(self.store.annotations())?;
        self.store.apply(&resolutions)?;
        let count = |status| self.store.annotations().iter().filter(|n| n.status() == Some(status)).count();
        let (moved, outdated) = (count(Status::Moved), count(Status::Outdated));
        let mut parts = Vec::new();
        if moved > 0 {
            parts.push(format!("{moved} moved"));
        }
        if outdated > 0 {
            parts.push(format!("{outdated} outdated"));
        }
        Ok((!parts.is_empty()).then(|| format!("notes: {}", parts.join(" · "))))
    }

    /// Recapture the diff (`r`): notes follow their lines, the ones whose code is gone move
    /// to Outdated. A capture that fails leaves the review as it was.
    fn reload(&mut self) -> Result<()> {
        let capture = match self.repo.capture(&self.capture.scope) {
            Ok(capture) => capture,
            Err(err) => {
                self.status = Some(format!("reload failed: {err}"));
                return Ok(());
            }
        };
        self.diff.replace(&capture)?;
        self.capture = capture;
        let placed = self.place_notes()?;
        self.clamp_rail_cursor();
        self.sync_markers();
        self.derive_send_state();
        let empty = if self.capture.is_empty() { " · no changes left" } else { "" };
        self.status =
            Some(format!("reloaded{empty}{}", placed.map(|p| format!(" · {p}")).unwrap_or_default()));
        Ok(())
    }

    /// The gutter markers: unsent notes in the comment colour, sent ones dimmed. Outdated
    /// notes and notes off the diff have none.
    fn sync_markers(&mut self) {
        let unsent = Style::new().fg(palette().comment);
        let sent = Style::new().fg(ratatui::style::Color::DarkGray);
        let marks: Vec<(Lines, Style)> = self
            .store
            .annotations()
            .iter()
            .filter(|note| notes::in_diff(note))
            .map(|note| {
                let lines = Lines {
                    path: note.file_path.clone(),
                    side: note.side,
                    first: note.line_start,
                    last: note.line_end,
                };
                (lines, if self.store.is_pending(note) { unsent } else { sent })
            })
            .collect();
        if let Err(err) = self.diff.set_markers(&marks) {
            self.status = Some(format!("could not mark notes in the diff: {err:#}"));
        }
    }

    fn file(&self, path: &str) -> Option<&FilePatch> {
        self.capture.file(path)
    }

    /// The lines a note made now would cover: the selection, else the cursor's line.
    fn target_lines(&self) -> Result<Option<Lines>> {
        match self.diff.selection() {
            Some(lines) => Ok(Some(lines)),
            None => self.diff.cursor(),
        }
    }

    /// `c`: comment on the selection or the cursor line.
    fn start_comment(&mut self) -> Result<()> {
        let Some(lines) = self.target_lines()? else {
            self.status = Some("move the cursor onto a line of code first".into());
            return Ok(());
        };
        self.compose = Compose::default();
        self.mode = Mode::Compose(Draft::Comment(lines));
        Ok(())
    }

    /// `s`: the comment box filled with the selected new-side lines; what the person makes
    /// of them is the suggested code.
    fn start_suggestion(&mut self) -> Result<()> {
        let Some(lines) = self.target_lines()? else {
            self.status = Some("move the cursor onto a line of code first".into());
            return Ok(());
        };
        if lines.side == Side::Old {
            self.status = Some("a suggestion replaces new lines: pick lines on the new side".into());
            return Ok(());
        }
        let Some(file) = self.file(&lines.path) else { return Ok(()) };
        let code: Vec<&str> = (lines.first..=lines.last).filter_map(|n| file.line(Side::New, n)).collect();
        self.compose = Compose::with_text(&code.join("\n"));
        self.mode = Mode::Compose(Draft::Suggestion(lines));
        Ok(())
    }

    /// `C`: comment on the whole file under the cursor.
    fn start_file_comment(&mut self) -> Result<()> {
        let path = match self.diff.selection() {
            Some(lines) => Some(lines.path),
            None => self.diff.cursor()?.map(|lines| lines.path),
        };
        let Some(path) = path else {
            self.status = Some("move the cursor into a file first".into());
            return Ok(());
        };
        self.compose = Compose::default();
        self.mode = Mode::Compose(Draft::File(path));
        Ok(())
    }

    /// `e` in the rail: the comment box with the note's comment, or its code for a
    /// suggestion.
    fn start_edit(&mut self) {
        let Some(note) = self.selected_note() else { return };
        let text = match note.kind {
            NoteType::Suggestion => note.suggested_code.clone(),
            NoteType::Comment | NoteType::Concern => note.text.clone(),
        };
        let id = note.id.clone();
        self.compose = Compose::with_text(&text.unwrap_or_default());
        self.mode = Mode::Compose(Draft::Edit(id));
    }

    /// Save what the comment box holds. An empty comment saves nothing.
    fn save_draft(&mut self, draft: &Draft) -> Result<()> {
        self.mode = Mode::Browse;
        let value = self.compose.value();
        let text = value.trim().to_owned();
        // Code keeps its indentation; only trailing blank lines go.
        let code = value.trim_end().to_owned();
        let key = self.capture.patch_key.clone();
        let note = match draft {
            _ if text.is_empty() => {
                self.status = Some("empty note not saved".into());
                return Ok(());
            }
            Draft::Comment(lines) => self.file(&lines.path).map(|file| {
                CodeAnnotation::line_comment(file, lines.side, lines.first, lines.last, text, &key)
            }),
            Draft::Suggestion(lines) => self
                .file(&lines.path)
                .map(|file| CodeAnnotation::suggestion(file, lines.first, lines.last, code, None, &key)),
            Draft::File(path) => self.file(path).map(|file| CodeAnnotation::file_comment(file, text, &key)),
            Draft::Edit(id) => {
                let changed = self.store.edit(id, |note| {
                    let field = match note.kind {
                        NoteType::Suggestion => &mut note.suggested_code,
                        NoteType::Comment | NoteType::Concern => &mut note.text,
                    };
                    let value = if note.kind == NoteType::Suggestion { code } else { text };
                    let changed = field.as_deref() != Some(value.as_str());
                    *field = Some(value);
                    changed
                })?;
                self.status = Some(if changed { "note updated" } else { "note unchanged" }.into());
                self.after_note_change();
                return Ok(());
            }
        };
        let Some(note) = note else {
            self.status = Some("that file is no longer in the diff".into());
            return Ok(());
        };
        self.add_note(note)?;
        self.status = Some("note saved".into());
        Ok(())
    }

    fn add_note(&mut self, note: CodeAnnotation) -> Result<()> {
        let id = note.id.clone();
        self.store.add(note)?;
        // The selection became the note; the next one starts fresh.
        let _ = self.diff.clear_selection();
        self.after_note_change();
        if let Some(position) = self.rail_order().iter().position(|&i| self.note_id(i) == Some(id.as_str())) {
            self.rail_cursor = position;
        }
        Ok(())
    }

    /// `x` in the rail.
    fn remove_selected(&mut self) -> Result<()> {
        let Some(id) = self.selected_note().map(|n| n.id.clone()) else { return Ok(()) };
        if self.store.remove(&id)? {
            self.status = Some("note removed".into());
        }
        self.after_note_change();
        if self.store.annotations().is_empty() {
            self.focus = Focus::Diff;
        }
        Ok(())
    }

    fn after_note_change(&mut self) {
        self.clamp_rail_cursor();
        self.sync_markers();
        self.derive_send_state();
    }

    fn rail_order(&self) -> Vec<usize> {
        notes::order(self.store.annotations())
    }

    fn note_id(&self, index: usize) -> Option<&str> {
        self.store.annotations().get(index).map(|n| n.id.as_str())
    }

    fn selected_note(&self) -> Option<&CodeAnnotation> {
        let index = *self.rail_order().get(self.rail_cursor)?;
        self.store.annotations().get(index)
    }

    fn clamp_rail_cursor(&mut self) {
        self.rail_cursor = self.rail_cursor.min(self.store.annotations().len().saturating_sub(1));
    }

    /// Move the rail's cursor by `delta` notes and show the note in the diff.
    fn move_rail(&mut self, delta: isize) {
        let count = self.store.annotations().len();
        if count == 0 {
            return;
        }
        self.rail_cursor = self.rail_cursor.saturating_add_signed(delta).min(count - 1);
        self.show_selected_note();
    }

    /// Scroll the diff to the selected note: its first line, or its file's first shown line
    /// for a file note. An outdated note has no place to go.
    fn show_selected_note(&mut self) {
        let Some(note) = self.selected_note() else { return };
        let path = note.file_path.clone();
        let place = match note.scope {
            NoteScope::Line if notes::in_diff(note) => Some((note.side, note.line_start)),
            NoteScope::Line if notes::is_outdated(note) => {
                self.status = Some("outdated: its code is gone from the diff".into());
                return;
            }
            _ => self.file(&path).and_then(|file| {
                let new = file.lines(Side::New).next().map(|(n, _)| (Side::New, n));
                new.or_else(|| file.lines(Side::Old).next().map(|(n, _)| (Side::Old, n)))
            }),
        };
        let Some((side, line)) = place else {
            self.status = Some("not in the diff: its lines are unchanged now".into());
            return;
        };
        if let Err(err) = self.diff.reveal(&path, side, line) {
            self.status = Some(format!("{err:#}"));
        }
    }

    fn derive_send_state(&mut self) {
        self.send_state = if self.store.all_delivered() { SendState::Sent } else { SendState::Ready };
    }

    /// The repository root as Pi and the feedback name it.
    fn root(&self) -> &Path {
        &self.capture.root
    }

    /// The repository folder's name, for the footer.
    fn repo_name(&self) -> String {
        self.root()
            .file_name()
            .map_or_else(|| self.root().display().to_string(), |n| n.to_string_lossy().into())
    }
}

/// Open a review of `dir` for `request` and fail before any screen opens when there is
/// nothing to show.
pub(crate) fn capture(dir: &Path, request: &plannotator_tui_review::Request) -> Result<(Repo, Capture)> {
    let repo = Repo::discover(dir)?;
    let capture = repo.open(request).with_context(|| format!("reviewing {}", repo.root().display()))?;
    if capture.is_empty() {
        let name = repo
            .root()
            .file_name()
            .map_or_else(|| repo.root().display().to_string(), |n| n.to_string_lossy().into());
        anyhow::bail!("No {} changes in {name}", capture.scope.title().to_lowercase());
    }
    Ok((repo, capture))
}
