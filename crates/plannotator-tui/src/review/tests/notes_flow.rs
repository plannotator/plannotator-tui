//! Making notes on the diff, and what becomes of them when the code changes.

use plannotator_tui_review::{NoteScope, NoteType, Side, Status};
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::review::{Draft, Mode};

#[test]
fn a_selection_becomes_a_line_comment_marked_in_the_gutter_and_listed_in_the_rail() {
    let fixture = Fixture::new("comment");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "vjv");
    let selected = screen.diff.selection().expect("selection");
    assert_eq!((selected.first, selected.last, selected.side), (9, 10, Side::New));
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    let [note] = screen.store.annotations() else { panic!("one note") };
    assert_eq!((note.line_start, note.line_end, note.side), (9, 10, Side::New));
    assert_eq!(note.anchor_text.as_deref(), Some("    total = a + b\n    return total"));
    assert_eq!(screen.diff.selection(), None, "the selection became the note");
    keys(&mut screen, "gg");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "total = a + b").starts_with('\u{258c}'), "marked: {rows:#?}");
    assert!(row_with(&rows, "return total").starts_with('\u{258c}'));
    assert!(row_with(&rows, "9-10 new").contains("\u{1f4ac}"));
    assert!(row_with(&rows, "name the total").contains('\u{2502}'), "the bubble holds the text");
    assert!(row_with(&rows, "Uncommitted").contains("Send 1 new \u{25b8} test agent (E)"));
}

#[test]
fn a_suggestion_starts_from_the_new_lines_and_an_old_side_one_is_refused() {
    let fixture = Fixture::new("suggest");
    let mut screen = fixture.open();
    at(&mut screen, 5);
    keys(&mut screen, "s");
    assert_eq!(screen.compose.value(), "    print(\"hello,\", name)");
    press(&mut screen, KeyCode::End);
    for _ in 0..15 {
        press(&mut screen, KeyCode::Backspace);
    }
    type_and_save(&mut screen, "f\"hello, {name}\")");
    let [note] = screen.store.annotations() else { panic!("one note") };
    assert_eq!(note.kind, NoteType::Suggestion);
    assert_eq!(note.original_code.as_deref(), Some("    print(\"hello,\", name)"));
    assert_eq!(note.suggested_code.as_deref(), Some("    print(f\"hello, {name}\")"));

    // The removed line before it is on the old side only.
    screen.diff.reveal("app.py", Side::Old, 2).expect("old line 2 is shown");
    keys(&mut screen, "s");
    assert_eq!(screen.mode, Mode::Browse, "no box for old lines");
    assert!(screen.status.as_deref().is_some_and(|s| s.contains("new side")), "{:?}", screen.status);
}

#[test]
fn a_file_comment_and_an_empty_note() {
    let fixture = Fixture::new("file-note");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(screen.store.annotations().len(), 0, "an empty comment saves nothing");
    keys(&mut screen, "C");
    type_and_save(&mut screen, "split this file");
    let [note] = screen.store.annotations() else { panic!("one note") };
    assert_eq!((note.scope, note.file_path.as_str()), (NoteScope::File, "app.py"));
}

#[test]
fn a_file_note_from_the_opening_heading_goes_on_that_file() {
    let fixture = Fixture::new("heading-note");
    let mut screen = fixture.open();
    keys(&mut screen, "C");
    assert!(
        matches!(screen.mode, Mode::Compose(Draft::File(ref path)) if path == "app.py"),
        "{:?}",
        screen.mode
    );
}

#[test]
fn reload_moves_a_note_whose_lines_shifted_and_keeps_it_sent() {
    let fixture = Fixture::new("moved");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    keys(&mut screen, "E");
    fixture.write("app.py", &format!("#!/usr/bin/env python3\n# greeter\n{AFTER}"));
    keys(&mut screen, "r");
    settle(&mut screen);
    let [note] = screen.store.annotations() else { panic!("one note") };
    assert_eq!((note.line_start, note.line_end), (11, 11));
    assert_eq!(note.status(), Some(Status::Moved));
    assert!(!screen.store.is_pending(note), "a move is not an edit");
    assert!(screen.status.as_deref().is_some_and(|s| s.contains("1 moved")), "{:?}", screen.status);
    keys(&mut screen, "gg");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "total = a + b").starts_with('\u{258c}'), "the marker moved too");
}

