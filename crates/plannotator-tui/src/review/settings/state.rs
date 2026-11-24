//! What the person last chose for the diff in the review, remembered for the next one:
//! `<data>/clients/plannotator-tui/review-settings.toml`, beside the app's other state. It
//! holds the same keys as `[review]` in the config file and wins over them; a key it lacks
//! falls back to the config file, then to the defaults.
//!
//! The file is the app's own and is read leniently: a key it does not know or a value out of
//! range is skipped (the good keys still count), and a file that does not parse is ignored;
//! either way the review says so once and opens. It is written leniently too: only the keys
//! changed since this review last wrote, onto the file as it is now, so two reviews open at
//! once, or a hand edit, keep each other's choices; through a temporary file in the same
//! folder, synced and renamed into place, so a reader never sees half of it.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::config::review::{Background, DiffTheme, Layout, ReviewConfig, SplitShare};

/// The state file in the data folder `data_dir`.
pub(crate) fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("clients").join("plannotator-tui").join("review-settings.toml")
}

/// The keys the file holds, as `[review]` names them.
const KEYS: [&str; 6] = ["layout", "wrap", "line_numbers", "theme", "background", "split_ratio"];

/// `settings` with the file's good keys over it, and what was wrong with the file, if
/// anything: one line for the footer.
pub(in crate::review) fn load(path: &Path, settings: ReviewConfig) -> (ReviewConfig, Option<String>) {
    let name = file_name(path);
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (settings, None),
        Err(e) => return (settings, Some(format!("could not read the remembered settings ({name}): {e}"))),
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        return (settings, Some(format!("ignored the remembered settings: {name} does not parse")));
    };
    let mut out = settings;
    let mut skipped: Vec<&str> = Vec::new();
    for (key, value) in &table {
        let value = value.clone();
        let taken = match key.as_str() {
            "layout" => value.try_into::<Layout>().map(|v| out.layout = v).is_ok(),
            "wrap" => value.try_into::<bool>().map(|v| out.wrap = v).is_ok(),
            "line_numbers" => value.try_into::<bool>().map(|v| out.line_numbers = v).is_ok(),
            "theme" => value.try_into::<DiffTheme>().map(|v| out.theme = v).is_ok(),
            "background" => value.try_into::<Background>().map(|v| out.background = v).is_ok(),
            "split_ratio" => value.try_into::<SplitShare>().map(|v| out.split_ratio = v).is_ok(),
            _ => false,
        };
        if !taken {
            skipped.push(key);
        }
    }
    let notice = (!skipped.is_empty())
        .then(|| format!("ignored {} in the remembered settings ({name})", skipped.join(", ")));
    (out, notice)
}

/// The keys of `now` that differ from `before`, as the file writes them. A split ratio is
/// written to two decimals: a drag lands anywhere, and `0.62` reads better than `0.617`.
pub(super) fn changes(before: ReviewConfig, now: ReviewConfig) -> Vec<(&'static str, toml::Value)> {
    let text = |value: &str| toml::Value::String(value.to_owned());
    let layout = |layout| match layout {
        Layout::Auto => "auto",
        Layout::Split => "split",
        Layout::Unified => "unified",
    };
    let theme = |theme| match theme {
        DiffTheme::Auto => "auto",
        DiffTheme::Dark => "dark",
        DiffTheme::Light => "light",
    };
    let background = |background| match background {
        Background::Terminal => "terminal",
        Background::Theme => "theme",
    };
    let mut out = Vec::new();
    if before.layout != now.layout {
        out.push(("layout", text(layout(now.layout))));
    }
    if before.wrap != now.wrap {
        out.push(("wrap", toml::Value::Boolean(now.wrap)));
    }
    if before.line_numbers != now.line_numbers {
        out.push(("line_numbers", toml::Value::Boolean(now.line_numbers)));
    }
    if before.theme != now.theme {
        out.push(("theme", text(theme(now.theme))));
    }
    if before.background != now.background {
        out.push(("background", text(background(now.background))));
    }
    if before.split_ratio != now.split_ratio {
        let hundredths = (now.split_ratio.permille() + 5) / 10;
        out.push(("split_ratio", toml::Value::Float(f64::from(hundredths) / 100.0)));
    }
    out
}

/// Write `changes` onto the file as it is now: its other keys, good or not, stay. A file
/// that does not parse is replaced: it was ignored anyway.
pub(super) fn write(path: &Path, changes: &[(&str, toml::Value)]) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut table = read_table(path)?;
    for (key, value) in changes {
        table.insert((*key).to_owned(), value.clone());
    }
    replace(path, &toml::to_string(&table)?)
}

/// Drop every setting from the file, and the file when nothing else is left in it.
pub(super) fn forget(path: &Path) -> Result<()> {
    let mut table = read_table(path)?;
    for key in KEYS {
        table.remove(key);
    }
    if table.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("forgetting the remembered settings in {}", path.display()))
            }
            _ => Ok(()),
        };
    }
    replace(path, &toml::to_string(&table)?)
}

