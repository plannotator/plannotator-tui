#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use super::super::review_test_support::file_app;
use super::*;
use crate::pi::link::Note;

fn note(app: &mut App, file: PathBuf, quote: Option<&str>, line: Option<usize>) -> Value {
    let (note, _answer) = Note::for_test(file, quote, line, "check this");
    app.take_pi_note(&note)
}

#[test]
fn a_pi_note_lands_on_the_quote_with_its_author() {
    let (root, mut app, _) = file_app("pi-note");
    let reply = note(&mut app, root.join("docs/a.md"), Some("two"), None);
    assert_eq!(reply, json!({"ok": true}));
    let placed = app.open.store.placed();
    let [one] = placed.as_slice() else { panic!("one note: {placed:?}") };
    assert_eq!(&app.open.doc.source[one.range.clone()], "two");
    assert_eq!(one.annotation.body, "check this");
    assert_eq!(one.annotation.author_name.as_deref(), Some("pi · test"));
    drop(placed);
    assert_eq!(app.status.as_deref(), Some("note from pi · test"));
    let feedback = app.feedback();
    assert!(feedback.contains("From: pi · test"), "{feedback}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_line_picks_the_nearest_occurrence_and_alone_picks_its_block() {
    let (root, mut app, _) = file_app("pi-line");
    let path = root.join("docs/a.md");
    std::fs::write(&path, "# Plan\n\nsame\n\nother\n\nsame\n").expect("rewrite");
    app.open = super::super::Open::new(
        super::super::read_file(&path).expect("file"),
        100,
        &app.data_dir,
        &app.project,
    )
    .expect("reopen");
    assert_eq!(note(&mut app, path.clone(), Some("same"), Some(7)), json!({"ok": true}));
    assert_eq!(note(&mut app, path.clone(), None, Some(5)), json!({"ok": true}));
    let source = app.open.doc.source.clone();
    let mut starts: Vec<(usize, String)> =
        app.open.store.placed().iter().map(|p| (p.range.start, source[p.range.clone()].to_owned())).collect();
    starts.sort();
    assert_eq!(starts, vec![(14, "other".to_owned()), (21, "same".to_owned())], "{source:?}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_pi_note_says_why_it_did_not_land() {
    let (root, mut app, _) = file_app("pi-refuse");
    let other = root.join("docs/b.md");
    std::fs::write(&other, "b\n").expect("b");
    assert_eq!(note(&mut app, other, Some("b"), None)["code"], "not_open");
    assert_eq!(note(&mut app, root.join("docs/a.md"), Some("nowhere"), None)["code"], "quote_not_found");
    app.mode = Mode::Compose;
    assert_eq!(note(&mut app, root.join("docs/a.md"), Some("two"), None)["code"], "busy");
    assert!(app.open.store.placed().is_empty());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn picking_a_pi_moves_send_and_the_last_row_moves_it_back() {
    let (root, mut app, _) = file_app("pi-pick");
    let before = app.delivery.describe();
    let entry = PiEntry {
        v: 1,
        host: "pi".into(),
        id: "0190abcd-0000".into(),
        pid: 1,
        cwd: root.clone(),
        name: Some("api".into()),
        transcript: None,
        port: 1,
        token: "t".into(),
        busy: false,
        updated: 0,
        path: PathBuf::new(),
    };
    app.pi_picker =
        PiPicker { sessions: vec![entry.clone(), PiEntry { name: Some("web".into()), ..entry }], cursor: 0 };
    app.choose_target(1);
    assert_eq!(app.delivery.describe(), "pi · web");
    app.choose_target(0);
    assert_eq!(app.delivery.describe(), "pi · api");
    assert_eq!(app.pi_rows(), 3, "two sessions and the way back");
    app.choose_target(2);
    assert_eq!(app.delivery.describe(), before);
    assert!(app.start_delivery.is_none());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn line_start_counts_from_one() {
    assert_eq!(line_start("a\nb\nc", 1), Some(0));
    assert_eq!(line_start("a\nb\nc", 3), Some(4));
    assert_eq!(line_start("a\nb\nc", 4), None);
}
