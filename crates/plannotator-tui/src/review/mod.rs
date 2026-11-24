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
//! - [`files`]: the changed files on the left, and filtering them.
//! - [`notes`]: the rail's order and wording.
//! - [`drafts`]: making, editing and removing notes.
//! - [`herdr`]: `plannotator-tui herdr review`. [`scope`]: what it shares with `rex review`
//!   and `hc review`, and the pane they open.
//! - [`input`]: keys and the mouse; [`mouse`]: clicks and drags in the diff. [`draw`],
//!   [`header`], [`footer`], [`rail`] and [`overlays`]: the frame; [`compose_at`]: where the
//!   comment box opens. [`divider`]: dragging the lines between the panes and between a split
//!   diff's sides.
//! - [`settings`]: how the diff is drawn (`[review]`), its menu, and remembering the choices.
//! - [`send`]: send, resend, finish and undo. [`pi`]: the Pi link.
//! - [`watch`]: noticing that the diff changed on disk, for the footer.
//! - [`whole`]: the whole-file view (`o`, `O`), a second diffs-rs engine.
//! - [`cli`]: `plannotator-tui review [--staged | --base REF | --commit SHA | PR] [DIR]`.
//!
//! A pull request review (`PR` is a GitHub link, `N` or `#N`) fetches the PR with the
//! person's `gh` login into their clone or a cache repository ([`capture`]), and sends to the
//! agent as any review does. Nothing is posted to GitHub.

pub(crate) mod cli;
mod compose_at;
mod diff;
mod divider;
mod drafts;
mod draw;
mod files;
mod footer;
mod header;
pub(crate) mod herdr;
mod input;
mod jump;
mod mouse;
mod notes;
mod overlays;
mod pi;
mod rail;
pub(crate) mod scope;
mod send;
pub(crate) mod settings;
#[cfg(test)]
mod tests;
mod viewed;
mod watch;
mod whole;

use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::time::Instant;

use anyhow::{Context as _, Result};
use plannotator_tui_review::{Capture, FilePatch, Progress, Repo, ReviewStore, Status};
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::delivery::Delivery;
use crate::keys::Keymap;
use crate::notes_rail::{NotesRail, RailStart};
use crate::theme::palette;
use crate::ui::agent_picker::{AgentPicker, Reach};
use crate::ui::compose::Compose;
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
    /// A comment on the whole change, on no file.
    Change,
    /// A comment on new-side lines outside every hunk, made in the whole file, with the code
    /// it quotes.
    Outside(Lines, String),
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
    /// Go to definition's list: which identifier, or which answer.
    Pick(jump::Pick),
    /// Typing the file list's name filter.
    FilterFiles,
    /// The kind picker, with its highlighted row.
    Kinds(usize),
    /// The diff settings menu, with its highlighted row.
    Settings(usize),
}

/// Which pane keys go to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Diff,
    Notes,
    Files,
}

/// Where things were drawn last frame, for the mouse.
#[derive(Debug, Default, Clone)]
struct Geometry {
    /// The whole frame: a mouse report outside it is ignored (`ui::drag::on_screen`).
    screen: Rect,
    diff: Rect,
    rail: Rect,
    /// The file list; empty while it is hidden.
    files: Rect,
    /// The file list's rows drawn last frame: their screen row and key.
    file_rows: Vec<(u16, String)>,
    send_button: Option<Rect>,
    review_button: Option<Rect>,
    settings_button: Option<Rect>,
    /// The layout switch's two halves: `true` for Split.
    layout_switch: Vec<(Rect, bool)>,
    wrap_switch: Option<Rect>,
    /// The `no ws` chip, while whitespace changes are hidden.
    whitespace_chip: Option<Rect>,
    /// Rail bubbles drawn last frame, with their note's index.
    bubbles: Vec<(Rect, usize)>,
    menu_rows: Vec<(Rect, usize)>,
    /// The settings menu's rows, and the whole box.
    settings_rows: Vec<(Rect, usize)>,
    settings_box: Option<Rect>,
    /// The buttons of the toolbar beside a mouse selection, and the whole bar.
    toolbar: Vec<(Rect, crate::keys::Action)>,
    toolbar_bar: Option<Rect>,
    /// The columns the comment box wrapped to last frame; 0 before its first.
    compose_wrap: usize,
    /// Where the comment box drew its text, and the first of the text's rows shown there.
    compose_text: Option<(Rect, usize)>,
}

