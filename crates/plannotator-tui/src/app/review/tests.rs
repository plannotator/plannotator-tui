#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use crate::app::review_test_support::{click, draw, file_app, folder_app, press, reopen};
use crate::app::{Focus, Mode};
use crate::store::Location;

#[test]
fn finish_undo_and_restore_keep_pending_edits_and_preserve_sent_status() {
    let (root, mut app, delivery) = file_app("finish");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    app.add_quote_annotation("two", Kind::Comment, "B".into()).expect("B");
    let b = app.open.store.placed()[1].annotation.id.clone();
    press(&mut app, 'E');
    app.open.store.edit_body(&b, "B revised".into()).expect("edit B");
    app.add_quote_annotation("three", Kind::Comment, "C".into()).expect("C");
    press(&mut app, 'F');
    assert_eq!(app.open.store.len(), 2);
    assert_eq!(app.open.store.archived().len(), 1);
    assert_eq!(app.send_count(), 2);
    assert_eq!(delivery.calls.borrow().len(), 1);
    assert!(!app.quit);
    let screen = draw(&mut app, 80, 24);
    assert!(screen.contains("archived 1 annotation(s)") && screen.contains("U Undo"), "{screen}");
    let undo = app.geometry.undo_button.expect("undo button");
    click(&mut app, undo);
    assert_eq!(app.open.store.len(), 3);
    assert_eq!(app.send_count(), 2, "undo did not make A pending");
    assert!(app.open.store.archived().is_empty());

    press(&mut app, 'F');
    reopen(&mut app);
    app.undo_archive.clear(); // a new process has no in-memory undo, but the archive persists
    press(&mut app, 'H');
    assert_eq!(app.mode, Mode::Archive);
    assert_eq!(app.archive_items.len(), 1);
    assert!(draw(&mut app, 80, 24).contains("Archived annotations (1)"));
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("restore");
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.send_count(), 2);
    press(&mut app, 'E');
    let calls = delivery.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert!(!calls[1].contains("> A"));
    assert!(calls[1].contains("B revised") && calls[1].contains("> C"));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn sent_markers_clear_on_edit_and_review_shortcuts_are_text_while_composing() {
    let (root, mut app, _) = file_app("edit-ui");
    press(&mut app, 'c');
    assert_eq!(app.mode, Mode::Compose);
    for ch in "FRHU".chars() {
        press(&mut app, ch);
    }
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("save");
    assert_eq!(app.open.store.placed()[0].annotation.body, "FRHU");
    press(&mut app, 'E');
    assert!(draw(&mut app, 100, 24).contains(" · sent"));
    app.focus = Focus::Rail;
    press(&mut app, 'e');
    press(&mut app, '!');
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("save edit");
    assert_eq!(app.send_count(), 1);
    assert!(!draw(&mut app, 100, 24).contains(" · sent"));
    press(&mut app, 'q');
    assert_eq!(app.mode, Mode::ConfirmQuit, "an edited sent note still needs sending");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_archive_picker_scrolls_and_restores_the_clicked_annotation() {
    let (root, mut app, _) = file_app("archive-scroll");
    for index in 0..10 {
        app.add_quote_annotation("one", Kind::Comment, format!("note {index}")).expect("note");
    }
    press(&mut app, 'E');
    press(&mut app, 'F');
    press(&mut app, 'H');
    for _ in 0..7 {
        press(&mut app, 'j');
    }
    assert_eq!(app.archive_cursor, 7);
    let selected = app.archive_items[7].annotation.clone();
    let screen = draw(&mut app, 80, 14);
    assert!(screen.contains(&selected.body), "{screen}");
    let rect = app.geometry.archive_rows.iter().find(|(_, i)| *i == 7).expect("selected row visible").0;
    click(&mut app, rect);
    assert_eq!(app.open.store.placed()[0].annotation, &selected);
    assert_eq!(app.open.store.archived().len(), 9);
    assert_eq!(app.send_count(), 0);
    assert_eq!(app.mode, Mode::Archive);
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("close");
    assert_eq!(app.mode, Mode::Browse);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn folder_finish_reports_partial_failure_and_undo_covers_only_committed_archives() {
    let (root, mut app, _) = folder_app("partial-archive");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    let other = root.join("docs/b.md");
    std::fs::write(&other, "beta\n\nnew\n").expect("B file");
    let (doc, mut store) = app.load_review_file(&other).expect("B store");
    store.add(&doc, 0..4, "beta".into(), Kind::Comment, "B".into()).expect("B");
    app.refresh_review_counts();
    press(&mut app, 'E');
    let (doc, mut store) = app.load_review_file(&other).expect("sent B store");
    store.add(&doc, 6..9, "new".into(), Kind::Comment, "C pending".into()).expect("C");
    app.refresh_review_counts();
    let location = Location::for_file(&app.data_dir, &app.project, &other);
    let blocked_tmp = location.record.with_extension("json.tmp");
    std::fs::create_dir(&blocked_tmp).expect("block B archive");
    press(&mut app, 'F');
    let status = app.status.as_deref().expect("status");
    assert!(
        status.starts_with("archived 1 annotation(s)")
            && status.contains("could not archive")
            && status.contains("b.md"),
        "{status}"
    );
    assert_eq!(app.open.store.archived().len(), 1);
    assert_eq!(app.load_review_file(&other).expect("B intact").1.len(), 2);
    assert_eq!(app.send_count(), 1);
    press(&mut app, 'U');
    assert_eq!(app.open.store.len(), 1);
    assert_eq!(app.send_count(), 1);
    std::fs::remove_dir(&blocked_tmp).expect("unblock");
    press(&mut app, 'F');
    assert_eq!(app.review_counts().archived, 2);
    assert_eq!(app.send_count(), 1);
    press(&mut app, 'U');
    assert_eq!(app.review_counts().archived, 0);
    assert_eq!(app.review_counts().sent, 2);
    assert_eq!(app.send_count(), 1);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_failed_undo_stays_available_for_retry_without_losing_the_archive() {
    let (root, mut app, _) = file_app("undo-failure");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    press(&mut app, 'F');
    let location = Location::for_file(&app.data_dir, &app.project, &root.join("docs/a.md"));
    let blocked_tmp = location.record.with_extension("json.tmp");
    std::fs::create_dir(&blocked_tmp).expect("block restore");
    press(&mut app, 'U');
    assert!(!app.undo_archive.is_empty());
    assert_eq!(app.open.store.len(), 0);
    assert_eq!(app.open.store.archived().len(), 1);
    assert!(app.status.as_deref().expect("status").contains("could not restore"));
    std::fs::remove_dir(&blocked_tmp).expect("unblock");
    press(&mut app, 'U');
    assert!(app.undo_archive.is_empty());
    assert_eq!(app.open.store.len(), 1);
    assert_eq!(app.send_count(), 0);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_deleted_files_archive_is_still_visible_and_restores_when_its_source_returns() {
    let (root, mut app, _) = folder_app("deleted-source");
    let path = root.join("docs/b.md");
    std::fs::write(&path, "beta\n").expect("source");
    let (doc, mut store) = app.load_review_file(&path).expect("store");
    store.add(&doc, 0..4, "beta".into(), Kind::Comment, "keep this feedback".into()).expect("note");
    press(&mut app, 'E');
    press(&mut app, 'F');
    std::fs::remove_file(&path).expect("source removed");
    app.refresh_review_counts();
    assert_eq!(app.review_counts().archived, 1);
    press(&mut app, 'H');
    assert_eq!(app.archive_items.len(), 1);
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("restore");
    assert_eq!(app.send_count(), 0);
    assert_eq!(app.load_review_file(&path).expect("missing source").1.orphans(), 1);
    std::fs::write(&path, "beta\n").expect("source returns");
    let (_, store) = app.load_review_file(&path).expect("reopen source");
    assert_eq!(store.placed().len(), 1);
    assert!(store.all_delivered());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn finished_notes_are_recoverable_with_submission_history_disabled() {
    let (root, mut app, _) = file_app("history-disabled");
    std::fs::create_dir_all(&app.data_dir).expect("data dir");
    std::fs::write(app.data_dir.join("config.json"), r#"{"feedbackHistory":false}"#)
        .expect("disable history");
    app.add_quote_annotation("one", Kind::Comment, "recover me".into()).expect("note");
    press(&mut app, 'E');
    assert!(!app.data_dir.join("feedback").exists());
    press(&mut app, 'F');
    reopen(&mut app);
    press(&mut app, 'H');
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("restore");
    assert_eq!(app.open.store.placed()[0].annotation.body, "recover me");
    assert_eq!(app.send_count(), 0);
    std::fs::remove_dir_all(root).expect("cleanup");
}
