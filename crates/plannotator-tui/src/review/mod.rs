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
//! - [`drafts`]: making, editing and removing notes.
//! - [`herdr`]: `plannotator-tui herdr review`. [`scope`]: what it shares with `rex review`
//!   and `hc review`, and the pane they open.
//! - [`input`]: keys and the mouse. [`draw`], [`rail`] and [`overlays`]: the frame.
//! - [`send`]: send, resend, finish and undo. [`pi`]: the Pi link.
//! - [`cli`]: `plannotator-tui review [--staged | --base REF | --commit SHA] [DIR]`.

pub(crate) mod cli;
mod diff;
mod drafts;
mod draw;
pub(crate) mod herdr;
mod input;
mod notes;
mod overlays;
mod pi;
mod rail;
pub(crate) mod scope;
mod send;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;

use anyhow::{Context as _, Result};
use plannotator_tui_review::{Capture, FilePatch, Repo, ReviewStore, Status};
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

    fn after_note_change(&mut self) {
        self.clamp_rail_cursor();
        self.sync_markers();
        self.derive_send_state();
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
/// nothing to show: `No uncommitted changes in app`.
pub(crate) fn capture(dir: &Path, request: &plannotator_tui_review::Request) -> Result<(Repo, Capture)> {
    use plannotator_tui_review::Scope;
    let repo = Repo::discover(dir)?;
    let capture = repo.open(request).with_context(|| format!("reviewing {}", repo.root().display()))?;
    if capture.is_empty() {
        let root = repo.root();
        let name =
            root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into());
        let what = match &capture.scope {
            Scope::Uncommitted => "uncommitted changes".to_owned(),
            Scope::Staged => "staged changes".to_owned(),
            Scope::Branch { base } => format!("changes since {base}"),
            Scope::Commit { sha } => format!("changes in commit {}", sha.get(..7).unwrap_or(sha)),
        };
        anyhow::bail!("No {what} in {name}");
    }
    Ok((repo, capture))
}
