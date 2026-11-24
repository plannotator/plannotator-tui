#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]

use super::*;

fn parse(text: &str) -> Result<Keymap, toml::de::Error> {
    toml::from_str(text)
}

fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn the_defaults_bind_no_key_twice() {
    assert_eq!(Keymap::default().check(), Ok(()));
}

#[test]
fn clear_all_on_shifted_x_is_distinct_from_clear_block_on_x() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('X'), KeyModifiers::SHIFT), DOCUMENT), Some(ClearAll));
    assert_eq!(keymap.action(&press(KeyCode::Char('x'), KeyModifiers::NONE), DOCUMENT), Some(ClearBlock));
    assert_eq!(keymap.action(&press(KeyCode::Enter, KeyModifiers::NONE), CONFIRM_CLEAR), None);
}

#[test]
fn an_entry_replaces_only_its_own_action() {
    let keymap =
        parse("left = [\"j\", \"Left\"]\ndown = [\"k\"]\nup = [\"l\"]\nright = [\";\"]\n").expect("parses");
    assert_eq!(keymap.action(&press(KeyCode::Char('j'), KeyModifiers::NONE), MOTION), Some(Left));
    assert_eq!(keymap.action(&press(KeyCode::Char(';'), KeyModifiers::NONE), TREE), Some(Right));
    assert_eq!(keymap.action(&press(KeyCode::Down, KeyModifiers::NONE), MOTION), None);
    assert_eq!(keymap.action(&press(KeyCode::Char('w'), KeyModifiers::NONE), MOTION), Some(WordForward));
    assert_eq!(keymap.directions_hint(), "jkl;");
}

#[test]
fn a_key_two_live_actions_share_is_an_error_naming_both() {
    let err = parse("left = [\"j\"]\n").expect_err("j is still down");
    let message = err.message();
    assert!(message.contains("\"j\"") && message.contains("down") && message.contains("left"), "{message}");
}

#[test]
fn a_key_may_mean_different_actions_in_different_modes() {
    // `Home` is `top` among blocks and `line-start` for the cursor; they never read the same press.
    let keymap = Keymap::default();
    let home = press(KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(keymap.action(&home, DOCUMENT), Some(Top));
    assert_eq!(keymap.action(&home, MOTION), Some(LineStart));
}

#[test]
fn an_unknown_action_error_names_it() {
    let err = parse("lefty = [\"h\"]\n").expect_err("rejected");
    assert!(err.message().contains("lefty"), "{}", err.message());
}

#[test]
fn an_unknown_key_error_names_it() {
    let err = parse("left = [\"Leftt\"]\n").expect_err("rejected");
    assert!(err.message().contains("Leftt"), "{}", err.message());
}

#[test]
fn a_ctrl_binding_needs_ctrl_and_a_plain_one_ignores_shift() {
    let keymap = Keymap::default();
    assert!(keymap.is(PageDown, &press(KeyCode::Char('d'), KeyModifiers::CONTROL)));
    assert!(!keymap.is(PageDown, &press(KeyCode::Char('d'), KeyModifiers::NONE)));
    assert!(keymap.is(Bottom, &press(KeyCode::Char('G'), KeyModifiers::SHIFT)));
}

#[test]
fn named_keys_parse_in_any_case_and_print_as_written_in_the_file() {
    let key: Key = "pageup".parse().expect("parses");
    assert_eq!(key.to_string(), "PageUp");
    assert_eq!("ctrl+u".parse::<Key>().expect("parses").to_string(), "ctrl+u");
}

#[test]
fn an_empty_list_unbinds_the_action() {
    let keymap = parse("quit = []\n").expect("parses");
    assert!(!keymap.is(Quit, &press(KeyCode::Char('q'), KeyModifiers::NONE)));
    assert_eq!(keymap.hint(Quit), "");
}

#[test]
fn a_ctrl_binding_and_a_plain_one_on_the_same_letter_stay_apart() {
    let keymap = parse("page-up = [\"ctrl+k\"]\n").expect("parses");
    assert_eq!(keymap.action(&press(KeyCode::Char('k'), KeyModifiers::CONTROL), DOCUMENT), Some(PageUp));
    assert_eq!(keymap.action(&press(KeyCode::Char('k'), KeyModifiers::NONE), DOCUMENT), Some(Up));
}

#[test]
fn a_plain_binding_does_not_answer_ctrl() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('j'), KeyModifiers::CONTROL), DOCUMENT), None);
    assert_eq!(keymap.action(&press(KeyCode::Down, KeyModifiers::CONTROL), DOCUMENT), None);
    assert!(!keymap.is(LooksGood, &press(KeyCode::Char('a'), KeyModifiers::CONTROL)));
    assert_eq!(keymap.action(&press(KeyCode::Char('d'), KeyModifiers::CONTROL), DOCUMENT), Some(PageDown));
}

