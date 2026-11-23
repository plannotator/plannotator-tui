//! The lines between the panes: the file list's right edge and the notes panel's left edge.
//! Dragging one resizes its pane, as `<`/`>` (the list) and `{`/`}` (the notes) do, and the
//! width set either way stays for the rest of the review, through filters and reloads; the
//! pane no longer fits itself to its contents. A double click gives the fitting back. The
//! line brightens under the pointer and while it is dragged.
//!
//! The line between a split diff's sides works the same way: a drag shares the diff's width
//! between the old and the new side, as `(`/`)` do, and a double click makes them even
//! again. That ratio is a setting (`settings`), written to the config file when the drag
//! ends. The viewer draws the line on rows of code only, so only those cells take a press.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};

use super::Screen;
use super::files::fit;

/// Columns `narrow-notes` and `widen-notes` take from or give to the notes panel.
const NOTES_STEP: u16 = 4;
/// Two presses on a line this close together are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// A line between two panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Divider {
    /// Right of the file list.
    Files,
    /// Left of the notes panel.
    Notes,
    /// Between the old and the new side of a split diff.
    Split,
}

/// The lines' state: which one is dragged or under the pointer, and the notes panel's width.
#[derive(Debug, Default)]
pub(super) struct Dividers {
    /// The line being dragged, from the press to the release: where it was pressed, and
    /// whether the pointer has left that cell since.
    dragging: Option<(Divider, Position, bool)>,
    /// The line under the pointer.
    hover: Option<Divider>,
    /// The last click on a line (a press and a release on one cell, no move between), for a
    /// double click.
    clicked: Option<(Divider, Position, Instant)>,
    /// The notes panel's width, once set by a drag or a key: the person's choice, kept as it
    /// is; a narrow window draws the panel narrower without changing it.
    notes_width: Option<u16>,
    /// The notes panel's width drawn last frame; 0 when hidden.
    notes_drawn: u16,
    /// Whether that was narrower than the width chosen.
    notes_clamped: bool,
    /// Where each line was drawn last frame.
    drawn: Vec<(Divider, Rect)>,
}

impl Dividers {
    #[cfg(test)]
    pub(super) fn dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// Whether the line between a split diff's sides is being dragged.
    pub(super) fn dragging_split(&self) -> bool {
        matches!(self.dragging, Some((Divider::Split, _, _)))
    }

    /// Drop a drag in progress; the pane keeps the width it was dragged to so far.
    pub(super) fn cancel(&mut self) {
        self.dragging = None;
    }

    /// Nothing under the pointer: it left the window.
    pub(super) fn unlight(&mut self) {
        self.hover = None;
    }

    /// The notes panel's width in `body` columns: the width set, else `fitted`, never so wide
    /// that the diff loses its minimum or (when it shows) the file list its narrowest.
    pub(super) fn notes_width(&self, fitted: u16, body: u16, list: bool) -> u16 {
        let Some(chosen) = self.notes_width else { return fitted };
        let list = if list { fit::MIN_WIDTH + 1 } else { 0 };
        let most = body.saturating_sub(1 + fit::DIFF_MIN_WIDTH + list);
        chosen.min(most).max(super::draw::RAIL_MIN_WIDTH)
    }

    /// The notes panel was drawn `width` wide (0: hidden).
    pub(super) fn notes_drawn(&mut self, width: u16) {
        self.notes_drawn = width;
        self.notes_clamped = width > 0 && self.notes_width.is_some_and(|chosen| chosen > width);
    }

    /// Where the lines were drawn this frame: right of `files` and left of `rail`, either
    /// empty when its pane is hidden.
    pub(super) fn place(&mut self, files: Rect, rail: Rect) {
        self.drawn.clear();
        if files.width > 0 {
            self.drawn.push((Divider::Files, Rect { x: files.right(), width: 1, ..files }));
        }
        if rail.width > 0 && rail.x > 0 {
            self.drawn.push((Divider::Notes, Rect { x: rail.x - 1, width: 1, ..rail }));
        }
    }

    fn at(&self, at: Position) -> Option<Divider> {
        self.drawn.iter().find(|(_, rect)| rect.contains(at)).map(|(divider, _)| *divider)
    }

    /// Draw the lines, the file list's again over its own: dim, and in the accent colour
    /// while dragged or under the pointer. Only the colour changes; a bold line is drawn
    /// with a shorter glyph in many terminals and would break into dashes.
    pub(super) fn draw(&self, frame: &mut Frame) {
        let lit = self.lit();
        let buf = frame.buffer_mut();
        // The viewer draws the line between the sides itself; `Screen::draw_split_line` lights it.
        for (divider, rect) in self.drawn.iter().filter(|(divider, _)| *divider != Divider::Split) {
            let style = if lit == Some(*divider) {
                Style::new().fg(crate::theme::palette().accent)
            } else {
                Style::new().fg(Color::DarkGray)
            };
            for y in rect.top()..rect.bottom() {
                buf.set_string(rect.x, y, "\u{2502}", style);
            }
        }
    }
}

