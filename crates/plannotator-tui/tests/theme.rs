//! `plannotator-tui config` reports the theme that would actually be used, through the
//! real binary: the environment overriding the file is the part a unit test cannot see.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::process::Command;

fn config_output(label: &str, theme_in_file: &str, theme_in_env: Option<&str>) -> String {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-theme-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("config.toml");
    std::fs::write(&path, format!("[ui]\ntheme = \"{theme_in_file}\"\n")).expect("config");
    let mut command = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"));
    command.env("PLANNOTATOR_TUI_CONFIG", &path).arg("config");
    match theme_in_env {
        Some(theme) => command.env("PLANNOTATOR_TUI_THEME", theme),
        None => command.env_remove("PLANNOTATOR_TUI_THEME"),
    };
    let out = command.output().expect("config runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    std::fs::remove_dir_all(&dir).expect("cleanup");
    assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
    text
}

#[test]
fn the_printed_theme_is_the_files_when_the_environment_is_silent() {
    assert!(config_output("file", "dark", None).contains("theme = \"dark\""));
}

#[test]
fn the_printed_theme_is_the_environments_when_it_speaks() {
    assert!(config_output("env", "dark", Some("light")).contains("theme = \"light\""));
}

#[test]
fn an_unreadable_theme_in_the_environment_is_reported_rather_than_ignored() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"));
    let out = command.env("PLANNOTATOR_TUI_THEME", "solarized").arg("config").output().expect("config runs");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("solarized"));
}
