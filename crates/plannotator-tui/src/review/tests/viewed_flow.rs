//! Viewed marks on the screen: `x`, the header count, the browser's record on disk, the jump
//! to the next file not viewed, a mark dropping once its file changes, and the ticks in the
//! viewer's file list.

use std::time::Duration;

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
    // The file list ticks the file after its counts, at its right edge; the other file has
    // none.
    let rows = frame(&mut screen, 160, 40);
    assert!(entry(&rows, "app.py").ends_with("\u{2713} "), "{rows:#?}");
    assert!(!entry(&rows, "notes.txt").contains('\u{2713}'), "{rows:#?}");

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

/// The file list's entry for `name`: the columns left of its divider.
pub(super) fn entry(rows: &[String], name: &str) -> String {
    rows.iter()
        .filter_map(|row| row.split('\u{2502}').next())
        .find(|left| left.contains(name))
        .unwrap_or_else(|| panic!("no file list entry for {name:?}:\n{}", rows.join("\n")))
        .to_owned()
}

/// Whether the file list, drawn at `cols` columns, ticks `name`.
pub(super) fn ticked(screen: &mut Screen, cols: u16, name: &str) -> bool {
    entry(&frame(screen, cols, 30), name).contains('\u{2713}')
}

#[test]
fn the_file_list_ticks_follow_the_marks_with_the_rail_open_or_hidden_and_across_a_reload() {
    let fixture = Fixture::new("viewed-ticks");
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open();
    screen.watch.every = Duration::ZERO;
    // A note, so the rail shows beside the diff.
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    assert!(row_with(&frame(&mut screen, 160, 30), "name the total").contains('\u{2502}'), "the rail");
    assert!(!ticked(&mut screen, 160, "app.py") && !ticked(&mut screen, 160, "notes.txt"));

    at(&mut screen, 9);
    keys(&mut screen, "x");
    assert!(ticked(&mut screen, 160, "app.py") && !ticked(&mut screen, 160, "notes.txt"));
    // ctrl+n only moves; x there ticks the second file too.
    ctrl_n(&mut screen);
    assert!(!ticked(&mut screen, 160, "notes.txt"), "ctrl+n marks nothing");
    keys(&mut screen, "x");
    assert!(ticked(&mut screen, 160, "app.py") && ticked(&mut screen, 160, "notes.txt"));

    // The rail hidden: the diff takes its columns and goes side by side; the ticks stay.
    keys(&mut screen, "z");
    at(&mut screen, 9);
    let rows = frame(&mut screen, 160, 30);
    assert!(!rows.iter().any(|r| r.contains("name the total")), "{rows:#?}");
    assert!(row_with(&rows, "total = a + b").matches('\u{2502}').count() >= 2, "split: {rows:#?}");
    assert!(entry(&rows, "app.py").contains('\u{2713}') && entry(&rows, "notes.txt").contains('\u{2713}'));
    keys(&mut screen, "z");
    assert!(ticked(&mut screen, 160, "app.py") && ticked(&mut screen, 160, "notes.txt"), "shown again");

    // An edit on disk: the footer says so, and the ticks stay until a reload. The edit
    // waits for the watch's baseline, which a second check starting proves taken.
    screen.last_key = None;
    let started = Instant::now();
    while screen.watch.started < 2 {
        screen.tick().expect("tick");
        assert!(started.elapsed() < Duration::from_secs(10), "the watch never took its baseline");
        std::thread::sleep(Duration::from_millis(10));
    }
    fixture.write("app.py", &format!("{AFTER}# more\n"));
    let started = Instant::now();
    while !screen.watch.changed() {
        screen.tick().expect("tick");
        assert!(started.elapsed() < Duration::from_secs(10), "the disk change was never noticed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(row_with(&frame(&mut screen, 160, 30), "changed on disk").contains("r refreshes"));
    assert!(ticked(&mut screen, 160, "app.py"), "nothing reloads by itself");

    // r: app.py changed since it was viewed, so its tick goes; notes.txt keeps its own.
    keys(&mut screen, "r");
    settle(&mut screen);
    assert!(!ticked(&mut screen, 160, "app.py") && ticked(&mut screen, 160, "notes.txt"));
    // x unticks too.
    ctrl_n(&mut screen);
    assert_eq!(screen.file_under_cursor().expect("cursor").as_deref(), Some("app.py"));
    keys(&mut screen, "x");
    assert!(ticked(&mut screen, 160, "app.py"));
    at(&mut screen, 9);
    keys(&mut screen, "x");
    assert!(!ticked(&mut screen, 160, "app.py"));
}
