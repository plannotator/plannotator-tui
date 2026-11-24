//! The diff's settings on the screen: the header's layout and wrap switches by click and by
//! key, the settings menu (`,` or its button) changing each setting live, Reset, and the
//! person's choices remembered in the data folder for the next review, over `[review]` in
//! the config file, which the app never writes.

use std::path::Path;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::*;
use crate::config::review::{Background, DiffTheme, Layout, ReviewConfig, SplitShare};
use crate::review::Mode;
use crate::review::settings::{Settings, state};
use crate::ui::chips;

const COLS: u16 = 160;
const ROWS: u16 = 30;

/// One frame at `cols`×`rows`, with its styles.
fn buffer(screen: &mut Screen, cols: u16, rows: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width).filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned())).collect()
}

fn mouse(screen: &mut Screen, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

fn click(screen: &mut Screen, rect: Rect) {
    mouse(screen, MouseEventKind::Down(MouseButton::Left), rect.x, rect.y);
    mouse(screen, MouseEventKind::Up(MouseButton::Left), rect.x, rect.y);
}

/// The layout switch's half for split (`true`) or unified, as last drawn.
fn half(screen: &Screen, split: bool) -> Rect {
    screen.geometry.layout_switch.iter().find(|(_, s)| *s == split).map(|(r, _)| *r).expect("the switch")
}

/// Whether the switch half at `rect` is drawn lit.
fn lit(buffer: &Buffer, rect: Rect) -> bool {
    buffer
        .cell((rect.x, rect.y))
        .is_some_and(|cell| cell.bg == chips::chips().switch_on.bg.expect("a background"))
}

/// The settings the next review would open with, from the remembered choices alone.
fn remembered(fixture: &Fixture) -> ReviewConfig {
    let (settings, notice) = state::load(&fixture.state(), ReviewConfig::DEFAULT);
    assert_eq!(notice, None);
    settings
}

/// The review screen remembering its choices at `path`.
fn open_remembering(fixture: &Fixture, path: &Path) -> Screen {
    let (repo, capture) =
        crate::review::capture(&fixture.repo(), &Request::Uncommitted { base: None }).expect("capture");
    let (wake, _ready) = std::sync::mpsc::sync_channel(1);
    let settings = Settings::load(ReviewConfig::DEFAULT, Some(path.to_owned()));
    let mut screen =
        Screen::open_with(repo, capture, &fixture.data(), Box::new(fixture.agent.clone()), wake, settings)
            .expect("screen");
    settle(&mut screen);
    screen
}

#[test]
fn the_header_switch_shows_the_layout_and_a_click_or_u_switches_it() {
    let fixture = Fixture::new("settings-switch");
    let mut screen = fixture.open_configured();
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(text(&drawn, 0).contains(" Split  Unified "), "{:?}", text(&drawn, 0));
    assert!(screen.diff.split(), "auto at 160 columns is split");
    assert!(lit(&drawn, half(&screen, true)) && !lit(&drawn, half(&screen, false)));

    // Clicking the layout in force changes nothing and remembers nothing.
    let rect = half(&screen, true);
    click(&mut screen, rect);
    assert!(screen.diff.split());
    assert!(!fixture.state().exists(), "nothing changed, nothing written");

    let rect = half(&screen, false);
    click(&mut screen, rect);
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.split());
    assert!(lit(&drawn, half(&screen, false)) && !lit(&drawn, half(&screen, true)));
    assert_eq!(remembered(&fixture).layout, Layout::Unified, "remembered");
    assert!(screen.diff.selection().is_none() && screen.drag.is_none());

    keys(&mut screen, "u");
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(screen.diff.split());
    assert!(lit(&drawn, half(&screen, true)));
    assert_eq!(remembered(&fixture).layout, Layout::Split);
}

#[test]
fn the_wrap_switch_and_w_wrap_long_lines_and_are_remembered() {
    let fixture = Fixture::new("settings-wrap");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    let wrap = screen.geometry.wrap_switch.expect("a wrap switch at 160 columns");
    click(&mut screen, wrap);
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(screen.diff.options().wrap);
    assert!(lit(&drawn, screen.geometry.wrap_switch.expect("switch")));
    assert!(remembered(&fixture).wrap);
    keys(&mut screen, "w");
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.options().wrap);
    assert!(!lit(&drawn, screen.geometry.wrap_switch.expect("switch")));
    assert!(!remembered(&fixture).wrap);
}

