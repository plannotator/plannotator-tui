//! The footer noticing that the diff changed on disk, and `r` refreshing it.

use std::time::Duration;

use plannotator_tui_review::Status;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::review::Mode;

const NOTICE: &str = "the diff changed on disk · r refreshes";

/// Tick as the event loop does until `done`, for at most ten seconds.
fn tick_until(screen: &mut Screen, what: &str, done: impl Fn(&Screen) -> bool) {
    let started = Instant::now();
    while !done(screen) {
        screen.tick().expect("tick");
        assert!(started.elapsed() < Duration::from_secs(10), "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The screen with checks due at once, its baseline taken.
fn watched(fixture: &Fixture) -> Screen {
    let mut screen = fixture.open();
    screen.watch.every = Duration::ZERO;
    tick_until(&mut screen, "a second check", |s| s.watch.started >= 2);
    screen
}

#[test]
fn an_edit_on_disk_shows_in_the_footer_and_r_refreshes_with_the_notes_kept() {
    let fixture = Fixture::new("disk-notice");
    let mut screen = watched(&fixture);
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    screen.last_key = None;
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|r| r.contains("changed on disk")), "nothing changed yet: {rows:#?}");

    fixture.write("app.py", &format!("#!/usr/bin/env python3\n# greeter\n{AFTER}"));
    tick_until(&mut screen, "the notice", |s| s.watch.changed());
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[39].starts_with(&format!(" {NOTICE} · ")), "{:?}", rows[39]);
    let started = screen.watch.started;
    for _ in 0..20 {
        screen.tick().expect("tick");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(screen.watch.started, started, "no more checks until a refresh");

    keys(&mut screen, "r");
    settle(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|r| r.contains("changed on disk")), "{:?}", rows[39]);
    let [note] = screen.store.annotations() else { panic!("one note") };
    assert_eq!((note.line_start, note.status()), (11, Some(Status::Moved)), "the note followed its line");
}

#[test]
fn no_check_starts_while_the_comment_box_is_open_or_just_after_a_key() {
    let fixture = Fixture::new("disk-typing");
    let mut screen = watched(&fixture);
    at(&mut screen, 9);
    keys(&mut screen, "c");
    assert!(matches!(screen.mode, Mode::Compose(_)));
    screen.last_key = None;
    fixture.write("app.py", &format!("# edited\n{AFTER}"));
    // Let any check that was already running finish.
    std::thread::sleep(Duration::from_millis(300));
    screen.tick().expect("tick");
    let started = screen.watch.started;
    for _ in 0..20 {
        screen.tick().expect("tick");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(screen.watch.started, started, "the comment box is open");
    press(&mut screen, KeyCode::Esc);
    screen.tick().expect("tick");
    assert_eq!(screen.watch.started, started, "a key was just pressed");
    screen.last_key = Instant::now().checked_sub(watch_quiet());
    tick_until(&mut screen, "the notice once typing stopped", |s| s.watch.changed());
}

#[test]
fn a_commit_review_has_nothing_on_disk_to_watch() {
    let fixture = Fixture::new("disk-commit");
    let (repo, capture) =
        crate::review::capture(&fixture.repo(), &Request::Commit { rev: "HEAD".into() }).expect("capture");
    let (wake, _ready) = std::sync::mpsc::sync_channel(1);
    let mut screen =
        Screen::open(repo, capture, &fixture.data(), Box::new(fixture.agent.clone()), wake).expect("screen");
    screen.watch.every = Duration::ZERO;
    for _ in 0..10 {
        screen.tick().expect("tick");
    }
    assert_eq!(screen.watch.started, 0);
}

fn watch_quiet() -> Duration {
    crate::review::watch::QUIET_AFTER_KEY
}