/// The review screen.
pub(crate) struct Screen {
    repo: Repo,
    capture: Capture,
    diff: Diff,
    store: ReviewStore,
    /// Viewed marks, shared with the browser app (`viewed`).
    progress: Progress,
    data_dir: PathBuf,
    project: String,
    delivery: Box<dyn Delivery>,
    /// The target the review started with, while Send goes to an agent picked instead.
    start_delivery: Option<Box<dyn Delivery>>,
    picker: AgentPicker,
    /// Where the picker looks for agents, and whether Send asks which one first.
    reach: Reach,
    /// What to send once the picker that Send opened has a choice.
    pick_then: Option<send::PickThen>,
    pi_link: Option<crate::pi::link::Link>,
    keys: Keymap,
    /// Whether the terminal reports Shift+Enter distinctly; the comment box's hint says so.
    shift_enter: bool,
    /// Copy to the terminal clipboard when a send cannot reach the agent (off headless).
    clipboard: bool,
    /// `[ui] copy_on_select`: whether a mouse selection is copied on release.
    on_release: mouse::OnRelease,
    mode: Mode,
    focus: Focus,
    /// The selected note, by its position in the rail's order.
    rail_cursor: usize,
    /// First rail row drawn.
    rail_scroll: usize,
    /// Whether the rail shows: `[ui] notes_rail`, the last session's choice, `toggle-notes`.
    notes_rail: NotesRail,
    /// The changed files on the left, and their filter.
    files: files::FileList,
    compose: Compose,
    /// The side of its lines the open comment box took.
    compose_at: Option<compose_at::Placed>,
    send_state: SendState,
    last_send: crate::ui::send_button::LastSend,
    /// Notes the last finish archived, for undo.
    undo: Vec<String>,
    status: Option<String>,
    geometry: Geometry,
    /// A drag with the left button in the diff, until the release.
    drag: Option<mouse::Drag>,
    /// The line the last drag's selection grew from, for a shift-click.
    grown: Option<mouse::Grown>,
    /// The toolbar beside the lines a drag selected, until the next key or click.
    toolbar: Option<mouse::Toolbar>,
    /// The lines between the panes, and the notes panel's width.
    dividers: divider::Dividers,
    /// How the diff is drawn: `[review]`, as changed since.
    settings: settings::Settings,
    /// Whether the diff changed on disk since the last capture.
    watch: watch::Watch,
    /// The last key or paste, so no disk check starts while the person types.
    last_key: Option<Instant>,
    /// The whole-file view, while it is open over the diff.
    whole: Option<whole::Whole>,
    /// The event loop's wake channel, for the whole-file view's engine and the agent finder.
    wake: SyncSender<()>,
    /// Go to definition: the search under way, and the places to go back to.
    nav: jump::Nav,
    /// The key help's scroll, and how far it can scroll at the size drawn last.
    help_scroll: usize,
    help_room: std::cell::Cell<usize>,
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
    /// Open the review of `capture` with the default settings, remembering none.
    #[cfg(test)]
    pub(crate) fn open(
        repo: Repo,
        capture: Capture,
        data_dir: &Path,
        delivery: Box<dyn Delivery>,
        wake: SyncSender<()>,
    ) -> Result<Self> {
        Self::open_with(repo, capture, data_dir, delivery, wake, settings::Settings::default())
    }