#[test]
fn the_settings_menu_changes_every_setting_live_and_remembers_each_change() {
    let fixture = Fixture::new("settings-menu");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    keys(&mut screen, ",");
    assert_eq!(screen.mode, Mode::Settings(0));
    let drawn = buffer(&mut screen, COLS, ROWS);
    let shown: String = (0..ROWS).map(|y| text(&drawn, y)).collect::<Vec<_>>().join("\n");
    for row in ["Layout", "Split ratio", "Wrap", "Line numbers", "Colors", "Background", "Reset to defaults"]
    {
        assert!(shown.contains(row), "{row}:\n{shown}");
    }
    assert!(shown.contains("auto \u{b7} split now") && shown.contains("50 : 50"), "{shown}");
    assert!(shown.contains("remembered for next time"), "{shown}");
    assert!(!shown.contains(".toml"), "no file path: {shown}");
    // Moving through the menu changes nothing.
    keys(&mut screen, "jjk");
    assert!(!fixture.state().exists());
    keys(&mut screen, "k");

    // Layout: auto → split → unified, live.
    press(&mut screen, KeyCode::Right);
    assert_eq!(screen.settings.now.layout, Layout::Split);
    press(&mut screen, KeyCode::Right);
    buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.split());
    assert_eq!(remembered(&fixture).layout, Layout::Unified);
    press(&mut screen, KeyCode::Left);
    press(&mut screen, KeyCode::Left);
    assert_eq!(screen.settings.now.layout, Layout::Auto);
    // The split ratio, by five hundredths, and Enter makes it even.
    keys(&mut screen, "j");
    buffer(&mut screen, COLS, ROWS);
    press(&mut screen, KeyCode::Right);
    assert_eq!(screen.settings.now.split_ratio.permille(), 550);
    press(&mut screen, KeyCode::Enter);
    assert_eq!(screen.settings.now.split_ratio.permille(), 500);
    press(&mut screen, KeyCode::Left);
    buffer(&mut screen, COLS, ROWS);
    assert_eq!(remembered(&fixture).split_ratio.permille(), 450);
    // Wrap with Space, line numbers with Enter, then colours and the background.
    keys(&mut screen, "j ");
    assert!(screen.diff.options().wrap);
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.options().line_numbers);
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Right);
    assert_eq!(screen.settings.now.theme, DiffTheme::Dark);
    press(&mut screen, KeyCode::Right);
    buffer(&mut screen, COLS, ROWS);
    assert_eq!(screen.diff.options().appearance, diffs_rs::theme::Appearance::Light);
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    buffer(&mut screen, COLS, ROWS);
    assert_eq!(screen.settings.now.background, Background::Theme);
    assert_eq!(
        remembered(&fixture),
        ReviewConfig {
            layout: Layout::Auto,
            wrap: true,
            line_numbers: false,
            theme: DiffTheme::Light,
            background: Background::Theme,
            split_ratio: SplitShare::from_permille(450),
        }
    );
    // The last row, Reset, stays the last.
    keys(&mut screen, "jjj");
    assert_eq!(screen.mode, Mode::Settings(6));

    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse);
    keys(&mut screen, ",,");
    assert_eq!(screen.mode, Mode::Browse, "the key closes it too");
}

