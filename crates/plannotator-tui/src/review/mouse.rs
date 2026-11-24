//! The mouse in the diff and in the whole file. A click moves the cursor (the viewer's own
//! click); a drag selects whole lines on the side it started on, within the hunk it started
//! in (the lines shown together, as GitHub's multi-line comments are), as the browser's code
//! review does; the release makes them the selection and floats a toolbar beside it, as a
//! mouse selection in a document does. `c`, `s` and `y` then act on it like on a `v`
//! selection. A shift-click grows it from the line first pressed.
//!
//! diffs-rs handles clicks and the wheel but has no drag. So each step of a drag onto a line
//! of code clicks the viewer at the pressed column on the pointer's row and reads the line the
//! cursor landed on. The range is kept here and shown as a gutter mark until the release,
//! because the viewer's selection follows its cursor (a cursor in another file drops it).
//!
//! A terminal can report the pointer many times for one cell, and several reports between two
//! frames: a report for the cell already reported changes nothing, and no report scrolls. A
//! drag inside the pane never moves the view; held past its top or bottom, the view scrolls
//! on a timer (`edge`, `ui::drag`).

mod edge;

use anyhow::Result;
use diffs_rs::engine::DiffRowKind;
use plannotator_tui_review::Side;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Clear;
use unicode_width::UnicodeWidthStr as _;

use super::diff::{Diff, Lines};
use super::whole::Version;
use super::{Focus, Screen};
use crate::delivery::Delivery as _;
use crate::keys::Action;
use crate::theme::palette;
use crate::ui::drag::Edge;

/// A drag with the left button, from a line of code.
#[derive(Debug, Clone)]
pub(super) struct Drag {
    /// The pressed column. Every step clicks there, so a split diff keeps the pressed side.
    column: u16,
    /// The pressed line (`first == last`).
    anchor: Lines,
    /// The first and last line of the anchor's hunk on its side: the drag stays inside.
    span: (u32, u32),
    /// The index of the anchor's hunk in its file.
    hunk: usize,
    /// The line at the pointer's end, on the anchor's side.
    end: u32,
    /// Whether the pointer moved with the button down; a press and a release is a click.
    moved: bool,
    /// The cell the pointer was last reported in, first the pressed one.
    cell: (u16, u16),
    /// The pane's edge the pointer is held past, if it is.
    edge: Option<Edge>,
    /// The view it started in (the whole file or the diff) and that view's patch, so a
    /// release over another one applies nothing.
    whole: bool,
    generation: u64,
}

/// The line a drag's selection grew from, while that selection stands, for a shift-click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Grown {
    anchor: Lines,
    span: (u32, u32),
    lines: Lines,
}

impl Drag {
    /// A drag over `lines` of the diff with patch `generation`, for tests that need lines
    /// no drag could reach.
    #[cfg(test)]
    pub(super) fn over(lines: Lines, generation: u64) -> Self {
        let end = lines.last;
        Self {
            column: 0,
            span: (lines.first, lines.last),
            hunk: 0,
            anchor: Lines { last: lines.first, ..lines },
            end,
            moved: true,
            cell: (0, 0),
            edge: None,
            whole: false,
            generation,
        }
    }

    /// End the range at the cursor's line, kept inside the anchor's hunk, when the cursor is
    /// on the anchor's side of its file.
    fn follow(&mut self, diff: &Diff) -> Result<()> {
        if let Some((path, line)) = diff.line_on(self.anchor.side)?
            && path == self.anchor.path
        {
            self.end = line.clamp(self.span.0, self.span.1);
        }
        Ok(())
    }

    /// The lines dragged over, once the pointer moved.
    pub(super) fn lines(&self) -> Option<Lines> {
        let (first, last) = (self.anchor.first.min(self.end), self.anchor.first.max(self.end));
        self.moved.then(|| Lines { first, last, ..self.anchor.clone() })
    }
}

/// Where the release was, for the toolbar, and whether the drag went up.
#[derive(Debug, Clone, Copy)]
pub(super) struct Toolbar {
    at: Position,
    up: bool,
}

/// What a mouse selection does when the button is released (`[ui] copy_on_select`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnRelease {
    /// Goes to the clipboard.
    Copy,
    /// Waits for `y`.
    Wait,
}

/// The toolbar's buttons: (glyph, label, action).
const BUTTONS: [(&str, &str, Action); 2] =
    [("\u{1f4ac}", "comment", Action::Comment), ("\u{270e}", "suggest", Action::Suggest)];

