#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use crate::app::review_test_support::{click, draw, file_app, folder_app, press, reply_app};
use crate::app::{App, Mode};

fn hosted(app: &mut App, root: &Path) -> PathBuf {
    let intent = root.join("send-intent");
    app.send_intent = Some(intent.clone());
    intent
}

fn ctrl(app: &mut App, c: char) {
    app.handle_event(&Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))).expect("key");
}

fn handed_back(intent: &Path) -> bool {
    std::fs::read_to_string(intent).is_ok_and(|text| text == "send\n")
}

fn footer(app: &mut App) -> String {
    draw(app, 160, 20).lines().last().unwrap_or_default().to_owned()
}

#[test]
fn every_send_key_hands_the_notes_back_and_leaves_them_pending_for_the_host() {
    for key in ['E', 'S', 's'] {
        let (root, mut app, delivery) =
            if key == 'S' { reply_app("host-send") } else { file_app("host-send") };
        let intent = hosted(&mut app, &root);
        app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
        if key == 's' {
            ctrl(&mut app, 's');
        } else {
            press(&mut app, key);
        }
        assert!(app.quit, "{key} returns to the host");
        assert!(handed_back(&intent), "{key} wrote the intent");
        assert!(delivery.calls.borrow().is_empty(), "the host delivers, not the app");
        assert_eq!(app.send_count(), 1, "the note is still pending for the host's export");
    }
}

#[test]
fn the_header_button_hands_the_notes_back() {
    let (root, mut app, delivery) = file_app("host-button");
    let intent = hosted(&mut app, &root);
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    draw(&mut app, 120, 20);
    let button = app.geometry.send_button.expect("send button");
    click(&mut app, button);
    assert!(app.quit && handed_back(&intent));
    assert!(delivery.calls.borrow().is_empty());
}

#[test]
fn the_send_chord_mid_comment_saves_the_comment_and_hands_it_back() {
    let (root, mut app, _delivery) = file_app("host-compose");
    let intent = hosted(&mut app, &root);
    for c in ['c', 'o', 'k'] {
        press(&mut app, c);
    }
    assert_eq!(app.mode, Mode::Compose);
    ctrl(&mut app, 's');
    assert!(app.quit && handed_back(&intent));
    assert_eq!(app.open.store.len(), 1, "the comment was saved first");
}

#[test]
fn a_send_with_nothing_to_send_stays_open() {
    let (root, mut app, _delivery) = file_app("host-empty");
    let intent = hosted(&mut app, &root);
    ctrl(&mut app, 's');
    assert!(!app.quit);
    assert!(!intent.exists());
    assert_eq!(app.status.as_deref(), Some("nothing to send"));
}

#[test]
fn quit_with_notes_hands_them_back_and_without_notes_just_quits() {
    let (root, mut app, _delivery) = file_app("host-quit");
    let intent = hosted(&mut app, &root);
    press(&mut app, 'q');
    assert!(app.quit && !intent.exists(), "nothing to send, so no intent");

    let (root, mut app, _delivery) = file_app("host-quit-notes");
    let intent = hosted(&mut app, &root);
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    press(&mut app, 'q');
    assert!(app.quit && handed_back(&intent));
}

#[test]
fn interrupt_leaves_without_sending_or_asking() {
    let (root, mut app, _delivery) = file_app("host-interrupt");
    let intent = hosted(&mut app, &root);
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    ctrl(&mut app, 'c');
    assert!(app.quit, "the host decides what to do with the notes");
    assert_ne!(app.mode, Mode::ConfirmQuit);
    assert!(!intent.exists());
}

#[test]
fn a_hosted_folder_review_has_one_send_button_and_no_review_menu() {
    let (root, mut app, _delivery) = folder_app("host-folder");
    hosted(&mut app, &root);
    let screen = draw(&mut app, 160, 20);
    let header = screen.lines().next().unwrap_or_default();
    assert!(header.contains("Send 0 back (q)") && !header.contains("Review"), "{header:?}");
    press(&mut app, 'm');
    assert_eq!(app.mode, Mode::Browse, "no review menu");
    assert!(footer(&mut app).contains("q quit"));
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    let footer = footer(&mut app);
    assert!(footer.contains("q send · ctrl+c leave") && !footer.contains("E send"), "{footer:?}");
}

#[test]
fn the_send_keys_follow_keys_rebinding() {
    let (root, mut app, _delivery) = file_app("host-rebound");
    let intent = hosted(&mut app, &root);
    app.keys = toml::from_str("send = [\"ctrl+g\"]\nquit = [\"Q\"]\n").expect("keys");
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    let screen = draw(&mut app, 160, 20);
    assert!(screen.lines().next().unwrap_or_default().contains("Send 1 back (Q)"), "{screen}");
    assert!(footer(&mut app).contains("Q send"));
    press(&mut app, 'c');
    ctrl(&mut app, 's');
    assert_eq!(app.mode, Mode::Compose, "ctrl+s is no longer a send");
    ctrl(&mut app, 'g');
    assert!(app.quit && handed_back(&intent));
}