#[test]
fn a_plain_binding_answers_altgr_which_windows_reports_as_ctrl_and_alt() {
    let keymap = Keymap::default();
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    assert!(keymap.is(LineEnd, &press(KeyCode::Char('$'), altgr)));
}

#[test]
fn every_action_name_is_the_one_the_config_file_writes() {
    for (action, _) in DEFAULTS {
        let serialized = toml::Value::try_from(action).expect("serializes");
        assert_eq!(serialized.as_str(), Some(action.as_str()));
    }
}

#[test]
fn an_unbound_action_drops_out_of_the_hints() {
    let keymap = parse("quit = []\n").expect("parses");
    assert_eq!(keymap.labelled(Quit, "quit"), None);
    assert_eq!(keymap.labelled(Send, "send").as_deref(), Some("E send"));
}

#[test]
fn approve_is_shifted_a_and_distinct_from_looks_good() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('A'), KeyModifiers::SHIFT), GLOBAL), Some(Approve));
    assert_eq!(keymap.action(&press(KeyCode::Char('a'), KeyModifiers::NONE), TOOLBAR), Some(LooksGood));
    assert_eq!(keymap.action(&press(KeyCode::Char('a'), KeyModifiers::NONE), GLOBAL), None);
}

#[test]
fn a_config_that_gave_approves_key_away_still_loads() {
    // Written before `approve` existed: `A` is the user's, and approve is left unbound.
    let keymap = parse("comment = [\"A\"]\n").expect("an older config keeps loading");
    assert_eq!(keymap.action(&press(KeyCode::Char('A'), KeyModifiers::SHIFT), DOCUMENT), Some(Comment));
    assert_eq!(keymap.hint(Approve), "");
    // Naming approve opts into the check like any other action.
    let err = parse("comment = [\"A\"]\napprove = [\"A\"]\n").expect_err("both live while browsing");
    assert!(err.message().contains("approve"), "{}", err.message());
    assert_eq!(parse("approve = [\"Y\"]\n").expect("rebinds").hint(Approve), "Y");
}

#[test]
fn word_end_and_visual_line_yield_to_an_older_config() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('e'), KeyModifiers::NONE), MOTION), Some(WordEnd));
    assert_eq!(keymap.action(&press(KeyCode::Char('V'), KeyModifiers::SHIFT), DOCUMENT), Some(VisualLine));
    assert_eq!(parse("left = [\"e\"]\n").expect("an older config keeps loading").hint(WordEnd), "");
    assert_eq!(parse("comment = [\"V\"]\n").expect("an older config keeps loading").hint(VisualLine), "");
}

#[test]
fn looks_good_and_delete_on_a_block_never_conflict_with_a_document_key() {
    let keymap = parse("looks-good = [\"j\"]\nclear-block = [\"d\"]\n").expect("loads as it did");
    assert_eq!(keymap.action(&press(KeyCode::Char('j'), KeyModifiers::NONE), DOCUMENT), Some(Down));
    assert_eq!(keymap.action(&press(KeyCode::Char('d'), KeyModifiers::NONE), DOCUMENT), Some(ClearBlock));
}

#[test]
fn copy_is_y_on_a_waiting_selection_and_yields_to_an_older_config() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('y'), KeyModifiers::NONE), TOOLBAR), Some(CopySelection));
    // Written before `copy` existed: `y` is the user's comment key, and copy is left unbound.
    let keymap = parse("comment = [\"y\"]\n").expect("an older config keeps loading");
    assert_eq!(keymap.hint(CopySelection), "");
    assert_eq!(keymap.hint(Approve), "A");
    assert_eq!(parse("copy = [\"Y\"]\n").expect("rebinds").hint(CopySelection), "Y");
}

#[test]
fn shift_with_a_lowercase_letter_counts_as_the_capital() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('x'), KeyModifiers::SHIFT), DOCUMENT), Some(ClearAll));
    assert!(keymap.is(Bottom, &press(KeyCode::Char('g'), KeyModifiers::SHIFT)));
    assert!(!keymap.is(Bottom, &press(KeyCode::Char('g'), KeyModifiers::NONE)));
}

