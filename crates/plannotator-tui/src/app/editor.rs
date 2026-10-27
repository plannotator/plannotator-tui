//! `open-in-editor`: hand the reviewed file to the user's editor and reload it on return.
//!
//! The app only asks; the event loop owns the terminal, so it suspends the screen, runs the
//! editor and reports back through `editor_returned`. A file that did not change is left
//! alone, so a GUI editor that returns at once does not cost a reload: the disk check says
//! "File changed on disk" when it saves later, as it does for any other writer.

use std::process::ExitStatus;

use plannotator_tui_schema::Provenance;

use super::App;
use super::disk::DiskStamp;
use crate::editor::{self, EditorRequest};

impl App {
    /// Ask the event loop to open the document in the editor, or say why it cannot.
    pub(super) fn request_editor(&mut self) {
        let Provenance::File { path } = &self.open.source.provenance else {
            self.status = Some("not a file on disk; nothing to open in an editor".into());
            return;
        };
        let Some(editor) = self.editor.clone() else {
            self.status = Some("set $EDITOR (or $VISUAL) to open the file in your editor".into());
            return;
        };
        if editor::split(&editor).is_none() {
            self.status = Some(format!("cannot read the editor command {editor:?}: a quote is left open"));
            return;
        }
        self.editor_request = Some(EditorRequest { editor, path: path.clone(), line: self.current_line() });
    }

    /// Whether a keypress asked for the editor; the event loop stops reading input then.
    pub(crate) fn wants_editor(&self) -> bool {
        self.editor_request.is_some()
    }

    pub(crate) fn take_editor_request(&mut self) -> Option<EditorRequest> {
        self.editor_request.take()
    }

    /// The editor is done (or never started). Reload when the file changed, so notes
    /// re-anchor the way `reload` does; otherwise only say what happened.
    pub(crate) fn editor_returned(&mut self, request: &EditorRequest, outcome: std::io::Result<ExitStatus>) {
        let name = editor::split(&request.editor)
            .and_then(|words| words.first().map(|program| editor::program_name(program)))
            .unwrap_or_else(|| request.editor.clone());
        let exit = match outcome {
            Err(error) => {
                self.status = Some(format!("could not start {name}: {error}"));
                return;
            }
            Ok(status) if status.success() => None,
            Ok(status) => Some(match status.code() {
                Some(code) => format!("{name} exited with {code}"),
                None => format!("{name} was stopped"),
            }),
        };
        let still_open =
            matches!(&self.open.source.provenance, Provenance::File { path } if *path == request.path);
        let changed = still_open && DiskStamp::of(&request.path) != self.open.disk;
        let mut status = if !changed {
            format!("no changes from {name}")
        } else if DiskStamp::of(&request.path).is_none() {
            format!("{} is gone after {name}; nothing reloaded", request.path.display())
        } else {
            match self.reload() {
                Ok(()) => self.status.take().unwrap_or_else(|| "reloaded".into()),
                Err(error) => format!("reloading after {name}: {error:#}"),
            }
        };
        if let Some(exit) = exit {
            status = format!("{status} · {exit}");
        }
        self.status = Some(status);
    }

