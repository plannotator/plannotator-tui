//! Which file and row the diff's cursor is on, worked out without moving it.
//!
//! On a line the viewer says. On a file's heading, a hunk's heading, a gap, a binary file's
//! note or a side with no line it does not, so this keeps track: every move the app makes
//! goes through [`Diff`], and from where the cursor was and what moved it (a row up or down,
//! a page, a jump to a hunk or a file, a click on a row of the last frame, the wheel) the row
//! it is on now follows from the rows the viewer lays out (`model`). The last frame says what
//! each screen row holds, for clicks and the wheel. What cannot be worked out (a page with
//! long lines wrapped, the wheel before the first frame) leaves the file it was in, until the
//! cursor reaches a line again; nothing is ever pressed to find out.

use anyhow::Result;
use diffs_rs::engine::{Command, Layout, Side};
use diffs_rs::reading::SourceRange;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr as _;

use super::Diff;
use super::model::Row;

/// Where the cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::review) enum At {
    /// On a line: its file and its numbers, as the viewer reports them.
    Line { file: usize, old: Option<u32>, new: Option<u32> },
    /// On row `row` of `file`.
    Row { file: usize, row: usize },
    /// Somewhere in `file`.
    File(usize),
}

impl At {
    fn file(self) -> usize {
        match self {
            Self::Line { file, .. } | Self::Row { file, .. } | Self::File(file) => file,
        }
    }
}

/// What moved the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Move {
    /// Nothing that changes its row.
    Still,
    /// Rows down (up when negative), one at a time.
    Step(isize),
    /// Pages down or up.
    Page(isize),
    /// To the next or previous hunk's heading.
    Hunk(bool),
    /// To the next or previous file's heading.
    File(bool),
    /// The first or the last line.
    Jump,
    /// The rows laid out again; true when the layout changed between split and unified.
    Reflow(bool),
    /// A click on screen row `y` of the last frame, counted from the diff's top.
    Click(u16),
    /// The wheel, a notch down or up.
    Wheel(isize),
}

/// A file and a row in it.
type Pos = (usize, usize);

/// What a screen row of the last frame held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    /// A row of the diff.
    Row(Pos),
    /// A row of the file, which row not known.
    In(usize),
    /// The heading kept at the top while the file's rows scroll under it; not a row.
    Sticky,
    /// Nothing: below the last row.
    Nothing,
}

/// The last frame, row by row.
#[derive(Debug, Clone, Default)]
pub(super) struct Seen {
    rows: Vec<Shown>,
}

/// The viewer's split layout: asked for, or chosen for a wide pane.
pub(super) fn split_for(layout: Layout, width: u16) -> bool {
    layout == Layout::Split || (layout == Layout::Auto && width >= 120)
}

impl Diff {
    /// The viewer's index of the file the cursor is in. `None` before any file arrived.
    pub(in crate::review) fn cursor_file(&self) -> Option<usize> {
        self.at.map(At::file).filter(|&file| file < self.paths.len())
    }

    /// The row the cursor is on as this file keeps it, for tests.
    #[cfg(test)]
    pub(in crate::review) fn tracked(&mut self) -> Option<Pos> {
        let at = self.at?;
        self.pos(at)
    }

    /// The row the cursor is on, found the hard way for tests: rows down to a line (up at
    /// the end), that line's row, and as many rows back. It can scroll a short pane.
    #[cfg(test)]
    pub(in crate::review) fn probe(&mut self) -> Option<Pos> {
        for (there, back, sign) in [(Command::Down, Command::Up, -1), (Command::Up, Command::Down, 1)] {
            let mut presses = 0;
            let mut found = None;
            for _ in 0..400 {
                if let Ok(Some(at)) = self.engine.selected_source() {
                    found = Some(at);
                    break;
                }
                if self.engine.execute(there).ok()? == diffs_rs::engine::InputEffect::Ignored {
                    break;
                }
                presses += 1;
            }
            for _ in 0..presses {
                self.engine.execute(back).ok()?;
            }
            if let Some(at) = found {
                let row = self.rows.row_of(at.file, self.split, at.old_line, at.new_line)?;
                return self.walk((at.file, row), sign * presses);
            }
        }
        None
    }