/// The file's table: empty when it is missing or does not parse.
fn read_table(path: &Path) -> Result<toml::Table> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text.parse::<toml::Table>().unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Put `text` at `path` all at once: written to a temporary file beside it (one per process,
/// so two reviews never share one), synced, then renamed over it. The folder is created.
fn replace(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("the settings file has no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(".{}.{}.tmp", file_name(path), std::process::id()));
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.with_context(|| format!("remembering the review settings in {}", path.display()))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("plannotator-tui-review-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        path(&dir)
    }

    fn cleanup(file: &Path) {
        let root = file.ancestors().nth(3).expect("the scratch folder");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_file_changes_nothing_and_a_written_one_is_read_back() {
        let file = scratch("roundtrip");
        let config = ReviewConfig { wrap: true, ..ReviewConfig::DEFAULT };
        assert_eq!(load(&file, config), (config, None));
        let now = ReviewConfig {
            layout: Layout::Unified,
            line_numbers: false,
            theme: DiffTheme::Light,
            background: Background::Theme,
            split_ratio: SplitShare::from_permille(617),
            ..config
        };
        write(&file, &changes(config, now)).expect("written");
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.contains("split_ratio = 0.62"), "two decimals: {text}");
        assert!(!text.contains("wrap"), "an unchanged key is not written: {text}");
        let (read, notice) = load(&file, config);
        assert_eq!(notice, None);
        assert_eq!(read, ReviewConfig { split_ratio: SplitShare::from_permille(620), ..now });
        // Nothing changed: nothing written, not even a missing file.
        let other = scratch("nothing");
        write(&other, &changes(now, now)).expect("nothing to write");
        assert!(!other.exists());
        cleanup(&file);
        cleanup(&other);
    }

    #[test]
    fn bad_keys_are_skipped_and_named_once_and_the_good_ones_count() {
        let file = scratch("lenient");
        std::fs::create_dir_all(file.parent().expect("dir")).expect("dir");
        std::fs::write(
            &file,
            "layout = \"diagonal\"\nwrap = true\nsplit_ratio = 5.0\ntab_width = 8\ntheme = \"dark\"\n",
        )
        .expect("write");
        let (read, notice) = load(&file, ReviewConfig::DEFAULT);
        assert_eq!(read, ReviewConfig { wrap: true, theme: DiffTheme::Dark, ..ReviewConfig::DEFAULT });
        let notice = notice.expect("a notice");
        for key in ["layout", "split_ratio", "tab_width"] {
            assert!(notice.contains(key), "{notice}");
        }
        assert!(!notice.contains("wrap"), "{notice}");

        std::fs::write(&file, "wrap = [true\n").expect("write");
        let (read, notice) = load(&file, ReviewConfig::DEFAULT);
        assert_eq!(read, ReviewConfig::DEFAULT);
        assert!(notice.expect("a notice").contains("does not parse"));
        // Writing over a broken file starts it again.
        write(&file, &[("wrap", toml::Value::Boolean(true))]).expect("written");
        assert!(load(&file, ReviewConfig::DEFAULT).0.wrap);
        cleanup(&file);
    }

    #[test]
    fn a_write_keeps_the_keys_it_did_not_change_and_forget_drops_only_settings() {
        let file = scratch("merge");
        std::fs::create_dir_all(file.parent().expect("dir")).expect("dir");
        std::fs::write(&file, "layout = \"split\"\nfuture_key = 1\n").expect("write");
        write(&file, &[("wrap", toml::Value::Boolean(true))]).expect("written");
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.contains("layout = \"split\"") && text.contains("future_key = 1"), "{text}");
        assert!(text.contains("wrap = true"), "{text}");
        forget(&file).expect("forgotten");
        assert_eq!(std::fs::read_to_string(&file).expect("read").trim(), "future_key = 1");
        std::fs::write(&file, "wrap = true\n").expect("write");
        forget(&file).expect("forgotten");
        assert!(!file.exists(), "nothing left: the file goes");
        forget(&file).expect("forgetting nothing is fine");
        let dir = file.parent().expect("dir");
        let leftovers: Vec<_> =
            std::fs::read_dir(dir).expect("dir").flatten().map(|e| e.file_name()).collect();
        assert!(leftovers.is_empty(), "no temporary file left: {leftovers:?}");
        cleanup(&file);
    }

    #[test]
    fn a_failed_write_is_an_error_and_leaves_no_temporary_file() {
        let file = scratch("fail");
        // A folder where the file should be: the rename cannot replace it.
        std::fs::create_dir_all(&file).expect("dir");
        let err = write(&file, &[("wrap", toml::Value::Boolean(true))]).expect_err("fails");
        assert!(format!("{err:#}").contains("review-settings.toml"), "{err:#}");
        let dir = file.parent().expect("dir");
        let names: Vec<_> = std::fs::read_dir(dir).expect("dir").flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        cleanup(&file);
    }
}
