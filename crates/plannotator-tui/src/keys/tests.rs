#![allow(clippy::expect_used, reason = "tests assert by panicking")]

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
fn an_exact_ctrl_binding_wins_over_a_plain_one_earlier_in_the_context() {
    // `up` comes before `page-up` in the document, and its plain `k` also matches ctrl+k.
    let keymap = parse("page-up = [\"ctrl+k\"]\n").expect("parses");
    assert_eq!(keymap.action(&press(KeyCode::Char('k'), KeyModifiers::CONTROL), DOCUMENT), Some(PageUp));
    assert_eq!(keymap.action(&press(KeyCode::Char('k'), KeyModifiers::NONE), DOCUMENT), Some(Up));
}

#[test]
fn a_plain_binding_still_answers_ctrl_when_no_ctrl_binding_claims_it() {
    // The defaults keep their behaviour: no action binds ctrl+j, so it still moves down.
    let keymap = Keymap::default();
    assert_eq!(keymap.action(&press(KeyCode::Char('j'), KeyModifiers::CONTROL), DOCUMENT), Some(Down));
    assert_eq!(keymap.action(&press(KeyCode::Char('d'), KeyModifiers::CONTROL), DOCUMENT), Some(PageDown));
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
