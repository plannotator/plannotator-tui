//! The agent picker inside Rex, in a document review, against a fake `rex` that answers from
//! recorded Rex replies (`tests/fixtures/rex-agents`). The fake is a POSIX shell script, so
//! these tests are Unix-only.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use super::super::review_test_support::{draw, file_app, press};
use super::*;
use crate::rex::agent::RexAgent;
use crate::rex::agents::{FIXTURE_OWN_BLOCK, FIXTURE_SESSION, RexAgents, fake_rex};
use crate::rex::command::Rex;
use crate::ui::agent_picker::AgentFinder;

/// The clipboard a review falls back to, recording what it would have copied, so no test
/// writes to the real terminal clipboard.
#[derive(Debug, Default, Clone)]
struct FallbackCopy {
    copied: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl crate::delivery::Delivery for FallbackCopy {
    fn describe(&self) -> String {
        "clipboard".into()
    }
    fn is_fallback(&self) -> bool {
        true
    }
    fn deliver(&self, feedback: &str) -> Result<(), crate::delivery::DeliveryError> {
        self.copied.borrow_mut().push(feedback.to_owned());
        Ok(())
    }
}

/// A file review running in the fixtures' shell block (`work › api`), with the recorded Rex.
fn in_rex(tag: &str) -> (PathBuf, App, PathBuf) {
    let (root, mut app, _) = file_app(tag);
    let fake = root.join("fake rex");
    let bin = fake_rex(&fake);
    let folder = agent_picker::real(&root.join("docs"));
    let finder = AgentFinder {
        pi_root: Some(root.join("registry")),
        herdr: None,
        rex: Some(RexAgents {
            bin,
            session: FIXTURE_SESSION.into(),
            own_block: Some(FIXTURE_OWN_BLOCK.into()),
        }),
        ask_on_send: true,
    };
    app.reach = Reach::start_with(finder, folder.clone(), None);
    app.reach.find(&folder);
    (root, app, fake)
}

/// The review as `plannotator-tui rex open` opens it beside the Claude in `work › api`.
fn launched_from_claude(app: &mut App, fake: &Path) {
    let rex = Rex::new(fake.join("rex"), FIXTURE_SESSION.into());
    let block = "block:01a1f658-997c-79c0-ae26-1bf77b9ba798".to_owned();
    app.delivery = Box::new(RexAgent::picked(rex, block, "claude".into(), 94903));
}

fn enter(app: &mut App) {
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("enter");
}

fn proof(name: &str, text: &str) {
    if let Some(dir) = std::env::var_os("REX_PICKER_PROOF_DIR") {
        std::fs::create_dir_all(&dir).expect("proof dir");
        std::fs::write(PathBuf::from(dir).join(name), format!("{text}\n")).expect("proof");
    }
}

fn calls(fake: &Path) -> String {
    std::fs::read_to_string(fake.join("calls")).unwrap_or_default()
}

/// The rows of the picker on screen, between its borders, with runs of spaces squeezed.
fn picker_rows(screen: &str) -> Vec<String> {
    screen
        .lines()
        .filter_map(|l| {
            let inner = l.split('│').nth(1)?;
            Some(inner.split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .filter(|row| !row.is_empty())
        .collect()
}

#[test]
fn launched_from_one_agent_send_goes_there_and_p_lists_every_rex_agent_this_tab_first() {
    let (root, mut app, fake) = in_rex("rex-launched");
    launched_from_claude(&mut app, &fake);
    assert!(app.send_label().contains("▸ claude in 9ba798"), "{}", app.send_label());

    press(&mut app, 'P');
    assert_eq!(app.mode, Mode::PickTarget);
    let screen = draw(&mut app, 110, 24);
    proof("doc-review-p-from-claude.txt", &screen);
    assert_eq!(
        picker_rows(&screen),
        [
            "claude · Fix login redirect /Users/me/work/api idle work › api · 9ba798",
            "codex /Users/me/work/api/src working work › api · 31496a",
            "claude · Migrate styles /Users/me/work/web blocked work › web · 63239c",
            "claude · release /Users/me/work/notes idle side › Window 122adf3e · fd82e9",
            "pi · π - notes /Users/me/work/notes idle work › web · eda7cc",
        ],
        "{screen}"
    );

    // A pane half as wide shortens the folders and keeps where each agent runs on screen.
    let narrow = draw(&mut app, 80, 24);
    proof("doc-review-p-from-claude-80-columns.txt", &narrow);
    let rows = picker_rows(&narrow);
    assert_eq!(rows.len(), 5, "{narrow}");
    for (row, place) in rows.iter().zip(["9ba798", "31496a", "63239c", "fd82e9", "eda7cc"]) {
        assert!(row.ends_with(place), "{row:?} in\n{narrow}");
    }
    assert!(rows[0].contains("…"), "{narrow}");

    // The Codex beside it: Send moves there, and the last row goes back to the Claude.
    press(&mut app, 'j');
    enter(&mut app);
    assert_eq!(app.status.as_deref(), Some("Send now goes to codex in 31496a"));
    app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
    press(&mut app, 'E');
    let pastes = std::fs::read_to_string(fake.join("pastes")).expect("a paste");
    assert!(pastes.contains("\"block_id\":\"block:01a1f658-99b3-7134-b64a-0b9d9d31496a\""), "{pastes}");
    assert!(pastes.contains("tighten this"), "{pastes}");
    let calls = calls(&fake);
    assert!(calls.contains("send-key -b block:01a1f658-99b3-7134-b64a-0b9d9d31496a enter"), "{calls}");
    assert!(
        calls.contains("api call block.event - -s session:01a1f658-7c0a"),
        "scoped to its session: {calls}"
    );
    assert_eq!(app.status.as_deref(), Some("sent 1 annotation(s) → codex in 31496a"));

    press(&mut app, 'P');
    assert_eq!(app.pi_rows(), 6, "five agents and the way back");
    let screen = draw(&mut app, 110, 24);
    assert!(screen.contains("back to claude in 9ba798"), "{screen}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_blocked_rex_agent_is_offered_but_a_send_waits_for_it() {
    let (root, mut app, fake) = in_rex("rex-blocked");
    launched_from_claude(&mut app, &fake);
    app.open_pi_picker();
    press(&mut app, 'j');
    press(&mut app, 'j');
    enter(&mut app);
    assert_eq!(app.delivery.describe(), "claude in 63239c");
    app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
    press(&mut app, 'E');
    assert!(!fake.join("pastes").exists(), "nothing pasted at a permission prompt");
    assert_eq!(app.status.as_deref(), Some("claude in 63239c is at a dialog · E retry"));
    let crate::ui::send_button::SendState::Blocked(why) = &app.send_state else {
        panic!("a blocked send: {:?}", app.send_state)
    };
    assert!(why.contains("waiting for a permission answer"), "{why}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn opened_on_its_own_in_rex_send_asks_which_rex_agent() {
    let (root, mut app, fake) = in_rex("rex-ask");
    let copy = FallbackCopy::default();
    app.delivery = Box::new(copy.clone());
    app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
    assert_eq!(app.send_label(), "Send 1 new ▸ choose agent (E)");

    press(&mut app, 'E');
    assert_eq!(app.mode, Mode::PickTarget);
    let screen = draw(&mut app, 110, 24);
    proof("doc-review-send-opens-picker.txt", &screen);
    let rows = picker_rows(&screen);
    assert_eq!(rows.first().map(String::as_str), Some("Copy to clipboard"), "{screen}");
    assert!(rows.get(1).is_some_and(|r| r.ends_with("work › api · 9ba798")), "{screen}");
    assert!(!fake.join("pastes").exists(), "nothing sent yet");

    enter(&mut app);
    let pastes = std::fs::read_to_string(fake.join("pastes")).expect("a paste");
    assert!(pastes.contains("block:01a1f658-997c-79c0-ae26-1bf77b9ba798"), "{pastes}");
    assert_eq!(app.status.as_deref(), Some("sent 1 annotation(s) → claude in 9ba798"));
    assert_eq!(copy.copied.borrow().len(), 0);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn an_agent_that_reports_no_state_says_so_on_its_row_and_send_refuses_it() {
    let (root, mut app, fake) = in_rex("rex-no-state");
    // The Claude in work › api, as before it has reported a state to Rex.
    std::fs::write(fake.join("9ba798.program_status.json"), r#"{"records":[]}"#).expect("status");
    app.open_pi_picker();
    let screen = draw(&mut app, 110, 24);
    proof("doc-review-p-no-state.txt", &screen);
    let rows = picker_rows(&screen);
    assert!(rows.iter().any(|r| r.contains("no state") && r.ends_with("work › api · 9ba798")), "{screen}");
    enter(&mut app);
    assert_eq!(app.delivery.describe(), "claude in 9ba798");
    app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
    press(&mut app, 'E');
    assert!(!fake.join("pastes").exists(), "nothing pasted");
    let status = app.status.clone().unwrap_or_default();
    assert!(
        status.starts_with("no agent to send to (claude in 9ba798 has not reported its state to Rex)"),
        "{status}"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}
