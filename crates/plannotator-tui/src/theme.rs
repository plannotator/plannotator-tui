//! Light and dark terminals: the palette of roles that carry an absolute colour.
//!
//! Almost everything on screen uses the terminal's own colours, so only the backgrounds we
//! paint ourselves need a light variant. Which palette is in force is decided once, before
//! the screen is taken over (`install`), and read from everywhere by [`palette`]. Nothing
//! installs a palette in tests or in headless runs, so those keep the dark one.
//!
//! Precedence: `PLANNOTATOR_TUI_THEME` → `[ui] theme` in the config file → asking the
//! terminal → dark. `[colors]` then replaces individual roles of whichever palette won.

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

/// Every colour a theme or `[colors]` can change.
///
/// The backgrounds are here because they are painted *under* text whose colour belongs to
/// the terminal: a fixed dark background hides dark text on a light terminal, and the other
/// way round. The foregrounds are ANSI colours that already follow the terminal, so both
/// palettes share them; they are here so `[colors]` can reach them. Styles that set both a
/// foreground and a background read the same either way and stay where they are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// Focused borders and gutter, the open file in the tree, active header text.
    pub(crate) accent: Color,
    /// Level one and two headings.
    pub(crate) heading: Color,
    /// Level three and deeper headings.
    pub(crate) subheading: Color,
    /// Comment titles, the compose box, annotated rows in the gutter, tree counts.
    pub(crate) comment: Color,
    /// Looks-good titles and toolbar label.
    pub(crate) looks_good: Color,
    /// Delete titles, toolbar label and struck-through text.
    pub(crate) delete: Color,
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
    accent: Color::Cyan,
    heading: Color::Cyan,
    subheading: Color::LightCyan,
    comment: Color::Yellow,
    looks_good: Color::Green,
    delete: Color::Red,
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
    accent: DARK.accent,
    heading: DARK.heading,
    subheading: DARK.subheading,
    comment: DARK.comment,
    looks_good: DARK.looks_good,
    delete: DARK.delete,
    block_bg: Color::Indexed(254),
    toolbar_bg: Color::Indexed(252),
    idle_fg: Color::DarkGray,
    comment_bg: Color::Indexed(222),
    approve_bg: Color::Indexed(157),
    // A dark cell with its own light foreground, so the glyph under the cursor stays legible.
    cursor: Style::new().bg(Color::Indexed(238)).fg(Color::White),
    selection: Style::new().bg(Color::Indexed(153)),
};

/// A colour as `[colors]` writes it: a name (`yellow`, `light-cyan`), a 256-colour index
/// (`238`) or `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ColorValue(pub(crate) Color);

impl FromStr for ColorValue {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        s.parse().map(Self).map_err(|_| {
            anyhow::anyhow!("unknown color {s:?}; expected a name like yellow, an index 0-255, or #rrggbb")
        })
    }
}

/// Prints the colour the way `[colors]` writes it, so `plannotator-tui config` output can be
/// pasted back: `light-blue`, `238`, `#1e1e2e`.
impl fmt::Display for ColorValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.0 {
            Color::Indexed(index) => return write!(f, "{index}"),
            Color::Rgb(r, g, b) => return write!(f, "#{r:02x}{g:02x}{b:02x}"),
            Color::Reset => "reset",
            Color::Black => "black",
            Color::Red => "red",
            Color::Green => "green",
            Color::Yellow => "yellow",
            Color::Blue => "blue",
            Color::Magenta => "magenta",
            Color::Cyan => "cyan",
            Color::Gray => "gray",
            Color::DarkGray => "dark-gray",
            Color::LightRed => "light-red",
            Color::LightGreen => "light-green",
            Color::LightYellow => "light-yellow",
            Color::LightBlue => "light-blue",
            Color::LightMagenta => "light-magenta",
            Color::LightCyan => "light-cyan",
            Color::White => "white",
        };
        f.write_str(name)
    }
}

impl Serialize for ColorValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// A string as above, or a bare 256-colour index: `heading = 238`.
impl<'de> Deserialize<'de> for ColorValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = ColorValue;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a color name, an index 0-255, or #rrggbb")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> std::result::Result<ColorValue, E> {
                text.parse().map_err(E::custom)
            }

            fn visit_i64<E: serde::de::Error>(self, index: i64) -> std::result::Result<ColorValue, E> {
                u8::try_from(index)
                    .map(|index| ColorValue(Color::Indexed(index)))
                    .map_err(|_| E::custom(format!("unknown color {index}; a color index is 0-255")))
            }

            fn visit_u64<E: serde::de::Error>(self, index: u64) -> std::result::Result<ColorValue, E> {
                u8::try_from(index)
                    .map(|index| ColorValue(Color::Indexed(index)))
                    .map_err(|_| E::custom(format!("unknown color {index}; a color index is 0-255")))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// `[colors]`: per-role replacements for the palette in force. A role left out keeps the
/// light or dark value; `cursor` and `selection` become plain backgrounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default, rename_all = "kebab-case")]
pub(crate) struct Colors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) accent: Option<ColorValue>,
    /// Every heading level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) heading: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) comment: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) looks_good: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) delete: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) comment_bg: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) looks_good_bg: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) block: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) toolbar: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<ColorValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) selection: Option<ColorValue>,
}

