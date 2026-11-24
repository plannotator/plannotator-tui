//! The notes rail (`toggle-notes`, `[ui] notes_rail`): shown when the review opens with
//! notes, away when it opens without, and a saved note never opens it (the diff would get
//! narrower under the reader). Once closed it stays closed. Hidden, the diff takes its
//! columns and the notes keep their gutter marks and their count.

use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::notes_rail::{NotesRail, RailStart};
use crate::review::Focus;

/// The rail's width at `cols` columns, as the review sizes it.
fn rail(cols: u16) -> u16 {
    (cols * 3 / 10).clamp(28, 36)
}

/// The fixture's review with a comment on lines 9-10 and one on line 14, and the rail as a
/// review that opens with notes has it: shown.
fn noted(fixture: &Fixture) -> Screen {
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "vjvc");
    type_and_save(&mut screen, "name the total");
    at(&mut screen, 14);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "read the name from argv");
    assert!(!screen.rail_shown(), "saving a note opens no rail");
    screen.start_notes_rail(RailStart::WhenNotes);
    assert!(screen.rail_shown(), "a review that opens with notes shows them");
    keys(&mut screen, "gg");
    screen
}

#[test]
fn hiding_the_rail_gives_the_diff_its_columns_and_keeps_marks_and_counts() {
    let fixture = Fixture::new("rail-widths");
    let mut screen = noted(&fixture);
    for cols in [80, 120, 200] {
        screen.notes_rail = NotesRail::new(RailStart::WhenNotes);
        let open = frame(&mut screen, cols, 40);
        assert!(row_with(&open, "name the total").contains('\u{2502}'), "at {cols} the rail shows");
        assert_eq!(screen.geometry.diff.width, cols - 1 - rail(cols), "at {cols}");

        keys(&mut screen, "z");
        let hidden = frame(&mut screen, cols, 40);
        assert!(!hidden.iter().any(|r| r.contains("name the total")), "at {cols}:\n{}", hidden.join("\n"));
        assert_eq!(screen.geometry.diff.width, cols, "at {cols} the diff takes the rail's columns");
        assert_eq!(screen.geometry.rail.width, 0, "no rail to click");
        assert!(row_with(&hidden, "total = a + b").contains('\u{258c}'), "the gutter mark stays");
        assert!(row_with(&hidden, "Uncommitted").contains("Send 2 new"), "the header still counts");
        assert!(row_with(&hidden, "2 notes").contains("notes hidden: z shows them"));
    }
    keys(&mut screen, "z");
    let shown = frame(&mut screen, 120, 40);
    assert!(row_with(&shown, "name the total").contains('\u{2502}'), "the same key shows it again");
}

#[test]
fn the_diff_reflows_into_the_columns_a_hidden_rail_leaves() {
    let fixture = Fixture::new("rail-reflow");
    let mut screen = noted(&fixture);
    keys(&mut screen, "w");
    let long = format!("{AFTER}LONG = \"{}\"\n", "x".repeat(150));
    fixture.write("app.py", &long);
    keys(&mut screen, "r");
    settle(&mut screen);
    let rows_with_x = |rows: &[String]| rows.iter().filter(|r| r.contains("xxxxxxxx")).count();
    let open = rows_with_x(&frame(&mut screen, 200, 40));
    keys(&mut screen, "z");
    let hidden = rows_with_x(&frame(&mut screen, 200, 40));
    assert!(hidden < open, "the wrapped line takes fewer rows without the rail: {open} then {hidden}");
}

#[test]
fn tab_shows_hidden_notes_and_focuses_them_and_hiding_them_returns_to_the_diff() {
    let fixture = Fixture::new("rail-tab");
    let mut screen = noted(&fixture);
    keys(&mut screen, "z");
    press(&mut screen, KeyCode::Tab);
    assert_eq!(screen.focus, Focus::Notes);
    assert!(screen.rail_shown());
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "name the total").contains('\u{2502}'));
    assert!(row_with(&rows, "z hide").contains("e edit"), "the rail's hints name the key");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(screen.focus, Focus::Diff, "Enter on a note still jumps to its lines");

    press(&mut screen, KeyCode::Tab);
    keys(&mut screen, "z");
    assert_eq!(screen.focus, Focus::Diff);
    assert!(!screen.rail_shown());
}

#[test]
fn a_note_added_while_the_rail_is_hidden_is_marked_and_leaves_it_hidden() {
    let fixture = Fixture::new("rail-add");
    let mut screen = noted(&fixture);
    keys(&mut screen, "z");
    at(&mut screen, 5);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "add a comma test");
    assert_eq!(screen.store.annotations().len(), 3);
    keys(&mut screen, "gg");
    let rows = frame(&mut screen, 200, 40);
    assert!(!rows.iter().any(|r| r.contains("add a comma test")));
    assert!(row_with(&rows, "print(\"hello,\"").contains('\u{258c}'), "{}", rows.join("\n"));
    assert!(row_with(&rows, "Uncommitted").contains("Send 3 new"));
}

