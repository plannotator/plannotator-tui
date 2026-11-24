//! `[review]`: how the code review draws its diff, as the person sets it by hand. Every key is
//! optional; a file without the table opens diffs as before. The app never writes this file:
//! what the person chooses in the review (its settings menu, `u`, `w`, a drag of the line
//! between the sides) is remembered apart, in the data folder (`review::settings::state`),
//! and wins over these values until the menu's Reset clears it.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// `[review]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct ReviewConfig {
    /// `auto` splits a diff 120 columns wide or wider and shows a narrower one unified.
    pub(crate) layout: Layout,
    /// Wrap long lines instead of scrolling them sideways.
    pub(crate) wrap: bool,
    /// Line numbers in the gutter. Off keeps the `+`/`-` signs.
    pub(crate) line_numbers: bool,
    /// The diff's colours: `auto` follows the app's theme (`[ui] theme`).
    pub(crate) theme: DiffTheme,
    /// `terminal` draws the diff on the terminal's own background, as the rest of the app;
    /// `theme` paints the diff theme's own (Pierre's near-black or white).
    pub(crate) background: Background,
    /// The old side's share of a split row, from 0.1 to 0.9; the new side takes the rest.
    pub(crate) split_ratio: SplitShare,
    /// Changes that only touch whitespace: shown, or hidden as `git diff -w` or `-b` hide them.
    pub(crate) whitespace: Whitespace,
    /// How many columns a tab advances to, from 1 to 16.
    pub(crate) tab_width: TabWidth,
    /// How the changed text inside a changed line is marked, by Pierre's names.
    pub(crate) inline_changes: InlineChanges,
}

impl ReviewConfig {
    /// The defaults the review has always had.
    pub(crate) const DEFAULT: Self = Self {
        layout: Layout::Auto,
        wrap: false,
        line_numbers: true,
        theme: DiffTheme::Auto,
        background: Background::Terminal,
        split_ratio: SplitShare::EVEN,
        whitespace: Whitespace::Show,
        tab_width: TabWidth::DEFAULT,
        inline_changes: InlineChanges::Word,
    };
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// `[review] layout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Layout {
    #[default]
    Auto,
    Split,
    Unified,
}

/// `[review] theme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DiffTheme {
    #[default]
    Auto,
    Dark,
    Light,
}

/// `[review] background`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Background {
    #[default]
    Terminal,
    Theme,
}

/// `[review] whitespace`: what `git diff` shows (`show`), `-w` (`ignore-all`) or `-b`
/// (`ignore-change`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Whitespace {
    #[default]
    Show,
    /// Lines equal once all whitespace is removed match, as `git diff -w`.
    IgnoreAll,
    /// A change in the amount of whitespace, and whitespace at a line's end, match, as
    /// `git diff -b`.
    IgnoreChange,
}

/// `[review] inline_changes`: the changed text inside a changed line, by the names of
/// Pierre's `lineDiffType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum InlineChanges {
    /// Changed words.
    #[default]
    Word,
    /// Changed words, two changes one character apart marked as one.
    WordAlt,
    /// Changed characters.
    Char,
    /// No marks inside lines; the lines' colours still show the change.
    None,
}

/// `[review] tab_width`: the columns a tab advances to, from 1 to 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct TabWidth(u8);

#[cfg_attr(not(unix), allow(dead_code, reason = "only the code review, Unix-only, changes them"))]
impl TabWidth {
    pub(crate) const DEFAULT: Self = Self(4);
    pub(crate) const MIN: u8 = 1;
    pub(crate) const MAX: u8 = 16;

    /// `columns`, kept within 1 and 16.
    pub(crate) fn new(columns: u8) -> Self {
        Self(columns.clamp(Self::MIN, Self::MAX))
    }

    pub(crate) fn get(self) -> u8 {
        self.0
    }
}

impl Default for TabWidth {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl<'de> Deserialize<'de> for TabWidth {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let columns = i64::deserialize(deserializer)?;
        match u8::try_from(columns) {
            Ok(columns) if (Self::MIN..=Self::MAX).contains(&columns) => Ok(Self(columns)),
            _ => Err(serde::de::Error::custom(format!(
                "tab_width {columns} is out of range; expected {} to {} columns",
                Self::MIN,
                Self::MAX
            ))),
        }
    }
}

