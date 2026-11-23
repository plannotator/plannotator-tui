//! The diff viewer: one diffs-rs [`Engine`] showing a capture's patch, with this app's
//! names for what it points at. Nothing outside this file speaks diffs-rs's types.
//!
//! The engine numbers files in patch order and lines by their original source line on each
//! side; a note names a file by path. [`Lines`] is the meeting point: a run of lines on one
//! side of one file, by path.

use std::collections::HashMap;
use std::io::{Seek as _, Write as _};
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::SyncSender;

use anyhow::{Context as _, Result};
use diffs_rs::engine::{self, Command, Engine, Input, InputEffect, Marker, Options, SidebarPreference};
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
    /// Each file's section of the capture's patch, by path, for the patch of the key.
    sections: Option<(String, Sections)>,
    /// The capture's path for each engine file, in engine order, as far as files have
    /// arrived.
    paths: Vec<String>,
    /// A whole-file view's one file, named by its capture path whatever the patch says.
    fixed: Option<String>,
}

impl std::fmt::Debug for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Diff").field("paths", &self.paths).finish_non_exhaustive()
    }
}

impl Diff {
    /// Show the files at `paths` of `capture`'s patch, in that order. The engine reads them
    /// on its own thread and sends on `wake` when it has something to show.
    pub(super) fn open(
        capture: &Capture,
        paths: &[String],
        light: bool,
        wake: SyncSender<()>,
    ) -> Result<Self> {
        let options = Options {
            appearance: if light { Appearance::Light } else { Appearance::Dark },
            // The header, footer, help and file list are this app's own (`review::draw`,
            // `review::files`).
            chrome: false,
            sidebar: SidebarPreference::Hide,
            ..Options::default()
        };
        let sections = Sections::of(&capture.patch);
        let patch = sections.select(&capture.patch, paths);
        let input = input_bytes(&patch, capture.scope.title())?;
        let mut engine = Engine::open(input, options, wake).context("opening the diff viewer")?;
        // Draw on the terminal's own background and text colour, as the document screen does;
        // the changed-line tints come from the light or dark palette picked above.
        engine.set_terminal_background(true)?;
        Ok(Self {
            engine,
            sections: Some((capture.patch_key.clone(), sections)),
            paths: Vec::new(),
            fixed: None,
        })
    }

    /// A whole-file view: `patch` is one file as a single unchanged hunk (`super::whole`),
    /// shown unified, its lines named by `path`.
    pub(super) fn open_whole(bytes: &[u8], name: &str, light: bool, wake: SyncSender<()>) -> Result<Self> {
        let options = Options {
            appearance: if light { Appearance::Light } else { Appearance::Dark },
            layout: engine::Layout::Unified,
            chrome: false,
            ..Options::default()
        };
        let input = input_bytes(bytes, name.to_owned())?;
        let mut engine = Engine::open(input, options, wake).context("opening the whole-file view")?;
        // The same terminal background as the diff, so both views match.
        engine.set_terminal_background(true)?;
        Ok(Self { engine, sections: None, paths: Vec::new(), fixed: Some(name.to_owned()) })
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
        if let Some(path) = &self.fixed {
            self.paths = vec![path.clone(); files.min(1)];
            return Ok(true);
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

/// Each file's section of a patch: from its `diff --git` line to the next one.
#[derive(Debug, Default)]
struct Sections(HashMap<String, Range<usize>>);

impl Sections {
    fn of(patch: &[u8]) -> Self {
        let mut starts = Vec::new();
        let mut offset = 0;
        for line in patch.split_inclusive(|b| *b == b'\n') {
            if line.starts_with(b"diff --git ") {
                starts.push(offset);
            }
            offset += line.len();
        }
        let ends = starts.iter().skip(1).copied().chain([patch.len()]);
        let sections = starts
            .iter()
            .zip(ends)
            .filter_map(|(&from, to)| {
                let text = String::from_utf8_lossy(patch.get(from..to)?);
                let file = plannotator_tui_review::patch::parse(&text).into_iter().next()?.path;
                Some((file, from..to))
            })
            .collect();
        Self(sections)
    }

    /// The sections of `paths`, in that order, as one patch.
    fn select(&self, patch: &[u8], paths: &[String]) -> Vec<u8> {
        paths
            .iter()
            .filter_map(|path| self.0.get(path).and_then(|range| patch.get(range.clone())))
            .flatten()
            .copied()
            .collect()
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
