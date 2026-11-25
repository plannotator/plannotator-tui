//! Notes drawn in the diff, after a review in the real program: a removed note comes back
//! with `U`; the key help names the box keys only while boxes show, in line with the rest;
//! the footer counts nothing in the whole file and says when the pane is too short for
//! boxes; a note being edited leaves no rows of its old box under the comment box; `c` on a
//! box comments on its lines; a note saved on the pane's last row says where its box is.

use plannotator_tui_review::Side;
use ratatui::crossterm::event::KeyCode;

use super::compose_near::draw;
use super::inline_flow::{SIZE, all, opened, set_notes};
use super::inline_support::{box_saying, cells, check_boxes, comment, row_of_cursor, row_of_spot, spot};
use super::*;
use crate::config::review::Notes;
use crate::review::{Draft, Mode};
use crate::ui::compose::Compose;

/// The screen row of new line `line` of file0.py in the last frame.
fn row_of_new(screen: &Screen, line: u32) -> Option<u16> {
    let spot = ("file0.py".to_owned(), None, Some(line), "Code".to_owned());
    row_of_spot(screen, &spot)
}

#[test]
fn a_removed_note_comes_back_with_u_where_it_was_as_sent_as_it_was_and_nothing_moves() {
    let (_fixture, mut screen) = opened("fix-undo", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (20, 20), "keep me");
    comment(&mut screen, "file0.py", Side::New, (24, 24), "after it");
    let id = screen.store.annotations().first().map(|n| n.id.clone()).expect("a note");
    screen.store.record_delivery("agent", std::slice::from_ref(&id)).expect("sent");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "before");
    // Onto its box, and Delete.
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    draw(&mut screen, SIZE);
    let line = row_of_new(&screen, 20).expect("line 20 on screen");
    let next = row_of_new(&screen, 21).expect("line 21 on screen");
    press(&mut screen, KeyCode::Char('j'));
    assert_eq!(screen.cursor_note().map(|n| n.id.as_str()), Some(id.as_str()), "on its box");
    press(&mut screen, KeyCode::Delete);
    assert_eq!(screen.status.as_deref(), Some("note removed \u{b7} U undo"));
    assert_eq!(screen.store.annotations().len(), 1);
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "removed");
    assert_eq!(row_of_new(&screen, 20), Some(line), "its line stays where it was");
    assert!(box_saying(&screen, "keep me").is_none());
    // The review menu's undo row says what it takes back.
    keys(&mut screen, "m");
    assert!(all(&draw(&mut screen, SIZE)).contains("Undo remove"));
    press(&mut screen, KeyCode::Esc);
    keys(&mut screen, "U");
    assert_eq!(screen.status.as_deref(), Some("note restored"));
    let notes = screen.store.annotations();
    assert_eq!(notes.first().map(|n| n.id.as_str()), Some(id.as_str()), "first again, as it was");
    assert!(notes.first().is_some_and(|n| !screen.store.is_pending(n)), "still sent");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "restored");
    assert!(box_saying(&screen, "keep me").is_some(), "its box is back");
    assert_eq!(row_of_new(&screen, 20), Some(line), "its line did not move");
    assert!(row_of_new(&screen, 21).is_some_and(|row| row == next), "nor the line under the box");
    keys(&mut screen, "U");
    assert_eq!(screen.status.as_deref(), Some("nothing to undo"));
    // From the panel too.
    open_notes(&mut screen);
    press(&mut screen, KeyCode::Tab);
    assert_eq!(screen.focus, crate::review::Focus::Notes);
    keys(&mut screen, "x");
    assert_eq!(screen.store.annotations().len(), 1);
    keys(&mut screen, "U");
    assert_eq!(screen.store.annotations().len(), 2, "{:?}", screen.status);
}

#[test]
fn the_key_help_names_the_box_keys_only_while_boxes_show_in_line_with_the_rest() {
    let help = |screen: &mut Screen| {
        keys(screen, "?");
        let text = all(&draw(screen, SIZE));
        press(screen, KeyCode::Esc);
        text
    };
    let (_fixture, mut screen) = opened("fix-help", "layout = \"unified\"");
    let panel = help(&mut screen);
    assert!(panel.contains("comment on the file") && !panel.contains("note's box"), "{panel}");
    set_notes(&mut screen, Notes::Inline);
    let inline = help(&mut screen);
    let column = |what: &str| {
        inline.lines().find_map(|row| {
            let at = row.find(what)?;
            Some(row.get(..at)?.chars().count())
        })
    };
    let edit = column("on a note's box in the diff: edit it").expect("the edit row");
    let remove = column("on a note's box: remove it").expect("the remove row");
    let file = column("comment on the file").expect("a row beside");
    assert_eq!((edit, remove), (file, file), "lined up with the rows around:\n{inline}");
}

