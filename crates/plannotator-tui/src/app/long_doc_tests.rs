//! A long document opens on its first screens while the rest is laid out in the
//! background; nothing the reader can reach may tell the difference.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::fmt::Write as _;

use plannotator_tui_schema::Kind;

use super::review_test_support::{draw, file_app, press, reopen};

const BLOCKS: usize = 3000;

#[test]
fn a_long_document_opens_lazily_and_g_and_saved_annotations_reach_its_end() {
    let (root, mut app, _) = file_app("long-doc");
    let path = root.join("docs/a.md");
    let mut text = String::new();
    for i in 0..BLOCKS - 1 {
        writeln!(text, "Paragraph {i} of the long document.\n").expect("write to a string");
    }
    text.push_str("The last paragraph.\n");
    std::fs::write(&path, text).expect("document");
    reopen(&mut app);
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