/// The drag's gutter mark. Bold tells it from a note's mark when the rows are tinted.
pub(super) fn drag_mark() -> Style {
    Style::new().fg(palette().accent).add_modifier(Modifier::BOLD)
}

impl Screen {
    /// The diff the mouse and the selection keys act on: the whole file while it is open.
    pub(super) fn active_diff(&self) -> Option<&Diff> {
        match &self.whole {
            Some(whole) => whole.diff(),
            None => Some(&self.diff),
        }
    }

    pub(super) fn active_diff_mut(&mut self) -> Option<&mut Diff> {
        match &mut self.whole {
            Some(whole) => whole.diff_mut(),
            None => Some(&mut self.diff),
        }
    }

    /// The lines selected or being dragged over.
    pub(super) fn selected_lines(&self) -> Option<Lines> {
        match &self.drag {
            Some(drag) if drag.moved => drag.lines(),
            _ => self.active_diff().and_then(Diff::selection),
        }
    }

    /// The left button down in the diff pane: the viewer moves the cursor, and on a line of
    /// code a drag can start. A shift-click grows the selection to the clicked line: from
    /// the line a drag first pressed, or as the viewer grows a `v` selection. The file list
    /// and the lines between the panes take their presses before this.
    pub(super) fn press(&mut self, mouse: MouseEvent) -> Result<()> {
        self.focus = Focus::Diff;
        self.drag = None;
        let whole = self.whole.is_some();
        let grown = self.grown.take();
        let Some(diff) = self.active_diff_mut() else { return Ok(()) };
        let generation = diff.generation();
        if mouse.modifiers.contains(KeyModifiers::SHIFT) {
            let grown = grown.filter(|grown| diff.selection().as_ref() == Some(&grown.lines));
            let Some(grown) = grown else {
                return diff.handle_event(Event::Mouse(mouse)).map(drop);
            };
            let click = MouseEvent { modifiers: KeyModifiers::NONE, ..mouse };
            diff.handle_event(Event::Mouse(click))?;
            let line = diff.line_on(grown.anchor.side)?.filter(|(path, _)| *path == grown.anchor.path);
            let end = line.map_or(grown.lines.last, |(_, line)| line.clamp(grown.span.0, grown.span.1));
            let (first, last) = (grown.anchor.first.min(end), grown.anchor.first.max(end));
            let lines = Lines { first, last, ..grown.anchor.clone() };
            diff.select_from(&lines, end < grown.anchor.first)?;
            self.grown = Some(Grown { lines, ..grown });
            return Ok(());
        }
        diff.clear_selection()?;
        diff.handle_event(Event::Mouse(mouse))?;
        let anchor = if diff.selectable()? { diff.cursor()? } else { None };
        self.drag = anchor.and_then(|anchor| {
            let (hunk, span) = diff.hunk_span(&anchor.path, anchor.side, anchor.first)?;
            Some(Drag {
                column: mouse.column,
                end: anchor.first,
                anchor,
                span,
                hunk,
                moved: false,
                cell: (mouse.column, mouse.row),
                edge: None,
                whole,
                generation,
            })
        });
        Ok(())
    }

