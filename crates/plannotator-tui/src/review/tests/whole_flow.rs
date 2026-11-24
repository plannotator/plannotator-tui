//! The whole-file view (`o`, `O`): opening it at the cursor's line, its gutter, going back,
//! notes in and outside the diff, and the files it cannot show.

use std::time::Duration;

use plannotator_tui_review::{NoteScope, Side, Status};
use ratatui::crossterm::event::KeyCode;

use super::viewed_flow::ticked;
use super::*;
use crate::review::Mode;

/// Forty lines; the change replaces line 20 and adds two lines after line 30, so new lines
/// 1-16, 24-27 and 36-42 are outside every hunk.
fn long_before() -> String {
    (1..=40).map(|i| format!("x{i} = {i}")).collect::<Vec<_>>().join("\n") + "\n"
}

fn long_after() -> String {
    let mut lines: Vec<String> = (1..=40).map(|i| format!("x{i} = {i}")).collect();
    lines[19] = "x20 = 2000".into();
    lines.splice(30..30, ["y1 = 1".to_owned(), "y2 = 2".to_owned()]);
    lines.join("\n") + "\n"
}

/// The fixture with `long.py` committed and changed too.
fn fixture(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    fixture.write("long.py", &long_before());
    fixture.git(&["add", "long.py"]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "long"]);
    fixture.write("long.py", &long_after());
    fixture
}

/// Tick until the whole-file view has its file (or shows why it cannot).
pub(super) fn settle_whole(screen: &mut Screen) {
    let started = Instant::now();
    loop {
        screen.tick().expect("tick");
        let _ = frame(screen, 120, 40);
        let whole = screen.whole.as_ref().expect("the whole file is open");
        let loaded = match &whole.view {
            crate::review::whole::View::Shown { diff, .. } => {
                diff.files_shown() == 1 && diff.progress() != crate::review::diff::Progress::Loading
            }
            crate::review::whole::View::Message(_) => true,
        };
        if loaded {
            screen.tick().expect("tick");
            return;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the whole file never loaded");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn whole_line(screen: &Screen) -> Option<u32> {
    screen.whole.as_ref().and_then(|w| w.target().expect("target")).map(|(first, _)| first)
}

#[test]
fn o_shows_the_file_now_at_the_same_line_with_changed_lines_marked_and_o_goes_back() {
    let fixture = fixture("whole-now");
    let mut screen = fixture.open();
    screen.diff.reveal("long.py", Side::New, 20).expect("in the diff");
    keys(&mut screen, "o");
    settle_whole(&mut screen);
    assert_eq!(whole_line(&screen), Some(20));
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Whole file · new · long.py "), "{:?}", rows[0]);
    assert!(rows[1].contains(" Modified   +3 \u{2212}1 "), "{:?}", rows[1]);
    assert!(!rows[1].contains("+0"), "the file's counts in the diff, not the made-up patch's");
    assert!(
        diff_side(&screen, row_with(&rows, "x20 = 2000")).starts_with('\u{258e}'),
        "the cursor is on it: {rows:#?}"
    );
    keys(&mut screen, "k");
    let rows = frame(&mut screen, 120, 40);
    assert!(
        diff_side(&screen, row_with(&rows, "x20 = 2000")).starts_with('\u{258c}'),
        "a bar on the changed line: {rows:#?}"
    );
    keys(&mut screen, "j");
    assert!(
        !diff_side(&screen, row_with(&rows, "x19 = 19")).starts_with('\u{258c}'),
        "none on an unchanged one"
    );
    assert!(row_with(&rows, "back to the diff").contains("o/O/esc back to the diff"), "{rows:#?}");

    // Whole file: lines far from any hunk are there, with their numbers.
    keys(&mut screen, "g");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "x1 = 1").contains(" 1 "), "{rows:#?}");
    keys(&mut screen, "]");
    assert_eq!(whole_line(&screen), Some(20), "] goes to the changed lines");
    keys(&mut screen, "]");
    assert_eq!(whole_line(&screen), Some(31));

    keys(&mut screen, "x");
    assert_eq!(
        screen.status.as_deref(),
        Some("\u{2713} long.py viewed \u{b7} 1/2 viewed"),
        "x marks its file"
    );
    keys(&mut screen, "o");
    assert!(screen.whole.is_none());
    let back = screen.diff.cursor().expect("cursor").expect("a line");
    assert_eq!((back.path.as_str(), back.side, back.first), ("long.py", Side::New, 20), "where it was");
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Uncommitted"), "{:?}", rows[0]);
    // The diff's file list ticks the file marked in the whole-file view.
    assert!(ticked(&mut screen, 160, "long.py"), "the list ticks it");
    assert!(!ticked(&mut screen, 160, "app.py"));
}

