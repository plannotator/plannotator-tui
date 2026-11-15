//! The header's Send button colours, by send state.

use ratatui::style::{Color, Modifier, Style};

use crate::theme::palette;

// These set their own foreground as well as their background, so they read the same on a
// light terminal as on a dark one; only the idle button borrows the theme's grey.
const SEND_BG: Color = Color::Indexed(30);
const SENT_BG: Color = Color::Indexed(22);
const BLOCKED_BG: Color = Color::Indexed(58);

/// Where a review's feedback stands, as the Send button shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendState {
    Ready,
    Sent,
    /// The agent is at a dialog; the message is what its host said.
    Blocked(String),
}

/// The button's style. At zero it is still drawn and clickable, dimmed.
pub(crate) fn style(state: &SendState, count: usize) -> Style {
    match state {
        SendState::Ready if count == 0 => {
            Style::new().fg(palette().idle_fg).bg(palette().toolbar_bg).add_modifier(Modifier::DIM)
        }
        SendState::Ready => Style::new().fg(Color::Black).bg(SEND_BG).bold(),
        SendState::Sent => Style::new().fg(Color::Black).bg(SENT_BG).bold(),
        SendState::Blocked(_) => Style::new().fg(Color::Black).bg(BLOCKED_BG).bold(),
    }
}
