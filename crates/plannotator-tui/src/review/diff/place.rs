//! Which file the diff's cursor is in. On a line the viewer says. On a file's heading, a
//! hunk's heading, a gap or the note of a file without hunks (binary, mode only) it does
//! not, so the cursor looks for the nearest line in one direction, comes back, looks in the
//! other and comes back, and the rows between say which file it is in. The viewer lays
//! files out the same way every time: a file without hunks takes two rows (its heading and
//! its note), a file with hunks starts with its heading, a gap when its first hunk does not
//! start at line one, and that hunk's heading.
//!
//! Every press of a look either moved the cursor or the look found nothing, so coming back
//! takes as many presses. A look that finds nothing ran into the top or the end of the diff;
//! from there the cursor counts its way back. Which way to look first follows the last move:
//! after a move down there is a line above, unless the cursor came from the top.

use anyhow::Result;
use diffs_rs::engine::Command;

use super::Diff;

/// The file the cursor was last found in, and whether a line is above it and below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::review) struct Place {
    file: usize,
    above: bool,
    below: bool,
}

/// A line a look reached: its file, the presses it took, and its line numbers.
struct Found {
    file: usize,
    presses: usize,
    old: Option<u32>,
    new: Option<u32>,
}

impl Diff {
    /// The viewer's index of the file the cursor is in. `None` before any file arrived.
    pub(in crate::review) fn cursor_file(&mut self) -> Result<Option<usize>> {
        let moved = self.moved.take();
        if let Some(at) = self.engine.selected_source()? {
            self.place = Some(Place { file: at.file, above: true, below: true });
            return Ok(Some(at.file));
        }
        let files = self.paths.len();
        let without = (0..files).filter(|&i| !self.has_hunks(i)).count();
        if files == 0 || without == files {
            // No line to count from: the file the last jump went to.
            return Ok(self.place.map(|p| p.file.min(files.saturating_sub(1))));
        }
        let limit = 2 * without + 6;
        let last = self.place.unwrap_or(Place { file: 0, above: true, below: true });
        let up_first = match moved {
            Some(true) | None => last.above,
            Some(false) => !last.below,
        };
        let place = if up_first { self.looking_up_first(limit)? } else { self.looking_down_first(limit)? };
        self.place = Some(place);
        Ok(Some(place.file))
    }

    /// `n`/`N`: from the file the cursor is in to the next or the previous one, its heading
    /// at the top. False at the last or the first file, where nothing moves.
    pub(in crate::review) fn step_file(&mut self, forward: bool) -> Result<bool> {
        let Some(file) = self.cursor_file()? else { return Ok(false) };
        let target = if forward { file.checked_add(1) } else { file.checked_sub(1) };
        let Some(target) = target.filter(|&t| t < self.paths.len()) else { return Ok(false) };
        self.engine.execute(if forward { Command::NextFile } else { Command::PreviousFile })?;
        self.place = Some(self.heading(target));
        Ok(true)
    }

    /// Put the heading of file `index` at the top with the cursor on it.
    pub(in crate::review) fn show_heading(&mut self, index: usize) -> Result<()> {
        if index >= self.paths.len() {
            return Ok(());
        }
        // From a line of the file, the previous file's heading and back is this file's
        // heading; from a line of an earlier file, as many files on.
        let mut from = None;
        for i in (0..=index).rev() {
            if self.has_hunks(i) && self.reveal_first(i) {
                from = Some(i);
                break;
            }
        }
        match from {
            Some(0) if index == 0 => self.go_to_top()?,
            Some(from) if from == index => {
                self.engine.execute(Command::PreviousFile)?;
                self.engine.execute(Command::NextFile)?;
            }
            Some(from) => {
                for _ in from..index {
                    self.engine.execute(Command::NextFile)?;
                }
            }
            None => {
                self.go_to_top()?;
                for _ in 0..index {
                    self.engine.execute(Command::NextFile)?;
                }
            }
        }
        self.moved = None;
        self.place = Some(self.heading(index));
        Ok(())
    }

    /// The cursor on the first row: the first file's heading.
    fn go_to_top(&mut self) -> Result<()> {
        self.engine.execute(Command::FirstSource)?;
        let rows = 2 * self.paths.len() + 4;
        self.press(Command::Up, rows)
    }

    /// Put the cursor on a line of file `index`'s first hunk.
    fn reveal_first(&mut self, index: usize) -> bool {
        let Some(hunk) = self.engine.file(index).and_then(|f| f.hunks.first()) else { return false };
        let (new, old) = (hunk.new_start, hunk.old_start);
        let new_side = new > 0 && self.engine.reveal(index, diffs_rs::engine::Side::New, new).is_ok();
        new_side || (old > 0 && self.engine.reveal(index, diffs_rs::engine::Side::Old, old).is_ok())
    }