#[test]
fn change_comment_is_m_in_the_code_review_and_yields_to_an_older_config() {
    let keymap = Keymap::default();
    let shifted_m = press(KeyCode::Char('M'), KeyModifiers::SHIFT);
    assert_eq!(keymap.action(&shifted_m, REVIEW_GLOBAL), Some(ChangeComment));
    assert_eq!(keymap.action(&shifted_m, REVIEW_MENU), Some(ChangeComment));
    // Written before `change-comment` existed: `M` is the user's, and it is left unbound.
    let keymap = parse("reload = [\"M\"]\n").expect("an older config keeps loading");
    assert_eq!(keymap.hint(ChangeComment), "");
}

#[test]
fn whole_file_is_o_and_whole_base_shifted_o_in_the_diff_and_yield_to_an_older_config() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('o'), KeyModifiers::NONE), DIFF), Some(WholeFile));
    assert_eq!(keymap.action(&press(KeyCode::Char('O'), KeyModifiers::SHIFT), DIFF), Some(WholeBase));
    // `o` stays open-in-editor in a document review.
    assert_eq!(keymap.action(&press(KeyCode::Char('o'), KeyModifiers::NONE), GLOBAL), Some(OpenInEditor));
    let keymap = parse("toggle-wrap = [\"o\"]\n").expect("an older config keeps loading");
    assert_eq!(keymap.hint(WholeFile), "");
    assert_eq!(keymap.hint(WholeBase), "O");
}

#[test]
fn search_is_slash_and_n_and_shift_n_step_without_taking_them_from_other_screens() {
    let keymap = Keymap::default();
    let n = press(KeyCode::Char('n'), KeyModifiers::NONE);
    let shift_n = press(KeyCode::Char('N'), KeyModifiers::SHIFT);
    assert_eq!(keymap.action(&press(KeyCode::Char('/'), KeyModifiers::NONE), DOCUMENT), Some(Search));
    assert_eq!(keymap.action(&n, DOCUMENT), Some(NextMatch));
    assert_eq!(keymap.action(&shift_n, DOCUMENT), Some(PreviousMatch));
    // The quit question and the code review keep their own meaning for the same keys.
    assert_eq!(keymap.action(&n, CONFIRM_QUIT), Some(No));
    assert_eq!(keymap.action(&n, DIFF), Some(NextFile));
    assert_eq!(keymap.action(&shift_n, DIFF), Some(PreviousFile));
}

#[test]
fn the_search_keys_yield_to_an_older_config_that_gave_them_away() {
    // The example at the top of this module: `n` moves down, so next-match is left unbound.
    let keymap = parse("down = [\"n\", \"Down\"]\n").expect("an older config keeps loading");
    assert_eq!(keymap.action(&press(KeyCode::Char('n'), KeyModifiers::NONE), DOCUMENT), Some(Down));
    assert_eq!(keymap.hint(NextMatch), "");
    assert_eq!(keymap.hint(PreviousMatch), "N");
    assert_eq!(parse("comment = [\"/\"]\n").expect("loads").hint(Search), "");
    assert_eq!(parse("search = [\"ctrl+f\"]\n").expect("rebinds").hint(Search), "ctrl+f");
}

#[test]
fn go_to_definition_is_d_with_ctrl_o_back_and_ctrl_i_forward_in_the_diff() {
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('d'), KeyModifiers::NONE), DIFF), Some(GoToDefinition));
    assert_eq!(keymap.action(&press(KeyCode::Char('o'), KeyModifiers::CONTROL), DIFF), Some(JumpBack));
    assert_eq!(keymap.action(&press(KeyCode::Char('i'), KeyModifiers::CONTROL), DIFF), Some(JumpForward));
    // Where the terminal sends Ctrl+I as Tab, Tab still cycles the focus.
    assert_eq!(keymap.action(&press(KeyCode::Tab, KeyModifiers::NONE), REVIEW_GLOBAL), Some(CycleFocus));
    // `d` stays a block mark in a document review.
    assert_eq!(keymap.action(&press(KeyCode::Char('d'), KeyModifiers::NONE), BLOCK_MARKS), Some(Delete));
    let keymap = parse("toggle-files = [\"d\"]\n").expect("an older config keeps loading");
    assert_eq!(keymap.hint(GoToDefinition), "");
    assert_eq!((keymap.hint(JumpBack), keymap.hint(JumpForward)), ("ctrl+o".into(), "ctrl+i".into()));
}

