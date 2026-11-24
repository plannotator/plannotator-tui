//! The diff viewer: one diffs-rs [`Engine`] showing a capture's patch, with this app's
//! names for what it points at. Nothing outside this file speaks diffs-rs's types.
//!
//! The engine numbers files in patch order and lines by their original source line on each
//! side; a note names a file by path. [`Lines`] is the meeting point: a run of lines on one
//! side of one file, by path.

use std::io::{Seek as _, Write as _};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::SyncSender;

use anyhow::{Context as _, Result};
use diffs_rs::engine::{self, Command, Engine, Input, InputEffect, Marker, Options, SidebarPreference};
use diffs_rs::reading::SourceRange;
use plannotator_tui_review::{Capture, Side};

use crate::config::review::ReviewConfig;
use ratatui::Frame;
use ratatui::crossterm::event::{Event, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Style;

mod insets;
mod place;
mod sections;
mod settings;
mod whitespace;

pub(super) use insets::{Columns, Slot};
use sections::Sections;
use settings::{layout, options};

/// A number no viewer's patch had before.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Lines `first..=last` on one side of the file at `path` (the capture's path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Lines {
    pub(super) path: String,
    pub(super) side: Side,
    pub(super) first: u32,
    pub(super) last: u32,
}

/// What the engine says it is doing, for the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Progress {
    Loading,
    Coloring,
    SourceFailed,
    Failed(String),
    Ready,
}

pub(super) struct Diff {
    engine: Engine,
    /// Each file's section of the capture's patch, by path, for the patch of the key.
    sections: Option<(String, Sections)>,
    /// The capture's path for each engine file, in engine order, as far as files have
    /// arrived.
    paths: Vec<String>,
    /// A whole-file view's one file, named by its capture path whatever the patch says.
    fixed: Option<String>,
    /// Where it was drawn last.
    area: Rect,
    /// Which patch is shown, unique among every viewer opened, so a drag can tell its lines
    /// are from an earlier one or from another view.
    generation: u64,
    /// What the footer should say about the last move, once.
    notice: Option<&'static str>,
    /// Drawn since its files last changed.
    drawn: bool,
    /// Whether it draws on the terminal's own background, as last set.
    terminal_background: bool,
    /// The whole-file view: always unified, so no layout or split ratio applies.
    whole: bool,
    /// Nothing moved the cursor or the view since the files were opened: the notes' boxes,
    /// laid out for the first time, start from the top (`insets`).
    at_start: bool,
}

impl std::fmt::Debug for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Diff").field("paths", &self.paths).finish_non_exhaustive()
    }
}

impl Diff {
    /// Show the files at `paths` of `capture`'s patch, in that order. The engine reads them
    /// on its own thread and sends on `wake` when it has something to show.
    /// `settings` say how it is drawn, `light` which colours `auto` means.
    pub(super) fn open(
        capture: &Capture,
        paths: &[String],
        settings: ReviewConfig,
        light: bool,
        wake: SyncSender<()>,
    ) -> Result<Self> {
        let options = Options {
            layout: layout(settings.layout),
            // The header, footer, help and file list are this app's own (`review::draw`,
            // `review::files`).
            chrome: false,
            sidebar: SidebarPreference::Hide,
            ..options(settings, light)
        };
        let sections = Sections::of(&capture.patch);
        let patch = sections.select(&capture.patch, paths);
        let input = input_bytes(&patch, capture.scope.title())?;
        let engine = Engine::open(input, options, wake).context("opening the diff viewer")?;
        let mut diff = Self::with(engine, Some((capture.patch_key.clone(), sections)), None);
        diff.apply(settings, light)?;
        Ok(diff)
    }

    fn with(engine: Engine, sections: Option<(String, Sections)>, fixed: Option<String>) -> Self {
        Self {
            engine,
            whole: fixed.is_some(),
            sections,
            paths: Vec::new(),
            fixed,
            area: Rect::default(),
            generation: next_generation(),
            notice: None,
            drawn: false,
            terminal_background: false,
            at_start: true,
        }
    }

