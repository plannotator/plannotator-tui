//! Host storage roots derived from the process environment.

use std::ffi::OsString;
use std::path::PathBuf;

use plannotator_tui_hosts::{hermes, opencode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Roots {
    pub(super) home: PathBuf,
    pub(super) claude_config: PathBuf,
    pub(super) codex_home: PathBuf,
    pub(super) copilot_home: PathBuf,
    pub(super) factory_config: PathBuf,
    pi_session_dir: Option<PathBuf>,
    pi_agent_dir: Option<PathBuf>,
    hermes_home: PathBuf,
    opencode_db: Option<PathBuf>,
    xdg_data_home: PathBuf,
}

impl Roots {
    pub(super) fn from_env() -> Self {
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self::from_lookup(home, |key| std::env::var_os(key))
    }

    fn from_lookup(home: PathBuf, env: impl Fn(&str) -> Option<OsString>) -> Self {
        let path = |key: &str| env(key).filter(|value| !value.is_empty()).map(PathBuf::from);
        let hermes_home = path("HERMES_HOME").unwrap_or_else(|| default_hermes_home(&home, &env));
        let xdg_data_home = path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"));
        Self {
            claude_config: path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            codex_home: path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            copilot_home: path("COPILOT_HOME").unwrap_or_else(|| home.join(".copilot")),
            factory_config: path("FACTORY_CONFIG_DIR").unwrap_or_else(|| home.join(".factory")),
            pi_session_dir: path("PI_CODING_AGENT_SESSION_DIR"),
            pi_agent_dir: path("PI_CODING_AGENT_DIR"),
            hermes_home,
            opencode_db: path("OPENCODE_DB"),
            xdg_data_home,
            home,
        }
    }

    pub(super) fn pi_sessions(&self, default_agent_dir: &str) -> PathBuf {
        self.pi_session_dir.clone().unwrap_or_else(|| {
            self.pi_agent_dir.clone().unwrap_or_else(|| self.home.join(default_agent_dir)).join("sessions")
        })
    }

    pub(super) fn hermes_database(&self) -> PathBuf {
        self.hermes_home.join(hermes::DB_FILE)
    }

    pub(super) fn opencode_databases(&self) -> Vec<PathBuf> {
        if let Some(db) = &self.opencode_db {
            return vec![db.clone()];
        }
        let data = self.xdg_data_home.join(opencode::DATA_DIR);
        let mut found: Vec<PathBuf> = std::fs::read_dir(&data)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                            name.starts_with("opencode") && name.to_ascii_lowercase().ends_with(".db")
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        if found.is_empty() {
            found.push(data.join(opencode::DB_FILE));
        }
        found
    }
}

#[cfg(windows)]
fn default_hermes_home(home: &std::path::Path, env: &impl Fn(&str) -> Option<OsString>) -> PathBuf {
    env("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map_or_else(|| home.join("AppData").join("Local"), PathBuf::from)
        .join("hermes")
}

#[cfg(not(windows))]
fn default_hermes_home(home: &std::path::Path, _env: &impl Fn(&str) -> Option<OsString>) -> PathBuf {
    home.join(".hermes")
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn every_documented_root_override_is_honoured() {
        let values: HashMap<&str, OsString> = [
            ("CLAUDE_CONFIG_DIR", "root/claude".into()),
            ("CODEX_HOME", "root/codex".into()),
            ("COPILOT_HOME", "root/copilot".into()),
            ("FACTORY_CONFIG_DIR", "root/factory".into()),
            ("PI_CODING_AGENT_SESSION_DIR", "root/pi sessions".into()),
            ("PI_CODING_AGENT_DIR", "root/pi agent ignored".into()),
            ("HERMES_HOME", "root/hermes".into()),
            ("OPENCODE_DB", "root/opencode/custom.db".into()),
            ("XDG_DATA_HOME", "root/xdg ignored".into()),
        ]
        .into_iter()
        .collect();
        let roots = Roots::from_lookup(PathBuf::from("home"), |key| values.get(key).cloned());
        assert_eq!(roots.claude_config, PathBuf::from("root/claude"));
        assert_eq!(roots.codex_home, PathBuf::from("root/codex"));
        assert_eq!(roots.copilot_home, PathBuf::from("root/copilot"));
        assert_eq!(roots.factory_config, PathBuf::from("root/factory"));
        assert_eq!(roots.pi_sessions(".pi/agent"), PathBuf::from("root/pi sessions"));
        assert_eq!(roots.hermes_database(), PathBuf::from("root/hermes").join(hermes::DB_FILE));
        assert_eq!(roots.opencode_databases(), vec![PathBuf::from("root/opencode/custom.db")]);
    }

    #[test]
    fn pi_agent_and_xdg_overrides_apply_when_the_more_specific_values_are_absent() {
        let values: HashMap<&str, OsString> =
            [("PI_CODING_AGENT_DIR", "root/pi agent ü".into()), ("XDG_DATA_HOME", "root/data ü".into())]
                .into_iter()
                .collect();
        let roots = Roots::from_lookup(PathBuf::from("home"), |key| values.get(key).cloned());
        assert_eq!(
            roots.pi_sessions(plannotator_tui_hosts::omp::DEFAULT_AGENT_DIR),
            PathBuf::from("root/pi agent ü/sessions")
        );
        assert_eq!(
            roots.opencode_databases(),
            vec![PathBuf::from("root/data ü").join(opencode::DATA_DIR).join(opencode::DB_FILE)]
        );
    }
}