#[test]
fn a_hidden_rail_stays_hidden_across_a_reload() {
    let fixture = Fixture::new("rail-reload");
    let mut screen = noted(&fixture);
    keys(&mut screen, "z");
    keys(&mut screen, "r");
    settle(&mut screen);
    assert!(screen.notes_rail.closed());
    assert!(!frame(&mut screen, 120, 40).iter().any(|r| r.contains("name the total")));
}

#[test]
fn a_review_without_notes_has_no_rail_and_a_saved_note_does_not_open_it() {
    let fixture = Fixture::new("rail-first-note");
    let mut screen = fixture.open();
    let empty = frame(&mut screen, 120, 40);
    assert_eq!(screen.geometry.diff.width, 120, "no empty rail:\n{}", empty.join("\n"));
    assert!(!empty.iter().any(|r| r.contains("notes hidden") || r.contains("No notes")));
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "1 note").contains("note saved"), "the footer says so");
    keys(&mut screen, "gg");
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|r| r.contains("name the total")), "no rail for the first note");
    assert_eq!(screen.geometry.diff.width, 120, "the diff keeps its columns");
    assert!(row_with(&rows, "total = a + b").contains('\u{258c}'), "the gutter marks the note");
    assert!(row_with(&rows, "Uncommitted").contains("Send 1 new"), "the header counts it");
    assert!(row_with(&rows, "1 note").contains("notes hidden: z shows them"), "and where it is");
    assert!(!screen.notes_rail.closed(), "away, not closed: nobody closed it");
    keys(&mut screen, "z");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "name the total").contains('\u{2502}'), "z shows it");
}

#[test]
fn hidden_in_the_config_starts_closed_and_the_choice_is_remembered_for_the_next_review() {
    let fixture = Fixture::new("rail-config");
    let mut screen = noted(&fixture);
    let hidden = crate::config::Config::parse("[ui]\nnotes_rail = \"hidden\"\n").expect("config");
    screen.start_notes_rail(hidden.ui.notes_rail);
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|r| r.contains("name the total")));
    assert_eq!(screen.geometry.diff.width, 120);
    keys(&mut screen, "z");
    let mut next = fixture.open();
    next.start_notes_rail(hidden.ui.notes_rail);
    assert!(next.rail_shown(), "the person opened it: that beats the setting");
    keys(&mut next, "z");
    screen.start_notes_rail(RailStart::Shown);
    assert!(!screen.rail_shown(), "and closing it beats it too");
}

#[test]
fn the_help_names_the_key() {
    let fixture = Fixture::new("rail-help");
    let mut screen = fixture.open();
    keys(&mut screen, "?");
    let help = frame(&mut screen, 120, 40);
    assert!(row_with(&help, "hide the notes").contains('z'), "{}", help.join("\n"));
}

#[test]
fn the_whole_file_view_keeps_the_rail_as_the_person_left_it_and_z_works_there() {
    let fixture = Fixture::new("rail-whole");
    let mut screen = noted(&fixture);
    keys(&mut screen, "z");
    at(&mut screen, 9);
    keys(&mut screen, "o");
    whole_flow::settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|r| r.contains("name the total")), "still closed in the view");
    assert_eq!(screen.geometry.diff.width, 120, "the whole file takes the rail's columns");
    assert!(row_with(&rows, "notes hidden: z shows them").contains("2 notes"));

    keys(&mut screen, "z");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "name the total").contains('\u{2502}'), "z opens it in the view");
    assert_eq!(screen.geometry.diff.width, 120 - 1 - rail(120));
    keys(&mut screen, "z");
    keys(&mut screen, "o");
    assert!(screen.whole.is_none(), "back in the diff");
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|r| r.contains("name the total")), "and still closed there");
    assert_eq!(screen.geometry.diff.width, 120);
}

#[test]
fn a_rail_left_open_shows_from_the_start_and_a_saved_note_changes_nothing() {
    for (remembered, shown) in [("open", true), ("closed", false)] {
        let fixture = Fixture::new(&format!("rail-left-{remembered}"));
        let state = fixture.data().join("clients").join("plannotator-tui");
        std::fs::create_dir_all(&state).expect("state dir");
        std::fs::write(state.join("ui-state.json"), format!(r#"{{"notesRail":"{remembered}"}}"#))
            .expect("state");
        let mut screen = fixture.open();
        screen.start_notes_rail(RailStart::WhenNotes);
        assert_eq!(screen.rail_shown(), shown, "left {remembered}, a review without notes");
        let width = |screen: &mut Screen| {
            frame(screen, 120, 40);
            screen.geometry.diff.width
        };
        let before = width(&mut screen);
        assert_eq!(before == 120, !shown, "left {remembered}: the rail's columns");
        at(&mut screen, 9);
        keys(&mut screen, "c");
        type_and_save(&mut screen, "name the total");
        assert_eq!(screen.rail_shown(), shown, "left {remembered}: a saved note changes nothing");
        assert_eq!(width(&mut screen), before, "left {remembered}: the diff keeps its width");
        let rows = frame(&mut screen, 120, 40);
        let footer = row_with(&rows, "1 note");
        assert_eq!(footer.contains("notes hidden"), !shown, "left {remembered}: {footer}");
        if shown {
            assert!(rows.iter().any(|r| r.contains("name the total")), "the open rail shows the note");
        }
    }
}
