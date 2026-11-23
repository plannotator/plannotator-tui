//! Go to definition (`d`), its lists, and going back (`ctrl+o`) and forward (`ctrl+i`).

use std::time::{Duration, Instant};

use plannotator_tui_review::Side;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::whole_flow::settle_whole;
use super::*;
use crate::review::Mode;

const BASE_APP: &str = "import sys\nfrom legacy import old_helper\n\n\ndef greet(name):\n    old_helper()\n    print(\"hello\", name)\n";
const NEW_APP: &str = "import sys\nfrom helpers import shout\n\n\ndef greet(name):\n    print(\"hello,\", shout(name))\n\n\ndef add(a, b):\n    total = a + b\n    return total\n\n\ndef main():\n    greet(sys.argv[1])\n    lonely_name_xyz = 1\n";
const HELPERS: &str = "\"\"\"Helpers.\"\"\"\n\n\ndef shout(text):\n    return text.upper()\n";
const OTHER: &str = "def greet():\n    return None\n";

fn legacy(header: &str) -> String {
    let body: Vec<String> = (2..=11).map(|i| format!("X{i} = {i}")).collect();
    format!("# {header}\n{}\ndef old_helper():\n    return 1\n", body.join("\n"))
}

/// `app.py` calls into `helpers.py` (unchanged), `other.py` defines another `greet`, and
/// `legacy.py` keeps `old_helper` at line 12, outside its one hunk.
fn fixture(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    fixture.write("app.py", BASE_APP);
    fixture.write("helpers.py", HELPERS);
    fixture.write("other.py", OTHER);
    fixture.write("legacy.py", &legacy("legacy"));
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "base"]);
    fixture.write("app.py", NEW_APP);
    fixture.write("legacy.py", &legacy("old code"));
    fixture
}

fn ctrl(screen: &mut Screen, c: char) {
    screen.handle_event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))).expect("key");
}

/// Tick until the search has answered.
fn settle_search(screen: &mut Screen) {
    let started = Instant::now();
    while screen.nav.notice().is_some() {
        screen.tick().expect("tick");
        assert!(started.elapsed() < Duration::from_secs(10), "the search never answered");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn cursor(screen: &Screen) -> (String, Side, u32) {
    let lines = screen.diff.cursor().expect("cursor").expect("on a line");
    (lines.path, lines.side, lines.first)
}

#[test]
fn d_on_a_line_with_one_identifier_goes_to_its_only_other_place_in_the_diff_and_ctrl_o_comes_back() {
    let fixture = fixture("jump-diff");
    let mut screen = fixture.open();
    at(&mut screen, 11);
    keys(&mut screen, "d");
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[39].starts_with(" searching for total\u{2026}"), "{:?}", rows[39]);
    settle_search(&mut screen);
    assert_eq!(cursor(&screen), ("app.py".into(), Side::New, 10), "total = a + b");
    assert!(screen.whole.is_none(), "the diff shows that line");
    // Python's patterns do not call an assignment a definition: the one use is the answer.
    assert_eq!(screen.status.as_deref(), Some("use: app.py:10 \u{b7} ctrl+o back"));

    ctrl(&mut screen, 'o');
    assert_eq!(cursor(&screen), ("app.py".into(), Side::New, 11));
    ctrl(&mut screen, 'i');
    assert_eq!(cursor(&screen), ("app.py".into(), Side::New, 10));
    ctrl(&mut screen, 'i');
    assert_eq!(screen.status.as_deref(), Some("no later place"));
}

#[test]
fn several_identifiers_are_offered_left_to_right_and_a_definition_outside_the_diff_opens_the_whole_file() {
    let fixture = fixture("jump-whole");
    let mut screen = fixture.open();
    at(&mut screen, 6);
    keys(&mut screen, "d");
    let rows = frame(&mut screen, 120, 40);
    let title = rows.iter().position(|r| r.contains(" go to the definition of ")).expect("the word list");
    let words: Vec<Vec<&str>> = rows[title + 1..title + 4]
        .iter()
        .map(|r| {
            // The list's own segment: a number, then the word (a file list may sit on the left).
            let segment = r.split('\u{2502}').find(|s| {
                let words: Vec<&str> = s.split_whitespace().collect();
                words.len() == 2 && words.first().is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
            });
            segment.unwrap_or_default().split_whitespace().collect()
        })
        .collect();
    assert_eq!(words, [["1", "print"], ["2", "shout"], ["3", "name"]], "{rows:#?}");
    keys(&mut screen, "2");
    assert_eq!(screen.mode, Mode::Browse);
    settle_search(&mut screen);
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Whole file · new · helpers.py "), "{:?}", rows[0]);
    assert!(rows[1].contains(" Not in the diff "), "{:?}", rows[1]);
    assert_eq!(screen.whole.as_ref().and_then(|w| w.target().expect("target")), Some((4, 4)));
    assert!(diff_side(&screen, row_with(&rows, "def shout(text):")).starts_with('\u{258e}'), "{rows:#?}");

    // From the whole file, `d` searches again; ctrl+o goes back through both views.
    keys(&mut screen, "j");
    keys(&mut screen, "d");
    let rows = frame(&mut screen, 120, 40);
    assert!(rows.iter().any(|r| r.contains(" go to the definition of ")), "text, upper: {rows:#?}");
    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse);
    ctrl(&mut screen, 'o');
    assert!(screen.whole.is_none());
    assert_eq!(cursor(&screen), ("app.py".into(), Side::New, 6));
    ctrl(&mut screen, 'i');
    assert_eq!(screen.whole.as_ref().map(|w| w.path.as_str()), Some("helpers.py"));
}

