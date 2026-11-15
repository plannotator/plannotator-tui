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
use diffs_rs::engine::{self, Command, Engine, Input, InputEffect, Marker, Options};
use diffs_rs::reading::SourceRange;
use diffs_rs::theme::Appearance;
use plannotator_tui_review::{Capture, Side};
use ratatui::Frame;
use ratatui::crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::style::Style;

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
    /// The capture's path for each engine file, in engine order, as far as files have
    /// arrived.
    paths: Vec<String>,
}

impl std::fmt::Debug for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Diff").field("paths", &self.paths).finish_non_exhaustive()
    }
}

impl Diff {
    /// Show `capture`'s patch. The engine reads it on its own thread and sends on `wake`
    /// when it has something to show.
    pub(super) fn open(capture: &Capture, light: bool, wake: SyncSender<()>) -> Result<Self> {
        let options = Options {
            appearance: if light { Appearance::Light } else { Appearance::Dark },
            // The header, footer and help are this app's own (`review::draw`).
            chrome: false,
            ..Options::default()
        };
        let engine = Engine::open(input(capture)?, options, wake).context("opening the diff viewer")?;
        Ok(Self { engine, paths: Vec::new() })
    }

    /// Show a new capture of the same review, keeping the layout and wrapping.
    pub(super) fn replace(&mut self, capture: &Capture) -> Result<()> {
        self.engine.replace(input(capture)?).context("reloading the diff viewer")?;
        self.paths.clear();
        Ok(())
    }

    /// Take what the engine's workers have ready. True when more files arrived, so the
    /// caller can place markers on them.
    pub(super) fn poll(&mut self, capture: &Capture) -> Result<bool> {
        self.engine.poll()?;
        let files = self.engine.status().files;
        if files == self.paths.len() {
            return Ok(false);
        }
        self.paths = (0..files).map(|index| self.capture_path(capture, index)).collect();
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
    #[cfg(test)]
    pub(super) fn files_shown(&self) -> usize {
        self.paths.len()
    }

    fn index(&self, path: &str) -> Option<usize> {
        self.paths.iter().position(|p| p == path)
    }

    pub(super) fn needs_draw(&self) -> bool {
        self.engine.needs_draw()
    }

    pub(super) fn draw(&mut self, frame: &mut Frame, area: Rect) -> Result<()> {
        self.engine.draw(frame, area)
    }

    pub(super) fn after_draw(&mut self) {
        self.engine.after_draw();
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
        self.engine.execute(command).map(drop)
    }

    /// Mouse and resize events, which the engine handles itself. Keys never come here:
    /// this app maps them to commands. True when the engine acted on it.
    pub(super) fn handle_event(&mut self, event: Event) -> Result<bool> {
        Ok(self.engine.handle_event(event, true)? != InputEffect::Ignored)
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

    pub(super) fn begin_or_end_selection(&mut self) -> Result<()> {
        self.run(Command::SelectLines)
    }

    pub(super) fn clear_selection(&mut self) -> Result<()> {
        self.run(Command::ClearSelection)
    }

    /// Put the cursor on `lines.first`. An error means the diff does not show that line.
    pub(super) fn reveal(&mut self, path: &str, side: Side, line: u32) -> Result<()> {
        let file = self.index(path).with_context(|| format!("{path} is not in the diff"))?;
        self.engine.reveal(file, engine_side(side), line)
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
fn input(capture: &Capture) -> Result<Input> {
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
    file.write_all(&capture.patch).context("writing the patch for the viewer")?;
    file.rewind().context("rewinding the patch for the viewer")?;
    Ok(Input::Stream { file, label: capture.scope.title() })
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
