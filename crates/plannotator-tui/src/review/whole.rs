//! The whole-file view: `o` shows the file under the cursor as it is now, `O` as it was
//! before the change (the base), each scrolled to the line the cursor was on. `o`, `O` or
//! Esc goes back to the diff, which was left where it was.
//!
//! The view is a second diffs-rs engine on a patch made here: the whole file as one hunk of
//! unchanged lines, shown unified, so it has the diff's line numbers, syntax colours and
//! selection. A gutter bar marks the lines this change added (current) or removed (base),
//! and notes keep their own marks over them.
//!
//! A comment on the current file is an ordinary diff note when the diff shows its lines,
//! so it shows in the diff too; on lines outside every hunk it is a note outside the diff,
//! in the browser's shape (`outsideDiff`, its code as `anchorText`). The base is read-only
//! for notes. Files are read through the capture (the work tree, the index or a commit, the
//! cache repository's commits for a pull request), at most 5 MiB per side.

use anyhow::Result;
use diffs_rs::engine::Command;
use plannotator_tui_review::{
    Capture, FileStatus, MAX_WHOLE_FILE_BYTES, NoteScope, Side, SideSource, WholeFile,
};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use std::sync::mpsc::SyncSender;

use super::diff::{Diff, Lines};
use super::{Draft, Mode, Screen, notes};
use crate::keys::Action;
use crate::theme::palette;
use crate::ui::compose::Compose;

/// Which version of the file the view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Version {
    /// As it is now: the work tree, the index or the reviewed commit; a pull request's head.
    Current,
    /// As it was before the change: the diff's old side.
    Base,
}

impl Version {
    pub(super) fn side(self) -> Side {
        match self {
            Self::Current => Side::New,
            Self::Base => Side::Old,
        }
    }

    pub(super) fn of(side: Side) -> Self {
        match side {
            Side::New => Self::Current,
            Side::Old => Self::Base,
        }
    }
}

/// What the view shows: the file, or why it cannot.
pub(super) enum View {
    Shown { diff: Box<Diff>, content: String },
    Message(String),
}

pub(super) struct Whole {
    /// The capture's path of the file (its new path).
    pub(super) path: String,
    pub(super) version: Version,
    pub(super) view: View,
    /// The file the view was opened on: back in the diff, a different file is revealed,
    /// the same one is left where it was.
    opened_on: String,
    /// The line to put the cursor on once the engine has the file.
    reveal: Option<u32>,
    /// What the file heading says in place of the engine's `Modified +0 −0`: the file's
    /// status and counts in the diff, or that the diff does not have it.
    summary: Summary,
}

/// The file heading's right-hand summary.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Summary {
    status: &'static str,
    counts: Option<(u32, u32)>,
}

