//! The header: the send button and, for file and folder reviews, the Review menu button.
//! The two wrap onto another row only when a pane is too narrow for both.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use super::App;
use super::send::SendState;

const SEND_BG: Color = Color::Indexed(30);
const IDLE_BG: Color = Color::Indexed(238);
const SENT_BG: Color = Color::Indexed(22);
const BLOCKED_BG: Color = Color::Indexed(58);

pub(super) const REVIEW_LABEL: &str = "Review \u{25be} (m)";

#[derive(Debug, Clone, Copy)]
enum Button {
    Send,
    Review,
}

impl App {
    /// Buttons with their rects relative to the header, right to left: send on the edge,
    /// the Review menu to its left.
    fn header_buttons(&self, width: u16) -> Vec<(Button, String, Rect)> {
        if width == 0 {
            return Vec::new();
        }
        let mut labels = vec![(Button::Send, self.send_label())];
        if self.is_file_review() {
            labels.push((Button::Review, REVIEW_LABEL.to_owned()));
        }
        let mut right = width;
        let mut y = 0;
        labels
            .into_iter()
            .map(|(button, label)| {
                let label = format!(" {label} ");
                let button_width = label.width().min(usize::from(width)) as u16;
                if button_width > right {
                    y += 1;
                    right = width;
                }
                let rect = Rect { x: right - button_width, y, width: button_width, height: 1 };
                right = rect.x.saturating_sub(1);
                (button, label, rect)
            })
            .collect()
    }

    pub(super) fn header_height(&self, width: u16) -> u16 {
        self.header_buttons(width).last().map_or(1, |(_, _, rect)| rect.y + 1)
    }

    pub(super) fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        for (button, label, mut rect) in self.header_buttons(area.width) {
            if rect.y >= area.height {
                continue;
            }
            rect.x += area.x;
            rect.y += area.y;
            let style = match button {
                Button::Send => {
                    self.geometry.send_button = Some(rect);
                    self.button_style()
                }
                Button::Review => {
                    self.geometry.review_button = Some(rect);
                    Style::new().fg(Color::Cyan).bg(IDLE_BG)
                }
            };
            frame.buffer_mut().set_span(rect.x, rect.y, &Span::styled(label, style), rect.width);
        }
    }

    /// At zero the button is still drawn and clickable; it reports the no-op in the footer.
    fn button_style(&self) -> Style {
        match &self.send_state {
            SendState::Ready if self.send_count() == 0 => {
                Style::new().fg(Color::Gray).bg(IDLE_BG).add_modifier(Modifier::DIM)
            }
            SendState::Ready => Style::new().fg(Color::Black).bg(SEND_BG).bold(),
            SendState::Sent => Style::new().fg(Color::Black).bg(SENT_BG).bold(),
            SendState::Blocked(_) => Style::new().fg(Color::Black).bg(BLOCKED_BG).bold(),
        }
    }
}