#[test]
fn reload_puts_a_note_whose_code_changed_under_outdated_without_a_marker() {
    let fixture = Fixture::new("outdated");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    fixture.write("app.py", &AFTER.replace("total = a + b", "result = a + b"));
    keys(&mut screen, "r");
    settle(&mut screen);
    let [note] = screen.store.annotations() else { panic!("one note") };
    assert!(note.outdated);
    let rows = frame(&mut screen, 120, 40);
    assert!(
        row_with(&rows, "Outdated").trim_start().starts_with("Outdated")
            || rows.iter().any(|r| r.contains(" Outdated"))
    );
    assert!(row_with(&rows, "app.py 9 new").contains("\u{1f4ac}"), "an outdated note names its file");
    assert!(!rows.iter().any(|r| r.starts_with('\u{258c}')), "no marker: {rows:#?}");
    let feedback = screen.feedback(&screen.store.annotations().iter().collect::<Vec<_>>());
    assert!(feedback.contains(plannotator_tui_review::OUTDATED_ANNOTATION_LABEL), "{feedback}");
}

#[test]
fn notes_are_saved_and_come_back_on_the_next_open() {
    let fixture = Fixture::new("reopen");
    let mut screen = fixture.open();
    at(&mut screen, 14);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "argv may be empty");
    drop(screen);
    let mut again = fixture.open();
    let [note] = again.store.annotations() else { panic!("the note came back") };
    assert_eq!(note.text.as_deref(), Some("argv may be empty"));
    let rows = frame(&mut again, 120, 40);
    assert!(row_with(&rows, "greet(sys.argv[1])").starts_with('\u{258c}'));
}

#[test]
fn the_rail_follows_notes_into_the_diff_and_edits_them() {
    let fixture = Fixture::new("rail");
    let mut screen = fixture.open();
    for (line, text) in [(14, "argv"), (5, "comma")] {
        at(&mut screen, line);
        keys(&mut screen, "c");
        type_and_save(&mut screen, text);
    }
    press(&mut screen, KeyCode::Tab);
    keys(&mut screen, "gg");
    keys(&mut screen, "k");
    let cursor = screen.diff.cursor().expect("cursor").expect("on a line");
    assert_eq!(cursor.first, 5, "the first note in rail order is line 5, and the diff went to it");
    keys(&mut screen, "j");
    assert_eq!(screen.diff.cursor().expect("cursor").expect("line").first, 14);
    keys(&mut screen, "e");
    assert_eq!(screen.compose.value(), "argv");
    screen.handle_event(Event::Paste(" can be empty".into())).expect("paste");
    press(&mut screen, KeyCode::Enter);
    let texts: Vec<_> = screen.store.annotations().iter().filter_map(|n| n.text.clone()).collect();
    assert_eq!(texts, ["argv can be empty", "comma"]);
    keys(&mut screen, "x");
    assert_eq!(screen.store.annotations().len(), 1);
}

#[test]
fn at_80_columns_the_header_footer_and_rail_all_fit() {
    let fixture = Fixture::new("narrow");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    let rows = frame(&mut screen, 80, 24);
    assert!(rows[0].starts_with(" Uncommitted \u{b7} 1 file +7 \u{2212}3"), "{:?}", rows[0]);
    assert!(rows[0].trim_end().ends_with("Send 1 new \u{25b8} test agent (E)"), "{:?}", rows[0]);
    assert!(row_with(&rows, "9 new").contains("\u{1f4ac}"), "the rail is there");
    assert!(rows[23].contains("repo \u{b7} 1 note "), "{:?}", rows[23]);
    keys(&mut screen, "?");
    let rows = frame(&mut screen, 80, 24);
    assert!(row_with(&rows, "code review keys").contains('\u{256d}'));
}