    /// The selection `v` is growing, if it is.
    pub(super) fn growing(&self) -> Option<SourceRange> {
        self.engine.selection().filter(|_| self.engine.selection_extending())
    }

    /// A selection grows within the hunk it started in, as a drag does. A move (`how`) that
    /// took it past the hunk's edge, or stepped the cursor off the hunk, brings the cursor
    /// back to the edge line, the selection still growing from where it started. Jumps to
    /// another hunk or file may leave the hunk; the selection stays as it was meanwhile.
    pub(super) fn keep_in_hunk(&mut self, before: Option<SourceRange>, how: Move) -> Result<()> {
        let (Some(before), Some(now)) = (before, self.growing()) else { return Ok(()) };
        let old = now.side == Side::Old;
        let Some((first, last)) = self.rows.hunk_span(now.file, old, before.first) else { return Ok(()) };
        let (Some(top), Some(bottom)) = (
            self.rows.row_on(now.file, self.split, old, first),
            self.rows.row_on(now.file, self.split, old, last),
        ) else {
            return Ok(());
        };
        let escaped = now.first < first || now.last > last;
        let stepped = matches!(how, Move::Step(_) | Move::Page(_) | Move::Wheel(_));
        let cursor = self.at.and_then(|at| self.pos(at));
        let off = cursor.is_some_and(|(file, row)| file != now.file || row < top || row > bottom);
        if !(escaped || stepped && off) {
            return Ok(());
        }
        let below = now.last > last || cursor.is_some_and(|pos| pos > (now.file, bottom));
        let edge = if below { last } else { first };
        self.engine.reveal(now.file, now.side, edge)?;
        if escaped {
            // A step off the edge line and back: the viewer grows the selection from its
            // first line to the line the cursor comes back to.
            let (out, back) = if below { (Command::Down, Command::Up) } else { (Command::Up, Command::Down) };
            self.engine.execute(out)?;
            self.engine.execute(back)?;
        }
        let before = self.at;
        self.settle(before, Move::Jump);
        self.notice = Some("a selection stays in one hunk");
        Ok(())
    }

    /// Whether screen row `y` of the last frame shows code (a line, or a missing newline's
    /// mark), not a heading, a hunk's heading or a gap. A row not identified counts as code.
    pub(in crate::review) fn code_row(&mut self, y: u16) -> bool {
        let Some(index) = y.checked_sub(self.area.y) else { return false };
        match self.seen.as_ref().and_then(|seen| seen.rows.get(usize::from(index))).copied() {
            Some(Shown::Row((file, row))) => {
                matches!(self.rows.kind(file, self.split, row), Some(Row::Code | Row::EndMark) | None)
            }
            Some(Shown::Sticky | Shown::Nothing) => false,
            Some(Shown::In(_)) | None => true,
        }
    }

    /// Whether the diff shows its two sides next to each other.
    pub(in crate::review) fn split(&self) -> bool {
        self.split
    }

    /// `n`/`N`: from the file the cursor is in to the next or the previous one, its heading
    /// at the top. False at the last or the first file, where nothing moves.
    pub(in crate::review) fn step_file(&mut self, forward: bool) -> Result<bool> {
        let Some(file) = self.cursor_file() else { return Ok(false) };
        let target = if forward { file.checked_add(1) } else { file.checked_sub(1) };
        let Some(target) = target.filter(|&t| t < self.paths.len()) else { return Ok(false) };
        self.engine.execute(if forward { Command::NextFile } else { Command::PreviousFile })?;
        self.on_heading(target);
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
        self.on_heading(index);
        Ok(())
    }

    /// The cursor was put on file `index`'s heading, at the top.
    fn on_heading(&mut self, index: usize) {
        self.at = Some(At::Row { file: index, row: 0 });
        self.top = Some((index, 0));
    }

    /// The cursor on the first row: the first file's heading.
    fn go_to_top(&mut self) -> Result<()> {
        self.engine.execute(Command::FirstSource)?;
        let rows = 2 * self.paths.len() + 4;
        for _ in 0..rows {
            self.engine.execute(Command::Up)?;
        }
        Ok(())
    }

