//! Light and dark terminals: the palette of roles that carry an absolute colour.
//!
//! Almost everything on screen uses the terminal's own colours, so only the backgrounds we
//! paint ourselves need a light variant. Which palette is in force is decided once, before
//! the screen is taken over (`install`), and read from everywhere by [`palette`]. Nothing
//! installs a palette in tests or in headless runs, so those keep the dark one.
//!
//! Precedence: `PLANNOTATOR_TUI_THEME` → `[ui] theme` in the config file → asking the
//! terminal → dark.

use std::fmt;
use std::io::IsTerminal as _;
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// How long the terminal gets to answer the background-colour query.
///
/// Most terminals answer the query itself, or the `DA1` sent behind it, within a few
/// milliseconds, so the answer arrives long before this. Herdr answers the colour query for
/// its panes but not `DA1`, so inside a pane the wait always runs to this deadline, and a key
/// pressed during it is lost: keep it short. A terminal that answers nothing at all is
/// treated as dark.
const DETECT_TIMEOUT: Duration = Duration::from_millis(30);

/// What the user asked for, as written in `[ui] theme` or `PLANNOTATOR_TUI_THEME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeSetting {
    /// Ask the terminal, and use the dark palette when it does not answer.
    #[default]
    Auto,
    Light,
    Dark,
}

impl ThemeSetting {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

impl fmt::Display for ThemeSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ThemeSetting {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(Self::Auto),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            other => anyhow::bail!("unknown theme {other:?}; expected auto, light or dark"),
        }
    }
}

/// The palette actually in force, once `auto` has been settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Theme {
    Dark,
    Light,
}

impl Theme {
    fn palette(self) -> &'static Palette {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
        }
    }
}

/// Every colour we paint that is not the terminal's own.
///
/// A role is here because it is painted *under* text whose colour belongs to the terminal:
/// a fixed dark background hides dark text on a light terminal, and the other way round.
/// Styles that set both a foreground and a background read the same either way and stay
/// where they are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// The focused block's bar, and the focused row of the file tree.
    pub(crate) block_bg: Color,
    /// The floating toolbar and the idle header buttons.
    pub(crate) toolbar_bg: Color,
    /// The foreground of a header button with nothing to do.
    pub(crate) idle_fg: Color,
    /// Text carrying a comment.
    pub(crate) comment_bg: Color,
    /// Text marked as looking good.
    pub(crate) approve_bg: Color,
    /// The keyboard cursor cell.
    pub(crate) cursor: Style,
    /// A character selection, a picker row, a review-menu row.
    pub(crate) selection: Style,
}

/// The palette this app has always had. Dark terminals must see no change at all.
pub(crate) static DARK: Palette = Palette {
    block_bg: Color::Indexed(236),
    toolbar_bg: Color::Indexed(238),
    idle_fg: Color::Gray,
    comment_bg: Color::Indexed(58),
    approve_bg: Color::Indexed(22),
    cursor: Style::new().bg(Color::Indexed(240)),
    selection: Style::new().add_modifier(Modifier::REVERSED),
};

/// The same roles, light. The greys mirror the dark ones about the middle of the ramp, so
/// the focused block still sits just off the page and the cursor is a shade firmer. The
/// annotation tints are the pale ends of the same hues. `REVERSED` becomes a real
/// background: reversing dark-on-light gives a black bar that swallows the tints under it.
pub(crate) static LIGHT: Palette = Palette {
    block_bg: Color::Indexed(254),
    toolbar_bg: Color::Indexed(252),
    idle_fg: Color::DarkGray,
    comment_bg: Color::Indexed(222),
    approve_bg: Color::Indexed(157),
    // A dark cell with its own light foreground, so the glyph under the cursor stays legible.
    cursor: Style::new().bg(Color::Indexed(238)).fg(Color::White),
    selection: Style::new().bg(Color::Indexed(153)),
};

static PALETTE: OnceLock<&'static Palette> = OnceLock::new();

/// The palette in force. Dark until [`install`] says otherwise, which is what tests,
/// `--snapshot`, `--export` and every other headless run get.
pub(crate) fn palette() -> &'static Palette {
    PALETTE.get().copied().unwrap_or(&DARK)
}

/// Fix the palette for this process. The first call wins; later ones are ignored.
pub(crate) fn install(theme: Theme) {
    let _ = PALETTE.set(theme.palette());
}

/// Ask the terminal whether its background is light or dark.
///
/// The query opens the tty itself rather than reading our stdin, so it cannot race the
/// event loop's reader or leave a half-parsed escape sequence behind it. It does hold the
/// tty in raw mode for one round trip, and anything typed into that window is read and
/// discarded with the reply; `theme = "light"`, `theme = "dark"` or `PLANNOTATOR_TUI_THEME`
/// skip the query outright for anyone that window bothers.
///
/// `None` means the terminal did not answer, answered something unreadable, or there is no
/// terminal to ask - all of which keep the dark palette.
pub(crate) fn detect() -> Option<Theme> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    #[allow(
        clippy::field_reassign_with_default,
        reason = "QueryOptions is #[non_exhaustive]; struct-update syntax is not available to us"
    )]
    let options = {
        let mut options = terminal_colorsaurus::QueryOptions::default();
        options.timeout = DETECT_TIMEOUT;
        options
    };
    match terminal_colorsaurus::theme_mode(options) {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Some(Theme::Light),
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Some(Theme::Dark),
        Err(_) => None,
    }
}