    /// The pointer moved with the button down: the line under it ends the range, kept inside
    /// the anchor's hunk. A row with nothing on the anchor's side, or in another file, leaves
    /// the range as it was; a row that is no code (a heading, a gap) takes no click, which
    /// could open the gap or move the view. Past the pane's top or bottom the line at that
    /// edge ends it, and the view scrolls from there while the pointer stays (`edge`).
    pub(super) fn drag_to(&mut self, mouse: MouseEvent) -> Result<()> {
        let Some(mut drag) = self.drag.take() else { return Ok(()) };
        if drag.cell == (mouse.column, mouse.row) {
            self.drag = Some(drag);
            return Ok(());
        }
        drag.cell = (mouse.column, mouse.row);
        drag.moved = true;
        let area = self.geometry.diff;
        let held = drag.edge;
        drag.edge = Edge::past(mouse.row, area, held);
        // Held past the same edge, the timer moves the cursor: a click would pull it back.
        let rows = match drag.edge {
            Some(edge) if held.is_some_and(|held| held.down() == edge.down()) => Vec::new(),
            // The sticky heading can cover the top row.
            Some(edge) if !edge.down() => vec![area.y, area.y + 1],
            Some(_) => vec![area.bottom().saturating_sub(1)],
            None => vec![mouse.row],
        };
        let Some(diff) = self.active_diff_mut() else { return Ok(()) };
        let code = rows
            .into_iter()
            .find(|&row| diff.row_at(drag.column, row).is_some_and(|shown| shown.kind == DiffRowKind::Code));
        if let Some(row) = code {
            let click = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: drag.column,
                row,
                modifiers: KeyModifiers::NONE,
            };
            diff.handle_event(Event::Mouse(click))?;
            drag.follow(diff)?;
        }
        self.drag = Some(drag);
        self.sync_markers();
        Ok(())
    }

    /// The button up: the lines dragged over become the selection, with the toolbar beside
    /// them, and go to the clipboard when `copy_on_select` says so. Lines from a view or a
    /// patch no longer shown apply nothing.
    pub(super) fn release(&mut self, mouse: MouseEvent) -> Result<()> {
        let Some(drag) = self.drag.take() else { return Ok(()) };
        self.sync_markers();
        let Some(lines) = drag.lines() else { return Ok(()) };
        let whole = self.whole.is_some();
        let Some(diff) = self.active_diff_mut() else { return Ok(()) };
        if drag.whole != whole || drag.generation != diff.generation() || !diff.paths().contains(&lines.path)
        {
            self.status = Some("those lines are no longer shown; nothing selected".into());
            return Ok(());
        }
        let up = drag.end < drag.anchor.first;
        diff.select_from(&lines, up)?;
        self.grown = Some(Grown { anchor: drag.anchor.clone(), span: drag.span, lines: lines.clone() });
        let base = self.whole.as_ref().is_some_and(|w| w.version == Version::Base);
        if !base {
            let area = self.geometry.diff;
            let row = mouse.row.clamp(area.y, area.bottom().saturating_sub(1));
            self.toolbar = Some(Toolbar { at: Position::new(mouse.column, row), up });
        }
        if self.on_release == OnRelease::Copy {
            self.copy_lines(&lines, false);
        }
        Ok(())
    }

    /// `y`: copy the selected lines.
    pub(super) fn copy_selection(&mut self) {
        match self.active_diff().and_then(Diff::selection) {
            Some(lines) => self.copy_lines(&lines, true),
            None => self.status = Some("select lines first".into()),
        }
    }

    fn copy_lines(&mut self, lines: &Lines, say: bool) {
        let Some(text) = self.selected_code(lines) else { return };
        if self.clipboard {
            let _ = crate::delivery::Clipboard.deliver(&text);
        }
        if say {
            let count = lines.last - lines.first + 1;
            self.status =
                Some(if count == 1 { "copied 1 line".into() } else { format!("copied {count} lines") });
        }
    }

    /// The code of `lines`, joined with `\n`: from the whole file while it is open, else the
    /// lines the diff shows on that side.
    pub(super) fn selected_code(&self, lines: &Lines) -> Option<String> {
        if let Some(whole) = &self.whole {
            return whole.code(lines.first, lines.last);
        }
        let file = self.file(&lines.path)?;
        let code: Vec<&str> = (lines.first..=lines.last).filter_map(|n| file.line(lines.side, n)).collect();
        (!code.is_empty()).then(|| code.join("\n"))
    }

    pub(super) fn toolbar_hit(&self, at: Position) -> Option<Action> {
        self.geometry.toolbar.iter().find(|(rect, _)| rect.contains(at)).map(|(_, action)| *action)
    }

    /// The toolbar below the selection, or above it when the drag went up, at the release
    /// column: comment, and suggest for new lines. It never covers a selected row while
    /// there is room for it, and goes with the selection. `selected` is the first and last
    /// screen row drawn selected.
    pub(super) fn draw_toolbar(&mut self, frame: &mut Frame, diff: Rect, selected: Option<(u16, u16)>) {
        let Some(Toolbar { at, up }) = self.toolbar else { return };
        let Some(lines) = self.active_diff().and_then(Diff::selection) else {
            self.toolbar = None;
            return;
        };
        let labels: Vec<(String, Action)> = BUTTONS
            .iter()
            .filter(|(_, _, action)| *action != Action::Suggest || lines.side == Side::New)
            .map(|(glyph, label, action)| match self.keys.hint(*action) {
                key if key.is_empty() => (format!(" {glyph} {label} "), *action),
                key => (format!(" {glyph} {label} ({key}) "), *action),
            })
            .collect();
        let width = labels.iter().map(|(label, _)| label.width() as u16).sum::<u16>() + 1;
        if width > diff.width || diff.height < 2 {
            return;
        }
        let (first, last) = selected.unwrap_or((at.y, at.y));
        let above = (first > diff.y).then(|| first - 1);
        let below = (last + 1 < diff.bottom()).then_some(last + 1);
        let beside = if at.y > diff.y { at.y - 1 } else { at.y + 1 };
        let y = if up { above.or(below) } else { below.or(above) }.unwrap_or(beside);
        let x = at.x.clamp(diff.x, diff.right() - width);
        let rect = Rect { x, y, width, height: 1 };
        frame.render_widget(Clear, rect);
        let buf = frame.buffer_mut();
        buf.set_style(rect, Style::new().bg(palette().toolbar_bg));
        self.geometry.toolbar_bar = Some(rect);
        let mut x = rect.x + 1;
        for (label, action) in labels {
            let w = label.width() as u16;
            let chips = crate::ui::chips::chips();
            let fg = if action == Action::Comment { chips.review_comment() } else { chips.review_suggest };
            let style = Style::new().fg(fg).bg(palette().toolbar_bg).add_modifier(Modifier::BOLD);
            buf.set_span(x, y, &Span::styled(label, style), w);
            self.geometry.toolbar.push((Rect { x, y, width: w, height: 1 }, action));
            x += w;
        }
    }

    /// Whether the cursor's line is one of the selected lines.
    pub(super) fn cursor_selected(&self) -> bool {
        let Some(lines) = self.selected_lines() else { return false };
        let cursor = self.active_diff().and_then(|diff| diff.line_on(lines.side).ok().flatten());
        cursor.is_some_and(|(path, line)| path == lines.path && (lines.first..=lines.last).contains(&line))
    }
}