    /// A whole-file view: `patch` is one file as a single unchanged hunk (`super::whole`),
    /// shown unified, its lines named by `path`, drawn as `settings` say otherwise.
    pub(super) fn open_whole(
        bytes: &[u8],
        name: &str,
        settings: ReviewConfig,
        light: bool,
        wake: SyncSender<()>,
    ) -> Result<Self> {
        let options = Options { layout: engine::Layout::Unified, chrome: false, ..options(settings, light) };
        let input = input_bytes(bytes, name.to_owned())?;
        let engine = Engine::open(input, options, wake).context("opening the whole-file view")?;
        let mut diff = Self::with(engine, None, Some(name.to_owned()));
        diff.apply(settings, light)?;
        Ok(diff)
    }

    /// Show the files at `paths` of `capture`, a new capture of the same review or the same
    /// one filtered, keeping the layout and wrapping.
    pub(super) fn replace(&mut self, capture: &Capture, paths: &[String]) -> Result<()> {
        let fresh = self.sections.as_ref().is_none_or(|(key, _)| *key != capture.patch_key);
        if fresh {
            self.sections = Some((capture.patch_key.clone(), Sections::of(&capture.patch)));
        }
        let patch = self.sections.as_ref().map(|(_, s)| s.select(&capture.patch, paths)).unwrap_or_default();
        let input = input_bytes(&patch, capture.scope.title())?;
        self.engine.replace(input).context("reloading the diff viewer")?;
        self.paths.clear();
        self.generation = next_generation();
        self.drawn = false;
        self.at_start = true;
        Ok(())
    }

    /// Counts the patches this viewer has shown.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Take what the engine's workers have ready. True when more files arrived, so the
    /// caller can place markers on them.
    pub(super) fn poll(&mut self, capture: &Capture) -> Result<bool> {
        self.engine.poll()?;
        let files = self.engine.status().files;
        if files == self.paths.len() {
            return Ok(false);
        }
        if let Some(path) = &self.fixed {
            self.paths = vec![path.clone(); files.min(1)];
            return Ok(true);
        }
        self.paths = (0..files).map(|index| self.capture_path(capture, index)).collect();
        self.drawn = false;
        Ok(true)
    }

    /// The capture's path for engine file `index`: the same new path, or for a deleted
    /// file (which the engine may name by either side) its old path.
    fn capture_path(&self, capture: &Capture, index: usize) -> String {
        let Some(meta) = self.engine.file(index) else { return String::new() };
        let by_path = capture.files.iter().find(|f| *f.path == *meta.path);
        by_path
            .or_else(|| capture.files.iter().find(|f| *f.path == *meta.old_path))
            .map_or_else(|| meta.path.to_string(), |f| f.path.clone())
    }

    /// How many files the viewer has so far.
    pub(super) fn files_shown(&self) -> usize {
        self.paths.len()
    }

    /// The capture's paths of the files shown so far, in the viewer's order.
    pub(super) fn paths(&self) -> &[String] {
        &self.paths
    }

    fn index(&self, path: &str) -> Option<usize> {
        self.paths.iter().position(|p| p == path)
    }

    pub(super) fn needs_draw(&self) -> bool {
        self.engine.needs_draw()
    }

    pub(super) fn draw(&mut self, frame: &mut Frame, area: Rect) -> Result<()> {
        self.engine.draw(frame, area)?;
        self.area = area.intersection(frame.area());
        Ok(())
    }

    pub(super) fn after_draw(&mut self) {
        self.engine.after_draw();
        self.drawn = !self.paths.is_empty();
    }

    /// Whether the files it has were drawn, so their rows are laid out.
    pub(super) fn drawn(&self) -> bool {
        self.drawn
    }

    pub(super) fn close(&mut self) -> Result<()> {
        self.engine.close()
    }

