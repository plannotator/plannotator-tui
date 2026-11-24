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
            }
        );
    }

    #[test]
    fn every_review_setting_is_read_and_roundtrips() {
        let text = "[review]\nlayout = \"unified\"\nwrap = true\nline_numbers = false\ntheme = \"light\"\nbackground = \"theme\"\nsplit_ratio = 0.62\n";
        let config = Config::parse(text).expect("parses");
        let review = config.review;
        assert_eq!(review.layout, Layout::Unified);
        assert!(review.wrap && !review.line_numbers);
        assert_eq!((review.theme, review.background), (DiffTheme::Light, Background::Theme));
        assert_eq!(review.split_ratio.permille(), 620);
        assert_eq!(review.split_ratio.to_string(), "62 : 38");
        let shown = config.to_toml().expect("toml");
        assert!(shown.contains("[review]\nlayout = \"unified\""), "{shown}");
        assert!(shown.contains("split_ratio = 0.62\n"), "{shown}");
        assert_eq!(Config::parse(&shown).expect("reparses"), config);
    }

    #[test]
    fn a_bad_review_value_or_key_is_named() {
        for (text, named) in [
            ("[review]\nlayout = \"side-by-side\"\n", "side-by-side"),
            ("[review]\nsplit_ratio = 0.95\n", "0.95"),
            ("[review]\nsplit_ratio = 0.05\n", "0.05"),
            ("[review]\nbackground = \"black\"\n", "black"),
            ("[review]\ntab_width = 8\n", "tab_width"),
        ] {
            let err = Config::parse(text).expect_err("rejected");
            assert!(err.to_string().contains(named), "{text:?}: {err}");
        }
    }
}
