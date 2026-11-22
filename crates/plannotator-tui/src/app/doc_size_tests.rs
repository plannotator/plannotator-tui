//! Documents at either end of the size range. A long one opens on its first screens while
//! the rest is laid out in the background, and nothing the reader can reach may tell the
//! difference; an empty one says so.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::fmt::Write as _;

use plannotator_tui_schema::Kind;

use super::review_test_support::{draw, file_app, press, reopen};

const BLOCKS: usize = 3000;

/// A file app on a document of `BLOCKS` paragraphs, the last one "The last paragraph.".
fn long_app(tag: &str) -> (std::path::PathBuf, super::App) {
    let (root, mut app, _) = file_app(tag);
    std::fs::write(root.join("docs/a.md"), long_text()).expect("document");
    reopen(&mut app);
    (root, app)
}

fn long_text() -> String {
    let mut text = String::new();
    for i in 0..BLOCKS - 1 {
        writeln!(text, "Paragraph {i} of the long document.\n").expect("write to a string");
    }
    text.push_str("The last paragraph.\n");
    text
}

#[test]
fn a_long_document_opens_lazily_and_g_and_saved_annotations_reach_its_end() {
    let (root, mut app) = long_app("long-doc");
    app.add_block_annotation(BLOCKS - 2, Kind::Comment, "far down".to_owned()).expect("annotation");

    reopen(&mut app);
    assert!(!app.open.layout.is_complete(), "opening laid out every block before the first frame");
    assert!(draw(&mut app, 120, 40).contains("Paragraph 0 of"));

    press(&mut app, 'G');
    assert!(app.open.layout.is_complete());
    assert_eq!(app.selected, BLOCKS - 1);
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("The last paragraph."), "{screen}");
    assert!(screen.contains(&format!("block {BLOCKS}/{BLOCKS}")), "{screen}");
    assert!(screen.contains("far down"), "the saved annotation's bubble is missing:\n{screen}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Scroll the long document to its end, edit the file, reload, and draw the first frame
/// after the reload at `width`. `eager` lays out every block first, as before the
/// background layout existed.
fn reload_at_the_end(width: u16, eager: bool) -> (usize, String) {
    let (root, mut app) = long_app("long-reload");
    press(&mut app, 'G');
    draw(&mut app, 120, 40);
    let mut text = long_text();
    text.push_str("\nAppended by an editor.\n");
    std::fs::write(root.join("docs/a.md"), text).expect("edited document");
    app.reload().expect("reload");
    assert!(!app.open.layout.is_complete(), "the reload laid out every block up front");
    if eager {
        app.open.layout.finish();
    }
    let screen = draw(&mut app, width, 40);
    let _ = std::fs::remove_dir_all(&root);
    (app.scroll, screen)
}

/// A reload restarts the layout from the top. When the view is deep in the document, the
/// first frame after it must show what a full layout shows, even when the width changed
/// too (an editor returning to a resized pane, `r` pressed during a resize).
#[test]
fn a_reload_deep_in_a_long_document_keeps_the_view_through_a_resize() {
    for width in [120, 100] {
        let (scroll, screen) = reload_at_the_end(width, false);
        assert!(screen.contains("Paragraph 2995 of"), "at width {width}:\n{screen}");
        assert_eq!((scroll, screen), reload_at_the_end(width, true), "at width {width}");
    }
}

#[test]
fn an_empty_document_says_so_instead_of_counting_blocks() {
    for text in ["", "\n\n"] {
        let (root, mut app, _) = file_app("empty-doc");
        std::fs::write(root.join("docs/a.md"), text).expect("document");
        reopen(&mut app);
        let screen = draw(&mut app, 100, 10);
        assert!(screen.contains("a.md · 0 annotations · empty document"), "{text:?}:\n{screen}");
        press(&mut app, 'G');
        press(&mut app, 'j');
        assert!(draw(&mut app, 100, 10).contains("empty document"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// Every block, those laid out before the first frame and those that came from the
/// background, wrapped to the layout's width.
fn every_row_fits(app: &super::App) -> bool {
    let width = app.open.layout.width;
    app.open.layout.blocks.iter().flat_map(|b| &b.rows).all(|row| row.cells.len() <= width)
}

/// The reading column changes width while the rest of a long document is still being laid
/// out: by a resize, then by `<` and `>`. The blocks that arrive late take the new width.
#[test]
fn the_reading_column_resizes_while_a_long_document_is_still_being_laid_out() {
    let (root, mut app) = long_app("long-column");
    app.notes_rail = crate::notes_rail::NotesRail::new(crate::notes_rail::RailStart::Shown);
    assert!(!app.open.layout.is_complete());
    draw(&mut app, 200, 40);
    assert_eq!(app.open.layout.width, usize::from(crate::config::DOC_WIDTH));
    draw(&mut app, 90, 40);
    let narrow = app.open.layout.width;
    assert!(narrow < usize::from(crate::config::DOC_WIDTH), "the pane binds at 90 columns: {narrow}");
    app.open.layout.finish();
    assert!(every_row_fits(&app), "a late block kept the width it was started at");

    reopen(&mut app);
    assert!(!app.open.layout.is_complete());
    draw(&mut app, 200, 40);
    press(&mut app, '<');
    draw(&mut app, 200, 40);
    assert_eq!(app.open.layout.width, usize::from(crate::config::DOC_WIDTH) - 8);
    assert!(app.open.layout.is_complete());
    assert!(every_row_fits(&app));
    press(&mut app, '>');
    press(&mut app, '>');
    draw(&mut app, 200, 40);
    assert_eq!(app.open.layout.width, usize::from(crate::config::DOC_WIDTH) + 8);
    assert!(every_row_fits(&app));
    let _ = std::fs::remove_dir_all(&root);
}

/// Closing the notes rail widens the text the way a resize does, also while the rest of a
/// long document is still being laid out, and also when the config closes it from the start.
#[test]
fn the_notes_rail_toggles_while_a_long_document_is_still_being_laid_out() {
    let (root, mut app) = long_app("long-rail");
    app.add_block_annotation(1, Kind::Comment, "near the top".to_owned()).expect("annotation");
    reopen(&mut app);
    app.ui.doc_width = 0;
    assert!(!app.open.layout.is_complete());
    draw(&mut app, 200, 40);
    assert_eq!(app.open.layout.width, 200 - 2 - 1 - 36);
    app.toggle_notes();
    draw(&mut app, 200, 40);
    assert_eq!(app.open.layout.width, 198);
    assert!(!app.open.layout.is_complete(), "the toggle waited for the whole layout");
    app.open.layout.finish();
    assert!(every_row_fits(&app), "a late block kept the width it was started at");
    assert_eq!(app.open.layout.width, 198);

    app.ui.notes_rail = crate::notes_rail::RailStart::Hidden;
    app.notes_rail = crate::notes_rail::NotesRail::new(app.ui.notes_rail);
    reopen(&mut app);
    assert!(!app.open.layout.is_complete());
    draw(&mut app, 200, 40);
    assert_eq!(app.open.layout.width, 198, "a rail hidden from the start");
    app.toggle_notes();
    draw(&mut app, 200, 40);
    assert_eq!(app.open.layout.width, 200 - 2 - 1 - 36);
    app.open.layout.finish();
    assert!(every_row_fits(&app));
    let _ = std::fs::remove_dir_all(&root);
}