#[test]
fn the_footer_counts_nothing_in_the_whole_file_and_says_when_the_pane_is_too_short() {
    let (_fixture, mut screen) = opened("fix-small", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (20, 20), "a box");
    keys(&mut screen, "M");
    type_and_save(&mut screen, "on everything");
    let text = all(&draw(&mut screen, SIZE));
    assert!(text.contains("1 note not shown inline"), "{text}");
    keys(&mut screen, "o");
    assert!(screen.whole.is_some(), "the whole file");
    let text = all(&draw(&mut screen, SIZE));
    assert!(!text.contains("not shown inline"), "{text}");
    keys(&mut screen, "o");
    draw(&mut screen, SIZE);
    // The cursor on line 20, its box under it.
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    draw(&mut screen, SIZE);
    let line = row_of_new(&screen, 20).expect("on screen");
    let cursor = row_of_cursor(&screen);
    // Too short: no boxes, and one phrase for it.
    for rows in [11, 6] {
        let small = (SIZE.0, rows);
        let text = all(&draw(&mut screen, small));
        assert!(screen.geometry.diff.height < crate::review::inline::MIN_PANE_ROWS, "{rows}");
        assert!(screen.diff.boxes_set().is_empty() && screen.geometry.boxes.is_empty(), "{rows}");
        assert!(text.contains("pane too small for inline notes"), "{rows}: {text}");
        assert!(!text.contains("not shown inline"), "{rows}: {text}");
    }
    // Tall enough again: the boxes, and the cursor's line where it was.
    let buffer = draw(&mut screen, (SIZE.0, 12));
    assert!(screen.geometry.diff.height >= crate::review::inline::MIN_PANE_ROWS);
    assert!(!screen.diff.boxes_set().is_empty(), "boxes at the least height");
    check_boxes(&screen, &buffer, "least height");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "grown");
    assert!(box_saying(&screen, "a box").is_some());
    assert_eq!(row_of_new(&screen, 20), Some(line), "its line's row again");
    assert_eq!(row_of_cursor(&screen), cursor);
    assert!(!all(&buffer).contains("pane too small"));
}

#[test]
fn a_note_being_edited_leaves_no_rows_of_its_old_box_under_the_comment_box() {
    let (_fixture, mut screen) = opened("fix-edit", "layout = \"unified\"\nnotes = \"inline\"");
    let long = "a long note that takes a few rows of its box: ".repeat(6);
    comment(&mut screen, "file0.py", Side::New, (20, 20), &long);
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    draw(&mut screen, SIZE);
    press(&mut screen, KeyCode::Char('j'));
    assert!(screen.cursor_note().is_some(), "on its box");
    draw(&mut screen, SIZE);
    let old = box_saying(&screen, &long).expect("drawn");
    assert!(old.height >= 6, "{old:?}");
    press(&mut screen, KeyCode::Enter);
    let Mode::Compose(draft) = screen.mode.clone() else { panic!("editing") };
    assert!(matches!(draft, Draft::Edit(_)));
    screen.compose = Compose::with_text("short");
    let buffer = draw(&mut screen, SIZE);
    let pane = screen.geometry.diff;
    let compose = screen.compose_rect(pane, &draft);
    for y in old.top()..old.bottom() {
        for x in old.left()..old.right() {
            if compose.contains((x, y).into()) {
                continue;
            }
            let cell = cells(&buffer, x, y, 1);
            assert_eq!(cell, " ", "({x}, {y}) of the old box, outside the comment box {compose:?}");
        }
    }
    press(&mut screen, KeyCode::Enter);
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "saved short");
    assert!(box_saying(&screen, "short").is_some_and(|b| b.height == 3));
}

#[test]
fn c_on_a_box_comments_on_its_lines_and_the_new_box_stacks_under_it() {
    let (_fixture, mut screen) = opened("fix-c", "layout = \"split\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (19, 20), "first");
    comment(&mut screen, "file0.py", Side::Old, (22, 22), "an old one");
    draw(&mut screen, SIZE);
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    press(&mut screen, KeyCode::Char('j'));
    assert!(screen.cursor_note().is_some_and(|n| n.text.as_deref() == Some("first")));
    keys(&mut screen, "c");
    let Mode::Compose(Draft::Comment(lines)) = screen.mode.clone() else {
        panic!("a comment: {:?}", screen.status)
    };
    assert_eq!((lines.side, lines.first, lines.last), (Side::New, 19, 20));
    type_and_save(&mut screen, "second");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "stacked");
    let (first, second) = (box_saying(&screen, "first"), box_saying(&screen, "second"));
    let (first, second) = (first.expect("first drawn"), second.expect("second drawn"));
    assert_eq!((second.x, second.y), (first.x, first.bottom()), "under the first");
    // An old line's box: a comment on the old side; a suggestion is for new lines.
    screen.diff.reveal("file0.py", Side::Old, 22).expect("old 22");
    press(&mut screen, KeyCode::Char('h'));
    press(&mut screen, KeyCode::Char('j'));
    assert!(
        screen.cursor_note().is_some_and(|n| n.text.as_deref() == Some("an old one")),
        "{:?}",
        spot(&screen)
    );
    keys(&mut screen, "s");
    assert_eq!(screen.mode, Mode::Browse);
    assert!(screen.status.as_deref().is_some_and(|s| s.contains("new lines")), "{:?}", screen.status);
    keys(&mut screen, "c");
    let Mode::Compose(Draft::Comment(lines)) = screen.mode.clone() else { panic!("a comment") };
    assert_eq!((lines.side, lines.first, lines.last), (Side::Old, 22, 22));
    press(&mut screen, KeyCode::Esc);
}

