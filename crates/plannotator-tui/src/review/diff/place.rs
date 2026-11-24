//! Which file and row the diff's cursor is on, and moves that depend on it. The viewer says,
//! on every kind of row (`Engine::cursor_row`), and for every screen cell of the last frame
//! (`Engine::row_at`); nothing here remembers or works out a position of its own.

use anyhow::Result;
use diffs_rs::engine::{Command, DiffRow, DiffRowKind, InputEffect, Side};
use diffs_rs::reading::SourceRange;
use ratatui::crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{Diff, Lines};

impl Diff {
    /// The viewer's index of the file the cursor is in. `None` before any file arrived.
    pub(in crate::review) fn cursor_file(&self) -> Option<usize> {
        self.engine.cursor_row().map(|row| row.file).filter(|&file| file < self.paths.len())
    }

    /// The row under the cursor, as the viewer has it now.
    pub(in crate::review) fn cursor_row(&self) -> Option<DiffRow> {
        self.engine.cursor_row()
    }

    /// The row drawn at screen cell (`column`, `row`) in the last frame.
    pub(in crate::review) fn row_at(&self, column: u16, row: u16) -> Option<DiffRow> {
        self.engine.row_at(column, row)
    }

    /// Whether screen row `y` of the last frame shows code (a line, or a missing newline's
    /// mark), not a heading, a hunk's heading, a gap or a note.
    pub(in crate::review) fn code_row(&self, y: u16) -> bool {
        self.row_at(self.area.right().saturating_sub(1), y)
            .is_some_and(|row| matches!(row.kind, DiffRowKind::Code | DiffRowKind::NoNewline))
    }

    /// The first and last screen row of the last frame that show any of `lines`, on their
    /// side: in a split diff the cells of that side's pane. `None` when none is on screen.
    pub(in crate::review) fn rows_showing(&self, lines: &Lines) -> Option<(u16, u16)> {
        let old = lines.side == plannotator_tui_review::Side::Old;
        let column = self.side_column(lines.side);
        let mut shown = (self.area.top()..self.area.bottom()).filter(|&y| {
            self.row_at(column, y).is_some_and(|row| {
                let line = if old { row.old_line } else { row.new_line };
                row.kind == DiffRowKind::Code
                    && self.paths.get(row.file) == Some(&lines.path)
                    && line.is_some_and(|n| (lines.first..=lines.last).contains(&n))
            })
        });
        let first = shown.next()?;
        Some((first, shown.last().unwrap_or(first)))
    }

    /// A column of the last frame in the pane of `side`: in a split diff the old side is on
    /// the left.
    fn side_column(&self, side: plannotator_tui_review::Side) -> u16 {
        if side == plannotator_tui_review::Side::Old && self.split() {
            self.area.x + 1
        } else {
            self.area.right().saturating_sub(2)
        }
    }

    /// The first column of the pane of `side` in the last frame, left of its code: in a split
    /// diff the new side's starts right of the line between the sides.
    fn side_start(&self, side: plannotator_tui_review::Side) -> u16 {
        match self.split_line() {
            Some(line) if side == plannotator_tui_review::Side::New => line + 1,
            _ => self.area.x + 1,
        }
    }

