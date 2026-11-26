//! Drawing the boxes the last frame kept rows for. Each is drawn whole off screen and its rows
//! on screen copied in, so a box cut by the pane's top or bottom loses its border there and
//! nothing ever lands outside its rows. The cells keep the diff's background.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget as _};

use super::super::compose_at::MAX_WIDTH;
use super::card::Card;
use crate::keys::Action;
use crate::review::Screen;
use crate::theme::palette;

/// How a box looks: its note sent or not, the cursor on it or not.
#[derive(Debug, Clone, Copy)]
struct Look {
    sent: bool,
    cursor: bool,
}

impl Screen {
    /// Draw the boxes on screen in the last frame of the diff, and note where for the mouse
    /// and the comment box.
    pub(in crate::review) fn draw_boxes(&mut self, frame: &mut Frame) {
        // The rows kept are only for notes while they show inline, and for Ask threads always.
        if self.whole.is_some() {
            return;
        }
        let more = self.keys.labelled(Action::Confirm, "for more").unwrap_or_else(|| "more".to_owned());
        let more = format!("\u{2026} {more}");
        let again = self.ask_box_hint();
        let areas = self.diff.box_areas().to_vec();
        let notes = self.store.annotations();
        // The note in the comment box: its box keeps its rows, blank, under the comment box,
        // which can be shorter than the box was.
        let editing = match &self.mode {
            crate::review::Mode::Compose(crate::review::Draft::Edit(id)) => Some(id.as_str()),
            _ => None,
        };
        for area in areas {
            if let Some(thread) = self.ask.thread(area.id) {
                let width = area.area.width.min(MAX_WIDTH);
                let rect = Rect { width, ..area.area };
                let card = self.inline.thread_card(thread, width);
                paint(
                    frame.buffer_mut(),
                    rect,
                    area.clipped_top,
                    card,
                    Look { sent: false, cursor: area.cursor },
                    Some(&again),
                );
                self.geometry.boxes.push((rect, area.id));
                continue;
            }
            let Some(note) =
                self.inline.notes.get(&area.id).and_then(|id| notes.iter().find(|n| n.id == *id))
            else {
                continue;
            };
            let width = area.area.width.min(MAX_WIDTH);
            let rect = Rect { width, ..area.area };
            if editing == Some(note.id.as_str()) {
                blank(frame.buffer_mut(), rect);
            } else {
                let look = Look { sent: !self.store.is_pending(note), cursor: area.cursor };
                let card = self.inline.card(area.id, note, width);
                let bottom = card.more.then_some(more.as_str());
                paint(frame.buffer_mut(), rect, area.clipped_top, card, look, bottom);
            }
            self.geometry.boxes.push((rect, area.id));
        }
    }
}

/// `rect` of `buf` empty, on the diff's background.
fn blank(buf: &mut Buffer, rect: Rect) {
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                let bg = cell.bg;
                cell.reset();
                cell.bg = bg;
            }
        }
    }
}

/// The box for `card`, its rows from `clipped_top` on, into `rect` of `buf`, with `bottom` on
/// its lower border: what a note cut short says, or the keys of an Ask box.
fn paint(buf: &mut Buffer, rect: Rect, clipped_top: u16, card: &Card, look: Look, bottom: Option<&str>) {
    let whole = Rect::new(0, 0, rect.width, card.height());
    let mut scratch = Buffer::empty(whole);
    let colour = if look.sent { Color::DarkGray } else { palette().comment };
    let border = if look.cursor { Style::new().fg(palette().accent).bold() } else { Style::new().fg(colour) };
    let mut title = vec![Span::raw(" "), Span::styled(card.label.clone(), Style::new().fg(colour).bold())];
    if let Some(author) = &card.author {
        title.push(Span::styled(format!(" \u{b7} {author}"), Style::new().fg(colour)));
    }
    if look.sent {
        title.push(Span::styled(" \u{b7} sent", Style::new().fg(Color::DarkGray)));
    }
    title.push(Span::raw(" "));
    let mut block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(title));
    if let Some(bottom) = bottom {
        block = block.title_bottom(Span::styled(format!(" {bottom} "), Style::new().fg(colour)));
    }
    let inner = block.inner(whole);
    block.render(whole, &mut scratch);
    let text = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    let style = if look.sent { Style::new().dim() } else { Style::new() };
    Paragraph::new(card.body.clone()).style(style).render(text, &mut scratch);
    for dy in 0..rect.height {
        let from = clipped_top + dy;
        for dx in 0..rect.width {
            let (Some(cell), Some(to)) = (scratch.cell((dx, from)), buf.cell_mut((rect.x + dx, rect.y + dy)))
            else {
                continue;
            };
            let bg = to.bg;
            *to = cell.clone();
            if cell.bg == Color::Reset {
                to.bg = bg;
            }
        }
    }
}