#[test]
fn several_answers_are_listed_with_their_kind_and_text_and_enter_goes_to_one() {
    let fixture = fixture("jump-list");
    let mut screen = fixture.open();
    at(&mut screen, 15);
    keys(&mut screen, "d1");
    settle_search(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    let Mode::Pick(_) = &screen.mode else { panic!("a list: {rows:#?}") };
    assert!(row_with(&rows, " greet: ").contains("places"), "{rows:#?}");
    let app = row_with(&rows, "app.py:5 ");
    assert!(app.contains("definition") && app.contains("def greet(name):"), "{app:?}");
    let other = row_with(&rows, "other.py:1 ");
    assert!(other.contains("definition") && other.contains("def greet():"), "{other:?}");
    let first = rows.iter().position(|r| r.contains("app.py:5 ")).expect("row");
    assert!(
        first < rows.iter().position(|r| r.contains("other.py:1 ")).expect("row"),
        "a changed file first"
    );
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Whole file · new · other.py "), "{:?}", rows[0]);
    ctrl(&mut screen, 'o');
    assert_eq!(cursor(&screen), ("app.py".into(), Side::New, 15));

    // Esc closes the list without going anywhere.
    keys(&mut screen, "d1");
    settle_search(&mut screen);
    press(&mut screen, KeyCode::Esc);
    assert_eq!((screen.mode.clone(), cursor(&screen)), (Mode::Browse, ("app.py".into(), Side::New, 15)));
}

#[test]
fn nothing_found_says_so_and_the_old_side_searches_the_base() {
    let fixture = fixture("jump-base");
    let mut screen = fixture.open();
    at(&mut screen, 16);
    keys(&mut screen, "d");
    settle_search(&mut screen);
    assert_eq!(screen.status.as_deref(), Some("no definition or use of lonely_name_xyz found"));
    assert_eq!(cursor(&screen), ("app.py".into(), Side::New, 16), "it stays");

    screen.diff.reveal("app.py", Side::Old, 6).expect("the removed line");
    keys(&mut screen, "d");
    settle_search(&mut screen);
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Whole file · base ("), "the base, where it is defined: {:?}", rows[0]);
    assert!(rows[0].contains(" · legacy.py "), "{:?}", rows[0]);
    assert_eq!(screen.whole.as_ref().and_then(|w| w.target().expect("target")), Some((12, 12)));
    assert!(diff_side(&screen, row_with(&rows, "def old_helper():")).starts_with('\u{258e}'));
    // The line under the cursor has nothing to search for.
    keys(&mut screen, "kk");
    let blank = screen.whole.as_ref().and_then(|w| w.target().expect("target"));
    assert_eq!(blank, Some((10, 10)));
    keys(&mut screen, "d");
    settle_search(&mut screen);
    assert_eq!(screen.status.as_deref(), Some("no definition or use of X10 found in the base"));
}

#[test]
fn the_help_lists_the_jump_keys_and_scrolls_when_the_terminal_is_short() {
    let fixture = fixture("jump-help");
    let mut screen = fixture.open();
    keys(&mut screen, "?");
    for (cols, rows) in [(160, 40), (120, 34)] {
        let mut frame = frame(&mut screen, cols, rows);
        if cols == 160 {
            assert!(
                frame.iter().any(|r| r.contains(" anywhere ") && r.contains(" diff ")),
                "two columns: {frame:#?}"
            );
        } else {
            assert!(row_with(&frame, "j/k scroll").contains("esc closes"), "{frame:#?}");
            // Every row is reachable by scrolling: gather what each step shows.
            for _ in 0..60 {
                keys(&mut screen, "j");
                frame.extend(super::frame(&mut screen, cols, rows));
            }
            let last = super::frame(&mut screen, cols, rows);
            assert!(!last.iter().any(|r| r.contains("send new and edited notes")), "scrolled past the top");
        }
        for needle in [
            "go to the definition of a word on the line",
            "ctrl+o ctrl+i back / forward to where you jumped from",
            "go to definition; on the base it searches the base",
            " notes ",
            "edit",
        ] {
            assert!(frame.iter().any(|r| r.contains(needle)), "{rows} rows, {needle:?}: {frame:#?}");
        }
    }
}

#[test]
fn with_the_notes_rail_closed_the_lists_and_the_whole_file_use_the_room() {
    let fixture = fixture("jump-rail");
    let mut screen = fixture.open();
    at(&mut screen, 11);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "return it directly");
    let rows = frame(&mut screen, 120, 40);
    assert!(rows.iter().any(|r| r.contains("return it directly")), "the rail shows the note: {rows:#?}");
    keys(&mut screen, "z");
    at(&mut screen, 15);
    keys(&mut screen, "d1");
    settle_search(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|r| r.contains("return it directly")), "the rail stays closed: {rows:#?}");
    let app = row_with(&rows, "app.py:5 ");
    assert!(
        app.contains("def greet(name):") && row_with(&rows, "other.py:1 ").contains("definition"),
        "{rows:#?}"
    );
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    settle_whole(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[0].starts_with(" Whole file · new · other.py "), "{:?}", rows[0]);
    // The file's lines run to the right edge, where the rail was.
    assert!(rows[1].trim_end().ends_with("Not in the diff"), "{:?}", rows[1]);
    assert!(rows[1].len() >= 118, "{:?}", rows[1]);
    assert!(rows[39].contains("notes hidden: z shows them"), "{:?}", rows[39]);
    ctrl(&mut screen, 'o');
    assert_eq!(cursor(&screen), ("app.py".into(), Side::New, 15));
}
