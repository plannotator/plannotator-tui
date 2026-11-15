//! Notes a Pi session leaves on the diff: new-side lines the diff shows, credited.

use serde_json::json;

use super::*;
use crate::pi::link::Note;

fn pi(screen: &mut Screen, file: &Path, quote: Option<&str>, line: Option<usize>) -> serde_json::Value {
    let (note, _answer) = Note::for_test(file.to_path_buf(), quote, line, "check this");
    screen.take_pi_note(&note)
}

#[test]
fn a_pi_note_on_a_shown_new_line_is_a_credited_line_comment() {
    let fixture = Fixture::new("pi-line");
    let mut screen = fixture.open();
    let file = fixture.repo().join("app.py");
    assert_eq!(pi(&mut screen, &file, None, Some(14)), json!({"ok": true}));
    // A quote picks its line, nearest the given one.
    assert_eq!(pi(&mut screen, &file, Some("return total"), Some(1)), json!({"ok": true}));
    let placed: Vec<_> = screen.store.annotations().iter().map(|n| (n.line_start, n.side)).collect();
    assert_eq!(placed, [(14, Side::New), (10, Side::New)]);
    let note = &screen.store.annotations()[0];
    assert_eq!((note.author.as_deref(), note.author_name.as_deref()), (Some("pi"), Some("pi \u{b7} test")));
    assert_eq!(screen.status.as_deref(), Some("note from pi \u{b7} test"));
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "14 new").contains("pi \u{b7} test"), "the rail credits Pi");
    let feedback = screen.feedback(&screen.store.annotations().iter().collect::<Vec<_>>());
    assert!(feedback.contains("### Line 14 (new)\nFrom: pi \u{b7} test\ncheck this"), "{feedback}");
}

#[test]
fn a_pi_note_with_no_line_is_a_file_comment() {
    let fixture = Fixture::new("pi-file");
    let mut screen = fixture.open();
    assert_eq!(pi(&mut screen, &fixture.repo().join("app.py"), None, None), json!({"ok": true}));
    assert_eq!(screen.store.annotations()[0].scope, plannotator_tui_review::NoteScope::File);
}

#[test]
fn a_pi_note_off_the_diff_says_why() {
    let fixture = Fixture::new("pi-refused");
    let mut screen = fixture.open();
    let file = fixture.repo().join("app.py");
    let code = |reply: serde_json::Value| reply["code"].as_str().map(str::to_owned);
    assert_eq!(code(pi(&mut screen, &file, None, Some(200))).as_deref(), Some("not_in_diff"));
    assert_eq!(code(pi(&mut screen, &file, Some("no such text"), None)).as_deref(), Some("quote_not_found"));
    let unchanged = fixture.repo().join("notes.txt");
    assert_eq!(code(pi(&mut screen, &unchanged, None, Some(1))).as_deref(), Some("not_open"));
    at(&mut screen, 14);
    keys(&mut screen, "c");
    assert_eq!(code(pi(&mut screen, &file, None, Some(14))).as_deref(), Some("busy"));
    assert_eq!(screen.store.annotations().len(), 0);
}

#[test]
fn the_registry_lists_the_changed_files() {
    let fixture = Fixture::new("pi-files");
    let screen = fixture.open();
    let root = std::fs::canonicalize(fixture.repo()).expect("real root");
    assert_eq!(screen.pi_files(), [root.join("app.py")]);
}

#[test]
fn a_credit_too_long_for_the_bubble_title_moves_into_the_note_instead_of_being_cut() {
    let fixture = Fixture::new("pi-credit");
    let mut screen = fixture.open();
    let file = fixture.repo().join("app.py");
    assert_eq!(pi(&mut screen, &file, None, Some(14)), json!({"ok": true}));
    let id = screen.store.annotations()[0].id.clone();
    let long = "pi \u{b7} billing-service-refactor";
    screen
        .store
        .edit(&id, |note| {
            note.author_name = Some(long.to_owned());
            true
        })
        .expect("edit");
    let rows = frame(&mut screen, 80, 30);
    // The credit may wrap, but none of it is cut.
    assert!(rows.iter().any(|row| row.contains("From: pi \u{b7}")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("billing-service-refactor")), "{rows:#?}");
}
