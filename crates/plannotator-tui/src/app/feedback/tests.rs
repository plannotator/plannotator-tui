#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::Path;

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use crate::app::review_test_support::{Outcome, click, draw, file_app, folder_app, press, reopen};
use crate::app::{App, Focus, Mode, Open};
use crate::doc::Document;
use crate::store::{Location, Store};
use crate::tree::Tree;

#[test]
fn incremental_send_and_explicit_resend_use_the_same_set_for_body_history_and_ids() {
    let (root, mut app, delivery) = file_app("incremental");
    app.add_quote_annotation("one", Kind::Comment, "note A".into()).expect("A");
    app.add_quote_annotation("two", Kind::Comment, "note B".into()).expect("B");
    let b = app.open.store.placed()[1].annotation.id.clone();
    press(&mut app, 'E');
    app.add_quote_annotation("three", Kind::Comment, "note C".into()).expect("C");
    assert_eq!(app.send_count(), 1);
    draw(&mut app, 80, 24);
    let send = app.geometry.send_button.expect("send button");
    click(&mut app, send);
    app.open.store.edit_body(&b, "note B revised".into()).expect("edit B");
    reopen(&mut app);
    assert_eq!(app.send_count(), 1, "edited B remains pending after reopening");
    press(&mut app, 'E');
    assert_eq!(app.send_count(), 0);
    let screen = draw(&mut app, 80, 24);
    assert!(screen.contains("Resend all (3 sent)"), "{screen}");
    let resend = app.geometry.resend_button.expect("resend button");
    click(&mut app, resend);

    let calls = delivery.calls.borrow();
    assert_eq!(calls.len(), 4);
    assert!(calls[0].contains("note A") && calls[0].contains("note B"));
    assert!(calls[1].contains("note C") && !calls[1].contains("note A") && !calls[1].contains("note B"));
    assert!(
        calls[2].contains("note B revised") && !calls[2].contains("note A") && !calls[2].contains("note C")
    );
    assert!(
        calls[3].contains("note A") && calls[3].contains("note B revised") && calls[3].contains("note C")
    );

    let location = Location::for_file(&app.data_dir, &app.project, &root.join("docs/a.md"));
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(location.record).expect("record")).expect("JSON");
    let ids: Vec<usize> = record["deliveries"]
        .as_array()
        .expect("deliveries")
        .iter()
        .map(|d| d["annotation_ids"].as_array().expect("ids").len())
        .collect();
    assert_eq!(ids, [2, 1, 1, 3]);
    let history =
        std::fs::read_to_string(app.data_dir.join("feedback").join(&app.project).join("index.jsonl"))
            .expect("history");
    let records: Vec<serde_json::Value> =
        history.lines().map(|s| serde_json::from_str(s).expect("history JSON")).collect();
    assert_eq!(records.len(), 4);
    assert_eq!(records[1]["annotations"].as_array().expect("selected annotations").len(), 1);
    assert_eq!(records[1]["annotations"][0]["text"], "note C");
    assert_eq!(records[2]["annotations"][0]["text"], "note B revised");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_zero_pending_button_stays_visible_and_does_not_send_or_record_another_delivery() {
    let (root, mut app, delivery) = file_app("nothing-new");
    press(&mut app, 'E');
    assert!(delivery.calls.borrow().is_empty());
    assert_eq!(app.status.as_deref(), Some("nothing new to send"));
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    let location = Location::for_file(&app.data_dir, &app.project, &root.join("docs/a.md"));
    let before = std::fs::read(&location.record).expect("record");
    let screen = draw(&mut app, 80, 24);
    assert!(screen.contains("Send 0 new"));
    let rect = app.geometry.send_button.expect("still clickable");
    click(&mut app, rect);
    assert_eq!(delivery.calls.borrow().len(), 1);
    assert_eq!(std::fs::read(&location.record).expect("same record"), before);
    assert!(draw(&mut app, 80, 24).contains("nothing new to send"));
    press(&mut app, 'q');
    assert!(app.quit, "no pending feedback means no quit confirmation");
    assert_eq!(app.mode, Mode::Browse);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn unsuccessful_sends_leave_the_record_pending_and_offer_retry() {
    for outcome in [Outcome::Blocked, Outcome::Unavailable, Outcome::Failed] {
        let (root, mut app, delivery) = file_app("failed-send");
        app.add_quote_annotation("one", Kind::Comment, "keep pending".into()).expect("annotation");
        let location = Location::for_file(&app.data_dir, &app.project, &root.join("docs/a.md"));
        let before = std::fs::read(&location.record).expect("record");
        delivery.outcome.set(outcome);
        press(&mut app, 'E');
        assert_eq!(std::fs::read(&location.record).expect("record"), before);
        assert_eq!(app.send_count(), 1);
        let status = app.status.as_deref().expect("failure status");
        assert!(status.contains("E retry"), "{status}");
        assert!(!status.contains("copied"), "clipboard was disabled");
        assert!(!app.data_dir.join("feedback").exists(), "no submission history for a failure");
        reopen(&mut app);
        assert_eq!(app.send_count(), 1);
        delivery.outcome.set(Outcome::Success);
        press(&mut app, 'E');
        assert_eq!(app.send_count(), 0);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}

#[test]
fn folder_counts_and_delivery_cover_collapsed_files_but_exclude_orphans_and_siblings() {
    let (root, mut app, delivery) = folder_app("folder-scope");
    app.add_quote_annotation("one", Kind::Comment, "already sent A".into()).expect("A");
    press(&mut app, 'E');
    app.add_quote_annotation("three", Kind::Comment, "new C".into()).expect("C");
    std::fs::create_dir_all(root.join("docs/deep")).expect("subdir");
    let nested = root.join("docs/deep/b.md");
    std::fs::write(&nested, "beta\n\ngone\n").expect("nested file");
    let (doc, mut store) = app.load_review_file(&nested).expect("nested review");
    store.add(&doc, 0..4, "beta".into(), Kind::Comment, "nested D".into()).expect("D");
    store.add(&doc, 6..10, "gone".into(), Kind::Comment, "orphaned note".into()).expect("orphan");
    std::fs::write(&nested, "beta\n").expect("remove orphan's quote");
    let outside = root.join("sibling.md");
    std::fs::write(&outside, "outside\n").expect("sibling");
    let (doc, mut store) = app.load_review_file(&outside).expect("sibling store");
    store.add(&doc, 0..7, "outside".into(), Kind::Comment, "other folder".into()).expect("sibling note");
    let archived = root.join("docs/deep/c.md");
    std::fs::write(&archived, "finished\n").expect("archive-only file");
    let (doc, mut store) = app.load_review_file(&archived).expect("archived store");
    store.add(&doc, 0..8, "finished".into(), Kind::Comment, "finished note".into()).expect("note");
    let id = store.placed()[0].annotation.id.clone();
    store.record_delivery("test agent", &[id]).expect("sent");
    store.archive_sent().expect("archive");

    app.refresh_review_counts();
    assert_eq!(app.send_count(), 2);
    assert_eq!(app.pending_file_count(), 2);
    assert_eq!(app.review_counts().archived, 1);
    assert!(app.send_label().contains("2 new across 2 files"));
    assert!(
        !app.tree.as_ref().expect("tree").rows.iter().any(|r| r.path == nested),
        "nested file is not listed"
    );
    press(&mut app, 'E');
    let calls = delivery.calls.borrow();
    let sent = calls.last().expect("folder feedback");
    assert!(sent.contains("new C") && sent.contains("nested D"), "{sent}");
    for excluded in ["already sent A", "orphaned note", "other folder", "finished note", "No annotations."] {
        assert!(!sent.contains(excluded), "{excluded} was included: {sent}");
    }
    assert!(app.status.as_deref().expect("status").contains("2 annotation(s) across 2 files"));
    let (_, nested_store) = app.load_review_file(&nested).expect("reload nested");
    assert!(!nested_store.all_delivered(), "the unsent orphan was not recorded as sent");
    assert_eq!(nested_store.placed().len(), 1);
    assert!(!nested_store.is_pending(nested_store.placed()[0].annotation));
    let (_, outside_store) = app.load_review_file(&outside).expect("reload sibling");
    assert!(!outside_store.all_delivered());
    std::fs::remove_dir_all(root).expect("cleanup");
}

/// Folder output is `writeln!` per file: every block, including the last, is followed by
/// a blank line, while a single file's export ends with the block's own newline only.
#[test]
fn folder_feedback_ends_every_file_block_with_a_blank_line() {
    let (root, mut app, delivery) = folder_app("folder-newline");
    app.add_quote_annotation("one", Kind::Comment, "note A".into()).expect("A");
    let other = root.join("docs/b.md");
    std::fs::write(&other, "beta\n").expect("second file");
    let (doc, mut store) = app.load_review_file(&other).expect("second store");
    store.add(&doc, 0..4, "beta".into(), Kind::Comment, "note B".into()).expect("B");
    app.refresh_review_counts();

    let single = app.feedback();
    assert!(single.ends_with("> note A\n\n") && !single.ends_with("\n\n\n"), "{single:?}");
    let folder = app.folder_feedback().expect("folder export");
    assert_eq!(
        folder,
        format!(
            "{single}\n# Annotations on b.md\n\n## Annotation 1 (line 1)\nComment on: \"beta\"\n> note B\n\n\n"
        )
    );
    assert!(folder.ends_with("> note B\n\n\n"), "{folder:?}");

    press(&mut app, 'E');
    let sent = delivery.calls.borrow()[0].clone();
    assert_eq!(sent, folder, "the send body is the export");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn reply_reviews_keep_sending_the_whole_transient_review() {
    let (root, mut app, delivery) = file_app("reply-scope");
    let source = DocumentSource::new(
        "one\n\ntwo\n".into(),
        "agent reply",
        true,
        Provenance::AgentMessage {
            host: "claude".into(),
            session: None,
            message_id: Some("message-1".into()),
        },
    );
    app.open = Open::new(source, 80, &app.data_dir, &app.project).expect("reply");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    app.add_quote_annotation("two", Kind::Comment, "B".into()).expect("B");
    press(&mut app, 'E');
    let calls = delivery.calls.borrow();
    assert!(calls[1].contains("> A") && calls[1].contains("> B"));
    press(&mut app, 'F');
    assert_eq!(app.open.store.len(), 2);
    assert!(app.open.store.archived().is_empty());
    assert!(app.open.store.is_transient());
    assert!(!draw(&mut app, 80, 24).contains("Finish review"));
    std::fs::remove_dir_all(root).expect("cleanup");
}

/// Make the document unreadable. `chmod 000` does it on Unix unless the tests run as root
/// (a container), in which case a directory in its place fails every read the same way.
fn make_unreadable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        if std::fs::read(path).is_err() {
            return;
        }
    }
    std::fs::remove_file(path).expect("replace the document");
    std::fs::create_dir(path).expect("directory in its place");
}

/// One annotated file that cannot be read is left out of the counts with a status note.
/// It never stops the folder from opening, expanding a directory, or reloading.
#[test]
fn a_folder_opens_expands_and_reloads_when_one_annotated_file_is_unreadable() {
    let (root, mut app, delivery) = folder_app("unreadable");
    let docs = root.join("docs");
    app.add_quote_annotation("one", Kind::Comment, "readable".into()).expect("A");
    let blocked = docs.join("b.md");
    std::fs::write(&blocked, "beta\n").expect("b");
    let (doc, mut store) = app.load_review_file(&blocked).expect("b store");
    store.add(&doc, 0..4, "beta".into(), Kind::Comment, "cannot be read".into()).expect("B");
    std::fs::create_dir_all(docs.join("deep")).expect("subdir");
    std::fs::write(docs.join("deep/c.md"), "gamma\n").expect("c");
    app.tree = Some(Tree::scan(&docs).expect("tree"));
    make_unreadable(&blocked);
    assert!(app.load_review_file(&blocked).is_err(), "fixture: the document is unreadable");

    app.refresh_review_counts();
    assert_eq!(app.unreadable_files, std::slice::from_ref(&blocked));
    assert_eq!(app.status.as_deref(), Some("skipped 1 unreadable file(s): b.md"));
    assert_eq!(app.send_count(), 1, "only the readable file counts");
    assert!(!app.folder_counts.contains_key(&blocked));

    // The note is shown once: an unchanged set does not overwrite a newer status.
    app.status = Some("something else".into());
    app.refresh_review_counts();
    assert_eq!(app.status.as_deref(), Some("something else"));

    app.focus = Focus::Tree;
    app.tree_cursor = app.tree.as_ref().expect("tree").position(&docs.join("deep")).expect("deep row");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("expanding a directory");
    assert!(app.tree.as_ref().expect("tree").rows.iter().any(|r| r.path == docs.join("deep/c.md")));

    app.focus = Focus::Document;
    press(&mut app, 'r');
    let status = app.status.as_deref().expect("status");
    assert!(status.starts_with("reloaded · 0 orphaned · skipped 1 unreadable file(s): b.md"), "{status}");
    assert_eq!(app.send_count(), 1);

    // Opening from scratch resolves the data dir itself, so the record goes there too.
    let data_dir = crate::workspace_paths::data_dir();
    let location = Location::for_file(&data_dir, &crate::workspace_paths::project_name(&docs), &blocked);
    let doc = Document::parse("beta\n".into());
    let mut store = Store::load(&location, &doc).expect("record in the resolved data dir");
    store.add(&doc, 0..4, "beta".into(), Kind::Comment, "cannot be read".into()).expect("B");
    let opened = App::open_folder(&docs, 100, Box::new(delivery));
    if let Some(record_dir) = location.record.parent() {
        std::fs::remove_dir_all(record_dir).expect("cleanup record");
    }
    let opened = opened.expect("the folder opens although one annotated file is unreadable");
    assert_eq!(opened.unreadable_files, std::slice::from_ref(&blocked));
    assert_eq!(opened.status.as_deref(), Some("skipped 1 unreadable file(s): b.md"));
    assert!(!opened.folder_counts.contains_key(&blocked));
    std::fs::remove_dir_all(root).expect("cleanup");
}
