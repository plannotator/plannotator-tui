//! User configuration: one small TOML file, strict about what it accepts.
//!
//! `$PLANNOTATOR_TUI_CONFIG` → `$XDG_CONFIG_HOME/plannotator-tui/config.toml` →
//! `~/.config/plannotator-tui/config.toml`. A missing file means defaults. An unknown key is an
//! error that names the key, so a typo never silently falls back to a default.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Config {
    pub(crate) herdr: HerdrConfig,
}

/// How plannotator-tui opens inside Herdr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct HerdrConfig {
    pub(crate) placement: Placement,
    /// Split only.
    pub(crate) split_direction: SplitDirection,
    /// Popup only; cells or a percentage like `90%`.
    pub(crate) popup_width: String,
    pub(crate) popup_height: String,
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            placement: Placement::Overlay,
            split_direction: SplitDirection::Right,
            popup_width: "90%".to_owned(),
            popup_height: "85%".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Placement {
    /// A real pane zoomed over the whole tab; Herdr restores focus and zoom on exit.
    #[default]
    Overlay,
    /// Beside the target pane.
    Split,
    /// A modal floating box.
    Popup,
}

impl Placement {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Split => "split",
            Self::Popup => "popup",
        }
    }
}

impl fmt::Display for Placement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Placement {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "overlay" => Ok(Self::Overlay),
            "split" => Ok(Self::Split),
            "popup" => Ok(Self::Popup),
            other => anyhow::bail!("unknown placement {other:?}; expected overlay, split or popup"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SplitDirection {
    #[default]
    Right,
    Down,
}

impl SplitDirection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

impl fmt::Display for SplitDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where the config file lives, given an environment lookup and the home directory.
pub(crate) fn config_path(env: impl Fn(&str) -> Option<String>, home: &Path) -> PathBuf {
    if let Some(explicit) = env("PLANNOTATOR_TUI_CONFIG").filter(|s| !s.is_empty()) {
        return PathBuf::from(explicit);
    }
    let xdg = env("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.unwrap_or_else(|| home.join(".config")).join("plannotator-tui").join("config.toml")
}

impl Config {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| anyhow::anyhow!("{}", e.message().trim()))
    }

    /// Read the config file for this process; a missing file is the default config.
    pub(crate) fn load() -> Result<Self> {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let path = config_path(|k| std::env::var(k).ok(), &home);
        Self::load_from(&path)
    }

    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("in {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The effective config as TOML, for `plannotator-tui config`.
    pub(crate) fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string(self)?)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_the_default_config() {
        assert_eq!(Config::parse("").expect("parses"), Config::default());
        assert_eq!(Config::default().herdr.placement, Placement::Overlay);
    }

    #[test]
    fn unknown_key_error_names_the_key() {
        let err = Config::parse("[herdr]\nplacment = \"popup\"\n").expect_err("rejected");
        assert!(err.to_string().contains("placment"), "{err}");
    }

    #[test]
    fn invalid_placement_error_names_the_value() {
        let err = Config::parse("[herdr]\nplacement = \"floating\"\n").expect_err("rejected");
        assert!(err.to_string().contains("floating"), "{err}");
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let config = Config::parse("[herdr]\nplacement = \"split\"\n").expect("parses");
        assert_eq!(config.herdr.placement, Placement::Split);
        assert_eq!(config.herdr.split_direction, SplitDirection::Right);
        assert_eq!(config.herdr.popup_width, "90%");
    }

    #[test]
    fn config_path_precedence() {
        let home = Path::new("/home/u");
        let lookup = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(name, _)| *name == k).map(|(_, v)| (*v).to_owned())
        };
        assert_eq!(
            config_path(
                lookup(&[("PLANNOTATOR_TUI_CONFIG", "/etc/p.toml"), ("XDG_CONFIG_HOME", "/x")]),
                home
            ),
            PathBuf::from("/etc/p.toml")
        );
        assert_eq!(
            config_path(lookup(&[("XDG_CONFIG_HOME", "/x")]), home),
            PathBuf::from("/x/plannotator-tui/config.toml")
        );
        // A relative XDG_CONFIG_HOME is ignored, as the spec requires.
        assert_eq!(
            config_path(lookup(&[("XDG_CONFIG_HOME", "rel")]), home),
            PathBuf::from("/home/u/.config/plannotator-tui/config.toml")
        );
        assert_eq!(
            config_path(lookup(&[]), home),
            PathBuf::from("/home/u/.config/plannotator-tui/config.toml")
        );
    }

    #[test]
    fn roundtrips_through_toml() {
        let text = Config::default().to_toml().expect("serializes");
        assert_eq!(Config::parse(&text).expect("parses"), Config::default());
    }
}