    /// Open the review of `capture`: load its notes, place them on this diff, and start the
    /// viewer as `settings` say. `wake` is the event loop's: the viewer sends on it when it
    /// has work ready.
    pub(crate) fn open_with(
        repo: Repo,
        capture: Capture,
        data_dir: &Path,
        delivery: Box<dyn Delivery>,
        wake: SyncSender<()>,
        settings: settings::Settings,
    ) -> Result<Self> {
        let (project, owner) = capture.record_owner();
        let store = ReviewStore::load(data_dir, &project, &owner, &capture.scope)?;
        let light = crate::theme::theme() == crate::theme::Theme::Light;
        let watch = watch::Watch::start(&capture.scope, wake.clone());
        let order = files::tree::order(&capture.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>());
        let paths: Vec<String> =
            order.into_iter().filter_map(|i| capture.files.get(i).map(|f| f.path.clone())).collect();
        let diff = Diff::open(&capture, &paths, settings.now, light, wake.clone())?;
        let progress = Progress::open(&capture, data_dir, viewed::saving(data_dir));
        let mut screen = Self {
            repo,
            capture,
            diff,
            store,
            progress,
            data_dir: data_dir.to_path_buf(),
            project,
            delivery,
            start_delivery: None,
            picker: AgentPicker::default(),
            reach: Reach::default(),
            pick_then: None,
            pi_link: None,
            keys: Keymap::default(),
            shift_enter: false,
            clipboard: false,
            on_release: mouse::OnRelease::Copy,
            mode: Mode::Browse,
            focus: Focus::Diff,
            rail_cursor: 0,
            rail_scroll: 0,
            notes_rail: NotesRail::default(),
            files: files::FileList::default(),
            compose: Compose::default(),
            compose_at: None,
            send_state: SendState::Ready,
            last_send: crate::ui::send_button::LastSend::None,
            undo: Vec::new(),
            status: None,
            geometry: Geometry::default(),
            drag: None,
            grown: None,
            toolbar: None,
            dividers: divider::Dividers::default(),
            settings,
            watch,
            last_key: None,
            whole: None,
            wake,
            nav: jump::Nav::default(),
            help_scroll: 0,
            help_room: std::cell::Cell::new(0),
            quit: false,
        };
        screen.files_for_capture();
        let placed = screen.place_notes()?;
        // What placing the notes found, and a remembered settings file that could not be read.
        screen.status = match (placed, screen.settings.take_notice()) {
            (Some(placed), Some(notice)) => Some(format!("{placed} \u{b7} {notice}")),
            (placed, notice) => placed.or(notice),
        };
        screen.derive_send_state();
        Ok(screen)
    }

    /// Say `notice` in the footer ahead of what it says already, as the review opens.
    pub(crate) fn add_notice(&mut self, notice: String) {
        self.status = Some(match self.status.take() {
            Some(status) => format!("{notice} \u{b7} {status}"),
            None => notice,
        });
    }

    pub(crate) fn set_keys(&mut self, keys: Keymap, shift_enter: bool) {
        self.keys = keys;
        self.shift_enter = shift_enter;
    }

    /// Start the notes rail as the last session left it, else as `[ui] notes_rail` says.
    pub(crate) fn start_notes_rail(&mut self, start: RailStart) {
        self.notes_rail = NotesRail::load(start, &self.data_dir);
    }

    /// Whether the rail is shown, width allowing. Hidden, notes keep their gutter marks.
    fn rail_shown(&self) -> bool {
        self.notes_rail.shown(!self.store.annotations().is_empty())
    }

    /// `toggle-notes`: close the rail, or open it again, and remember which. The next frame
    /// gives the diff the rail's columns, or takes them back, and the viewer reflows as on a
    /// resize.
    fn toggle_notes(&mut self) {
        let remembered = self.notes_rail.toggle(!self.store.annotations().is_empty(), &self.data_dir);
        self.after_rail_change(remembered);
    }

    /// Moving the focus to the notes opens a closed rail.
    fn reveal_notes(&mut self) {
        let remembered = self.notes_rail.reveal(!self.store.annotations().is_empty(), &self.data_dir);
        self.after_rail_change(remembered);
    }

    fn after_rail_change(&mut self, remembered: Result<()>) {
        if let Err(error) = remembered {
            self.status = Some(format!("{error:#}"));
        }
        if !self.rail_shown() && self.focus == Focus::Notes {
            self.focus = Focus::Diff;
        }
    }

    pub(crate) fn enable_clipboard(&mut self) {
        self.clipboard = true;
    }

    /// `[ui] copy_on_select`: off, a mouse selection waits for `y` to be copied.
    pub(crate) fn set_copy_on_select(&mut self, on: bool) {
        self.on_release = if on { mouse::OnRelease::Copy } else { mouse::OnRelease::Wait };
    }

    pub(crate) fn wants_quit(&self) -> bool {
        self.quit
    }

    /// Take what the viewers, the Pi link, the disk watch and the agent finder have ready, and
    /// scroll for a drag held past the diff's edge.
    /// True when the screen changed.
    pub(crate) fn tick(&mut self) -> Result<bool> {
        let mut changed = false;
        // A setting the viewer could not take yet (before its first file or frame) now.
        self.apply_settings()?;
        if self.diff.poll(&self.capture)? {
            self.sync_markers();
            changed = true;
        }
        changed |= self.show_pending_file()?;
        changed |= self.take_attributes()?;
        changed |= self.poll_pi();
        changed |= self.poll_search()?;
        let typing = matches!(self.mode, Mode::Compose(_))
            || self.last_key.is_some_and(|at| at.elapsed() < watch::QUIET_AFTER_KEY);
        changed |= self.watch.poll(&self.repo, &self.capture.scope, !typing);
        if let Some(whole) = &mut self.whole
            && whole.poll(&self.capture)?
        {
            self.sync_whole_markers();
            changed = true;
        }
        changed |= self.reach.poll();
        changed |= self.step_at_edge(Instant::now())?;
        let whole_draw = self.whole.as_ref().is_some_and(whole::Whole::needs_draw);
        Ok(changed || self.diff.needs_draw() || whole_draw)
    }