/// The old side's share of a split row, in thousandths, written as a fraction: `0.5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SplitShare(u16);

#[cfg_attr(not(unix), allow(dead_code, reason = "only the code review, Unix-only, changes them"))]
impl SplitShare {
    pub(crate) const EVEN: Self = Self(500);
    pub(crate) const MIN: u16 = 100;
    pub(crate) const MAX: u16 = 900;

    /// `old` thousandths, kept within 0.1 and 0.9.
    pub(crate) fn from_permille(old: u16) -> Self {
        Self(old.clamp(Self::MIN, Self::MAX))
    }

    pub(crate) fn permille(self) -> u16 {
        self.0
    }
}

impl Default for SplitShare {
    fn default() -> Self {
        Self::EVEN
    }
}

/// `55 : 45`, by whole percents.
impl fmt::Display for SplitShare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let old = (u32::from(self.0) + 5) / 10;
        write!(f, "{old} : {}", 100 - old)
    }
}

impl Serialize for SplitShare {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(f64::from(self.0) / 1000.0)
    }
}

impl<'de> Deserialize<'de> for SplitShare {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let share = f64::deserialize(deserializer)?;
        let (least, most) = (f64::from(Self::MIN) / 1000.0, f64::from(Self::MAX) / 1000.0);
        if !(least..=most).contains(&share) {
            return Err(serde::de::Error::custom(format!(
                "split_ratio {share} is out of range; expected a number from {least} to {most}, \
                 the old side's share of the width"
            )));
        }
        Ok(Self((share * 1000.0).round() as u16))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn configs_written_before_review_settings_load_unchanged_with_the_old_defaults() {
        // Files as 0.11.1 read them: none had a [review] table.
        let table = [
            "",
            "[ui]\ntheme = \"light\"\ncopy_on_select = false\n",
            "[keys]\ntoggle-layout = [\">\"]\nnarrow-files = [\",\"]\n",
            "[herdr]\nplacement = \"popup\"\n[colors]\naccent = \"magenta\"\n",
            "[ui]\nnotes_rail = \"hidden\"\ndoc_width = 0\n",
        ];
        for text in table {
            let config = Config::parse(text).unwrap_or_else(|e| panic!("{text:?} loads: {e}"));
            assert_eq!(config.review, ReviewConfig::DEFAULT, "{text:?}");
        }
        assert_eq!(ReviewConfig::default(), ReviewConfig::DEFAULT);
        assert_eq!(
            ReviewConfig::DEFAULT,
            ReviewConfig {
                layout: Layout::Auto,
                wrap: false,
                line_numbers: true,
                theme: DiffTheme::Auto,
                background: Background::Terminal,
                split_ratio: SplitShare::from_permille(500),
                whitespace: Whitespace::Show,
                tab_width: TabWidth::new(4),
                inline_changes: InlineChanges::Word,
            }
        );
    }

    #[test]
    fn configs_written_for_0_11_2_load_unchanged_and_draw_whitespace_tabs_and_changes_as_before() {
        // [review] as 0.11.2 read it, with every key it knew; the keys added since default.
        let table = [
            ("[review]\n", ReviewConfig::DEFAULT),
            (
                "[review]\nlayout = \"split\"\nwrap = true\n",
                ReviewConfig { layout: Layout::Split, wrap: true, ..ReviewConfig::DEFAULT },
            ),
            (
                "[ui]\ncopy_on_select = false\n\n[review]\nline_numbers = false\ntheme = \"dark\"\nbackground = \"theme\"\nsplit_ratio = 0.62\n",
                ReviewConfig {
                    line_numbers: false,
                    theme: DiffTheme::Dark,
                    background: Background::Theme,
                    split_ratio: SplitShare::from_permille(620),
                    ..ReviewConfig::DEFAULT
                },
            ),
            (
                "[review]\nlayout = \"unified\"\nwrap = false\nline_numbers = true\ntheme = \"light\"\nbackground = \"terminal\"\nsplit_ratio = 0.5\n",
                ReviewConfig { layout: Layout::Unified, theme: DiffTheme::Light, ..ReviewConfig::DEFAULT },
            ),
        ];
        for (text, review) in table {
            let config = Config::parse(text).unwrap_or_else(|e| panic!("{text:?} loads: {e}"));
            assert_eq!(config.review, review, "{text:?}");
            assert_eq!(
                (config.review.whitespace, config.review.tab_width.get(), config.review.inline_changes),
                (Whitespace::Show, 4, InlineChanges::Word),
                "{text:?}"
            );
        }
    }

