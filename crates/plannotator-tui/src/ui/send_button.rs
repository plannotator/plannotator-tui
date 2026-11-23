//! The header's Send button colours, by send state.
//!
//! Every state sets its own foreground as well as its background, from the 256-colour cube
//! and grey ramp only: terminal themes remap the 16 ANSI colours, so black or white text
//! taken from them can land on a background it no longer contrasts with. Each pair holds at
//! least 4.5:1 (see the tests). The idle button sits on the toolbar grey, like the Review
//! button beside it.

use ratatui::style::{Color, Style};

use crate::theme::{Palette, Theme, palette, theme};

/// Where a review's feedback stands, as the Send button shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendState {
    Ready,
    Sent,
    /// The agent is at a dialog; the message is what its host said.
    Blocked(String),
}

/// One theme's button colours.
struct Colors {
    /// New notes to send: the call to action, the loudest of the four.
    ready: Style,
    /// Everything is sent: done, and quiet.
    sent: Style,
    /// The agent is at a dialog: a warning.
    blocked: Style,
    /// Nothing to send: the toolbar grey with a muted foreground.
    idle_fg: Color,
}

const BLACK: Color = Color::Indexed(16);
const READY: Style = Style::new().fg(BLACK).bg(Color::Indexed(37));
const BLOCKED: Style = Style::new().fg(BLACK).bg(Color::Indexed(214));

/// Dark green with pale green text: done, without drawing the eye on a dark page.
const DARK: Colors = Colors {
    ready: READY,
    sent: Style::new().fg(Color::Indexed(194)).bg(Color::Indexed(22)),
    blocked: BLOCKED,
    idle_fg: Color::Indexed(250),
};

/// The same green turned round: a pale chip with dark green text, quiet on a light page.
const LIGHT: Colors = Colors {
    ready: READY,
    sent: Style::new().fg(Color::Indexed(22)).bg(Color::Indexed(194)),
    blocked: BLOCKED,
    idle_fg: Color::Indexed(239),
};

/// The button's style. At zero it is still drawn and clickable, muted.
pub(crate) fn style(state: &SendState, count: usize) -> Style {
    style_in(theme(), palette(), state, count)
}

fn style_in(theme: Theme, palette: &Palette, state: &SendState, count: usize) -> Style {
    let colors = match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    };
    match state {
        SendState::Ready if count == 0 => Style::new().fg(colors.idle_fg).bg(palette.toolbar_bg),
        SendState::Ready => colors.ready.bold(),
        SendState::Sent => colors.sent,
        SendState::Blocked(_) => colors.blocked.bold(),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    const THEMES: [(Theme, &Palette); 2] =
        [(Theme::Dark, &crate::theme::DARK), (Theme::Light, &crate::theme::LIGHT)];

    fn states() -> [(&'static str, SendState, usize); 4] {
        [
            ("new notes", SendState::Ready, 3),
            ("nothing to send", SendState::Ready, 0),
            ("sent", SendState::Sent, 0),
            ("blocked", SendState::Blocked("at a dialog".into()), 3),
        ]
    }

    /// The colour xterm's 256-colour table gives an index from 16 up. `None` for the 16 ANSI
    /// colours, which the terminal's theme decides, and for anything else without a fixed value.
    fn rgb(color: Color) -> Option<[f64; 3]> {
        let Color::Indexed(index @ 16..) = color else { return None };
        let level = |step: u8| if step == 0 { 0.0 } else { f64::from(55 + 40 * step) };
        Some(match index {
            16..=231 => {
                let cube = index - 16;
                [level(cube / 36), level(cube / 6 % 6), level(cube % 6)]
            }
            _ => [f64::from(8 + 10 * (index - 232)); 3],
        })
    }

    /// WCAG 2 relative luminance.
    fn luminance([r, g, b]: [f64; 3]) -> f64 {
        let linear = |channel: f64| {
            let c = channel / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    #[test]
    fn every_state_reads_at_least_4_5_to_1_in_both_themes_without_ansi_colours() {
        for (theme, palette) in THEMES {
            for (name, state, count) in states() {
                let style = style_in(theme, palette, &state, count);
                let fg = style.fg.and_then(rgb).expect("a fixed 256-colour foreground");
                let bg = style.bg.and_then(rgb).expect("a fixed 256-colour background");
                let (fg, bg) = (luminance(fg), luminance(bg));
                let ratio = (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05);
                assert!(ratio >= 4.5, "{theme:?} {name}: {ratio:.2}:1 ({style:?})");
            }
        }
    }

    #[test]
    fn every_state_has_its_own_background() {
        for (theme, palette) in THEMES {
            let backgrounds: Vec<_> =
                states().iter().map(|(_, state, count)| style_in(theme, palette, state, *count).bg).collect();
            for (i, bg) in backgrounds.iter().enumerate() {
                assert!(
                    !backgrounds.get(i + 1..).unwrap_or_default().contains(bg),
                    "{theme:?}: {backgrounds:?}"
                );
            }
        }
    }
}
