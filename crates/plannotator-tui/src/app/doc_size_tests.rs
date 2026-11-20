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