    /// The place of file `index`'s heading.
    fn heading(&self, index: usize) -> Place {
        Place {
            file: index,
            above: (0..index).any(|i| self.has_hunks(i)),
            below: (index..self.paths.len()).any(|i| self.has_hunks(i)),
        }
    }

    fn has_hunks(&self, index: usize) -> bool {
        self.engine.file(index).is_some_and(|f| !f.hunks.is_empty())
    }

    fn press(&mut self, command: Command, times: usize) -> Result<()> {
        for _ in 0..times {
            self.engine.execute(command)?;
        }
        Ok(())
    }

    /// Press `command` until the cursor is on a line, at most `limit` times.
    fn look(&mut self, command: Command, limit: usize) -> Result<Option<Found>> {
        for presses in 1..=limit {
            self.engine.execute(command)?;
            if let Some(at) = self.engine.selected_source()? {
                return Ok(Some(Found { file: at.file, presses, old: at.old_line, new: at.new_line }));
            }
        }
        Ok(None)
    }

    /// Look up first, then down.
    fn looking_up_first(&mut self, limit: usize) -> Result<Place> {
        let files = self.paths.len();
        let Some(up) = self.look(Command::Up, limit)? else {
            // Nothing above: the cursor is on the first row now, the first file's heading.
            return Ok(self.heading(0));
        };
        self.press(Command::Down, up.presses)?;
        if up.file + 1 >= files {
            return Ok(Place { file: up.file, above: true, below: false });
        }
        if let Some(down) = self.look(Command::Down, limit)? {
            self.press(Command::Up, down.presses)?;
            return Ok(Place { file: self.between(&up, &down), above: true, below: true });
        }
        // Nothing below: the presses went to the end. Up from there to the same line counts
        // the rows after it; down again as far as the first look comes back.
        let end = self.look(Command::Up, 2 * limit)?.map_or(up.presses, |f| f.presses);
        self.press(Command::Down, up.presses)?;
        // After the line: the rest of its file, then two rows for each file after it.
        let trailing = end.saturating_sub(2 * (files - 1 - up.file));
        let file = if up.presses <= trailing {
            up.file
        } else {
            (up.file + 1 + (up.presses - trailing - 1) / 2).min(files - 1)
        };
        Ok(Place { file, above: true, below: false })
    }

    /// Look down first, then up.
    fn looking_down_first(&mut self, limit: usize) -> Result<Place> {
        let Some(down) = self.look(Command::Down, limit)? else {
            // Nothing below: the cursor is on the last row now, in the last file.
            let last = self.paths.len().saturating_sub(1);
            return Ok(Place { file: last, above: true, below: false });
        };
        self.press(Command::Up, down.presses)?;
        if down.file == 0 {
            return Ok(Place { file: 0, above: false, below: true });
        }
        if let Some(up) = self.look(Command::Up, limit)? {
            self.press(Command::Down, up.presses)?;
            return Ok(Place { file: self.between(&up, &down), above: true, below: true });
        }
        // Nothing above: the presses went to the top. Down from there to the same line
        // counts the rows before it; up again as far as the first look comes back.
        let top = self.look(Command::Down, 2 * limit)?.map_or(down.presses, |f| f.presses);
        self.press(Command::Up, down.presses)?;
        // Before the line: two rows for each file before its own, then its own first rows.
        let leading = top.saturating_sub(2 * down.file);
        let file = if down.presses <= leading {
            down.file
        } else {
            down.file - 1 - ((down.presses - leading - 1) / 2).min(down.file - 1)
        };
        Ok(Place { file, above: false, below: true })
    }

    /// The file of a row between a line of `up.file` above and a line of `down.file` below.
    fn between(&self, up: &Found, down: &Found) -> usize {
        if up.file == down.file {
            return up.file;
        }
        let leading = self.leading_rows(down);
        if down.presses <= leading {
            return down.file;
        }
        let back = (down.presses - leading - 1) / 2;
        let without = down.file - up.file - 1;
        if back < without { down.file - 1 - back } else { up.file }
    }

    /// The rows of `found.file` above `found`, its first line: the heading, a gap when the
    /// first hunk does not start at line one, and that hunk's heading. A line shown from
    /// the gap comes after the gap's row, or right after the heading once all of it shows.
    fn leading_rows(&self, found: &Found) -> usize {
        let Some(hunk) = self.engine.file(found.file).and_then(|f| f.hunks.first()) else { return 2 };
        let before = |line: Option<u32>, start: u32| line.is_some_and(|n| n < start);
        if before(found.new, hunk.new_start) || before(found.old, hunk.old_start) {
            return if found.new == Some(1) || found.old == Some(1) { 1 } else { 2 };
        }
        2 + usize::from(hunk.old_start > 1 || hunk.new_start > 1)
    }
}