#[test]
fn reset_forgets_the_choices_and_goes_back_to_the_config_file_or_the_defaults() {
    let fixture = Fixture::new("settings-reset");
    fixture.write_config("[review]\nlayout = \"split\"\nwrap = true\n");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, 100, ROWS);
    assert!(screen.diff.split() && screen.diff.options().wrap, "the config file's values");
    keys(&mut screen, "uw)");
    buffer(&mut screen, 100, ROWS);
    assert!(!screen.diff.split() && !screen.diff.options().wrap);
    assert!(fixture.state().exists());

    // The choices win over the config file in the next review.
    let mut next = fixture.open_configured();
    buffer(&mut next, 100, ROWS);
    assert!(!next.diff.split() && !next.diff.options().wrap);
    // Reset, by Enter on the last row.
    keys(&mut next, ",");
    for _ in 0..6 {
        keys(&mut next, "j");
    }
    press(&mut next, KeyCode::Enter);
    buffer(&mut next, 100, ROWS);
    assert!(next.diff.split() && next.diff.options().wrap, "back to the config file's values");
    assert_eq!(next.settings.now.layout, Layout::Split);
    assert_eq!(next.status.as_deref(), Some("settings back to [review] in the config file"));
    assert!(!fixture.state().exists(), "nothing left to remember");
    // And so the next review opens as the config file says.
    let mut third = fixture.open_configured();
    buffer(&mut third, 100, ROWS);
    assert!(third.diff.split() && third.diff.options().wrap);

    // Without a [review] table, Reset goes back to the defaults, `auto` among them.
    let fixture = Fixture::new("settings-reset-defaults");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    keys(&mut screen, "u");
    assert_eq!(screen.settings.now.layout, Layout::Unified);
    keys(&mut screen, ",");
    buffer(&mut screen, COLS, ROWS);
    let reset = screen.geometry.settings_rows.last().map(|(rect, _)| *rect).expect("the Reset row");
    click(&mut screen, reset);
    assert_eq!(screen.settings.now, ReviewConfig::DEFAULT);
    assert_eq!(screen.status.as_deref(), Some("settings back to the defaults"));
}

#[test]
fn the_settings_button_opens_the_menu_and_a_click_changes_a_row_or_closes_it() {
    let fixture = Fixture::new("settings-click");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    let button = screen.geometry.settings_button.expect("the button");
    click(&mut screen, button);
    assert_eq!(screen.mode, Mode::Settings(0));
    buffer(&mut screen, COLS, ROWS);
    let rows = screen.geometry.settings_rows.clone();
    let boxed = screen.geometry.settings_box.expect("the menu");
    assert!(boxed.x >= screen.geometry.diff.x && boxed.right() <= COLS);
    let (wrap, _) = rows.iter().copied().find(|(_, index)| *index == 2).expect("the wrap row");
    click(&mut screen, wrap);
    assert_eq!(screen.mode, Mode::Settings(2));
    assert!(screen.settings.now.wrap && remembered(&fixture).wrap);
    // A click on the box's border does nothing; one outside closes it, moving nothing.
    click(&mut screen, Rect { x: boxed.x, y: boxed.y, width: 1, height: 1 });
    assert_eq!(screen.mode, Mode::Settings(2));
    let cursor = screen.diff.cursor().expect("cursor");
    click(&mut screen, Rect { x: 2, y: ROWS - 6, width: 1, height: 1 });
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(screen.diff.cursor().expect("cursor"), cursor, "the click only closed the menu");
}

#[test]
fn the_next_review_opens_as_the_last_one_left_it_and_the_config_file_sets_the_rest() {
    let fixture = Fixture::new("settings-next");
    fixture.write_config("[review]\nline_numbers = false\n");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    keys(&mut screen, ")))uw");
    buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.split(), "u at 160 columns: unified");
    let line_after = |screen: &mut Screen| {
        keys(screen, "u");
        buffer(screen, COLS, ROWS);
        let line = screen.diff.split_line();
        keys(screen, "u");
        line
    };
    let mut first = screen;
    let line = line_after(&mut first);
    first.close().expect("close");
    let review = remembered(&fixture);
    assert_eq!((review.layout, review.wrap, review.split_ratio.permille()), (Layout::Unified, true, 650));

    // Unified, wrapped, the ratio set, from the first frame; line numbers as the config says.
    let mut next = fixture.open_configured();
    let drawn = buffer(&mut next, COLS, ROWS);
    assert!(!next.diff.split() && next.diff.options().wrap);
    assert!(!next.diff.options().line_numbers);
    assert!(lit(&drawn, half(&next, false)));
    assert_eq!(line_after(&mut next), line, "the same split ratio");

    // Set by hand in the config file alone.
    let fixture = Fixture::new("settings-by-hand");
    fixture.write_config("[review]\nlayout = \"split\"\nline_numbers = false\nsplit_ratio = 0.3\n");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, 120, ROWS);
    assert!(screen.diff.split(), "split below 120 columns as asked");
    assert!(!screen.diff.options().line_numbers);
    assert_eq!(screen.diff.split_line(), Some(35));
}