#[test]
fn shifted_o_shows_the_base_at_the_matching_old_line_read_only() {
    let fixture = fixture("whole-base");
    let mut screen = fixture.open();
    screen.diff.reveal("long.py", Side::New, 34).expect("in the diff");
    keys(&mut screen, "O");
    settle_whole(&mut screen);
    assert_eq!(whole_line(&screen), Some(32), "new 34 was old 32");
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Whole file · base ("), "{:?}", rows[0]);
    assert!(rows[1].contains(" Modified   +3 \u{2212}1 "), "{:?}", rows[1]);
    assert!(!rows.iter().any(|r| r.contains("y1 = 1")), "the added lines are not in the base");
    assert!(rows[39].contains("base version: read-only for notes"), "{:?}", rows[39]);
    screen.whole.as_mut().expect("open").reveal_line(20).expect("line 20");
    keys(&mut screen, "j");
    let rows = frame(&mut screen, 120, 40);
    assert!(
        diff_side(&screen, row_with(&rows, "x20 = 20")).starts_with('\u{258c}'),
        "the removed line is marked: {rows:#?}"
    );
    keys(&mut screen, "c");
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(screen.status.as_deref(), Some("base version: read-only for notes"));
    press(&mut screen, KeyCode::Esc);
    assert!(screen.whole.is_none());
}

#[test]
fn notes_in_the_whole_file_go_in_the_diff_or_outside_it_and_enter_finds_them_again() {
    let fixture = fixture("whole-notes");
    let mut screen = fixture.open();
    screen.diff.reveal("long.py", Side::New, 20).expect("in the diff");
    let _ = frame(&mut screen, 120, 40);
    keys(&mut screen, "o");
    settle_whole(&mut screen);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "why 2000?");
    let in_diff =
        screen.store.annotations().iter().find(|n| n.text.as_deref() == Some("why 2000?")).expect("note");
    assert!(!in_diff.outside_diff, "inside a hunk it is an ordinary diff note");
    assert_eq!(in_diff.anchor_text.as_deref(), Some("x20 = 2000"));

    screen.whole.as_mut().expect("open").reveal_line(5).expect("line 5");
    keys(&mut screen, "c");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "comment on long.py:5 (not in the diff)").contains('\u{256d}'), "{rows:#?}");
    type_and_save(&mut screen, "why 5?");
    keys(&mut screen, "j");
    let outside =
        screen.store.annotations().iter().find(|n| n.text.as_deref() == Some("why 5?")).expect("note");
    assert!(outside.outside_diff);
    assert_eq!(
        (outside.side, outside.line_start, outside.anchor_text.as_deref()),
        (Side::New, 5, Some("x5 = 5"))
    );
    assert_eq!(outside.status(), Some(Status::NotInDiff));
    open_notes(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(
        diff_side(&screen, row_with(&rows, "x5 = 5")).starts_with('\u{258c}'),
        "marked in the whole file: {rows:#?}"
    );
    let heading = rows.iter().position(|r| r.contains(" Not in the diff")).expect("its heading");
    let bubble = rows.iter().position(|r| r.contains("long.py 5 new")).expect("its bubble");
    assert!(heading < bubble, "{rows:#?}");
    let feedback = screen.feedback(&screen.store.annotations().iter().collect::<Vec<_>>());
    assert!(
        feedback.contains("### Line 5 (new) (not in the diff)\nwhy 5?\n\n**Code:**\n```\nx5 = 5\n```\n"),
        "{feedback}"
    );

    // Back in the diff, the diff note is marked; Enter on the other opens the whole file.
    keys(&mut screen, "o");
    assert_eq!(screen.diff.cursor().expect("cursor").map(|l| l.first), Some(20), "where the diff was");
    keys(&mut screen, "j");
    let rows = frame(&mut screen, 120, 40);
    assert!(diff_side(&screen, row_with(&rows, "x20 = 2000")).starts_with('\u{258c}'), "{rows:#?}");
    press(&mut screen, KeyCode::Tab);
    while screen.selected_note().is_some_and(|n| !n.outside_diff) {
        keys(&mut screen, "j");
    }
    press(&mut screen, KeyCode::Enter);
    settle_whole(&mut screen);
    assert_eq!(whole_line(&screen), Some(5));

    // A reload keeps it not in the diff, and an edit above moves it with its text.
    fixture.write("long.py", &format!("# header\n{}", long_after()));
    keys(&mut screen, "r");
    settle(&mut screen);
    settle_whole(&mut screen);
    let outside = screen.store.annotations().iter().find(|n| n.outside_diff).expect("still there");
    assert_eq!((outside.line_start, outside.status(), outside.outdated), (6, Some(Status::NotInDiff), false));
}