    /// Select `lines` as `v` on the line a drag `up` (or down) started from and the cursor to
    /// the other end would: a click on each where the last frame drew it, so nothing scrolls
    /// and the viewer knows the screen row the cursor is on (it keeps that row when the pane
    /// is resized). False when either line is not on screen, or the clicks did not end in
    /// `lines`; the caller then selects them its own way.
    ///
    /// The clicks land left of the code, in the gutter of the side's pane, where the
    /// viewer counts them on that side whatever the line's length.
    pub(in crate::review) fn select_in_place(&mut self, lines: &Lines, up: bool) -> Result<bool> {
        let row = |line: u32| self.rows_showing(&Lines { first: line, last: line, ..lines.clone() });
        let (start, end) = if up { (lines.last, lines.first) } else { (lines.first, lines.last) };
        let (Some((from, _)), Some((to, _))) = (row(start), row(end)) else { return Ok(false) };
        let column = self.side_start(lines.side);
        let click = |row: u16| {
            let kind = MouseEventKind::Down(MouseButton::Left);
            Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE })
        };
        self.at_start = false;
        self.engine.execute(Command::ClearSelection)?;
        self.engine.handle_event(click(from), true)?;
        self.engine.execute(Command::SelectLines)?;
        self.engine.handle_event(click(to), true)?;
        self.engine.execute(Command::SelectLines)?;
        Ok(self.selection().as_ref() == Some(lines))
    }

    /// The screen row of the last frame that shows the heading of the file at `path`: its own
    /// heading, or the one kept over the top row while its lines scroll under it.
    pub(in crate::review) fn heading_row(&self, path: &str) -> Option<u16> {
        let column = self.area.right().saturating_sub(2);
        (self.area.top()..self.area.bottom()).rev().find(|&y| {
            self.row_at(column, y).is_some_and(|row| {
                matches!(row.kind, DiffRowKind::Header | DiffRowKind::StickyHeader)
                    && self.paths.get(row.file).is_some_and(|p| p == path)
            })
        })
    }

    /// Whether the diff shows its two sides next to each other, as last drawn.
    pub(in crate::review) fn split(&self) -> bool {
        split_for(self.engine.options().layout, self.area.width)
    }

    /// `n`/`N`: from the file the cursor is in to the next or the previous one, its heading
    /// at the top. False at the last or the first file, where nothing moves.
    pub(in crate::review) fn step_file(&mut self, forward: bool) -> Result<bool> {
        let Some(file) = self.cursor_file() else { return Ok(false) };
        let target = if forward { file.checked_add(1) } else { file.checked_sub(1) };
        if target.is_none_or(|t| t >= self.paths.len()) {
            return Ok(false);
        }
        self.run(if forward { Command::NextFile } else { Command::PreviousFile })?;
        Ok(true)
    }

    /// Put the heading of file `index` at the top with the cursor on it. The viewer's file
    /// jumps land on a heading and put it at the top.
    pub(in crate::review) fn show_heading(&mut self, index: usize) -> Result<()> {
        if index >= self.paths.len() {
            return Ok(());
        }
        self.at_start = false;
        for _ in 0..=self.paths.len() {
            let Some(row) = self.cursor_row() else { return Ok(()) };
            let command = match row.file.cmp(&index) {
                std::cmp::Ordering::Less => Command::NextFile,
                std::cmp::Ordering::Greater => Command::PreviousFile,
                std::cmp::Ordering::Equal if row.kind == DiffRowKind::Header && row.part == 0 => break,
                // On the file but below its heading: to a neighbour and back.
                std::cmp::Ordering::Equal if index > 0 => {
                    self.engine.execute(Command::PreviousFile)?;
                    Command::NextFile
                }
                std::cmp::Ordering::Equal if index + 1 < self.paths.len() => {
                    self.engine.execute(Command::NextFile)?;
                    Command::PreviousFile
                }
                // The only file: up to its first row. The viewer reports a move up from the
                // top as a redraw, so stop once the cursor stays where it is.
                std::cmp::Ordering::Equal => {
                    let mut before = self.cursor_row();
                    while self.engine.execute(Command::Up)? != InputEffect::Ignored {
                        let now = self.cursor_row();
                        if now == before {
                            break;
                        }
                        before = now;
                    }
                    break;
                }
            };
            if self.engine.execute(command)? == InputEffect::Ignored {
                break;
            }
        }
        Ok(())
    }

    /// The selection `v` is growing, if it is.
    pub(super) fn growing(&self) -> Option<SourceRange> {
        self.engine.selection().filter(|_| self.engine.selection_extending())
    }

    /// A selection grows within the hunk it started in, as a drag does. A step (`j`, `k`, a
    /// page, the wheel) that took it past the hunk's edge, or the cursor off the hunk, brings
    /// the cursor back to the edge line, the selection still growing. A jump elsewhere (`]`,
    /// `n`, a shift-click) ends the growing there: the selection stays as it was, kept to its
    /// hunk, and the cursor moves on freely.
    /// `step` is the direction of a step (`Some(true)` down), `None` for any other move.
    pub(super) fn keep_in_hunk(&mut self, before: Option<SourceRange>, step: Option<bool>) -> Result<()> {
        let (Some(before), Some(now)) = (before, self.growing()) else { return Ok(()) };
        let old = now.side == Side::Old;
        let Some((hunk, (first, last))) = self.hunk_of(now.file, old, before.first) else { return Ok(()) };
        let escaped = now.first < first || now.last > last;
        let cursor = self.cursor_row();
        let off = cursor.as_ref().is_some_and(|row| row.file != now.file || row.hunk != Some(hunk));
        if !escaped && !off {
            return Ok(());
        }
        self.notice = Some("a selection stays in one hunk");
        let Some(down) = step else {
            if escaped {
                let (first, last) = (now.first.max(first), now.last.min(last));
                self.engine.select_range(SourceRange { first, last, ..now })?;
            } else {
                self.engine.execute(Command::SelectLines)?;
            }
            return Ok(());
        };
        let below = if escaped { now.last > last } else { down };
        let edge = if below { last } else { first };
        self.engine.reveal(now.file, now.side, edge)?;
        if escaped {
            // A step off the edge line and back: the viewer grows the selection from its
            // first line to the line the cursor comes back to.
            let (out, back) = if below { (Command::Down, Command::Up) } else { (Command::Up, Command::Down) };
            self.engine.execute(out)?;
            self.engine.execute(back)?;
        }
        Ok(())
    }

    /// The hunk of file `file` that holds `line` on the old side (`old`) or the new one: its
    /// index, and its first and last line on that side.
    pub(super) fn hunk_of(&self, file: usize, old: bool, line: u32) -> Option<(usize, (u32, u32))> {
        let meta = self.engine.file(file)?;
        meta.hunks.iter().enumerate().find_map(|(index, hunk)| {
            let span = (*spans(&hunk.header)?.get(usize::from(!old))?)?;
            (span.0..=span.1).contains(&line).then_some((index, span))
        })
    }
}

