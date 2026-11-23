//! `[review]`: how the code review draws its diff. The review's settings menu (`,`) and its
//! header switches change these live and write the change back here, into the person's own
//! config file, so the next review opens the same way; the file can set them by hand too.
//!
//! Writing edits the file in place: only the `[review]` keys whose value changed are written,
//! and every other line, comment and table stays as it was.

use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

/// The `[review]` table of a config file, whatever else the file holds.
#[derive(Deserialize)]
struct ReviewOnly {
    #[serde(default)]
    review: ReviewConfig,
}

/// Write `review` into the `[review]` table of the config file at `path`, creating the file
/// and the table when they are missing. Only keys whose value differs from what the file
/// says now (its value, or the default when it has none) are written; everything else in the
/// file is kept byte for byte. A file that does not parse is left alone and is an error.
pub(crate) fn save(path: &Path, review: ReviewConfig) -> Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("not saving the review settings: {} does not parse", path.display()))?;
    let written = toml::from_str::<ReviewOnly>(&text)
        .map_err(|e| anyhow::anyhow!("{}", e.message().trim()))
        .with_context(|| format!("not saving the review settings: [review] in {}", path.display()))?
        .review;
    let now = toml::Table::try_from(review)?;
    let before = toml::Table::try_from(written)?;
    let changed: Vec<(&String, &toml::Value)> =
        now.iter().filter(|(key, value)| before.get(*key) != Some(value)).collect();
    if changed.is_empty() {
        return Ok(());
    }
    let table = doc.entry("review").or_insert_with(toml_edit::table);
    let table = table.as_table_like_mut().with_context(|| {
        format!("not saving the review settings: review in {} is not a table", path.display())
    })?;
    for (key, value) in changed {
        let mut value: toml_edit::Value = value.to_string().parse()?;
        // The old value's line keeps its comments, above it and after it.
        match table.get_mut(key).and_then(toml_edit::Item::as_value_mut) {
            Some(old) => {
                *value.decor_mut() = old.decor().clone();
                *old = value;
            }
            None => {
                table.insert(key, toml_edit::Item::Value(value));
            }
        }
    }
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("saving the review settings in {}", path.display()))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use crate::config::Config;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("plannotator-tui-review-config-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("plannotator-tui").join("config.toml")
    }

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

    #[test]
    fn saving_writes_only_changed_keys_and_keeps_the_rest_of_the_file() {
        let path = scratch("keep");
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        let original = "# my settings\n[ui]\ncopy_on_select = false # keep the clipboard\n\n[review]\n# how diffs open\nwrap = true # long lines\n";
        std::fs::write(&path, original).expect("write");
        let mut review = Config::parse(original).expect("parses").review;
        save(&path, review).expect("nothing changed");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            original,
            "an unchanged save writes nothing"
        );

        review.wrap = false;
        review.layout = Layout::Split;
        review.split_ratio = SplitShare::from_permille(375);
        save(&path, review).expect("saves");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(
            text.starts_with("# my settings\n[ui]\ncopy_on_select = false # keep the clipboard\n"),
            "{text}"
        );
        assert!(text.contains("# how diffs open\nwrap = false # long lines\n"), "{text}");
        assert!(text.contains("layout = \"split\"\n") && text.contains("split_ratio = 0.375\n"), "{text}");
        assert!(
            !text.contains("line_numbers") && !text.contains("background"),
            "defaults stay unwritten: {text}"
        );
        let reread = Config::parse(&text).expect("still parses");
        assert_eq!(reread.review, review);
        assert!(!reread.ui.copy_on_select);
        let _ = std::fs::remove_dir_all(path.parent().and_then(Path::parent).expect("dir"));
    }

    #[test]
    fn saving_creates_the_file_and_leaves_a_broken_one_alone() {
        let path = scratch("create");
        let review = ReviewConfig { layout: Layout::Unified, ..ReviewConfig::DEFAULT };
        save(&path, review).expect("creates");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "[review]\nlayout = \"unified\"\n");
        assert_eq!(Config::load_from(&path).expect("loads").review, review);

        let broken = "[review\nwrap = true\n";
        std::fs::write(&path, broken).expect("write");
        assert!(save(&path, ReviewConfig::DEFAULT).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), broken);
        let bad_value = "[review]\nsplit_ratio = 2.0\n";
        std::fs::write(&path, bad_value).expect("write");
        let err = save(&path, review).expect_err("a bad [review] is not guessed at");
        assert!(format!("{err:#}").contains("split_ratio"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), bad_value);
        let _ = std::fs::remove_dir_all(path.parent().and_then(Path::parent).expect("dir"));
    }

    #[test]
    fn a_dotted_or_inline_review_table_is_written_in_its_own_style() {
        let path = scratch("inline");
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        std::fs::write(&path, "review = { wrap = true }\n").expect("write");
        let review = ReviewConfig { wrap: true, line_numbers: false, ..ReviewConfig::DEFAULT };
        save(&path, review).expect("saves");
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(Config::parse(&text).expect("parses").review, review, "{text}");
        assert!(text.starts_with("review = {"), "{text}");
        let _ = std::fs::remove_dir_all(path.parent().and_then(Path::parent).expect("dir"));
    }
}