#[test]
fn a_note_saved_on_the_panes_last_row_says_its_box_is_below_and_nothing_moves() {
    let (_fixture, mut screen) = opened("fix-below", "layout = \"unified\"\nnotes = \"inline\"");
    draw(&mut screen, SIZE);
    let bottom = screen.geometry.diff.bottom() - 1;
    for _ in 0..60 {
        if row_of_cursor(&screen) == Some(bottom) {
            break;
        }
        press(&mut screen, KeyCode::Char('j'));
        draw(&mut screen, SIZE);
    }
    assert_eq!(row_of_cursor(&screen), Some(bottom), "the cursor on the last row");
    let at = spot(&screen);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "down here");
    draw(&mut screen, SIZE);
    assert_eq!(screen.status.as_deref(), Some("note saved \u{b7} below, \u{2193} shows it"));
    assert_eq!((spot(&screen), row_of_cursor(&screen)), (at, Some(bottom)), "nothing moved");
    assert!(box_saying(&screen, "down here").is_none());
    press(&mut screen, KeyCode::Down);
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "shown");
    assert!(box_saying(&screen, "down here").is_some(), "Down shows it");
    // A note whose box shows says only that it was saved.
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    draw(&mut screen, SIZE);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "in view");
    draw(&mut screen, SIZE);
    assert_eq!(screen.status.as_deref(), Some("note saved"));
}

#[test]
fn c_on_a_file_notes_box_opens_under_the_headings_boxes_lined_up_and_s_says_why_not() {
    for layout in ["unified", "split"] {
        let (_fixture, mut screen) =
            opened(&format!("fix-file-{layout}"), &format!("layout = \"{layout}\"\nnotes = \"inline\""));
        keys(&mut screen, "C");
        type_and_save(&mut screen, "about the whole file");
        screen.diff.show_heading(0).expect("heading");
        draw(&mut screen, SIZE);
        press(&mut screen, KeyCode::Char('j'));
        assert!(
            screen.cursor_note().is_some_and(|n| n.scope == plannotator_tui_review::NoteScope::File),
            "{layout}: on the file note's box"
        );
        draw(&mut screen, SIZE);
        let boxed = box_saying(&screen, "about the whole file").expect("drawn");
        keys(&mut screen, "s");
        assert_eq!(screen.mode, Mode::Browse, "{layout}");
        assert_eq!(
            screen.status.as_deref(),
            Some("a suggestion needs lines of code; c adds a file comment"),
            "{layout}"
        );
        keys(&mut screen, "c");
        let Mode::Compose(draft) = screen.mode.clone() else { panic!("{layout}: a file comment") };
        assert!(matches!(&draft, Draft::File(path) if path == "file0.py"), "{layout}: {draft:?}");
        let buffer = draw(&mut screen, SIZE);
        let compose = screen.compose_rect(screen.geometry.diff, &draft);
        assert_eq!((compose.x, compose.width), (boxed.x, boxed.width), "{layout}: lined up with the box");
        assert_eq!(compose.y, boxed.bottom(), "{layout}: right under the box, over none of it");
        assert_eq!(cells(&buffer, boxed.x, boxed.y, 1), "\u{256d}", "{layout}: the box still shows");
        press(&mut screen, KeyCode::Esc);
    }
}

#[test]
fn the_key_help_fits_forty_rows_in_both_modes() {
    for notes in [Notes::Panel, Notes::Inline] {
        let (_fixture, mut screen) = opened(&format!("fix-help-{notes:?}"), "layout = \"unified\"");
        set_notes(&mut screen, notes);
        keys(&mut screen, "?");
        let text = all(&draw(&mut screen, (SIZE.0, 40)));
        assert_eq!(screen.help_room.get(), 0, "{notes:?}: nothing below the fold:\n{text}");
        assert!(text.contains("remove") && text.contains("undo a finish or a removal"), "{notes:?}:\n{text}");
        press(&mut screen, KeyCode::Esc);
    }
}