    pub(super) fn progress(&self) -> Progress {
        let status = self.engine.status();
        if let Some(error) = status.error {
            Progress::Failed(error.to_owned())
        } else if status.loading {
            Progress::Loading
        } else if status.coloring || status.preparing_source {
            Progress::Coloring
        } else if status.source_failed {
            Progress::SourceFailed
        } else {
            Progress::Ready
        }
    }

    pub(super) fn run(&mut self, command: Command) -> Result<()> {
        self.at_start = false;
        let growing = self.growing();
        let effect = self.engine.execute(command)?;
        let step = match command {
            _ if effect == InputEffect::Ignored => return Ok(()),
            Command::Down | Command::PageDown => Some(true),
            Command::Up | Command::PageUp => Some(false),
            _ => None,
        };
        self.keep_in_hunk(growing, step)
    }

    /// Mouse and resize events, which the engine handles itself. Keys never come here:
    /// this app maps them to commands. True when the engine acted on it.
    pub(super) fn handle_event(&mut self, event: Event) -> Result<bool> {
        let step = match &event {
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => Some(true),
                MouseEventKind::ScrollUp => Some(false),
                _ => None,
            },
            _ => None,
        };
        let growing = self.growing();
        let acted = self.engine.handle_event(event, true)? != InputEffect::Ignored;
        self.at_start &= !acted;
        if acted {
            self.keep_in_hunk(growing, step)?;
        }
        Ok(acted)
    }

    /// What the footer should say about the last move (a selection stopped at its hunk's
    /// edge), once.
    pub(super) fn take_notice(&mut self) -> Option<&'static str> {
        self.notice.take()
    }

    /// The hunk that shows `line` of `path` on `side`: its index in the file, and its first
    /// and last line on that side.
    pub(super) fn hunk_span(&self, path: &str, side: Side, line: u32) -> Option<(usize, (u32, u32))> {
        let file = self.index(path)?;
        self.hunk_of(file, side == Side::Old, line)
    }

    /// The finished or growing selection, when there is one.
    pub(super) fn selection(&self) -> Option<Lines> {
        let range = self.engine.selection()?;
        Some(Lines {
            path: self.paths.get(range.file)?.clone(),
            side: side_of(range.side),
            first: range.first,
            last: range.last,
        })
    }

    /// Whether moving the cursor grows the selection (between the two `v`s).
    pub(super) fn selecting(&self) -> bool {
        self.engine.selection_extending()
    }

    /// The cursor's line on the side it is on. `None` on a file header or a gap.
    pub(super) fn cursor(&self) -> Result<Option<Lines>> {
        let Some(at) = self.engine.selected_source()? else { return Ok(None) };
        let line = match at.side {
            engine::Side::Old => at.old_line,
            engine::Side::New => at.new_line,
        };
        let path = self.paths.get(at.file).cloned();
        Ok(path.zip(line).map(|(path, line)| Lines { path, side: side_of(at.side), first: line, last: line }))
    }

    /// The side the cursor is on.
    pub(super) fn cursor_side(&self) -> Result<Option<Side>> {
        Ok(self.engine.selected_source()?.map(|at| side_of(at.side)))
    }

    /// The cursor's line on `side`, with its file: `None` on a header, a gap, or a row that
    /// has nothing on that side (an added line has no old number).
    pub(super) fn line_on(&self, side: Side) -> Result<Option<(&str, u32)>> {
        let Some(at) = self.engine.selected_source()? else { return Ok(None) };
        let line = match side {
            Side::Old => at.old_line,
            Side::New => at.new_line,
        };
        Ok(self.paths.get(at.file).map(String::as_str).zip(line))
    }

    /// Whether `v` would start a selection here: the cursor is on a line of code and the
    /// diff, not the file list, has the focus. Asks by starting one and clearing it.
    pub(super) fn selectable(&mut self) -> Result<bool> {
        if self.engine.execute(Command::SelectLines)? == InputEffect::Ignored {
            return Ok(false);
        }
        self.run(Command::ClearSelection)?;
        Ok(true)
    }

    /// Select `lines` as `v` would, the cursor on the last.
    pub(super) fn select(&mut self, lines: &Lines) -> Result<()> {
        let file = self.index(&lines.path).with_context(|| format!("{} is not in the diff", lines.path))?;
        let range = SourceRange {
            review: self.engine.review_id(),
            file,
            side: engine_side(lines.side),
            first: lines.first,
            last: lines.last,
        };
        self.at_start = false;
        self.engine.select_range(range)?;
        Ok(())
    }

    /// Select `lines` from a drag, or a shift-click, that went `up` to `lines.first` from
    /// `lines.last` or down to `lines.last`, the cursor on the line it went to, and leave
    /// the view where it is.
    ///
    /// [`Self::select`] puts the cursor on the last line, scrolling when that line is not on
    /// screen, and forgets which screen rows the viewer drew: until the next frame every
    /// line looks off screen to it, so a [`Self::reveal`] of the first line right after a
    /// drag up brought that line to the pane's bottom, and a pane resized before the next
    /// frame (a footer that takes a row less) put the cursor's line on the wrong row. So a
    /// selection is made by clicking where its two ends are drawn
    /// ([`Self::select_in_place`]), and otherwise only when one end is off screen: a drag
    /// held past the pane's edge, where the view goes to the line the pointer reached.
    pub(super) fn select_from(&mut self, lines: &Lines, up: bool) -> Result<()> {
        if self.select_in_place(lines, up)? {
            return Ok(());
        }
        self.select(lines)?;
        if up {
            self.reveal(&lines.path, lines.side, lines.first)?;
        }
        Ok(())
    }

    pub(super) fn begin_or_end_selection(&mut self) -> Result<()> {
        self.run(Command::SelectLines)
    }

    pub(super) fn clear_selection(&mut self) -> Result<()> {
        self.run(Command::ClearSelection)
    }

    /// Put the cursor on `lines.first`. An error means the diff does not show that line.
    pub(super) fn reveal(&mut self, path: &str, side: Side, line: u32) -> Result<()> {
        let file = self.index(path).with_context(|| format!("{path} is not in the diff"))?;
        self.at_start = false;
        self.engine.reveal(file, engine_side(side), line)?;
        Ok(())
    }

    /// Replace every gutter marker. Lines in files that have not arrived are skipped; the
    /// caller sets them again when [`Self::poll`] says more arrived.
    pub(super) fn set_markers(&mut self, marks: &[(Lines, Style)]) -> Result<()> {
        let review = self.engine.review_id();
        let markers: Vec<Marker> = marks
            .iter()
            .filter(|(lines, _)| lines.first >= 1 && lines.first <= lines.last)
            .filter_map(|(lines, style)| {
                let file = self.index(&lines.path)?;
                let range = SourceRange {
                    review,
                    file,
                    side: engine_side(lines.side),
                    first: lines.first,
                    last: lines.last,
                };
                Some(Marker { range, style: *style })
            })
            .collect();
        self.engine.set_markers(&markers)
    }
}

/// The patch as a stream: a temporary file, unlinked as soon as it is open, so nothing is
/// left behind however the review ends.
fn input_bytes(bytes: &[u8], label: String) -> Result<Input> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "plannotator-tui-patch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("creating {}", path.display()))?;
    let _ = std::fs::remove_file(&path);
    file.write_all(bytes).context("writing the patch for the viewer")?;
    file.rewind().context("rewinding the patch for the viewer")?;
    Ok(Input::Stream { file, label })
}

fn side_of(side: engine::Side) -> Side {
    match side {
        engine::Side::Old => Side::Old,
        engine::Side::New => Side::New,
    }
}

fn engine_side(side: Side) -> engine::Side {
    match side {
        Side::Old => engine::Side::Old,
        Side::New => engine::Side::New,
    }
}
