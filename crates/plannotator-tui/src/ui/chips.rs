//! Text we draw on a background we paint ourselves: the header buttons and the floating
//! toolbar.
//!
//! Both colours of each pair come from the 256-colour cube and grey ramp, never from the 16
//! ANSI colours: terminal themes remap those, so text taken from them can land on a
//! background it no longer contrasts with. Every pair holds at least 4.5:1 in its theme (see
//! the test). The toolbar grey itself stays a palette role, so `[colors] toolbar` still
//! reaches it.

use plannotator_tui_schema::Kind;
use ratatui::style::{Color, Style};

use crate::theme::{Theme, theme};

/// One theme's chip colours.
pub(crate) struct Chips {
    /// Send with new notes: the call to action, the loudest of the Send states.
    pub(crate) send: Style,
    /// Everything is sent: done, and quiet.
    pub(crate) sent: Style,
    /// The agent is at a dialog: a warning.
    pub(crate) send_blocked: Style,
    /// The Send button's text with nothing to send, on the toolbar grey.
    pub(crate) send_idle_fg: Color,
    /// Approve, in a gated review.
    pub(crate) approve: Style,
    /// The Review ▾ menu button's text, on the toolbar grey.
    pub(crate) menu_fg: Color,
    /// The floating toolbar's labels, on the toolbar grey: comment, looks good, delete.
    toolbar_comment: Color,
    toolbar_looks_good: Color,
    toolbar_delete: Color,
}

impl Chips {
    /// The floating toolbar label for one kind of annotation.
    pub(crate) fn toolbar(&self, kind: Kind) -> Color {
        match kind {
            Kind::Comment => self.toolbar_comment,
            Kind::LooksGood => self.toolbar_looks_good,
            Kind::Delete => self.toolbar_delete,
        }
    }
}

const BLACK: Color = Color::Indexed(16);
const SEND: Style = Style::new().fg(BLACK).bg(Color::Indexed(37)).bold();
const SEND_BLOCKED: Style = Style::new().fg(BLACK).bg(Color::Indexed(214)).bold();
const APPROVE: Style = Style::new().fg(BLACK).bg(Color::Indexed(34)).bold();

/// Pale text on the dark toolbar grey; sent is a dark green chip with pale green text.
static DARK: Chips = Chips {
    send: SEND,
    sent: Style::new().fg(Color::Indexed(194)).bg(Color::Indexed(22)),
    send_blocked: SEND_BLOCKED,
    send_idle_fg: Color::Indexed(250),
    approve: APPROVE,
    menu_fg: Color::Indexed(80),
    toolbar_comment: Color::Indexed(221),
    toolbar_looks_good: Color::Indexed(114),
    toolbar_delete: Color::Indexed(217),
};

/// Dark text on the light toolbar grey; sent turns round to a pale chip with dark green text.
static LIGHT: Chips = Chips {
    send: SEND,
    sent: Style::new().fg(Color::Indexed(22)).bg(Color::Indexed(194)),
    send_blocked: SEND_BLOCKED,
    send_idle_fg: Color::Indexed(239),
    approve: APPROVE,
    menu_fg: Color::Indexed(23),
    toolbar_comment: Color::Indexed(58),
    toolbar_looks_good: Color::Indexed(22),
    toolbar_delete: Color::Indexed(124),
};

/// The chips of the theme in force.
pub(crate) fn chips() -> &'static Chips {
    of(theme())
}

pub(crate) fn of(theme: Theme) -> &'static Chips {
    match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::theme::Palette;
    use crate::ui::send_button::{SendState, style_in};

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

    /// Every pair of text and background we paint, in one theme: the chips, and the palette
    /// styles that fix their text as well as their background.
    fn pairs(theme: Theme, palette: &Palette) -> Vec<(String, Style)> {
        let chips = of(theme);
        let on_toolbar = |fg: Color| Style::new().fg(fg).bg(palette.toolbar_bg);
        let mut pairs = vec![
            ("send, new notes", style_in(theme, palette, &SendState::Ready, 3)),
            ("send, nothing to send", style_in(theme, palette, &SendState::Ready, 0)),
            ("send, sent", style_in(theme, palette, &SendState::Sent, 0)),
            ("send, at a dialog", style_in(theme, palette, &SendState::Blocked("dialog".into()), 3)),
            ("approve", chips.approve),
            ("review menu", on_toolbar(chips.menu_fg)),
            ("toolbar comment", on_toolbar(chips.toolbar(Kind::Comment))),
            ("toolbar looks good", on_toolbar(chips.toolbar(Kind::LooksGood))),
            ("toolbar delete", on_toolbar(chips.toolbar(Kind::Delete))),
        ];
        let styles = [
            ("cursor", palette.cursor),
            ("selection", palette.selection),
            ("search", palette.search),
            ("search current", palette.search_current),
        ];
        pairs.extend(styles.into_iter().filter(|(_, style)| style.fg.is_some() && style.bg.is_some()));
        pairs.into_iter().map(|(name, style)| (name.to_owned(), style)).collect()
    }

    #[test]
    fn every_painted_pair_reads_at_least_4_5_to_1_in_both_themes_without_ansi_colours() {
        for (theme, palette) in [(Theme::Dark, &crate::theme::DARK), (Theme::Light, &crate::theme::LIGHT)] {
            for (name, style) in pairs(theme, palette) {
                let fg = style.fg.and_then(rgb).expect("a fixed 256-colour foreground");
                let bg = style.bg.and_then(rgb).expect("a fixed 256-colour background");
                let (fg, bg) = (luminance(fg), luminance(bg));
                let ratio = (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05);
                assert!(ratio >= 4.5, "{theme:?} {name}: {ratio:.2}:1 ({style:?})");
            }
        }
    }
}