#[test]
fn the_config_file_is_never_written_whatever_the_review_does() {
    let fixture = Fixture::new("settings-config-untouched");
    let config = "# mine\r\n[ui]\r\ncopy_on_select = false  # keep it\r\n\r\n[review]\r\nwrap = true\r\n";
    fixture.write_config(config);
    let bytes = std::fs::read(fixture.config()).expect("config");
    let mut screen = fixture.open_configured();
    let check = |screen: &mut Screen, what: &str| {
        buffer(screen, COLS, ROWS);
        assert_eq!(std::fs::read(fixture.config()).expect("config"), bytes, "after {what}");
    };
    check(&mut screen, "opening");
    for key in ["u", "w", "u", ")", "(", "(", "w"] {
        keys(&mut screen, key);
        check(&mut screen, key);
    }
    // The header's switches.
    for split in [false, true] {
        let rect = half(&screen, split);
        click(&mut screen, rect);
        check(&mut screen, "the layout switch");
    }
    let wrap = screen.geometry.wrap_switch.expect("wrap");
    click(&mut screen, wrap);
    check(&mut screen, "the wrap switch");
    // A drag of the line between the sides, and a double click on it.
    let column = screen.diff.split_line().expect("split");
    let row = (2..ROWS - 2).find(|&y| screen.diff.code_row(y)).expect("a code row");
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), column, row);
    mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), column - 20, row);
    mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), column - 20, row);
    check(&mut screen, "a drag");
    let column = screen.diff.split_line().expect("split");
    for _ in 0..2 {
        click(&mut screen, Rect { x: column, y: row, width: 1, height: 1 });
    }
    check(&mut screen, "a double click");
    // Every row of the menu, every way, then Reset.
    keys(&mut screen, ",");
    for _ in 0..crate::review::settings::ROWS.len() - 1 {
        for code in [KeyCode::Right, KeyCode::Left, KeyCode::Enter, KeyCode::Char(' ')] {
            press(&mut screen, code);
            check(&mut screen, &format!("{code:?} in the menu"));
        }
        keys(&mut screen, "j");
    }
    press(&mut screen, KeyCode::Enter);
    check(&mut screen, "Reset");
    screen.close().expect("close");
    assert_eq!(std::fs::read(fixture.config()).expect("config"), bytes, "after closing");
}

#[test]
fn two_reviews_and_a_hand_edit_keep_each_others_choices() {
    let fixture = Fixture::new("settings-two");
    let mut one = fixture.open_configured();
    let mut two = fixture.open_configured();
    buffer(&mut one, COLS, ROWS);
    buffer(&mut two, COLS, ROWS);
    keys(&mut one, "w");
    keys(&mut two, "u");
    let both = remembered(&fixture);
    assert!(both.wrap, "the second review kept the first one's wrap");
    assert_eq!(both.layout, Layout::Unified);
    keys(&mut one, ")");
    let all = remembered(&fixture);
    assert_eq!((all.wrap, all.layout, all.split_ratio.permille()), (true, Layout::Unified, 550));
    // A hand edit while a review is open stays, but for the keys the review changes next.
    let text = read(&fixture.state());
    std::fs::write(fixture.state(), format!("{text}line_numbers = false\n")).expect("edit");
    // The second review never saw the first one's wrap: its `w` turns its own on.
    keys(&mut two, "w");
    let edited = remembered(&fixture);
    assert!(!edited.line_numbers, "the hand edit stays");
    assert!(edited.wrap, "the second review's wrap");
    assert_eq!(edited.layout, Layout::Unified);
    assert_eq!(edited.split_ratio.permille(), 550, "the first review's ratio stays");
}

#[test]
fn a_bad_state_file_is_named_once_and_its_good_keys_still_count() {
    let fixture = Fixture::new("settings-bad-state");
    let path = fixture.state();
    std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
    std::fs::write(&path, "wrap = true\nlayout = \"sideways\"\nsplit_ratio = 3\nzoom = 2\n").expect("write");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    assert!(screen.diff.options().wrap && screen.diff.split(), "wrap taken, layout left at auto");
    let notice = screen.status.clone().unwrap_or_default();
    for key in ["layout", "split_ratio", "zoom"] {
        assert!(notice.contains(key), "{notice}");
    }
    screen.status = None;
    keys(&mut screen, "u");
    buffer(&mut screen, COLS, ROWS);
    assert_eq!(screen.status, None, "said once, not again on a change");
    // Writing keeps the keys it did not change, even ones it skipped.
    let text = read(&path);
    assert!(text.contains("layout = \"unified\"") && text.contains("zoom = 2"), "{text}");

    std::fs::write(&path, "wrap = [\n").expect("write");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    assert!(screen.status.as_deref().is_some_and(|s| s.contains("does not parse")), "{:?}", screen.status);
    assert_eq!(screen.settings.now, ReviewConfig::DEFAULT);
}