/// The background of selected lines: the theme's selection colour, or on the dark theme
/// (whose selection is reversed video) a grey that keeps every syntax colour readable and
/// is neither the added nor the removed tint.
pub(super) fn selection_bg() -> Color {
    palette().selection.bg.unwrap_or(Color::Indexed(239))
}

/// Draw the selected rows the viewer drew in `area` as one block of background colour, and
/// say which screen rows that block runs from and to.
///
/// A row has one pane (unified) or two (split): the split's left pane ends at `middle`, the
/// column the viewer draws the line between the sides in, and the right one starts after it. A
/// pane is selected when its first cell holds the viewer's selection bar, the drag's bold
/// mark, or the cursor's bar while `cursor_selected`. That cell is blanked, the selection's
/// underline is taken off (a search match, bold as well, keeps its own), and the pane takes
/// the selection background to its end.
///
/// Rows strictly between the first and the last selected row are filled too, so the
/// selection reads as one block: in the unified view the other side's lines sit among the
/// selected ones (a removed line inside new lines). They are only drawn so; the note still
/// covers the selected side's lines alone. `code` says which rows hold code: a heading or a
/// gap between them is left as it is.
pub(super) fn tint_selection(
    buf: &mut Buffer,
    area: Rect,
    middle: Option<u16>,
    cursor_selected: bool,
    code: impl Fn(u16) -> bool,
) -> Option<(u16, u16)> {
    let bg = selection_bg();
    let panes: Vec<(u16, u16)> = match middle {
        Some(middle) => vec![(area.left(), middle), (middle + 1, area.right())],
        None => vec![(area.left(), area.right())],
    };
    // Each pane's selected rows.
    let mut rows: Vec<Vec<u16>> = vec![Vec::new(); panes.len()];
    for y in area.top()..area.bottom() {
        for (&(left, right), rows) in panes.iter().zip(rows.iter_mut()) {
            let selected = buf.cell((left, y)).is_some_and(|cell| match cell.symbol() {
                "\u{2503}" | "\u{258c}" => cell.modifier.contains(Modifier::BOLD),
                "\u{258e}" => cursor_selected,
                _ => false,
            });
            if !selected {
                continue;
            }
            for x in left..right {
                let Some(cell) = buf.cell_mut((x, y)) else { continue };
                if x == left {
                    cell.set_symbol(" ");
                    cell.modifier.remove(Modifier::BOLD);
                }
                if !cell.modifier.contains(Modifier::BOLD) {
                    cell.modifier.remove(Modifier::UNDERLINED);
                }
                cell.set_bg(bg);
            }
            rows.push(y);
        }
    }
    let mut block: Option<(u16, u16)> = None;
    for (&(left, right), rows) in panes.iter().zip(&rows) {
        let (Some(&first), Some(&last)) = (rows.first(), rows.last()) else { continue };
        block = Some(block.map_or((first, last), |(a, b)| (a.min(first), b.max(last))));
        for y in (first + 1..last).filter(|y| !rows.contains(y) && code(*y)) {
            for x in left..right {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_bg(bg);
                }
            }
        }
    }
    block
}