    /// Put the cursor on a line of file `index`'s first hunk.
    fn reveal_first(&mut self, index: usize) -> bool {
        let Some(hunk) = self.engine.file(index).and_then(|f| f.hunks.first()) else { return false };
        let (new, old) = (hunk.new_start, hunk.old_start);
        let new_side = new > 0 && self.engine.reveal(index, Side::New, new).is_ok();
        new_side || (old > 0 && self.engine.reveal(index, Side::Old, old).is_ok())
    }

    fn has_hunks(&self, index: usize) -> bool {
        self.engine.file(index).is_some_and(|f| !f.hunks.is_empty())
    }

    /// After `how` moved the cursor from `before`: where it is now.
    pub(super) fn settle(&mut self, before: Option<At>, how: Move) {
        let pos = before.and_then(|at| self.pos(at));
        // Where the first row drawn is now, as far as it follows.
        let top = match how {
            Move::Still | Move::Click(_) => self.top,
            Move::Wheel(notches) => self.top.and_then(|top| self.walk(top, 3 * notches)),
            _ => None,
        };
        self.top = top;
        let now = match self.engine.selected_source() {
            Ok(Some(at)) => Some(At::Line { file: at.file, old: at.old_line, new: at.new_line }),
            Ok(None) => before.map(|before| self.predict(before, pos, how)),
            Err(_) => before.map(|before| At::File(before.file())),
        };
        self.at = now;
        if matches!(how, Move::Hunk(_) | Move::File(_)) {
            // The viewer puts the row it jumps to at the top.
            self.top = now.and_then(|at| self.pos(at));
            self.at_top = !matches!(now, Some(At::Row { .. } | At::Line { .. }));
        }
    }

    fn predict(&mut self, before: At, pos: Option<Pos>, how: Move) -> At {
        let file = before.file();
        let row = |(file, row): Pos| At::Row { file, row };
        let unknown = At::File(file);
        let wrap = self.engine.options().wrap;
        match how {
            // A side with no line, after `h`/`l`; the same rows laid out again.
            Move::Still | Move::Reflow(false) => pos.map_or(before, row),
            Move::Step(rows) => {
                // A wrapped row of a split diff can be taller on the other side: a step
                // may stay on it.
                let code = matches!(before, At::Line { .. })
                    || pos.is_some_and(|(f, r)| self.rows.kind(f, self.split, r) == Some(Row::Code));
                if wrap && self.split && code {
                    return unknown;
                }
                pos.and_then(|pos| self.walk(pos, rows)).map_or(unknown, row)
            }
            Move::Page(pages) if !wrap => {
                let page = isize::try_from(self.size.1).unwrap_or(0).saturating_sub(2).max(1);
                pos.and_then(|pos| self.walk(pos, pages * page)).map_or(unknown, row)
            }
            Move::Page(_) => unknown,
            Move::Hunk(forward) => {
                let files = self.paths.len();
                let split = self.split;
                match pos {
                    Some((f, r)) => self.rows.hunk_from(f, split, r, forward, files).map_or(before, row),
                    None => unknown,
                }
            }
            Move::File(forward) => {
                let target = if forward { file.checked_add(1) } else { file.checked_sub(1) };
                target.filter(|&t| t < self.paths.len()).map_or(before, |file| At::Row { file, row: 0 })
            }
            Move::Jump => before,
            Move::Reflow(true) => At::Row { file, row: 0 },
            Move::Click(y) => match self.seen.as_ref().and_then(|seen| seen.rows.get(usize::from(y))) {
                Some(Shown::Row(pos)) => row(*pos),
                Some(Shown::In(file)) => At::File(*file),
                Some(Shown::Sticky | Shown::Nothing) => pos.map_or(before, row),
                None => unknown,
            },
            Move::Wheel(_) if wrap => unknown,
            Move::Wheel(_) => {
                let (Some(cursor), Some(top)) = (pos, self.top) else { return unknown };
                let sticky = self.rows.kind(top.0, self.split, top.1) != Some(Row::Heading);
                let rows = isize::try_from(self.size.1).unwrap_or(0) - isize::from(sticky);
                let Some(bottom) = self.walk(top, rows - 1) else { return unknown };
                row(cursor.clamp(top, bottom))
            }
        }
    }