#[test]
fn the_file_list_width_keys_yield_to_an_older_config_and_the_reading_column_keeps_its_own() {
    let keymap = Keymap::default();
    let wider = press(KeyCode::Char('>'), KeyModifiers::SHIFT);
    assert_eq!(keymap.action(&wider, DIFF), Some(WidenFiles));
    assert_eq!(keymap.action(&wider, FILES), Some(WidenFiles));
    assert_eq!(keymap.action(&wider, DOCUMENT), Some(WidenColumn));
    // Written when `>` meant nothing in the code review: it keeps the user's meaning there,
    // and the reading column keeps `>`.
    let keymap = parse("toggle-layout = [\">\"]\n").expect("an older config keeps loading");
    assert_eq!(keymap.action(&wider, DIFF), Some(ToggleLayout));
    assert_eq!(keymap.action(&wider, DOCUMENT), Some(WidenColumn));
    assert_eq!(keymap.hint(WidenFiles), "");
    assert_eq!(keymap.hint(NarrowFiles), "<");
}

#[test]
fn configs_written_before_copy_lines_load_and_keep_copy_on_the_document_screen() {
    // (config, the document's `copy`, the code review's `copy-lines`, `toggle-viewed`)
    let table = [
        ("", "y", "y", "x"),
        ("toggle-viewed = [\"y\"]\n", "y", "", "y"),
        ("copy = [\"y\"]\ntoggle-viewed = [\"y\"]\n", "y", "", "y"),
        ("copy = [\"x\"]\n", "x", "", "x"),
        ("copy = [\"Y\"]\n", "Y", "Y", "x"),
        ("comment = [\"y\"]\n", "", "", "x"),
        ("down = [\"y\"]\n", "y", "", "x"),
        ("copy = [\"y\"]\ncopy-lines = [\"Y\"]\n", "y", "Y", "x"),
        ("copy-lines = []\n", "y", "", "x"),
    ];
    for (config, copy, lines, viewed) in table {
        let keymap = parse(config).unwrap_or_else(|err| panic!("{config:?} loads: {err}"));
        assert_eq!(keymap.hint(CopySelection), copy, "copy in {config:?}");
        assert_eq!(keymap.hint(CopyLines), lines, "copy-lines in {config:?}");
        assert_eq!(keymap.hint(ToggleViewed), viewed, "toggle-viewed in {config:?}");
        // The document screen reads `copy` from the toolbar's keys, as it did.
        if !copy.is_empty() {
            let key: Key = copy.parse().expect("a key");
            let event = press(key.code, KeyModifiers::NONE);
            assert_eq!(keymap.action(&event, TOOLBAR), Some(CopySelection), "{config:?}");
        }
    }
    // A key the person gave both on purpose is still an error.
    let err = parse("toggle-viewed = [\"y\"]\ncopy-lines = [\"y\"]\n").expect_err("both explicit");
    assert!(err.message().contains("copy-lines"), "{}", err.message());
}

#[test]
fn the_settings_and_split_keys_yield_to_an_older_config_that_gave_them_away() {
    let comma = press(KeyCode::Char(','), KeyModifiers::NONE);
    let open = press(KeyCode::Char('('), KeyModifiers::SHIFT);
    let close = press(KeyCode::Char(')'), KeyModifiers::SHIFT);
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&comma, REVIEW_GLOBAL), Some(DiffSettings));
    assert_eq!(keymap.action(&comma, SETTINGS), Some(DiffSettings));
    assert_eq!(keymap.action(&open, DIFF), Some(SplitLeft));
    assert_eq!(keymap.action(&close, DIFF), Some(SplitRight));
    // (config, `diff-settings`, `split-left`, `split-right`)
    let table = [
        ("", ",", "(", ")"),
        ("toggle-notes = [\",\"]\n", "", "(", ")"),
        ("toggle-layout = [\"(\"]\ntoggle-wrap = [\")\"]\n", ",", "", ""),
        ("jump-back = [\"(\"]\n", ",", "", ")"),
        ("diff-settings = [\"ctrl+g\"]\n", "ctrl+g", "(", ")"),
        ("split-left = [\"[\"]\nsplit-right = [\"]\"]\n", ",", "[", "]"),
        ("diff-settings = []\nsplit-left = []\n", "", "", ")"),
    ];
    for (config, settings, left, right) in table {
        let keymap = parse(config).unwrap_or_else(|err| panic!("{config:?} loads: {err}"));
        assert_eq!(keymap.check(), Ok(()), "{config:?}");
        assert_eq!(keymap.hint(DiffSettings), settings, "diff-settings in {config:?}");
        assert_eq!(keymap.hint(SplitLeft), left, "split-left in {config:?}");
        assert_eq!(keymap.hint(SplitRight), right, "split-right in {config:?}");
    }
    let keymap = parse("toggle-layout = [\"(\"]\n").expect("loads");
    assert_eq!(keymap.action(&open, DIFF), Some(ToggleLayout), "the user's meaning stays");
}
