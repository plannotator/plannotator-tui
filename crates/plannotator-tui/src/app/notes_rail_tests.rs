//! The notes rail (`toggle-notes`, `[ui] notes_rail`): away while a document has no notes,
//! shown at the first, and once closed it stays closed, this session and the next. Hidden,
//! the text takes its columns and the notes keep their marks and their count.

#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]

use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use super::review_test_support::{draw, file_app, folder_app, press};
use super::{App, Focus};
use crate::notes_rail::{NotesRail, RailStart};

/// The fixture document with a note on "one" and one on "three".
fn noted_app(tag: &str) -> (std::path::PathBuf, App) {
    let (root, mut app, _) = file_app(tag);
    app.add_quote_annotation("one", Kind::Comment, "first note".to_owned()).expect("note");
    app.add_quote_annotation("three", Kind::Comment, "second note".to_owned()).expect("note");
    (root, app)
}

/// The rail's width at `width` columns, as the document screen sizes it.
fn rail(width: u16) -> u16 {
    (width * 3 / 10).clamp(28, 36)
}

fn screen_row<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen.lines().find(|row| row.contains(needle)).unwrap_or_else(|| panic!("no {needle:?} in\n{screen}"))
}

fn add_comment_on_block_two(app: &mut App, text: &str) {
    press(app, 'g');
    press(app, 'j');
    press(app, 'j');
    press(app, 'c');
    app.handle_event(&Event::Paste(text.to_owned())).expect("paste");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("enter");
    press(app, 'g');
}

