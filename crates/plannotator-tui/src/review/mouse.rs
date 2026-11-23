//! The mouse in the diff and in the whole file. A click moves the cursor (the viewer's own
//! click); a drag selects whole lines on the side it started on, as the browser's code review
//! does; the release makes them the selection and floats a toolbar beside it, as a mouse
//! selection in a document does. `c`, `s` and `y` then act on it like on a `v` selection.
//!
//! diffs-rs handles clicks and the wheel but has no drag, and cannot say which line is under
//! a cell. So each step of a drag clicks the viewer at the pressed column on the pointer's
//! row and reads the line the cursor landed on. The range is kept here and shown as a gutter
//! mark until the release, because the viewer's selection follows its cursor (a cursor in
//! another file drops it). Past the top or bottom of the pane, each step moves the cursor a
//! line, which scrolls.

use anyhow::Result;
use diffs_rs::engine::Command;
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

/// A drag with the left button, from a line of code.
#[derive(Debug, Clone)]
pub(super) struct Drag {
    /// The pressed column. Every step clicks there, so a split diff keeps the pressed side.
    column: u16,
    /// The pressed line (`first == last`).
    anchor: Lines,
    /// The line at the pointer's end, on the anchor's side.
    end: u32,
    /// Whether the pointer moved with the button down; a press and a release is a click.
    moved: bool,
    /// Whether a step went past the top or bottom of the pane.
    scrolled: bool,
}

impl Drag {
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

    fn active_diff_mut(&mut self) -> Option<&mut Diff> {
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

    /// The left button down in the diff pane: the viewer moves the cursor (or, in its file
    /// list, opens the file), and on a line of code a drag can start. A shift-click is the
    /// viewer's own: it grows the selection to the clicked line.
    pub(super) fn press(&mut self, mouse: MouseEvent) -> Result<()> {
        self.focus = Focus::Diff;
        self.drag = None;
        let Some(diff) = self.active_diff_mut() else { return Ok(()) };
        if mouse.modifiers.contains(KeyModifiers::SHIFT) {
            return diff.handle_event(Event::Mouse(mouse)).map(drop);
        }
        diff.clear_selection()?;
        diff.handle_event(Event::Mouse(mouse))?;
        let anchor = if diff.selectable()? { diff.cursor()? } else { None };
        self.drag = anchor.map(|anchor| Drag {
            column: mouse.column,
            end: anchor.first,
            anchor,
            moved: false,
            scrolled: false,
        });
        Ok(())
    }

    /// The pointer moved with the button down: the line under it, or past the pane's top or
    /// bottom the next line that way, ends the range. A row with nothing on the anchor's
    /// side, or in another file, leaves the range as it was.
    pub(super) fn drag_to(&mut self, mouse: MouseEvent) -> Result<()> {
        let Some(mut drag) = self.drag.take() else { return Ok(()) };
        let area = self.geometry.diff;
        let Some(diff) = self.active_diff_mut() else { return Ok(()) };
        if mouse.row < area.y {
            diff.run(Command::Up)?;
            drag.scrolled = true;
        } else if mouse.row >= area.bottom() {
            diff.run(Command::Down)?;
            drag.scrolled = true;
        } else {
            let click = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: drag.column,
                row: mouse.row,
                modifiers: KeyModifiers::NONE,
            };
            diff.handle_event(Event::Mouse(click))?;
        }
        if let Some((path, line)) = diff.line_on(drag.anchor.side)?
            && path == drag.anchor.path
        {
            drag.end = line;
        }
        drag.moved = true;
        self.drag = Some(drag);
        self.sync_markers();
        Ok(())
    }

    /// The button up: the lines dragged over become the selection, with the toolbar beside
    /// them, and go to the clipboard when `copy_on_select` says so.
    pub(super) fn release(&mut self, mouse: MouseEvent) -> Result<()> {
        let Some(drag) = self.drag.take() else { return Ok(()) };
        self.sync_markers();
        let Some(lines) = drag.lines() else { return Ok(()) };
        let Some(diff) = self.active_diff_mut() else { return Ok(()) };
        diff.select(&lines)?;
        let up = drag.end < drag.anchor.first;
        // Scrolled up and away from the anchor, the viewer has just brought the anchor back
        // into view, so the release point no longer marks the selection.
        let moved_away = drag.scrolled && up;
        let base = self.whole.as_ref().is_some_and(|w| w.version == Version::Base);
        if !moved_away && !base {
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

    /// The toolbar under the release point, or over it when the drag went up: comment, and
    /// suggest for new lines. It goes with the selection.
    pub(super) fn draw_toolbar(&mut self, frame: &mut Frame, diff: Rect) {
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
        let below = at.y + 1 < diff.bottom();
        let above = at.y > diff.y;
        let y = match (up, above, below) {
            (true, true, _) | (false, true, false) => at.y - 1,
            _ => at.y + 1,
        };
        let x = at.x.clamp(diff.x, diff.right() - width);
        let rect = Rect { x, y, width, height: 1 };
        frame.render_widget(Clear, rect);
        let buf = frame.buffer_mut();
        buf.set_style(rect, Style::new().bg(palette().toolbar_bg));
        let mut x = rect.x + 1;
        for (label, action) in labels {
            let w = label.width() as u16;
            let fg = if action == Action::Comment { palette().comment } else { palette().accent };
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

/// Tint the selected rows the viewer drew in `area`: each side of a row that starts with the
/// viewer's selection bar or the drag's bold mark, and the cursor's row when
/// `cursor_selected`, from that cell to the side's end. The viewer marks a selection only
/// with that bar and an underline; this makes it read as one at a glance.
pub(super) fn tint_selection(buf: &mut Buffer, area: Rect, cursor_selected: bool) {
    let style = Style::new().bg(palette().selection.bg.unwrap_or(Color::Indexed(239)));
    for y in area.top()..area.bottom() {
        let mut starts_side = true;
        let mut tinting = false;
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            if cell.symbol() == "\u{2502}" {
                starts_side = true;
                tinting = false;
                continue;
            }
            if starts_side {
                let bold = cell.modifier.contains(Modifier::BOLD);
                tinting = match cell.symbol() {
                    "\u{2503}" | "\u{258c}" => bold,
                    "\u{258e}" => cursor_selected,
                    _ => false,
                };
                starts_side = false;
            }
            if tinting {
                cell.set_style(style);
            }
        }
    }
}