    /// The 1-based source line the reviewer is on: the start of a pending selection, the
    /// roaming cursor's line, or the selected block's first line.
    fn current_line(&self) -> usize {
        let roaming =
            || self.open.layout.row(self.cursor.0).and_then(|row| row.cells.iter().find_map(|cell| *cell));
        let block = || self.open.doc.blocks.get(self.selected).map(|b| b.range.start);
        let offset = match &self.pending {
            Some(pending) => Some(pending.range.start),
            None if self.roam => roaming().or_else(block),
            None => block(),
        };
        let before = offset.and_then(|offset| self.open.doc.source.get(..offset)).unwrap_or("");
        before.matches('\n').count() + 1
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests assert by panicking")]

    use std::path::Path;
    use std::time::Duration;

    use crate::app::review_test_support::{file_app, press, reply_app};

    #[cfg(unix)]
    fn exited(code: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt as _;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exited(code: i32) -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt as _;
        std::process::ExitStatus::from_raw(code.cast_unsigned())
    }

    /// What an editor would do: new content, and a later mtime so the change is visible
    /// even on a filesystem with coarse timestamps.
    fn save(root: &Path, content: &str) {
        let path = root.join("docs/a.md");
        std::fs::write(&path, content).expect("rewrite");
        let loaded = std::fs::metadata(&path).and_then(|meta| meta.modified()).expect("mtime");
        let later = loaded + Duration::from_secs(5);
        std::fs::File::options().write(true).open(&path).expect("open").set_modified(later).expect("mtime");
    }

    #[test]
    fn without_an_editor_the_key_says_to_set_one() {
        let (root, mut app, _) = file_app("editor-unset");
        press(&mut app, 'o');
        assert!(!app.wants_editor());
        assert_eq!(app.status.as_deref(), Some("set $EDITOR (or $VISUAL) to open the file in your editor"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_reply_review_has_no_file_to_open() {
        let (root, mut app, _) = reply_app("editor-reply");
        app.editor = Some("vim".into());
        press(&mut app, 'o');
        assert!(!app.wants_editor());
        assert_eq!(app.status.as_deref(), Some("not a file on disk; nothing to open in an editor"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_request_names_the_file_and_the_selected_line() {
        let (root, mut app, _) = file_app("editor-line");
        app.editor = Some("nvim -u x".into());
        press(&mut app, 'j');
        press(&mut app, 'j');
        press(&mut app, 'o');
        let request = app.take_editor_request().expect("request");
        assert_eq!(request.path, root.join("docs/a.md"));
        assert_eq!(request.editor, "nvim -u x");
        // `# Plan`, blank, `one`, blank, `two`: the third block starts on line 5.
        assert_eq!(request.line, 5);
        assert!(!app.wants_editor());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_saved_file_reloads_and_reanchors_the_notes() {
        let (root, mut app, _) = file_app("editor-saved");
        app.editor = Some("vim".into());
        app.add_quote_annotation("two", plannotator_tui_schema::Kind::Comment, "check".into()).expect("note");
        press(&mut app, 'o');
        let request = app.take_editor_request().expect("request");
        save(&root, "# Plan\n\nzero\n\none\n\ntwo\n\nthree\n");
        app.editor_returned(&request, Ok(exited(0)));
        assert!(app.open.doc.source.contains("zero"));
        assert_eq!(app.open.store.orphans(), 0);
        assert_eq!(app.status.as_deref(), Some("reloaded · 0 orphaned"));
        // The disk check does not report the editor's own save afterwards.
        assert!(!app.check_disk());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unchanged_file_is_not_reloaded_and_a_later_save_is_still_noticed() {
        let (root, mut app, _) = file_app("editor-unchanged");
        app.editor = Some("code".into());
        press(&mut app, 'o');
        let request = app.take_editor_request().expect("request");
        app.editor_returned(&request, Ok(exited(0)));
        assert_eq!(app.status.as_deref(), Some("no changes from code"));
        // A GUI editor that returned at once saves later: the usual hint covers it.
        save(&root, "# Plan\n\nedited later\n");
        assert!(app.check_disk());
        assert_eq!(app.status.as_deref(), Some("File changed on disk. Press r to reload."));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_editor_says_so_and_still_reloads_what_it_saved() {
        let (root, mut app, _) = file_app("editor-failed");
        app.editor = Some("/usr/bin/vim -u NONE".into());
        press(&mut app, 'o');
        let request = app.take_editor_request().expect("request");
        save(&root, "# Plan\n\nsaved before quitting\n");
        app.editor_returned(&request, Ok(exited(1)));
        assert_eq!(app.status.as_deref(), Some("reloaded · 0 orphaned · vim exited with 1"));

        app.editor_returned(&request, Err(std::io::Error::from(std::io::ErrorKind::NotFound)));
        assert!(
            app.status.as_deref().is_some_and(|s| s.starts_with("could not start vim: ")),
            "{:?}",
            app.status
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