    #[test]
    fn every_review_setting_is_read_and_roundtrips() {
        let text = "[review]\nlayout = \"unified\"\nwrap = true\nline_numbers = false\ntheme = \"light\"\nbackground = \"theme\"\nsplit_ratio = 0.62\nwhitespace = \"ignore-all\"\ntab_width = 8\ninline_changes = \"word-alt\"\n";
        let config = Config::parse(text).expect("parses");
        let review = config.review;
        assert_eq!(review.layout, Layout::Unified);
        assert!(review.wrap && !review.line_numbers);
        assert_eq!((review.theme, review.background), (DiffTheme::Light, Background::Theme));
        assert_eq!(review.split_ratio.permille(), 620);
        assert_eq!(review.split_ratio.to_string(), "62 : 38");
        assert_eq!(review.whitespace, Whitespace::IgnoreAll);
        assert_eq!(review.tab_width.get(), 8);
        assert_eq!(review.inline_changes, InlineChanges::WordAlt);
        let shown = config.to_toml().expect("toml");
        assert!(shown.contains("[review]\nlayout = \"unified\""), "{shown}");
        assert!(shown.contains("split_ratio = 0.62\n"), "{shown}");
        assert!(
            shown.contains("whitespace = \"ignore-all\"\ntab_width = 8\ninline_changes = \"word-alt\"\n"),
            "{shown}"
        );
        assert_eq!(Config::parse(&shown).expect("reparses"), config);
        // Every value of the three by its name in the file, and any width from 1 to 16.
        for (name, value) in [
            ("show", Whitespace::Show),
            ("ignore-all", Whitespace::IgnoreAll),
            ("ignore-change", Whitespace::IgnoreChange),
        ] {
            let config = Config::parse(&format!("[review]\nwhitespace = \"{name}\"\n")).expect("parses");
            assert_eq!(config.review.whitespace, value, "{name}");
        }
        for (name, value) in [
            ("word", InlineChanges::Word),
            ("word-alt", InlineChanges::WordAlt),
            ("char", InlineChanges::Char),
            ("none", InlineChanges::None),
        ] {
            let config = Config::parse(&format!("[review]\ninline_changes = \"{name}\"\n")).expect("parses");
            assert_eq!(config.review.inline_changes, value, "{name}");
        }
        for width in 1..=16 {
            let config = Config::parse(&format!("[review]\ntab_width = {width}\n")).expect("parses");
            assert_eq!(config.review.tab_width.get(), width);
        }
    }

    #[test]
    fn a_bad_review_value_or_key_is_named() {
        for (text, named) in [
            ("[review]\nlayout = \"side-by-side\"\n", "side-by-side"),
            ("[review]\nsplit_ratio = 0.95\n", "0.95"),
            ("[review]\nsplit_ratio = 0.05\n", "0.05"),
            ("[review]\nbackground = \"black\"\n", "black"),
            ("[review]\ntab_size = 8\n", "tab_size"),
            ("[review]\ntab_width = 0\n", "tab_width 0"),
            ("[review]\ntab_width = 17\n", "tab_width 17"),
            ("[review]\ntab_width = -4\n", "tab_width -4"),
            ("[review]\ntab_width = \"8\"\n", "8"),
            ("[review]\nwhitespace = \"hide\"\n", "hide"),
            ("[review]\nwhitespace = true\n", "wanted string"),
            ("[review]\ninline_changes = \"words\"\n", "words"),
        ] {
            let err = Config::parse(text).expect_err("rejected");
            assert!(err.to_string().contains(named), "{text:?}: {err}");
        }
    }
}