#[test]
fn a_state_file_that_cannot_be_written_is_said_once_and_the_setting_still_applies() {
    let fixture = Fixture::new("settings-unwritable");
    // Its folder is created; a folder where the file goes cannot be replaced.
    let fresh = fixture.dir.join("fresh").join("deeper").join("review-settings.toml");
    let mut screen = open_remembering(&fixture, &fresh);
    keys(&mut screen, "w");
    assert!(fresh.is_file(), "the folders are created");
    let blocked = fixture.dir.join("blocked").join("review-settings.toml");
    std::fs::create_dir_all(&blocked).expect("a folder in the way");
    let mut screen = open_remembering(&fixture, &blocked);
    buffer(&mut screen, COLS, ROWS);
    keys(&mut screen, "w");
    buffer(&mut screen, COLS, ROWS);
    assert!(screen.diff.options().wrap);
    let said = screen.status.clone().unwrap_or_default();
    assert!(said.contains("review-settings.toml") && said.contains("hold for this review"), "{said}");
    screen.status = None;
    keys(&mut screen, "u");
    buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.split(), "still applies");
    assert!(screen.status.as_deref().is_none_or(|s| !s.contains("review-settings")), "{:?}", screen.status);
}

#[test]
fn at_60_columns_the_switch_and_the_menu_fit_and_the_file_list_hides() {
    let fixture = Fixture::new("settings-narrow");
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open_configured();
    let drawn = buffer(&mut screen, 60, ROWS);
    assert_eq!(screen.files.drawn_width, 0, "no room for the file list");
    let header = text(&drawn, 0);
    assert!(!screen.geometry.layout_switch.is_empty(), "{header:?}");
    let send = screen.geometry.send_button.expect("send");
    assert!(screen.geometry.layout_switch.iter().all(|(r, _)| r.right() < send.x), "{header:?}");
    let rect = half(&screen, true);
    click(&mut screen, rect);
    buffer(&mut screen, 60, ROWS);
    assert!(screen.diff.split(), "a 60-column diff split when asked");
    keys(&mut screen, ",");
    let drawn = buffer(&mut screen, 60, ROWS);
    let boxed = screen.geometry.settings_box.expect("the menu");
    assert!(boxed.right() <= 60 && boxed.bottom() <= ROWS, "{boxed:?}");
    for (index, row) in crate::review::settings::ROWS.iter().enumerate() {
        let (rect, _) = screen.geometry.settings_rows[index];
        let shown = text(&drawn, rect.y);
        assert!(shown.contains(row.label()), "{shown:?}");
        assert!(shown.contains(&screen.setting_value(*row)), "the whole value: {shown:?}");
    }
}

#[test]
fn the_header_keeps_the_switch_and_review_at_60_80_100_and_120_columns() {
    let fixture = Fixture::new("settings-widths");
    let mut screen = fixture.open_configured();
    for cols in [60, 80, 100, 120] {
        let drawn = buffer(&mut screen, cols, ROWS);
        let header = text(&drawn, 0);
        assert!(!screen.geometry.layout_switch.is_empty(), "{cols}: {header:?}");
        // The fixture's Send label is long (`Send 0 new ▸ test agent (E)`): at 60 the
        // switch alone joins it.
        if cols >= 80 {
            assert!(screen.geometry.review_button.is_some(), "{cols}: {header:?}");
        }
        if screen.geometry.settings_button.is_some() {
            assert!(screen.geometry.review_button.is_some(), "{cols}: {header:?}");
        }
        if screen.geometry.wrap_switch.is_some() {
            assert!(screen.geometry.settings_button.is_some(), "{cols}: {header:?}");
        }
        assert!(header.trim_end().ends_with("(E)"), "{cols}: {header:?}");
    }
}
