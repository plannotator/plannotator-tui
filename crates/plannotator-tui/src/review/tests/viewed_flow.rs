//! Viewed marks on the screen: `x`, the header count, the browser's record on disk, the jump
//! to the next file not viewed, and a mark dropping once its file changes.

use plannotator_tui_review::progress;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn ctrl_n(screen: &mut Screen) {
    screen.handle_event(Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL))).expect("key");
}

#[test]
fn x_marks_the_file_viewed_in_the_browsers_record_and_an_edit_drops_it() {
    let fixture = Fixture::new("viewed");
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open();
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "Uncommitted").contains("0/2 viewed"), "{rows:#?}");

    at(&mut screen, 9);
    keys(&mut screen, "x");
    assert!(screen.progress.is_viewed("app.py"));
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "Uncommitted").contains("1/2 viewed"));
    assert!(
        rows.last().is_some_and(|r| r.contains("\u{2713} viewed")),
        "the footer ticks the file: {rows:#?}"
    );
    // The viewer's file list, shown when the diff has room, ticks the file after its counts;
    // the other file has none.
    let rows = frame(&mut screen, 160, 40);
    assert!(row_with(&rows, "*app.py").contains("\u{2713}  \u{2502}"), "{rows:#?}");
    assert!(!row_with(&rows, "notes.txt  ").contains('\u{2713}'), "{rows:#?}");

    // The record is the browser's: <data>/review-progress/<key>/<sha256(path)>.json.
    let dir = screen.progress.dir().expect("saved").to_path_buf();
    assert!(dir.starts_with(fixture.data().join("review-progress")));
    let record = read(&progress::record_path(&dir, "app.py"));
    let fingerprint = screen.progress.fingerprint("app.py").expect("fingerprint").to_owned();
    assert_eq!(
        record,
        format!(
            r#"{{"v":1,"path":"app.py","fingerprint":"{fingerprint}","viewed":true,"suppressed":false}}"#
        )
    );

    // ctrl+n: the next file not viewed.
    ctrl_n(&mut screen);
    assert_eq!(screen.file_under_cursor().expect("cursor").as_deref(), Some("notes.txt"));
    keys(&mut screen, "x");
    ctrl_n(&mut screen);
    assert_eq!(screen.status.as_deref(), Some("every file is viewed"));

    // A second open of the same review reads the marks back.
    drop(screen);
    let mut screen = fixture.open();
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "Uncommitted").contains("2/2 viewed"));

    // Editing app.py and reloading: its fingerprint changes, so its mark no longer counts.
    fixture.write("app.py", &format!("{AFTER}# more\n"));
    keys(&mut screen, "r");
    settle(&mut screen);
    assert!(!screen.progress.is_viewed("app.py"));
    assert!(screen.progress.is_viewed("notes.txt"));
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "Uncommitted").contains("1/2 viewed"));

    // x again un-marks: the record says so, and the browser reads it as not viewed.
    at(&mut screen, 9);
    keys(&mut screen, "xx");
    let record = read(&progress::record_path(&dir, "app.py"));
    assert!(record.contains(r#""viewed":false,"suppressed":true"#), "{record}");
}