    pub(crate) fn after_draw(&mut self) {
        self.diff.after_draw();
        if let Some(whole) = &mut self.whole {
            whole.after_draw();
        }
    }

    /// Stop the viewer's workers. Call it once the loop ends, so their errors are reported.
    pub(crate) fn close(&mut self) -> Result<()> {
        let whole = self.whole.as_mut().map_or(Ok(()), whole::Whole::close);
        self.diff.close().and(whole)
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
        let captured = {
            let _no_check_meanwhile = self.watch.hold_git();
            self.repo.capture(&self.capture.scope)
        };
        let capture = match captured {
            Ok(capture) => capture,
            Err(err) => {
                self.status = Some(format!("reload failed: {err}"));
                return Ok(());
            }
        };
        self.watch.recaptured();
        self.progress = self.progress.reopen(&capture, &self.data_dir);
        self.capture = capture;
        self.files_for_capture();
        self.diff.replace(&self.capture, self.files.shown())?;
        self.files.retarget();
        let placed = self.place_notes()?;
        // The whole file is read again too, at the line it was on.
        if let Some(whole) = &self.whole {
            let (path, version, line) =
                (whole.path.clone(), whole.version, whole.target()?.map(|(first, _)| first));
            self.show_whole(&path, version, line);
        }
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
            .chain(
                self.whole
                    .is_none()
                    .then(|| self.drag.as_ref().and_then(mouse::Drag::lines))
                    .flatten()
                    .map(|lines| (lines, mouse::drag_mark())),
            )
            .collect();
        if let Err(err) = self.diff.set_markers(&marks) {
            self.status = Some(format!("could not mark notes in the diff: {err:#}"));
        }
        self.sync_whole_markers();
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

    /// The repository root as Pi and the feedback name it: for a pull request fetched into
    /// the cache, the folder the review was opened in.
    fn root(&self) -> &Path {
        self.repo.workdir()
    }

    /// The repository folder's name, for the footer; a pull request's `owner/repo`.
    fn repo_name(&self) -> String {
        if let Some(pull) = &self.capture.pull {
            return pull.view.repo.slug();
        }
        self.root()
            .file_name()
            .map_or_else(|| self.root().display().to_string(), |n| n.to_string_lossy().into())
    }
}

/// Open a review of `dir` for `request` and fail before any screen opens when there is
/// nothing to show: `No uncommitted changes in app`. A pull request is fetched with `gh`
/// into `dir`'s clone of its repository, else into a bare cache repository under the data
/// dir (`clients/plannotator-tui/pulls/<host>/<owner>/<repo>.git`).
pub(crate) fn capture(dir: &Path, request: &plannotator_tui_review::Request) -> Result<(Repo, Capture)> {
    use plannotator_tui_review::{Gh, Request, Scope};
    let (repo, capture) = if let Request::PullRequest { pr } = request {
        let cache = crate::workspace_paths::data_dir().join("clients").join("plannotator-tui").join("pulls");
        Repo::pull_request(dir, pr, &cache, &Gh::default())
            .with_context(|| format!("reviewing pull request {pr}"))?
    } else {
        let repo = Repo::discover(dir)?;
        let capture = repo.open(request).with_context(|| format!("reviewing {}", repo.root().display()))?;
        (repo, capture)
    };
    if capture.is_empty() {
        let root = repo.root();
        let name =
            root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into());
        let what = match &capture.scope {
            Scope::Uncommitted => "uncommitted changes".to_owned(),
            Scope::Staged => "staged changes".to_owned(),
            Scope::Branch { base } => format!("changes since {base}"),
            Scope::Commit { sha } => format!("changes in commit {}", sha.get(..7).unwrap_or(sha)),
            Scope::PullRequest { url, .. } => anyhow::bail!("No changes in {url}"),
        };
        anyhow::bail!("No {what} in {name}");
    }
    Ok((repo, capture))
}
