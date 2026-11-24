//! The comment box and the diff's settings agree. With the line between a split diff's sides
//! moved, the box sits over the side of its lines, from where the viewer drew that line. While
//! the box is open the settings stay as they are: `,`, `(`, `)`, `u` and `w` are text in the
//! draft, and a click on the header's switches or its Settings button, or a drag of the line
//! between the sides, does nothing. The settings menu and the box are never open together.

use plannotator_tui_review::Side;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::layout::Rect;

use super::compose_near::{COLS, ROWS, drag, draw, file0, long, report, the_box, tinted, within};
use super::*;
use crate::review::{Draft, Mode};

const SIZE: (u16, u16) = (COLS, ROWS);

/// The box's widest.
const WIDEST: u16 = 84;

/// A long review, split, its middle line moved with `(` or `)` to `permille` of the width.
fn uneven(tag: &str, permille: u16) -> (Fixture, Screen) {
    let fixture = long(tag, false);
    let mut screen = fixture.open();
    draw(&mut screen, SIZE);
    assert!(screen.diff.split(), "auto at {COLS} columns");
    let key = if permille < 500 { '(' } else { ')' };
    for _ in 0..(permille.abs_diff(500) / 50) {
        press(&mut screen, KeyCode::Char(key));
    }
    assert_eq!(screen.settings.now.split_ratio.permille(), permille);
    (fixture, screen)
}

/// The line between the sides as drawn, checked against the ratio: `permille` of the cells
/// beside it are left of it.
fn middle(screen: &mut Screen, permille: u16) -> u16 {
    draw(screen, SIZE);
    let pane = screen.geometry.diff;
    let line = screen.diff.split_line().expect("split");
    let expected = u32::from(pane.width - 1) * u32::from(permille) / 1000;
    assert!(u32::from(line - pane.x).abs_diff(expected) <= 1, "{line} in {pane:?} at {permille}");
    line
}

/// What the settings are, and how the box and the diff are drawn: equal before and after
/// means nothing changed.
fn state(screen: &mut Screen) -> (crate::config::review::ReviewConfig, bool, bool, Option<u16>, Rect) {
    let buffer = draw(screen, SIZE);
    let found = the_box(&buffer, "comment on");
    (screen.settings.now, screen.diff.split(), screen.diff.options().wrap, screen.diff.split_line(), found)
}

#[test]
fn at_an_uneven_split_the_box_sits_over_its_lines_side_from_the_drawn_middle_line() {
    for permille in [250, 750] {
        for side in [Side::Old, Side::New] {
            let (_fixture, mut screen) = uneven(&format!("uneven-{permille}-{side:?}"), permille);
            let line = middle(&mut screen, permille);
            drag(&mut screen, SIZE, side, 18, 21);
            let selected = tinted(&draw(&mut screen, SIZE), screen.geometry.diff);
            press(&mut screen, KeyCode::Char('c'));
            let buffer = draw(&mut screen, SIZE);
            let (pane, found) = (screen.geometry.diff, the_box(&buffer, "comment on"));
            let case = format!("{permille} {side:?}: {found:?}, line at {line}, {pane:?}");
            assert!(within(found, pane), "{case}");
            assert_eq!(found.y, selected.last().expect("selected") + 1, "{case}: right under");
            assert_eq!(tinted(&buffer, pane), selected, "{case}: the lines still show");
            // A side of three quarters holds the box; one of a quarter is too narrow, and the
            // box is as wide as it gets, from that side's outer edge.
            let room = if side == Side::Old { line - pane.x } else { pane.right() - line - 1 };
            assert_eq!(room >= 48, (permille > 500) == (side == Side::Old), "{case}");
            match (permille, side) {
                (750, Side::Old) => assert!(found.x == pane.x && found.right() <= line, "{case}"),
                (250, Side::New) => assert!(found.x == line + 1 && found.right() <= pane.right(), "{case}"),
                (250, Side::Old) => assert_eq!((found.x, found.width), (pane.x, WIDEST), "{case}"),
                _ => assert_eq!((found.right(), found.width), (pane.right(), WIDEST), "{case}"),
            }
            type_and_save(&mut screen, "a note");
            let note = screen.store.annotations().first().expect("saved");
            assert_eq!((note.side, note.line_start, note.line_end), (side, 18, 21));
        }
    }
}

#[test]
fn the_settings_keys_are_text_in_an_open_draft_and_the_menu_and_the_box_never_share_the_screen() {
    let (_fixture, mut screen) = uneven("draft-keys", 350);
    middle(&mut screen, 350);
    drag(&mut screen, SIZE, Side::New, 18, 21);
    press(&mut screen, KeyCode::Char('c'));
    let before = state(&mut screen);
    keys(&mut screen, ",()uw");
    assert_eq!(screen.mode, Mode::Compose(Draft::Comment(file0(Side::New, 18, 21))));
    assert_eq!(screen.compose.value(), ",()uw");
    assert_eq!(state(&mut screen), before, "the settings and the box stay");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(screen.store.annotations().first().and_then(|note| note.text.as_deref()), Some(",()uw"));
    assert_eq!(screen.settings.now, before.0);

    // In the menu, the keys that open the box do nothing.
    keys(&mut screen, ",");
    assert_eq!(screen.mode, Mode::Settings(0));
    keys(&mut screen, "cCMs");
    assert!(matches!(screen.mode, Mode::Settings(_)), "{:?}", screen.mode);
}

#[test]
fn with_a_draft_open_the_header_switches_settings_button_and_middle_line_do_nothing() {
    let (_fixture, mut screen) = uneven("draft-clicks", 350);
    let line = middle(&mut screen, 350);
    drag(&mut screen, SIZE, Side::New, 18, 21);
    press(&mut screen, KeyCode::Char('c'));
    screen.handle_event(Event::Paste("keep me".into())).expect("paste");
    let before = state(&mut screen);
    let geometry = &screen.geometry;
    let mut targets: Vec<(Rect, &str)> =
        geometry.layout_switch.iter().map(|(rect, _)| (*rect, "layout")).collect();
    targets.push((geometry.wrap_switch.expect("the wrap switch"), "wrap"));
    targets.push((geometry.settings_button.expect("the settings button"), "settings"));
    assert_eq!(targets.len(), 4, "both halves of the layout switch");
    for (rect, what) in targets {
        for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
            report(&mut screen, kind, (rect.x, rect.y));
        }
        assert!(matches!(screen.mode, Mode::Compose(_)), "{what}: {:?}", screen.mode);
        assert_eq!(screen.compose.value(), "keep me", "{what}");
        assert_eq!(state(&mut screen), before, "{what}");
    }
    // A drag of the middle line, on a row it is drawn on outside the box.
    let buffer = draw(&mut screen, SIZE);
    let pane = screen.geometry.diff;
    let row = (pane.top()..pane.bottom())
        .find(|&y| {
            !(before.4.top()..before.4.bottom()).contains(&y) && buffer[(line, y)].symbol() == "\u{2502}"
        })
        .expect("a row with the middle line");
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), (line, row));
    report(&mut screen, MouseEventKind::Drag(MouseButton::Left), (line + 30, row));
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), (line + 30, row));
    assert!(!screen.dividers.dragging_split());
    assert_eq!(screen.compose.value(), "keep me");
    assert_eq!(state(&mut screen), before, "the middle line stayed");
}