    /// The row the cursor is on, when it is known.
    fn pos(&mut self, at: At) -> Option<Pos> {
        match at {
            At::Line { file, old, new } => Some((file, self.rows.row_of(file, self.split, old, new)?)),
            At::Row { file, row } => Some((file, row)),
            At::File(_) => None,
        }
    }

    /// `rows` rows down from `from` (up when negative), from file to file, stopping at the
    /// first and the last row.
    fn walk(&mut self, from: Pos, rows: isize) -> Option<Pos> {
        let (mut file, mut row) = from;
        let mut left = rows.unsigned_abs();
        let files = self.paths.len();
        if rows >= 0 {
            loop {
                let len = self.rows.len(file, self.split)?;
                if row + left < len {
                    return Some((file, row + left));
                }
                if file + 1 >= files {
                    return Some((file, len.saturating_sub(1)));
                }
                left -= len - row;
                (file, row) = (file + 1, 0);
            }
        }
        loop {
            if row >= left {
                return Some((file, row - left));
            }
            if file == 0 {
                return Some((0, 0));
            }
            left -= row + 1;
            file -= 1;
            row = self.rows.len(file, self.split)?.saturating_sub(1);
        }
    }

    /// Read what the frame just drawn in `area` shows, row by row: which file each row is in
    /// by the headings on it, and which row by the rows the viewer lays out, counted from a
    /// row it names (a heading, a hunk's heading, a binary file's note, the cursor's line).
    pub(super) fn read_frame(&mut self, buf: &Buffer, area: Rect) {
        if self.paths.is_empty() || area.width < 40 || area.height < 8 {
            return;
        }
        if (area.width, area.height) != self.size {
            // The viewer laid its rows out again for the new size.
            let split = split_for(self.engine.options().layout, area.width);
            let changed = split != self.split;
            self.size = (area.width, area.height);
            self.split = split;
            let before = self.at;
            self.settle(before, Move::Reflow(changed));
        }
        let texts: Vec<String> = (area.top()..area.bottom()).map(|y| row_text(buf, area, y)).collect();
        let width = area.width;
        let files = self.paths.len();
        // The top row is always a heading: a file's first row, or the one kept over its rows.
        let hint = self.at.map_or(0, At::file);
        let Some(first) = [hint, hint.saturating_sub(1), hint + 1]
            .into_iter()
            .chain(0..files)
            .find(|&f| f < files && texts.first().is_some_and(|text| self.heading_at(text, f, width)))
        else {
            self.seen = None;
            return;
        };
        let mut file_of = Vec::with_capacity(texts.len());
        let mut named: Vec<Option<Pos>> = vec![None; texts.len()];
        let mut file = first;
        for (y, (text, slot)) in texts.iter().zip(named.iter_mut()).enumerate() {
            if y > 0 && file + 1 < files && self.heading_at(text, file + 1, width) {
                file += 1;
                *slot = Some((file, 0));
            } else if y > 0 {
                *slot = self.named_row(text, file, width);
            }
            file_of.push(file);
        }
        // The cursor's line, by its bar.
        if let Some(At::Line { file, old, new }) = self.at
            && let Some(row) = self.rows.row_of(file, self.split, old, new)
        {
            let right = area.x + width.saturating_sub(1) / 2 + 1;
            let starts: &[u16] = if self.split { &[area.x, right] } else { &[area.x] };
            for (y, slot) in named.iter_mut().enumerate() {
                let y16 = area.y + y as u16;
                if starts.iter().any(|&x| buf.cell((x, y16)).is_some_and(|c| c.symbol() == "\u{258e}")) {
                    *slot = Some((file, row));
                }
            }
        }
        let wrap = self.engine.options().wrap;
        let anchor = named.iter().enumerate().find_map(|(y, pos)| pos.map(|pos| (y, pos)));
        let counted = match anchor {
            Some((y, pos)) if !wrap => self.count_rows(y, pos, texts.len()),
            _ => None,
        };
        let rows: Vec<Shown> = match counted {
            Some(counted) => {
                // A frame whose named rows do not fall where they were counted is read by name.
                let fits = named.iter().zip(&counted).all(|(n, c)| n.is_none() || n == c);
                let real_top = counted.first() == Some(&Some((first, 0)));
                if fits {
                    counted
                        .iter()
                        .enumerate()
                        .map(|(y, pos)| match pos {
                            _ if y == 0 && !real_top => Shown::Sticky,
                            Some(pos) => Shown::Row(*pos),
                            None => Shown::Nothing,
                        })
                        .collect()
                } else {
                    by_name(&named, &file_of)
                }
            }
            None => by_name(&named, &file_of),
        };
        self.top = rows.iter().find_map(|shown| match shown {
            Shown::Row(pos) => Some(*pos),
            _ => None,
        });
        if !matches!(rows.first(), Some(Shown::Row(_) | Shown::Sticky)) || wrap {
            self.top = None;
        }
        // After a jump from a row not known, the cursor is on the first row drawn.
        if std::mem::take(&mut self.at_top)
            && let (Some(top), Some(At::File(_))) = (self.top, self.at)
        {
            self.at = Some(At::Row { file: top.0, row: top.1 });
        }
        self.seen = Some(Seen { rows });
    }