impl Palette {
    /// This palette with every role `colors` names replaced.
    pub(crate) fn with(mut self, colors: &Colors) -> Self {
        let set = |slot: &mut Color, value: Option<ColorValue>| {
            if let Some(ColorValue(color)) = value {
                *slot = color;
            }
        };
        set(&mut self.accent, colors.accent);
        set(&mut self.heading, colors.heading);
        set(&mut self.subheading, colors.heading);
        set(&mut self.comment, colors.comment);
        set(&mut self.looks_good, colors.looks_good);
        set(&mut self.delete, colors.delete);
        set(&mut self.comment_bg, colors.comment_bg);
        set(&mut self.approve_bg, colors.looks_good_bg);
        set(&mut self.block_bg, colors.block);
        set(&mut self.toolbar_bg, colors.toolbar);
        if let Some(ColorValue(color)) = colors.cursor {
            self.cursor = Style::new().bg(color);
        }
        if let Some(ColorValue(color)) = colors.selection {
            self.selection = Style::new().bg(color);
        }
        self
    }
}

static PALETTE: OnceLock<Palette> = OnceLock::new();

/// The palette in force. Dark until [`install`] says otherwise, which is what tests,
/// `--snapshot`, `--export` and every other headless run get.
pub(crate) fn palette() -> &'static Palette {
    PALETTE.get().unwrap_or(&DARK)
}

/// Fix the palette for this process. The first call wins; later ones are ignored.
pub(crate) fn install(theme: Theme, colors: &Colors) {
    let _ = PALETTE.set(theme.palette().with(colors));
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
        assert_eq!(
            (DARK.accent, DARK.heading, DARK.subheading),
            (Color::Cyan, Color::Cyan, Color::LightCyan)
        );
        assert_eq!((DARK.comment, DARK.looks_good, DARK.delete), (Color::Yellow, Color::Green, Color::Red));
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
    fn no_colors_leave_both_palettes_alone() {
        assert_eq!(DARK.with(&Colors::default()), DARK);
        assert_eq!(LIGHT.with(&Colors::default()), LIGHT);
    }

    #[test]
    fn a_color_replaces_its_role_and_nothing_else() {
        let colors = Colors { block: Some(ColorValue(Color::Rgb(1, 2, 3))), ..Colors::default() };
        let palette = LIGHT.with(&colors);
        assert_eq!(palette.block_bg, Color::Rgb(1, 2, 3));
        assert_eq!(Palette { block_bg: LIGHT.block_bg, ..palette }, LIGHT);
    }

    #[test]
    fn heading_covers_every_level() {
        let colors = Colors { heading: Some(ColorValue(Color::Magenta)), ..Colors::default() };
        let palette = DARK.with(&colors);
        assert_eq!((palette.heading, palette.subheading), (Color::Magenta, Color::Magenta));
    }

    /// A configured cursor or selection is the whole style: the dark selection's reversal
    /// and the light cursor's white glyph would fight a colour picked to match a theme.
    #[test]
    fn cursor_and_selection_become_plain_backgrounds() {
        let colors = Colors {
            cursor: Some(ColorValue(Color::Indexed(7))),
            selection: Some(ColorValue(Color::Indexed(8))),
            ..Colors::default()
        };
        assert_eq!(LIGHT.with(&colors).cursor, Style::new().bg(Color::Indexed(7)));
        assert_eq!(DARK.with(&colors).selection, Style::new().bg(Color::Indexed(8)));
    }

    #[test]
    fn a_color_reads_names_indexes_and_hex_and_prints_back_the_same() {
        for (text, color) in [
            ("light-cyan", Color::LightCyan),
            ("238", Color::Indexed(238)),
            ("#1e1e2e", Color::Rgb(30, 30, 46)),
        ] {
            let parsed: ColorValue = text.parse().expect("parses");
            assert_eq!(parsed.0, color);
            assert_eq!(parsed.to_string(), text);
        }
    }

    #[test]
    fn every_color_prints_in_a_spelling_that_parses_back() {
        for color in [
            Color::Reset,
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightMagenta,
            Color::LightCyan,
            Color::White,
            Color::Indexed(0),
            Color::Indexed(255),
            Color::Rgb(0, 0, 255),
        ] {
            let text = ColorValue(color).to_string();
            assert_eq!(text.parse::<ColorValue>().expect("parses").0, color, "{text}");
        }
    }

    #[test]
    fn an_unreadable_color_is_an_error_naming_it() {
        let err = "chartreuse".parse::<ColorValue>().expect_err("rejected");
        assert!(err.to_string().contains("chartreuse"), "{err}");
    }

    #[test]
    fn a_theme_setting_roundtrips_through_its_own_name() {
        for setting in [ThemeSetting::Auto, ThemeSetting::Light, ThemeSetting::Dark] {
            assert_eq!(setting.as_str().parse::<ThemeSetting>().expect("parses"), setting);
        }
    }
}
