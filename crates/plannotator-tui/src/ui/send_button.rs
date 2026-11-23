//! The header's Send button colours, by send state. The colours themselves are chips
//! (`ui::chips`), checked for contrast there; the idle button sits on the toolbar grey, like
//! the Review button beside it.

use ratatui::style::Style;

use crate::theme::{Palette, Theme, palette, theme};
use crate::ui::chips;

/// Where a review's feedback stands, as the Send button shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendState {
    Ready,
    Sent,
    /// The agent is at a dialog; the message is what its host said.
    Blocked(String),
}

/// Where this session's last send went. After a copy the button says copied, not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LastSend {
    /// Nothing sent yet this session; a reopened review's notes may still be marked sent.
    #[default]
    None,
    Agent,
    Clipboard,
}

impl LastSend {
    pub(crate) fn to(is_agent: bool) -> Self {
        if is_agent { Self::Agent } else { Self::Clipboard }
    }
}

/// The button's style. At zero it is still drawn and clickable, muted.
pub(crate) fn style(state: &SendState, count: usize) -> Style {
    style_in(theme(), palette(), state, count)
}

pub(crate) fn style_in(theme: Theme, palette: &Palette, state: &SendState, count: usize) -> Style {
    let chips = chips::of(theme);
    match state {
        SendState::Ready if count == 0 => Style::new().fg(chips.send_idle_fg).bg(palette.toolbar_bg),
        SendState::Ready => chips.send,
        SendState::Sent => chips.sent,
        SendState::Blocked(_) => chips.send_blocked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_its_own_background() {
        for (theme, palette) in [(Theme::Dark, &crate::theme::DARK), (Theme::Light, &crate::theme::LIGHT)] {
            let backgrounds: Vec<_> = [
                (SendState::Ready, 3),
                (SendState::Ready, 0),
                (SendState::Sent, 0),
                (SendState::Blocked("at a dialog".into()), 3),
            ]
            .iter()
            .map(|(state, count)| style_in(theme, palette, state, *count).bg)
            .collect();
            for (i, bg) in backgrounds.iter().enumerate() {
                assert!(
                    !backgrounds.get(i + 1..).unwrap_or_default().contains(bg),
                    "{theme:?}: {backgrounds:?}"
                );
            }
        }
    }
}