/// A hunk heading's first and last line on the old side and on the new one, from its
/// `@@ -start,count +start,count @@`. A side with no lines has none.
fn spans(header: &str) -> Option<[Option<(u32, u32)>; 2]> {
    let mut parts = header.strip_prefix("@@ ")?.split(' ');
    let side = |part: Option<&str>, sign: char| -> Option<Option<(u32, u32)>> {
        let range = part?.strip_prefix(sign)?;
        let (start, count) = match range.split_once(',') {
            Some((start, count)) => (start.parse::<u32>().ok()?, count.parse::<u32>().ok()?),
            None => (range.parse::<u32>().ok()?, 1),
        };
        Some((count > 0).then(|| (start, start + count - 1)))
    };
    Some([side(parts.next(), '-')?, side(parts.next(), '+')?])
}

/// Whether the viewer shows the two sides next to each other at `width`: always for split,
/// never for unified, and from 120 columns for auto, as it decides.
pub(super) fn split_for(layout: diffs_rs::engine::Layout, width: u16) -> bool {
    use diffs_rs::engine::Layout;
    layout == Layout::Split || (layout == Layout::Auto && width >= 120)
}

#[cfg(test)]
mod tests {
    use super::spans;

    #[test]
    fn a_hunk_headings_ranges_give_each_sides_lines() {
        assert_eq!(spans("@@ -9,7 +9,8 @@ line 8"), Some([Some((9, 15)), Some((9, 16))]));
        assert_eq!(spans("@@ -1 +1 @@"), Some([Some((1, 1)), Some((1, 1))]));
        assert_eq!(spans("@@ -0,0 +1,2 @@"), Some([None, Some((1, 2))]));
        assert_eq!(spans("@@ -1,2 +0,0 @@"), Some([Some((1, 2)), None]));
        assert_eq!(spans("not a hunk"), None);
    }
}