    /// Each screen row's row, counted from `pos` drawn on screen row `at`; `None` below the
    /// last row.
    fn count_rows(&mut self, at: usize, pos: Pos, height: usize) -> Option<Vec<Option<Pos>>> {
        let mut rows = vec![None; height];
        for (y, slot) in rows.iter_mut().enumerate() {
            let here = self.walk(pos, y as isize - at as isize)?;
            *slot = Some(here);
        }
        // The last row repeats where the diff ends.
        for y in (at + 1..height).rev() {
            if rows.get(y) == rows.get(y - 1)
                && let Some(slot) = rows.get_mut(y)
            {
                *slot = None;
            }
        }
        Some(rows)
    }

    /// Whether `text` is file `index`'s heading, as the viewer draws it `width` wide: its
    /// path on the left, cut to fit, and its status and counts on the right.
    fn heading_at(&self, text: &str, index: usize, width: u16) -> bool {
        let Some(meta) = self.engine.file(index) else { return false };
        let path = if meta.old_path == meta.path {
            format!(" {}", meta.path)
        } else {
            format!(" {} \u{2192} {}", meta.old_path, meta.path)
        };
        let summary = format!(" {}   +{} \u{2212}{} ", meta.status, meta.additions, meta.deletions);
        let room = usize::from(width).saturating_sub(summary.width());
        text.ends_with(summary.as_str()) && text.starts_with(&cut(&path, room))
    }

    /// The row `text` names in file `index`: a hunk's heading, or a binary file's note.
    fn named_row(&mut self, text: &str, index: usize, width: u16) -> Option<Pos> {
        let meta = self.engine.file(index)?;
        let room = usize::from(width).saturating_sub(2);
        let body = text.strip_prefix(' ')?;
        if meta.hunks.is_empty() {
            let note = format!(" {}  \u{b7}  mode {} \u{2192} {}", meta.status, meta.old_mode, meta.new_mode);
            return body.starts_with(&cut(&note, room)).then_some((index, 1));
        }
        let hunk = meta.hunks.iter().position(|h| body.starts_with(&cut(&h.header, room)))?;
        Some((index, self.rows.hunk_row(index, self.split, hunk)?))
    }
}

/// Each row as far as its name says: a named row, or the file it is in.
fn by_name(named: &[Option<Pos>], file_of: &[usize]) -> Vec<Shown> {
    named.iter().zip(file_of).map(|(pos, file)| pos.map_or(Shown::In(*file), Shown::Row)).collect()
}

/// `text`'s first `width` cells, as the viewer cuts a label.
fn cut(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

/// Screen row `y` of `area` as text, a wide character once.
fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    let mut text = String::new();
    let mut x = area.left();
    while x < area.right() {
        let symbol = buf.cell((x, y)).map_or(" ", |c| c.symbol());
        text.push_str(symbol);
        x += (symbol.width() as u16).max(1);
    }
    text
}
