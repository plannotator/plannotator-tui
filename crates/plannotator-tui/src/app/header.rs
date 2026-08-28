//! The header row: the open file's name on the left, the Send button and the brand on the
//! right. The button is the only clickable thing here, so its rect is recorded for
//! hit-testing.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::App;
use super::send::SendState;

const BRAND: &str = "plannotator-tui ";
/// The name keeps at least this much room; below it the button is dropped instead.
const MIN_NAME_WIDTH: u16 = 8;

const SEND_BG: Color = Color::Indexed(30);
const IDLE_BG: Color = Color::Indexed(238);
const SENT_BG: Color = Color::Indexed(22);
const BLOCKED_BG: Color = Color::Indexed(58);

impl App {
    pub(super) fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let button = format!(" {} ", self.send_label());
        let button_width = button.width() as u16;
        let brand_width = BRAND.width() as u16;
        // The name is drawn with one leading space.
        let name_width = self.open.source.name.width() as u16 + 1;

        // Space runs out from the right: the brand goes first, then the button. The name is
        // truncated to whatever is left of the button, never painted over.
        let (brand, button_x) = if area.width >= name_width + button_width + brand_width {
            (true, Some(area.right().saturating_sub(brand_width + button_width)))
        } else if area.width >= MIN_NAME_WIDTH + button_width {
            (false, Some(area.right().saturating_sub(button_width)))
        } else {
            (false, None)
        };

        let room = button_x.map_or(area.width, |x| x.saturating_sub(area.x));
        let name: String = self.open.source.name.chars().take(usize::from(room.saturating_sub(1))).collect();
        let title = Line::from(vec![Span::raw(" "), Span::styled(name, Style::new().bold())]);
        frame.render_widget(Paragraph::new(title), area);
        if brand {
            frame.render_widget(Paragraph::new(Line::from(Span::raw(BRAND).dim()).right_aligned()), area);
        }

        self.geometry.send_button = button_x.map(|x| Rect { x, y: area.y, width: button_width, height: 1 });
        if let Some(rect) = self.geometry.send_button {
            let span = Span::styled(button, self.button_style());
            frame.buffer_mut().set_span(rect.x, rect.y, &span, rect.width);
        }
    }

    /// Teal when there is something to send, grey at zero, green once sent, yellow when the
    /// agent refused it.
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
