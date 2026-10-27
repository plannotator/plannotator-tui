//! Noticing that the reviewed file changed on disk, typically an agent editing it, so the
//! footer can say a reload is due. Nothing reloads by itself: the reviewer decides when.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use plannotator_tui_schema::Provenance;

use super::App;
use crate::keys::Action;

/// How often the event loop may stat the open file.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// What a file looked like on disk: enough to tell that it was rewritten without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DiskStamp {
    modified: SystemTime,
    len: u64,
}

impl DiskStamp {
    /// The file's stamp now, or `None` when it is missing or cannot be read.
    pub(super) fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self { modified: meta.modified().ok()?, len: meta.len() })
    }

    /// The stamp of a document's file, when the document is one.
    pub(super) fn of_source(provenance: &Provenance) -> Option<Self> {
        match provenance {
            Provenance::File { path } => Self::of(path),
            _ => None,
        }
    }
}

impl App {
    /// Called from the event loop on every wake; checks at most once per `CHECK_EVERY`.
    /// Returns whether the screen needs a redraw.
    pub(crate) fn poll_disk(&mut self) -> bool {
        let now = Instant::now();
        if self.disk_checked.is_some_and(|last| now.duration_since(last) < CHECK_EVERY) {
            return false;
        }
        self.disk_checked = Some(now);
        self.check_disk()
    }

    /// Compare the open file with the stamp it had when it was loaded, and say so once per
    /// change. A missing or unreadable file says nothing.
    pub(super) fn check_disk(&mut self) -> bool {
        let Some(loaded) = self.open.disk else { return false };
        let Some(now) = DiskStamp::of_source(&self.open.source.provenance) else { return false };
        if now == loaded || self.open.disk_noticed == Some(now) {
            return false;
        }
        self.open.disk_noticed = Some(now);
        self.status = Some(match self.keys.hint(Action::Reload) {
            key if key.is_empty() => "File changed on disk.".to_owned(),
            key => format!("File changed on disk. Press {key} to reload."),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests assert by panicking")]

    use std::path::Path;
    use std::time::Duration;

    use crate::app::review_test_support::{file_app, press};

    const HINT: &str = "File changed on disk. Press r to reload.";

    /// Rewrite `file_app`'s document with different content and a later mtime.
    fn edit_on_disk(root: &Path, app: &crate::app::App) {
        let path = root.join("docs/a.md");
        std::fs::write(&path, "# Plan\n\none\n\ntwo\n\nthree\n\nfour, from the agent\n").expect("rewrite");
        let later = app.open.disk.expect("stamp").modified + Duration::from_secs(5);
        std::fs::File::options().write(true).open(&path).expect("open").set_modified(later).expect("mtime");
    }

    #[test]
    fn an_unchanged_file_says_nothing() {
        let (root, mut app, _) = file_app("disk-unchanged");
        assert!(!app.check_disk());
        assert!(!app.poll_disk());
        assert_eq!(app.status, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_change_on_disk_asks_for_a_reload_until_reloaded() {
        let (root, mut app, _) = file_app("disk-changed");
        edit_on_disk(&root, &app);
        assert!(app.poll_disk());
        assert_eq!(app.status.as_deref(), Some(HINT));
        // Said once per change, and a cleared status is not put back for the same change.
        app.status = None;
        assert!(!app.check_disk());
        assert_eq!(app.status, None);

        press(&mut app, 'r');
        let status = app.status.clone().expect("reload status");
        assert!(status.starts_with("reloaded"), "{status}");
        assert!(!app.check_disk());
        assert!(app.open.doc.source.contains("four, from the agent"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_hint_names_the_bound_reload_key() {
        let (root, mut app, _) = file_app("disk-key");
        app.keys = toml::from_str("reload = [\"ctrl+l\"]\n").expect("keys");
        edit_on_disk(&root, &app);
        assert!(app.check_disk());
        assert_eq!(app.status.as_deref(), Some("File changed on disk. Press ctrl+l to reload."));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_removed_file_says_nothing() {
        let (root, mut app, _) = file_app("disk-removed");
        std::fs::remove_file(root.join("docs/a.md")).expect("remove");
        assert!(!app.check_disk());
        assert_eq!(app.status, None);
        let _ = std::fs::remove_dir_all(root);
    }
}
