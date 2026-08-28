//! Keyboard and mouse handling.

use std::io::Write as _;

use anyhow::Result;
use plannotui_schema::Kind;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use tui_input::backend::crossterm::EventHandler;

use super::selection::Selection;
use super::{App, GUTTER, Mode, Pending, TOOLBAR};

impl App {
    pub(crate) fn handle_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match self.mode {
                Mode::Browse => self.browse_key(*key),
                Mode::Compose => self.compose_key(*key, event),
            },
            Event::Mouse(mouse) if self.mode == Mode::Browse => self.mouse(*mouse),
            _ => Ok(()),
        }
    }

    fn browse_key(&mut self, key: KeyEvent) -> Result<()> {
        let page = i64::from(self.geometry.doc.height.max(1));
        // With a selection waiting, the toolbar keys win.
        if self.pending.is_some()
            && let Some(&(_, _, _, kind)) = TOOLBAR.iter().find(|(_, _, k, _)| KeyCode::Char(*k) == key.code)
        {
            return self.act(kind);
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.quit = true,
            (KeyCode::Esc, _) => {
                if self.pending.is_some() || self.selection.is_some() {
                    self.clear_selection();
                } else {
                    self.quit = true;
                }
            }
            (KeyCode::Char('j') | KeyCode::Down, _) => self.select_block(self.selected + 1),
            (KeyCode::Char('k') | KeyCode::Up, _) => self.select_block(self.selected.saturating_sub(1)),
            (KeyCode::Char('d'), KeyModifiers::CONTROL) | (KeyCode::PageDown, _) => self.scroll_by(page / 2),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) | (KeyCode::PageUp, _) => self.scroll_by(-page / 2),
            (KeyCode::Char('g') | KeyCode::Home, _) => self.select_block(0),
            (KeyCode::Char('G') | KeyCode::End, _) => {
                self.select_block(self.doc.blocks.len().saturating_sub(1));
            }
            (KeyCode::Char('c') | KeyCode::Enter, _) => {
                // No selection: comment on the whole selected block.
                if let (Some(block), Some(rendered)) =
                    (self.doc.blocks.get(self.selected), self.layout.blocks.get(self.selected))
                {
                    self.pending = Some(Pending { range: block.range.clone(), at: (rendered.first_row, 0) });
                    self.act(Kind::Comment)?;
                }
            }
            (KeyCode::Char('x'), _) => {
                let removed = self.store.remove_in_block(&self.doc, self.selected)?;
                self.status = Some(format!("removed {removed} annotation(s) on block"));
            }
            (KeyCode::Char('r'), _) => self.reload()?,
            _ => {}
        }
        Ok(())
    }

    fn compose_key(&mut self, key: KeyEvent, event: &Event) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Browse,
            KeyCode::Enter => {
                let body = self.input.value().trim().to_owned();
                if !body.is_empty() {
                    if let Some(pending) = self.pending.take() {
                        self.annotate(pending.range, Kind::Comment, body)?;
                        self.status = Some("comment saved".into());
                    }
                    self.clear_selection();
                }
                self.mode = Mode::Browse;
            }
            _ => {
                self.input.handle_event(event);
            }
        }
        Ok(())
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(kind) = self.toolbar_hit(mouse.column, mouse.row) {
                    return self.act(kind);
                }
                let Some(pos) = self.doc_position(mouse.column, mouse.row, false) else { return Ok(()) };
                if let Some(block) = self.layout.block_at_row(pos.0) {
                    self.selected = block;
                }
                self.pending = None;
                self.selection = Some(Selection::start(pos));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if !self.selection.is_some_and(|s| s.dragging) {
                    return Ok(());
                }
                let doc = self.geometry.doc;
                // Dragging past the top or bottom edge scrolls the document.
                if mouse.row < doc.y {
                    self.scroll_by(-1);
                } else if mouse.row >= doc.bottom() {
                    self.scroll_by(1);
                }
                if let Some(pos) = self.doc_position(mouse.column, mouse.row, true)
                    && let Some(sel) = self.selection.as_mut()
                {
                    sel.set_head(pos);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.finish_selection(),
            _ => {}
        }
        Ok(())
    }

    fn toolbar_hit(&self, column: u16, row: u16) -> Option<Kind> {
        let (rect, spans) = self.geometry.toolbar.as_ref()?;
        if row != rect.y || column < rect.x || column >= rect.right() {
            return None;
        }
        spans.iter().zip(TOOLBAR.iter()).find(|(span, _)| span.contains(&column)).map(|(_, item)| item.3)
    }

    /// Screen cell -> document (row, column). With `clamp`, positions outside the
    /// document area snap to its nearest edge so a drag can leave the area.
    fn doc_position(&self, column: u16, row: u16, clamp: bool) -> Option<(usize, usize)> {
        let doc = self.geometry.doc;
        let inside = column >= doc.x && column < doc.right() && row >= doc.y && row < doc.bottom();
        if !inside && !clamp {
            let in_gutter = column >= doc.x.saturating_sub(GUTTER)
                && column < doc.x
                && row >= doc.y
                && row < doc.bottom();
            if !in_gutter {
                return None;
            }
        }
        let col = column.clamp(doc.x, doc.right().saturating_sub(1)) - doc.x;
        let row = row.clamp(doc.y, doc.bottom().saturating_sub(1)) - doc.y;
        let doc_row = (self.scroll + usize::from(row)).min(self.layout.total_rows.saturating_sub(1));
        Some((doc_row, usize::from(col)))
    }

    /// Turn the drag into a source range; it may span blocks.
    pub(super) fn finish_selection(&mut self) {
        let Some(sel) = self.selection.as_mut() else { return };
        sel.dragging = false;
        if sel.is_empty() {
            self.selection = None;
            return;
        }
        let (a, b) = sel.ordered();
        let sel = *sel;
        let mut bounds: Option<(usize, usize)> = None;
        for row in a.0..=b.0 {
            let Some(r) = self.layout.row(row) else { continue };
            let cols = sel.columns_on(row, r.cells.len()).unwrap_or(0..0);
            for offset in r.cells.iter().skip(cols.start).take(cols.len()).flatten() {
                bounds =
                    Some(bounds.map_or((*offset, *offset), |(lo, hi)| (lo.min(*offset), hi.max(*offset))));
            }
        }
        let Some((lo, hi)) = bounds else {
            self.selection = None;
            return;
        };
        let end_char = self.doc.source.get(hi..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        let range = lo..hi + end_char;
        if self.clipboard
            && let Some(text) = self.doc.source.get(range.clone())
        {
            copy_to_clipboard(text);
        }
        if let Some(block) = self.doc.block_containing(range.start) {
            self.selected = block;
        }
        self.pending = Some(Pending { range, at: a });
    }
}

/// OSC 52: hand the selection to the terminal's clipboard so Cmd-C habits keep working.
fn copy_to_clipboard(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            let ch =
                if i <= chunk.len() { TABLE.get(((n >> (18 - 6 * i)) & 63) as usize).copied() } else { None };
            out.push(ch.map_or('=', char::from));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