impl Dividers {
    /// The line dragged, else the one under the pointer.
    fn lit(&self) -> Option<Divider> {
        self.dragging.map(|(divider, _, _)| divider).or(self.hover)
    }
}

impl Screen {
    /// Find the line between the sides of a split diff in the frame drawn: the column the
    /// viewer says, on the rows it drew the line on that nothing covers since. Those cells take
    /// a press, and light in the accent colour while dragged or under the pointer.
    pub(super) fn draw_split_line(&mut self, frame: &mut Frame, diff: Rect) {
        let Some(column) = self.whole.is_none().then(|| self.diff.split_line()).flatten() else { return };
        let lit = self.dividers.lit() == Some(Divider::Split);
        let buf = frame.buffer_mut();
        let mut rows: Vec<u16> = Vec::new();
        for y in diff.top()..diff.bottom() {
            let drawn = buf.cell((column, y)).is_some_and(|cell| cell.symbol() == "\u{2502}");
            if drawn && self.diff.code_row(y) {
                rows.push(y);
                if lit && let Some(cell) = buf.cell_mut((column, y)) {
                    cell.set_fg(crate::theme::palette().accent);
                }
            }
        }
        // One rect per run of rows.
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for y in rows {
            match runs.last_mut() {
                Some((_, last)) if *last + 1 == y => *last = y,
                _ => runs.push((y, y)),
            }
        }
        for (first, last) in runs {
            let rect = Rect { x: column, y: first, width: 1, height: last - first + 1 };
            self.dividers.drawn.push((Divider::Split, rect));
        }
    }

    /// A mouse event for the lines: a press on one starts a resize, a drag moves the line,
    /// the release ends it, and two clicks in a row on one cell give the pane its fitted
    /// width back. The line under the pointer lights. True when the event was the lines',
    /// so nothing else, a line selection least of all, starts from it.
    pub(super) fn divider_mouse(&mut self, mouse: MouseEvent) -> bool {
        let at = Position::new(mouse.column, mouse.row);
        self.dividers.hover = self.dividers.at(at);
        match (mouse.kind, self.dividers.dragging) {
            (MouseEventKind::Drag(MouseButton::Left), Some((divider, pressed, moved))) => {
                self.dividers.dragging = Some((divider, pressed, moved || at != pressed));
                self.move_divider(divider, mouse.column);
                true
            }
            (MouseEventKind::Up(MouseButton::Left), Some((divider, pressed, moved))) => {
                self.dividers.dragging = None;
                let now = Instant::now();
                if moved || at != pressed {
                    self.dividers.clicked = None;
                    return true;
                }
                let double = self.dividers.clicked.is_some_and(|(last, cell, when)| {
                    last == divider && cell == at && now.duration_since(when) < DOUBLE_CLICK
                });
                if double {
                    self.fit_divider(divider);
                    self.dividers.clicked = None;
                } else {
                    self.dividers.clicked = Some((divider, at, now));
                }
                true
            }
            (MouseEventKind::Down(MouseButton::Left), _) => {
                let Some(divider) = self.dividers.at(at) else { return false };
                self.dividers.dragging = Some((divider, at, false));
                true
            }
            _ => false,
        }
    }

    /// The line dragged to `column`: the list ends left of it, the notes start right of it.
    /// Widths past the limits are kept within them when drawn, as the keys' are.
    fn move_divider(&mut self, divider: Divider, column: u16) {
        match divider {
            Divider::Files => {
                let width = column.saturating_sub(self.geometry.files.x);
                self.files.set_width(Some(width.max(fit::MIN_WIDTH)));
            }
            Divider::Notes => {
                let width = self.geometry.rail.right().saturating_sub(column + 1);
                self.dividers.notes_width = Some(width.max(super::draw::RAIL_MIN_WIDTH));
            }
            Divider::Split => self.drag_split(column),
        }
    }

    /// A double click on a line: its pane fits itself again.
    fn fit_divider(&mut self, divider: Divider) {
        match divider {
            Divider::Files => self.files.set_width(None),
            Divider::Notes => self.dividers.notes_width = None,
            Divider::Split => self.even_split(),
        }
    }

    /// `narrow-notes`, `widen-notes`: the notes panel's width, by four columns.
    pub(super) fn resize_notes(&mut self, wider: bool) {
        // From the width last asked for, or the one shown when the room could not give it.
        let now = match (self.dividers.notes_width, self.dividers.notes_drawn) {
            (Some(_), drawn) if self.dividers.notes_clamped => drawn,
            (Some(chosen), _) => chosen,
            (None, 0) => super::draw::RAIL_MIN_WIDTH,
            (None, drawn) => drawn,
        };
        let next = if wider { now.saturating_add(NOTES_STEP) } else { now.saturating_sub(NOTES_STEP) };
        self.dividers.notes_width = Some(next.max(super::draw::RAIL_MIN_WIDTH));
        self.reveal_notes();
    }
}