#[test]
fn closing_the_rail_gives_the_text_its_columns_and_keeps_marks_and_counts() {
    let (root, mut app) = noted_app("rail-widths");
    app.ui.doc_width = 0;
    for width in [80, 120, 200] {
        app.notes_rail = NotesRail::new(RailStart::WhenNotes);
        let open = draw(&mut app, width, 20);
        assert!(open.contains("first note"), "at {width} the rail shows:\n{open}");
        assert_eq!(app.geometry.doc.width, width - 2 - 1 - rail(width), "at {width}");
        assert!(!open.contains("notes hidden"));

        press(&mut app, 'z');
        let hidden = draw(&mut app, width, 20);
        assert!(!hidden.contains("first note"), "at {width} the rail is gone:\n{hidden}");
        assert_eq!(app.geometry.doc.width, width - 2, "at {width} the text takes the rail's columns");
        assert_eq!(app.open.layout.width, usize::from(width - 2), "the text is laid out again");
        assert_eq!(app.geometry.bubbles.len(), 0, "no bubble is left to click");
        assert!(screen_row(&hidden, "one").starts_with('▍'), "the note's mark stays:\n{hidden}");
        assert!(screen_row(&hidden, "Send").contains("2 new"), "the header still counts:\n{hidden}");
        assert!(hidden.contains("2 annotations"));
        assert!(hidden.contains("notes hidden: z shows them"), "{hidden}");
        assert!(!hidden.contains("rail hidden: widen"));
    }
    press(&mut app, 'z');
    assert!(draw(&mut app, 120, 20).contains("first note"), "the same key shows it again");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_reading_column_recentres_in_the_columns_a_closed_rail_leaves() {
    let (root, mut app) = noted_app("rail-column");
    draw(&mut app, 200, 20);
    let column_room = 200 - 2 - 1 - rail(200);
    assert_eq!((app.geometry.doc.x, app.geometry.doc.width), ((column_room - 80) / 2 + 2, 80));
    press(&mut app, 'z');
    draw(&mut app, 200, 20);
    assert_eq!((app.geometry.doc.x, app.geometry.doc.width), ((198 - 80) / 2 + 2, 80));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_document_without_notes_has_no_rail_until_its_first_note() {
    let (root, mut app, _) = file_app("rail-first-note");
    app.ui.doc_width = 0;
    let empty = draw(&mut app, 120, 20);
    assert_eq!(app.geometry.doc.width, 118, "no empty rail takes columns:\n{empty}");
    assert!(!empty.contains("notes hidden") && !empty.contains("rail hidden"), "{empty}");
    add_comment_on_block_two(&mut app, "the first note");
    let noted = draw(&mut app, 120, 20);
    assert!(noted.contains("the first note"), "the rail comes with the first note:\n{noted}");
    assert_eq!(app.geometry.doc.width, 120 - 2 - 1 - rail(120));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn shown_keeps_the_rail_with_no_notes_and_hidden_starts_it_closed() {
    let (root, mut app, _) = file_app("rail-start");
    app.ui.doc_width = 0;
    app.ui.notes_rail = RailStart::Shown;
    app.start_notes_rail();
    draw(&mut app, 120, 20);
    assert_eq!(app.geometry.doc.width, 120 - 2 - 1 - rail(120));

    app.ui =
        crate::config::Config::parse("[ui]\nnotes_rail = \"hidden\"\ndoc_width = 0\n").expect("config").ui;
    app.start_notes_rail();
    add_comment_on_block_two(&mut app, "a note");
    let screen = draw(&mut app, 120, 20);
    assert!(!screen.contains("a note") && screen.contains("notes hidden: z shows them"), "{screen}");
    press(&mut app, 'z');
    assert!(draw(&mut app, 120, 20).contains("a note"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tab_opens_a_closed_rail_and_focuses_it_and_closing_it_returns_to_the_document() {
    let (root, mut app) = noted_app("rail-tab");
    press(&mut app, 'z');
    draw(&mut app, 120, 20);
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Tab))).expect("tab");
    assert_eq!(app.focus, Focus::Rail);
    let screen = draw(&mut app, 120, 20);
    assert!(screen.contains("first note"), "{screen}");
    assert!(screen.contains("z hide"), "the rail's hints name the key:\n{screen}");

    press(&mut app, 'z');
    assert_eq!(app.focus, Focus::Document);
    assert!(!draw(&mut app, 120, 20).contains("first note"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_note_added_while_the_rail_is_closed_is_marked_and_leaves_it_closed() {
    let (root, mut app) = noted_app("rail-add");
    press(&mut app, 'z');
    add_comment_on_block_two(&mut app, "third note");
    assert_eq!(app.open.store.len(), 3);
    let screen = draw(&mut app, 200, 20);
    assert!(!screen.contains("third note"), "a wider pane does not open it either:\n{screen}");
    assert!(screen_row(&screen, "two").trim_start().starts_with('▍'), "the new note is marked:\n{screen}");
    assert!(screen_row(&screen, "Send").contains("3 new"), "{screen}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_closed_rail_stays_closed_across_a_reload_and_another_file_of_the_folder() {
    let (root, mut app, _) = folder_app("rail-session");
    std::fs::write(root.join("docs/b.md"), "# Other\n\nfour\n").expect("second file");
    app.add_quote_annotation("one", Kind::Comment, "first note".to_owned()).expect("note");
    press(&mut app, 'z');
    app.reload().expect("reload");
    assert!(!draw(&mut app, 160, 20).contains("first note"));
    app.open_file_path(&root.join("docs/b.md")).expect("open b");
    app.add_quote_annotation("four", Kind::Comment, "b note".to_owned()).expect("note");
    let screen = draw(&mut app, 160, 20);
    assert!(!screen.contains("b note") && screen.contains("notes hidden"), "{screen}");
    app.open_file_path(&root.join("docs/a.md")).expect("back to a");
    assert!(!draw(&mut app, 160, 20).contains("first note"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_next_session_opens_the_rail_as_this_one_left_it() {
    let (root, mut app) = noted_app("rail-memory");
    press(&mut app, 'z');
    let state = app.data_dir.join("clients/plannotator-tui/ui-state.json");
    assert_eq!(std::fs::read_to_string(&state).expect("remembered"), r#"{"notesRail":"closed"}"#);
    app.notes_rail = NotesRail::default();
    app.start_notes_rail();
    assert!(!draw(&mut app, 120, 20).contains("first note"), "closed again in the next session");
    press(&mut app, 'z');
    app.start_notes_rail();
    assert!(draw(&mut app, 120, 20).contains("first note"), "and open once reopened");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_rebound_toggle_is_the_key_the_footer_names() {
    let (root, mut app) = noted_app("rail-rebound");
    app.keys = toml::from_str("toggle-notes = [\"Z\"]\n").expect("keymap");
    press(&mut app, 'z');
    assert!(draw(&mut app, 120, 20).contains("first note"), "z is no longer the key");
    press(&mut app, 'Z');
    assert!(draw(&mut app, 120, 20).contains("notes hidden: Z shows them"));
    let _ = std::fs::remove_dir_all(&root);
}