impl std::fmt::Debug for Whole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Whole")
            .field("path", &self.path)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl Whole {
    fn load(
        capture: &Capture,
        path: &str,
        version: Version,
        reveal: Option<u32>,
        light: bool,
        wake: SyncSender<()>,
    ) -> Self {
        let file = capture.file(path);
        let side_path = match (version, file.and_then(|f| f.old_path.as_deref())) {
            (Version::Base, Some(old)) => old,
            _ => path,
        };
        let view = match capture.read_whole(version.side(), side_path) {
            Ok(WholeFile::Text(content)) if content.is_empty() => View::Message(format!("{path} is empty.")),
            Ok(WholeFile::Text(content)) => {
                match Diff::open_whole(&whole_patch(path, &content), path, light, wake) {
                    Ok(diff) => View::Shown { diff: Box::new(diff), content },
                    Err(err) => View::Message(format!("could not show {path}: {err:#}")),
                }
            }
            Ok(WholeFile::Missing) => View::Message(missing(path, version, file.is_some())),
            Ok(WholeFile::TooLarge { bytes }) => View::Message(format!(
                "{path} is {:.1} MiB; the whole-file view shows files up to {} MiB.",
                bytes as f64 / 1_048_576.0,
                MAX_WHOLE_FILE_BYTES / 1_048_576
            )),
            Ok(WholeFile::Binary) => View::Message(format!("{path} is a binary file.")),
            Err(err) => View::Message(format!("could not read {path}: {err}")),
        };
        let reveal = match &view {
            View::Shown { content, .. } => reveal.map(|line| line.clamp(1, line_count(content))),
            View::Message(_) => None,
        };
        let summary = match file {
            Some(file) => Summary {
                status: match file.status {
                    FileStatus::Added => "Added",
                    FileStatus::Deleted => "Deleted",
                    FileStatus::Modified => "Modified",
                    FileStatus::Renamed => "Renamed",
                },
                counts: Some((file.additions, file.deletions)),
            },
            None => Summary { status: "Not in the diff", counts: None },
        };
        Self { path: path.to_owned(), version, view, opened_on: path.to_owned(), reveal, summary }
    }

    pub(super) fn diff(&self) -> Option<&Diff> {
        match &self.view {
            View::Shown { diff, .. } => Some(&**diff),
            View::Message(_) => None,
        }
    }

    pub(super) fn diff_mut(&mut self) -> Option<&mut Diff> {
        match &mut self.view {
            View::Shown { diff, .. } => Some(&mut **diff),
            View::Message(_) => None,
        }
    }

    /// Take what the engine has ready. True when the file arrived, so marks can go on it.
    pub(super) fn poll(&mut self, capture: &Capture) -> Result<bool> {
        let Some(diff) = self.diff_mut() else { return Ok(false) };
        if !diff.poll(capture)? {
            return Ok(false);
        }
        if let Some(line) = self.reveal.take() {
            let path = self.path.clone();
            if let Some(diff) = self.diff_mut() {
                diff.reveal(&path, Side::New, line)?;
            }
        }
        Ok(true)
    }

    pub(super) fn needs_draw(&self) -> bool {
        self.diff().is_some_and(Diff::needs_draw)
    }

    pub(super) fn after_draw(&mut self) {
        if let Some(diff) = self.diff_mut() {
            diff.after_draw();
        }
    }

    pub(super) fn close(&mut self) -> Result<()> {
        self.diff_mut().map_or(Ok(()), Diff::close)
    }

    /// Mouse and resize events, which the engine handles itself.
    pub(super) fn handle_event(&mut self, event: ratatui::crossterm::event::Event) -> Result<()> {
        self.diff_mut().map_or(Ok(()), |diff| diff.handle_event(event).map(drop))
    }

    /// Put the cursor on `line`, now or once the file has arrived.
    pub(super) fn reveal_line(&mut self, line: u32) -> Result<()> {
        let path = self.path.clone();
        match self.diff_mut() {
            Some(diff) if diff.files_shown() > 0 => diff.reveal(&path, Side::New, line),
            Some(_) => {
                self.reveal = Some(line);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// The cursor's line, or the selection's lines, in the file.
    pub(super) fn target(&self) -> Result<Option<(u32, u32)>> {
        let Some(diff) = self.diff() else { return Ok(None) };
        if let Some(lines) = diff.selection() {
            return Ok(Some((lines.first.min(lines.last), lines.first.max(lines.last))));
        }
        Ok(diff.cursor()?.map(|lines| (lines.first, lines.first)))
    }

    /// Lines `first..=last` of the file, joined with `\n`, as a note quotes them.
    pub(super) fn code(&self, first: u32, last: u32) -> Option<String> {
        let View::Shown { content, .. } = &self.view else { return None };
        let body = content.strip_suffix('\n').unwrap_or(content);
        let lines: Vec<&str> = body
            .split('\n')
            .skip(first.saturating_sub(1) as usize)
            .take((last - first + 1) as usize)
            .collect();
        (lines.len() == (last - first + 1) as usize).then(|| lines.join("\n"))
    }

    pub(super) fn draw(&mut self, frame: &mut Frame, area: Rect) -> Result<()> {
        match &mut self.view {
            View::Shown { diff, .. } => {
                diff.draw(frame, area)?;
                relabel(frame.buffer_mut(), area, &self.summary);
                Ok(())
            }
            View::Message(text) => {
                let lines = vec![Line::default(), Line::from(Span::raw(format!(" {text}")))];
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
                Ok(())
            }
        }
    }
}

/// What diffs-rs writes at the right of the file heading for the made-up patch: the lines
/// are all unchanged, so `Modified` with no additions or deletions.
const ENGINE_SUMMARY: &str = " Modified   +0 \u{2212}0 ";

/// Write the file's own status and counts over the engine's summary on the heading row (the
/// view's first row: the heading, or the engine's sticky copy of it once scrolled). The new
/// text ends where the engine's did and keeps its background; diffs-rs has no option to
/// set the heading's text.
fn relabel(buf: &mut Buffer, area: Rect, summary: &Summary) {
    let pattern: Vec<char> = ENGINE_SUMMARY.chars().collect();
    let width = pattern.len() as u16;
    let y = area.y;
    let symbol = |buf: &Buffer, x: u16| buf.cell((x, y)).map(|c| c.symbol().to_owned()).unwrap_or_default();
    let Some(start) = (area.x..area.right().saturating_sub(width).saturating_add(1))
        .rev()
        .find(|&x| pattern.iter().enumerate().all(|(i, ch)| symbol(buf, x + i as u16) == ch.to_string()))
    else {
        return;
    };
    let end = start + width;
    let style_at =
        |buf: &Buffer, x: u16| buf.cell((x, y)).map(ratatui::buffer::Cell::style).unwrap_or_default();
    let dim = style_at(buf, start);
    let status_fg = match summary.status {
        "Added" => Some(palette().looks_good),
        "Deleted" => Some(palette().delete),
        "Not in the diff" => None,
        // The engine's own colour for a modified file; a rename gets it too.
        _ => style_at(buf, start + 1).fg,
    };
    let mut spans =
        vec![(" ".to_owned(), dim), (summary.status.to_owned(), status_fg.map_or(dim, |fg| dim.fg(fg)))];
    if let Some((added, removed)) = summary.counts {
        let count = |n: u32, fg: Color| if n == 0 { dim } else { dim.fg(fg) };
        spans.push(("   ".to_owned(), dim));
        spans.push((format!("+{added}"), count(added, palette().looks_good)));
        spans.push((" ".to_owned(), dim));
        spans.push((format!("\u{2212}{removed}"), count(removed, palette().delete)));
    }
    spans.push((" ".to_owned(), dim));
    let text_width: u16 = spans.iter().map(|(t, _)| t.chars().count() as u16).sum();
    let from = end.saturating_sub(text_width).max(area.x);
    for x in start.min(from)..end {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(" ").set_style(dim);
        }
    }
    let mut x = from;
    for (text, style) in spans {
        let (next, _) = buf.set_stringn(x, y, &text, usize::from(end.saturating_sub(x)), style);
        x = next;
    }
}

/// Why there is no such version of the file.
fn missing(path: &str, version: Version, in_diff: bool) -> String {
    match (version, in_diff) {
        (Version::Current, true) => format!("{path} is deleted by this change: there is no current version."),
        (Version::Base, true) => format!("{path} is added by this change: there is no base version."),
        (Version::Current, false) => format!("{path} does not exist now."),
        (Version::Base, false) => format!("{path} did not exist before this change."),
    }
}

/// The lines of `content`; a final newline does not start another.
fn line_count(content: &str) -> u32 {
    let body = content.strip_suffix('\n').unwrap_or(content);
    u32::try_from(body.split('\n').count()).unwrap_or(u32::MAX)
}

/// `content` as a patch diffs-rs reads: one hunk of unchanged lines covering the whole file,
/// with git's marker when the last line has no newline. The path names the file (its
/// extension picks the syntax); characters a patch header cannot carry become `_`.
pub(super) fn whole_patch(path: &str, content: &str) -> Vec<u8> {
    let name: String =
        path.chars().map(|c| if c.is_control() || matches!(c, '"' | '\\') { '_' } else { c }).collect();
    let body = content.strip_suffix('\n').unwrap_or(content);
    let n = line_count(content);
    let mut out = format!("diff --git a/{name} b/{name}\n--- a/{name}\n+++ b/{name}\n@@ -1,{n} +1,{n} @@\n");
    out.reserve(content.len() + n as usize);
    for line in body.split('\n') {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    if !content.ends_with('\n') {
        out.push_str("\\ No newline at end of file\n");
    }
    out.into_bytes()
}

impl Screen {
    /// `o` or `O` in the diff: the whole file under the cursor, at its line on that side.
    pub(super) fn open_whole(&mut self, version: Version) -> Result<()> {
        let at = match self.diff.selection() {
            Some(lines) => Some(lines),
            None => self.diff.cursor()?,
        };
        let (path, line) = if let Some(Lines { path, side, first, .. }) = at {
            let line = match self.file(&path) {
                Some(file) if side != version.side() => file.map_line(side, first),
                _ => first,
            };
            (path, line)
        } else {
            let Some(path) = self.file_under_cursor() else {
                self.status = Some("move the cursor onto a file first".into());
                return Ok(());
            };
            (path, 1)
        };
        self.show_whole(&path, version, Some(line));
        Ok(())
    }

    /// Show `path`'s `version` in the whole-file view, at `line`, replacing any shown now.
    pub(super) fn show_whole(&mut self, path: &str, version: Version, line: Option<u32>) {
        let opened_on = match self.whole.take() {
            Some(mut old) => {
                let _ = old.close();
                old.opened_on
            }
            None => path.to_owned(),
        };
        let light = crate::theme::theme() == crate::theme::Theme::Light;
        let mut whole = Whole::load(&self.capture, path, version, line, light, self.wake.clone());
        whole.opened_on = opened_on;
        self.status = None;
        self.whole = Some(whole);
    }

    /// Back to the diff: where it was, or at the file the view moved to.
    pub(super) fn close_whole(&mut self) -> Result<()> {
        let Some(mut whole) = self.whole.take() else { return Ok(()) };
        let line = whole.target()?.map(|(first, _)| first);
        whole.close()?;
        if whole.path == whole.opened_on {
            return Ok(());
        }
        let side = whole.version.side();
        let place = self.file(&whole.path).and_then(|file| match line {
            Some(n) if file.shows(side, n, n) => Some((side, n)),
            _ => {
                let new = file.lines(Side::New).next().map(|(n, _)| (Side::New, n));
                new.or_else(|| file.lines(Side::Old).next().map(|(n, _)| (Side::Old, n)))
            }
        });
        if let Some((side, n)) = place {
            let _ = self.diff.reveal(&whole.path, side, n);
        }
        Ok(())
    }

    /// A key for the diff pane while the whole-file view is open.
    pub(super) fn whole_key(&mut self, action: Action) -> Result<()> {
        let selecting = self.whole.as_ref().and_then(Whole::diff).is_some_and(|d| d.selection().is_some());
        let command = match action {
            Action::Back if selecting => Command::ClearSelection,
            Action::Back | Action::WholeFile | Action::WholeBase => return self.close_whole(),
            Action::NextFile => {
                self.whole_next_file(1);
                return Ok(());
            }
            Action::PreviousFile => {
                self.whole_next_file(-1);
                return Ok(());
            }
            Action::NextHunk => return self.whole_next_change(true),
            Action::PreviousHunk => return self.whole_next_change(false),
            Action::Comment => return self.start_whole_comment(),
            Action::Suggest => return self.start_whole_suggestion(),
            Action::FileComment => {
                let Some(path) = self.whole.as_ref().map(|w| w.path.clone()) else { return Ok(()) };
                if self.file(&path).is_none() {
                    self.status = Some("that file is not in the diff".into());
                    return Ok(());
                }
                self.compose = Compose::default();
                self.mode = Mode::Compose(Draft::File(path));
                return Ok(());
            }
            Action::ToggleViewed => {
                let Some(path) = self.whole.as_ref().map(|w| w.path.clone()) else { return Ok(()) };
                self.toggle_viewed_of(&path);
                return Ok(());
            }
            Action::ToggleLayout => {
                self.status = Some("the whole file is always one column".into());
                return Ok(());
            }
            Action::Visual | Action::VisualLine => Command::SelectLines,
            Action::Down => Command::Down,
            Action::Up => Command::Up,
            Action::PageDown => Command::PageDown,
            Action::PageUp => Command::PageUp,
            Action::Top => Command::FirstSource,
            Action::Bottom => Command::LastSource,
            Action::ToggleWrap => Command::ToggleWrap,
            _ => return Ok(()),
        };
        match self.whole.as_mut().and_then(Whole::diff_mut) {
            Some(diff) => diff.run(command),
            None => Ok(()),
        }
    }

    /// `n`/`N` in the view: the next or previous file the list shows, same version, from its
    /// top.
    fn whole_next_file(&mut self, delta: isize) {
        let Some(whole) = &self.whole else { return };
        let (path, version) = (whole.path.clone(), whole.version);
        let shown = self.files.shown();
        let at = shown.iter().position(|p| *p == path);
        let next = match at {
            Some(i) => i.checked_add_signed(delta).filter(|&n| n < shown.len()),
            None => Some(0),
        };
        match next.and_then(|n| shown.get(n)).cloned() {
            Some(next) => self.show_whole(&next, version, None),
            None => self.status = Some(if delta > 0 { "last file" } else { "first file" }.into()),
        }
    }

    /// `]`/`[` in the view: the next or previous run of lines this change added (current)
    /// or removed (base).
    fn whole_next_change(&mut self, forward: bool) -> Result<()> {
        let Some(whole) = &self.whole else { return Ok(()) };
        let side = whole.version.side();
        let cursor = whole.target()?.map_or(0, |(first, _)| first);
        let ranges = self.file(&whole.path).map(|f| f.changed_ranges(side)).unwrap_or_default();
        let next = if forward {
            ranges.iter().map(|r| r.0).find(|&start| start > cursor)
        } else {
            ranges.iter().rev().find(|r| r.1 < cursor).map(|r| r.0)
        };
        let path = whole.path.clone();
        match (next, self.whole.as_mut().and_then(Whole::diff_mut)) {
            (Some(line), Some(diff)) => diff.reveal(&path, Side::New, line)?,
            _ => self.status = Some("no more changes in this file".into()),
        }
        Ok(())
    }

    /// `c` in the view: a diff note when the diff shows the lines, else a note outside it.
    fn start_whole_comment(&mut self) -> Result<()> {
        let Some(whole) = &self.whole else { return Ok(()) };
        if whole.version == Version::Base {
            self.status = Some(BASE_READ_ONLY.into());
            return Ok(());
        }
        let Some((first, last)) = whole.target()? else {
            self.status = Some("move the cursor onto a line first".into());
            return Ok(());
        };
        let lines = Lines { path: whole.path.clone(), side: Side::New, first, last };
        let in_diff = self.file(&whole.path).is_some_and(|f| f.shows(Side::New, first, last));
        let draft = if in_diff {
            Draft::Comment(lines)
        } else {
            let Some(code) = whole.code(first, last) else { return Ok(()) };
            Draft::Outside(lines, code)
        };
        if let Some(diff) = self.whole.as_mut().and_then(Whole::diff_mut) {
            let _ = diff.clear_selection();
        }
        self.compose = Compose::default();
        self.mode = Mode::Compose(draft);
        Ok(())
    }

    /// `s` in the view: a suggestion, on lines the diff shows.
    fn start_whole_suggestion(&mut self) -> Result<()> {
        let Some(whole) = &self.whole else { return Ok(()) };
        if whole.version == Version::Base {
            self.status = Some(BASE_READ_ONLY.into());
            return Ok(());
        }
        let Some((first, last)) = whole.target()? else { return Ok(()) };
        let path = whole.path.clone();
        let Some(file) = self.file(&path).filter(|f| f.shows(Side::New, first, last)) else {
            self.status = Some("a suggestion needs lines the diff shows; c comments here".into());
            return Ok(());
        };
        let code: Vec<&str> = (first..=last).filter_map(|n| file.line(Side::New, n)).collect();
        self.compose = Compose::with_text(&code.join("\n"));
        self.mode = Mode::Compose(Draft::Suggestion(Lines { path, side: Side::New, first, last }));
        Ok(())
    }

    /// The view's gutter: a bar on the lines this change added (current) or removed (base),
    /// and the marks of the notes on those lines of this side, over them.
    pub(super) fn sync_whole_markers(&mut self) {
        let Some(whole) = &self.whole else { return };
        let (path, side) = (whole.path.clone(), whole.version.side());
        let colour = if side == Side::New { palette().looks_good } else { palette().delete };
        let bar = Style::new().fg(colour);
        let mut marks: Vec<(Lines, Style)> = Vec::new();
        for (first, last) in self.file(&path).map(|f| f.changed_ranges(side)).unwrap_or_default() {
            // Both engine sides carry the same numbers in this one-hunk patch.
            for engine_side in [Side::Old, Side::New] {
                marks.push((Lines { path: path.clone(), side: engine_side, first, last }, bar));
            }
        }
        let unsent = Style::new().fg(palette().comment);
        let sent = Style::new().fg(Color::DarkGray);
        for note in self.store.annotations() {
            let placed = notes::in_diff(note) || notes::is_not_in_diff(note);
            if note.scope != NoteScope::Line || !placed || note.file_path != path || note.side != side {
                continue;
            }
            let style = if self.store.is_pending(note) { unsent } else { sent };
            for engine_side in [Side::Old, Side::New] {
                let lines = Lines {
                    path: path.clone(),
                    side: engine_side,
                    first: note.line_start,
                    last: note.line_end,
                };
                marks.push((lines, style));
            }
        }
        // The lines a drag in the view is going over, over everything else.
        if let Some(lines) = self.drag.as_ref().and_then(super::mouse::Drag::lines) {
            for engine_side in [Side::Old, Side::New] {
                marks.push((Lines { side: engine_side, ..lines.clone() }, super::mouse::drag_mark()));
            }
        }
        if let Some(Err(err)) = self.whole.as_mut().and_then(Whole::diff_mut).map(|d| d.set_markers(&marks)) {
            self.status = Some(format!("could not mark the whole file: {err:#}"));
        }
    }

    /// The header's title while the view is open: `Whole file · new · src/a.rs`, or
    /// `Whole file · base (1a2b3c4) · src/a.rs`.
    pub(super) fn whole_title(&self) -> Option<String> {
        let whole = self.whole.as_ref()?;
        let version = match (whole.version, &self.capture.old) {
            (Version::Current, _) => "new".to_owned(),
            (Version::Base, SideSource::Rev(rev)) => format!("base ({})", rev.get(..7).unwrap_or(rev)),
            (Version::Base, _) => "base".to_owned(),
        };
        Some(format!("Whole file \u{b7} {version} \u{b7} {}", whole.path))
    }
}

/// What the footer and `c` say in the base version.
pub(super) const BASE_READ_ONLY: &str = "base version: read-only for notes";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_file_is_one_unchanged_hunk_with_the_no_newline_marker() {
        assert_eq!(
            String::from_utf8_lossy(&whole_patch("src/a.rs", "fn a() {}\nfn b() {}\n")),
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n fn b() {}\n"
        );
        assert_eq!(
            String::from_utf8_lossy(&whole_patch("we\"ird\tname", "last")),
            "diff --git a/we_ird_name b/we_ird_name\n--- a/we_ird_name\n+++ b/we_ird_name\n@@ -1,1 +1,1 @@\n last\n\\ No newline at end of file\n"
        );
        assert_eq!(line_count("a\n\nb"), 3);
    }
}