#[test]
fn added_and_deleted_files_say_what_they_lack_and_n_moves_between_files() {
    let fixture = fixture("whole-missing");
    fixture.write("new.py", "print('new')\n");
    std::fs::remove_file(fixture.repo().join("notes.txt")).expect("delete");
    let mut screen = fixture.open();
    screen.diff.reveal("new.py", Side::New, 1).expect("in the diff");
    keys(&mut screen, "O");
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(
        diff_side(&screen, row_with(&rows, "new.py is added by this change: there is no base version."))
            .starts_with(' ')
    );
    keys(&mut screen, "O");
    keys(&mut screen, "o");
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(
        diff_side(&screen, &rows[1]).starts_with(" new.py ") && rows[1].contains(" Added   +1 \u{2212}0 "),
        "{:?}",
        rows[1]
    );
    keys(&mut screen, "o");
    screen.diff.reveal("notes.txt", Side::Old, 1).expect("in the diff");
    keys(&mut screen, "o");
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(
        rows.iter().any(|r| r.contains("notes.txt is deleted by this change: there is no current version."))
    );

    // n and N walk the changed files in the diff's order, same version.
    let order: Vec<String> = screen.capture.files.iter().map(|f| f.path.clone()).collect();
    let at = order.iter().position(|p| p == "notes.txt").expect("listed");
    assert_eq!(at + 1, order.len(), "the last file");
    keys(&mut screen, "n");
    assert_eq!(screen.status.as_deref(), Some("last file"));
    keys(&mut screen, "NN");
    settle_whole(&mut screen);
    assert_eq!(screen.whole.as_ref().map(|w| w.path.clone()), order.get(at - 2).cloned());
    keys(&mut screen, "n");
    settle_whole(&mut screen);
    assert_eq!(screen.whole.as_ref().map(|w| w.path.as_str()), Some("new.py"));
    // Back to the diff on the file the view moved to.
    keys(&mut screen, "o");
    let cursor = screen.diff.cursor().expect("cursor").expect("a line");
    assert_eq!(cursor.path, "new.py");
    assert_eq!(screen.store.annotations().iter().filter(|n| n.scope == NoteScope::Line).count(), 0);
}
