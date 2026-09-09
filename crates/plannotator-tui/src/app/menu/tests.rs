#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::app::review_test_support::{click, draw, file_app, press};
use crate::app::{App, Mode, Open};

fn key(app: &mut App, code: KeyCode) {
    app.handle_event(&Event::Key(KeyEvent::from(code))).expect("key");
}

/// One frame, then the modifiers of the first cell of each menu row: `(dim, reversed)`.
fn row_styles(app: &mut App) -> Vec<(bool, bool)> {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    app.geometry
        .menu_rows
        .iter()
        .map(|(rect, _)| {
            let modifiers = buffer.cell((rect.x, rect.y)).expect("cell").style().add_modifier;
            (modifiers.contains(Modifier::DIM), modifiers.contains(Modifier::REVERSED))
        })
        .collect()
}

#[test]
fn the_header_holds_send_and_review_and_m_opens_and_closes_the_menu() {
    let (root, mut app, _) = file_app("menu-open");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    let screen = draw(&mut app, 100, 24);
    let header = screen.lines().next().expect("header");
    assert!(
        header.contains("Review \u{25be} (m)") && header.contains("Send 1 new \u{25b8} test agent (E)"),
        "{header}"
    );
    for gone in ["Resend all", "Finish review", "Archive"] {
        assert!(!header.contains(gone), "{gone} is behind the menu now: {header}");
    }
    let review = app.geometry.review_button.expect("review button");
    let send = app.geometry.send_button.expect("send button");
    assert!(review.right() < send.x && send.right() == 100, "review sits left of send: {review:?} {send:?}");

    press(&mut app, 'm');
    assert_eq!(app.mode, Mode::ReviewMenu);
    let screen = draw(&mut app, 100, 24);
    for row in [
        "R  Resend all \u{b7} 0 sent",
        "F  Finish review \u{b7} archive 0 sent",
        "U  Undo finish",
        "H  Archive \u{b7} 0 notes",
    ] {
        assert!(screen.contains(row), "{row} missing:\n{screen}");
    }
    assert!(screen.contains(" review "), "{screen}");
    let menu = app.geometry.menu.expect("menu rect");
    assert_eq!(menu.y, review.bottom(), "the menu hangs under the header");
    assert_eq!(menu.right(), review.right(), "flush with the button's right edge");

    press(&mut app, 'm');
    assert_eq!(app.mode, Mode::Browse, "m closes the menu");
    press(&mut app, 'm');
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Browse, "esc closes the menu");
    assert!(!draw(&mut app, 100, 24).contains("Resend all"));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn movement_skips_dimmed_rows_and_opens_on_the_first_that_applies() {
    let (root, mut app, _) = file_app("menu-skip");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    press(&mut app, 'm');
    // One sent note: resend and finish apply; nothing to undo and the archive is empty.
    assert_eq!(app.menu_cursor, 0);
    assert_eq!(row_styles(&mut app), [(false, true), (false, false), (true, false), (true, false)]);
    press(&mut app, 'j');
    assert_eq!(app.menu_cursor, 1);
    press(&mut app, 'j');
    assert_eq!(app.menu_cursor, 1, "undo and archive are dimmed, so the cursor stays");
    key(&mut app, KeyCode::Up);
    assert_eq!(app.menu_cursor, 0);
    key(&mut app, KeyCode::Up);
    assert_eq!(app.menu_cursor, 0);

    press(&mut app, 'F');
    assert_eq!(app.mode, Mode::Browse);
    press(&mut app, 'm');
    // Now nothing is sent: resend and finish are dimmed; undo and archive apply.
    assert_eq!(app.menu_cursor, 2, "opens on the first row that applies");
    assert_eq!(row_styles(&mut app), [(true, false), (true, false), (false, true), (false, false)]);
    press(&mut app, 'k');
    assert_eq!(app.menu_cursor, 2);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.menu_cursor, 3);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.menu_cursor, 3);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn enter_runs_the_highlighted_row_and_closes_the_menu() {
    let (root, mut app, delivery) = file_app("menu-enter");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    press(&mut app, 'm');
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(delivery.calls.borrow().len(), 2, "enter on the first row resent");
    assert!(app.status.as_deref().expect("status").starts_with("sent 1 annotation(s)"), "{:?}", app.status);

    press(&mut app, 'm');
    press(&mut app, 'j');
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.status.as_deref(), Some("archived 1 annotation(s)"));
    assert_eq!(app.open.store.archived().len(), 1);

    press(&mut app, 'm');
    assert_eq!(app.menu_cursor, 2);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.status.as_deref(), Some("restored 1 annotation(s)"));
    assert_eq!(app.open.store.len(), 1);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_rows_own_key_runs_it_from_the_menu() {
    let (root, mut app, _) = file_app("menu-keys");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    press(&mut app, 'm');
    press(&mut app, 'F');
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.status.as_deref(), Some("archived 1 annotation(s)"));

    press(&mut app, 'm');
    press(&mut app, 'H');
    assert_eq!(app.mode, Mode::Archive);
    assert_eq!(app.archive_items.len(), 1);
    key(&mut app, KeyCode::Esc);

    press(&mut app, 'm');
    press(&mut app, 'R');
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.status.as_deref(), Some("nothing to send"), "the direct key's own message");

    press(&mut app, 'm');
    press(&mut app, 'U');
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.status.as_deref(), Some("restored 1 annotation(s)"));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_menu_is_unavailable_while_composing_and_in_reply_reviews() {
    let (root, mut app, _) = file_app("menu-gate");
    press(&mut app, 'c');
    assert_eq!(app.mode, Mode::Compose);
    press(&mut app, 'm');
    assert_eq!(app.mode, Mode::Compose, "m is text while composing");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.open.store.placed()[0].annotation.body, "m");

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
    let screen = draw(&mut app, 100, 24);
    assert!(!screen.contains("Review \u{25be}"), "{screen}");
    assert!(app.geometry.review_button.is_none());
    press(&mut app, 'm');
    assert_eq!(app.mode, Mode::Browse, "a reply review has no menu");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_narrow_pane_keeps_both_header_buttons_on_one_line() {
    let (root, mut app, _) = file_app("menu-narrow");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    let screen = draw(&mut app, 60, 24);
    let mut lines = screen.lines();
    let header = lines.next().expect("header");
    let second = lines.next().expect("second row");
    assert_eq!(app.header_height(60), 1);
    assert!(header.contains("Review \u{25be} (m)") && header.contains("(E)"), "{header}");
    assert!(!second.contains("Review") && !second.contains("(E)"), "nothing wrapped: {second}");
    let review = app.geometry.review_button.expect("review");
    let send = app.geometry.send_button.expect("send");
    assert_eq!((review.y, send.y), (0, 0));
    assert!(review.right() < send.x && send.right() == 60);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn clicking_the_review_button_opens_the_menu_and_clicking_a_row_runs_it() {
    let (root, mut app, _) = file_app("menu-click");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    draw(&mut app, 80, 24);
    let review = app.geometry.review_button.expect("review");
    click(&mut app, review);
    assert_eq!(app.mode, Mode::ReviewMenu);
    draw(&mut app, 80, 24);
    let finish = app.geometry.menu_rows[1].0;
    click(&mut app, finish);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.open.store.archived().len(), 1);

    press(&mut app, 'm');
    draw(&mut app, 80, 24);
    let resend = app.geometry.menu_rows[0].0;
    click(&mut app, resend);
    assert_eq!(app.mode, Mode::ReviewMenu, "a dimmed row ignores the click");
    let archive = app.geometry.menu_rows[3].0;
    click(&mut app, archive);
    assert_eq!(app.mode, Mode::Archive);
    key(&mut app, KeyCode::Esc);

    press(&mut app, 'm');
    draw(&mut app, 80, 24);
    let outside = Rect { x: 0, y: 12, width: 1, height: 1 };
    app.handle_event(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: outside.x,
        row: outside.y,
        modifiers: KeyModifiers::NONE,
    }))
    .expect("click outside");
    assert_eq!(app.mode, Mode::Browse, "a click outside the menu closes it");
    std::fs::remove_dir_all(root).expect("cleanup");
}
