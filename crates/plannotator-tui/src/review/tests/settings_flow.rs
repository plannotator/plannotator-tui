//! The diff's settings on the screen: the header's layout and wrap switches by click and by
//! key, the settings menu (`,` or its button) changing each setting live, the config file
//! that remembers them for the next review, and a narrow screen.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::*;
use crate::config::Config;
use crate::config::review::{Background, DiffTheme, Layout, ReviewConfig};
use crate::review::Mode;
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

fn click(screen: &mut Screen, rect: Rect) {
    for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        let event = MouseEvent { kind, column: rect.x, row: rect.y, modifiers: KeyModifiers::NONE };
        screen.handle_event(Event::Mouse(event)).expect("mouse");
    }
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

fn written(fixture: &Fixture) -> ReviewConfig {
    Config::load_from(&fixture.config()).expect("config").review
}

#[test]
fn the_header_switch_shows_the_layout_and_a_click_or_u_switches_it() {
    let fixture = Fixture::new("settings-switch");
    let mut screen = fixture.open_configured();
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(text(&drawn, 0).contains(" Split  Unified "), "{:?}", text(&drawn, 0));
    assert!(screen.diff.split(), "auto at 160 columns is split");
    assert!(lit(&drawn, half(&screen, true)) && !lit(&drawn, half(&screen, false)));

    let rect = half(&screen, false);
    click(&mut screen, rect);
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.split());
    assert!(lit(&drawn, half(&screen, false)) && !lit(&drawn, half(&screen, true)));
    assert_eq!(written(&fixture).layout, Layout::Unified, "remembered");
    assert!(screen.diff.selection().is_none() && screen.drag.is_none());

    // Clicking the layout in force changes nothing, not even the file.
    let before = read(&fixture.config());
    let rect = half(&screen, false);
    click(&mut screen, rect);
    buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.split());
    assert_eq!(read(&fixture.config()), before);

    keys(&mut screen, "u");
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(screen.diff.split());
    assert!(lit(&drawn, half(&screen, true)));
    assert_eq!(written(&fixture).layout, Layout::Split);
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
    assert!(written(&fixture).wrap);
    keys(&mut screen, "w");
    let drawn = buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.options().wrap);
    assert!(!lit(&drawn, screen.geometry.wrap_switch.expect("switch")));
    assert!(!written(&fixture).wrap);
}

#[test]
fn the_settings_menu_changes_every_setting_live_and_writes_each_change() {
    let fixture = Fixture::new("settings-menu");
    fixture.write_config("# mine\n[ui]\ncopy_on_select = false\n");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    keys(&mut screen, ",");
    assert_eq!(screen.mode, Mode::Settings(0));
    let drawn = buffer(&mut screen, COLS, ROWS);
    let shown: String = (0..ROWS).map(|y| text(&drawn, y)).collect::<Vec<_>>().join("\n");
    for row in ["Layout", "Split ratio", "Wrap", "Line numbers", "Colors", "Background"] {
        assert!(shown.contains(row), "{row}:\n{shown}");
    }
    assert!(shown.contains("auto \u{b7} split now") && shown.contains("50 : 50"), "{shown}");
    assert!(shown.contains("saved to config.toml [review]"), "{shown}");

    // Layout: auto → split → unified, live.
    press(&mut screen, KeyCode::Right);
    assert_eq!(screen.settings.now.layout, Layout::Split);
    press(&mut screen, KeyCode::Right);
    buffer(&mut screen, COLS, ROWS);
    assert!(!screen.diff.split());
    assert_eq!(written(&fixture).layout, Layout::Unified);
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
    assert_eq!(written(&fixture).split_ratio.permille(), 450);
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
    // The last row stays the last.
    keys(&mut screen, "jj");
    assert_eq!(screen.mode, Mode::Settings(5));

    let review = written(&fixture);
    assert_eq!(
        review,
        ReviewConfig {
            layout: Layout::Auto,
            wrap: true,
            line_numbers: false,
            theme: DiffTheme::Light,
            background: Background::Theme,
            split_ratio: crate::config::review::SplitShare::from_permille(450),
        }
    );
    let file = read(&fixture.config());
    assert!(file.starts_with("# mine\n[ui]\ncopy_on_select = false\n"), "the rest is kept: {file}");

    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse);
    keys(&mut screen, ",,");
    assert_eq!(screen.mode, Mode::Browse, "the key closes it too");
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
    assert!(screen.settings.now.wrap && written(&fixture).wrap);
    // A click on the box's border does nothing; one outside closes it, moving nothing.
    click(&mut screen, Rect { x: boxed.x, y: boxed.y, width: 1, height: 1 });
    assert_eq!(screen.mode, Mode::Settings(2));
    let cursor = screen.diff.cursor().expect("cursor");
    click(&mut screen, Rect { x: 2, y: ROWS - 6, width: 1, height: 1 });
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(screen.diff.cursor().expect("cursor"), cursor, "the click only closed the menu");
}

#[test]
fn the_next_review_opens_as_the_last_one_left_it_and_the_file_can_say_so_too() {
    let fixture = Fixture::new("settings-next");
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
    let review = written(&fixture);
    assert_eq!((review.layout, review.wrap), (Layout::Unified, true));

    // Unified, wrapped, the ratio set, from the first frame.
    let mut next = fixture.open_configured();
    let drawn = buffer(&mut next, COLS, ROWS);
    assert!(!next.diff.split() && next.diff.options().wrap);
    assert!(lit(&drawn, half(&next, false)));
    assert_eq!(line_after(&mut next), line, "the same split ratio");

    // Set by hand in the file.
    let fixture = Fixture::new("settings-by-hand");
    fixture.write_config("[review]\nlayout = \"split\"\nline_numbers = false\nsplit_ratio = 0.3\n");
    let mut screen = fixture.open_configured();
    buffer(&mut screen, 120, ROWS);
    assert!(screen.diff.split(), "split below 120 columns as asked");
    assert!(!screen.diff.options().line_numbers);
    assert_eq!(screen.diff.split_line(), Some(35));
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
    for (rect, _) in &screen.geometry.layout_switch {
        assert!(rect.right() <= 60);
    }
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
fn a_config_file_that_cannot_be_written_says_so_and_the_setting_still_applies() {
    let fixture = Fixture::new("settings-unwritable");
    fixture.write_config("[review\n");
    let (repo, capture) =
        crate::review::capture(&fixture.repo(), &Request::Uncommitted { base: None }).expect("capture");
    let (wake, _ready) = std::sync::mpsc::sync_channel(1);
    let settings = crate::review::settings::Settings::new(ReviewConfig::DEFAULT, Some(fixture.config()));
    let mut screen =
        Screen::open_with(repo, capture, &fixture.data(), Box::new(fixture.agent.clone()), wake, settings)
            .expect("screen");
    settle(&mut screen);
    keys(&mut screen, "w");
    buffer(&mut screen, COLS, ROWS);
    assert!(screen.diff.options().wrap);
    assert!(screen.status.as_deref().is_some_and(|s| s.contains("does not parse")), "{:?}", screen.status);
    assert_eq!(read(&fixture.config()), "[review\n", "left alone");
}