/// `PLANNOTATOR_TUI_THEME`, when it is set to something. An unreadable value is an error
/// that names it, like a bad value in the config file.
fn setting_from_env(env: &impl Fn(&str) -> Option<String>) -> Result<Option<ThemeSetting>> {
    match env("PLANNOTATOR_TUI_THEME").filter(|value| !value.is_empty()) {
        Some(value) => value.parse().map(Some),
        None => Ok(None),
    }
}

/// What `plannotator-tui config` should print: the environment's answer if it has one,
/// else the file's.
pub(crate) fn effective_setting(
    env: impl Fn(&str) -> Option<String>,
    configured: ThemeSetting,
) -> Result<ThemeSetting> {
    Ok(setting_from_env(&env)?.unwrap_or(configured))
}

/// Settle on a palette: environment, then config file, then the terminal, then dark.
pub(crate) fn resolve(
    env: impl Fn(&str) -> Option<String>,
    configured: ThemeSetting,
    detect: impl FnOnce() -> Option<Theme>,
) -> Result<Theme> {
    Ok(match effective_setting(env, configured)? {
        ThemeSetting::Light => Theme::Light,
        ThemeSetting::Dark => Theme::Dark,
        ThemeSetting::Auto => detect().unwrap_or(Theme::Dark),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |key: &str| vars.iter().find(|(name, _)| *name == key).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn the_environment_beats_the_config_file() {
        let vars = env(&[("PLANNOTATOR_TUI_THEME", "light")]);
        assert_eq!(resolve(vars, ThemeSetting::Dark, || None).expect("resolves"), Theme::Light);
    }

    #[test]
    fn the_config_file_beats_detection() {
        let detect = || Some(Theme::Light);
        assert_eq!(resolve(env(&[]), ThemeSetting::Dark, detect).expect("resolves"), Theme::Dark);
    }

    #[test]
    fn auto_in_the_environment_reopens_the_question_a_configured_theme_closed() {
        let vars = env(&[("PLANNOTATOR_TUI_THEME", "auto")]);
        let resolved = resolve(vars, ThemeSetting::Dark, || Some(Theme::Light)).expect("resolves");
        assert_eq!(resolved, Theme::Light);
    }

    #[test]
    fn auto_uses_the_terminals_answer_and_falls_back_to_dark_without_one() {
        let auto = ThemeSetting::Auto;
        assert_eq!(resolve(env(&[]), auto, || Some(Theme::Light)).expect("resolves"), Theme::Light);
        assert_eq!(resolve(env(&[]), auto, || None).expect("resolves"), Theme::Dark);
    }

    #[test]
    fn an_empty_environment_variable_is_not_a_choice() {
        let vars = env(&[("PLANNOTATOR_TUI_THEME", "")]);
        assert_eq!(resolve(vars, ThemeSetting::Light, || None).expect("resolves"), Theme::Light);
    }

    #[test]
    fn an_unreadable_environment_variable_is_an_error_naming_the_value() {
        let vars = env(&[("PLANNOTATOR_TUI_THEME", "solarized")]);
        let err = resolve(vars, ThemeSetting::Auto, || None).expect_err("rejected");
        assert!(err.to_string().contains("solarized"), "{err}");
    }

    /// The dark palette is the one shipped before light terminals were supported; a change
    /// here is a visible change for every existing user.
    #[test]
    fn the_dark_palette_is_unchanged() {
        assert_eq!(DARK.block_bg, Color::Indexed(236));
        assert_eq!(DARK.toolbar_bg, Color::Indexed(238));
        assert_eq!(DARK.idle_fg, Color::Gray);
        assert_eq!(DARK.comment_bg, Color::Indexed(58));
        assert_eq!(DARK.approve_bg, Color::Indexed(22));
        assert_eq!(DARK.cursor, Style::new().bg(Color::Indexed(240)));
        assert_eq!(DARK.selection, Style::new().add_modifier(Modifier::REVERSED));
    }

    /// Nothing installs a palette in a test binary, so every existing snapshot and style
    /// assertion keeps reading the dark one.
    #[test]
    fn the_palette_is_dark_until_something_installs_one() {
        assert_eq!(*palette(), DARK);
    }

    /// Both palettes must answer for every role; a light background painted on a light
    /// terminal is the bug this exists to fix.
    #[test]
    fn no_light_role_reuses_its_dark_value() {
        assert_ne!(LIGHT.block_bg, DARK.block_bg);
        assert_ne!(LIGHT.toolbar_bg, DARK.toolbar_bg);
        assert_ne!(LIGHT.comment_bg, DARK.comment_bg);
        assert_ne!(LIGHT.approve_bg, DARK.approve_bg);
        assert_ne!(LIGHT.cursor, DARK.cursor);
        assert_ne!(LIGHT.selection, DARK.selection);
    }

    /// `REVERSED` is what made a selection unreadable on a light terminal; the light
    /// palette must not reach for it again.
    #[test]
    fn the_light_selection_is_a_background_not_a_reversal() {
        assert!(!LIGHT.selection.add_modifier.contains(Modifier::REVERSED));
        assert!(LIGHT.selection.bg.is_some());
    }

    #[test]
    fn a_theme_setting_roundtrips_through_its_own_name() {
        for setting in [ThemeSetting::Auto, ThemeSetting::Light, ThemeSetting::Dark] {
            assert_eq!(setting.as_str().parse::<ThemeSetting>().expect("parses"), setting);
        }
    }
}
