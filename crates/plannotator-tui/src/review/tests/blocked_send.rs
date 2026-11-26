//! Choosing an agent that is at a dialog, in a code review: nothing is sent, nothing is
//! copied, the note stays pending, and the screen says why. The review starts with the real
//! clipboard fallback and copying turned on; in tests [`Clipboard`] only records what it
//! would copy.

use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::delivery::{Clipboard, test_clipboard};
use crate::herdr::agents::{HerdrAgents, fake_herdr};
use crate::review::Mode;
use crate::rex::agent::{AGENT_VAR, BLOCK_VAR, PID_VAR, RexAgent};
use crate::rex::agents::{FIXTURE_OWN_BLOCK, FIXTURE_SESSION, RexAgents, fake_rex};
use crate::ui::agent_picker::{AgentFinder, Reach};
use crate::ui::send_button::SendState;

fn screen_with(fixture: &Fixture, finder: AgentFinder) -> Screen {
    let mut screen = fixture.open();
    screen.delivery = Box::new(Clipboard);
    screen.clipboard = true;
    let root = screen.real_root();
    screen.reach = Reach::start_with(finder, root.clone(), None);
    screen.reach.find(&root);
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    screen
}

fn assert_waits(screen: &Screen, target: &str) {
    assert_eq!(screen.status.as_deref(), Some(format!("{target} is at a dialog · E retry").as_str()));
    assert!(matches!(screen.send_state, SendState::Blocked(_)), "{:?}", screen.send_state);
    assert_eq!(test_clipboard::copied(), [] as [String; 0], "nothing copied");
    assert_eq!(screen.store.pending().len(), 1, "the note is still pending");
    assert_eq!(screen.delivery.describe(), target);
}

#[test]
fn a_herdr_agent_at_a_dialog_gets_nothing_and_nothing_is_copied() {
    let fixture = Fixture::new("blocked-herdr");
    let fake = fixture.dir.join("fake herdr");
    let bin = fake_herdr(
        &fake,
        &serde_json::json!({"result": {"type": "agent_list", "agents": [
            {"agent": "codex", "agent_status": "blocked", "workspace_id": "w1", "tab_id": "w1:t2", "pane_id": "w1:p4", "cwd": "/elsewhere"},
        ]}}),
    );
    std::fs::write(fake.join("blocked"), "").expect("blocked flag");
    let finder = AgentFinder {
        pi_root: Some(fixture.dir.join("registry")),
        herdr: Some(HerdrAgents { bin, own_pane: None }),
        rex: None,
        ask_on_send: true,
    };
    let mut screen = screen_with(&fixture, finder);
    keys(&mut screen, "E");
    assert_eq!(screen.mode, Mode::PickTarget);
    press(&mut screen, KeyCode::Enter);
    assert_waits(&screen, "codex in w1:p4");
    assert!(!fake.join("prompts").exists());
}

#[test]
fn a_rex_agent_at_a_dialog_gets_nothing_and_nothing_is_copied() {
    let fixture = Fixture::new("blocked-rex");
    let fake = fixture.dir.join("fake rex");
    let bin = fake_rex(&fake);
    let finder = AgentFinder {
        pi_root: Some(fixture.dir.join("registry")),
        herdr: None,
        rex: Some(RexAgents {
            bin,
            session: FIXTURE_SESSION.into(),
            own_block: Some(FIXTURE_OWN_BLOCK.into()),
        }),
        ask_on_send: true,
    };
    let mut screen = screen_with(&fixture, finder);
    keys(&mut screen, "E");
    assert_eq!(screen.mode, Mode::PickTarget);
    keys(&mut screen, "jj");
    press(&mut screen, KeyCode::Enter);
    assert_waits(&screen, "claude in 63239c");
    assert!(!fake.join("pastes").exists());
}

#[test]
fn a_rex_agent_the_review_cannot_reach_is_named_and_the_note_stays_pending() {
    let fixture = Fixture::new("unreachable-rex");
    let finder = AgentFinder {
        pi_root: Some(fixture.dir.join("registry")),
        herdr: None,
        rex: None,
        ask_on_send: true,
    };
    let mut screen = screen_with(&fixture, finder);
    // A launcher named claude's block, but this pane has no Rex session to reach it in.
    let env = |k: &str| match k {
        BLOCK_VAR => Some("block:01a17dd5-e47c-754f-b72e-ef739ea1d6db".to_owned()),
        PID_VAR => Some("7".to_owned()),
        AGENT_VAR => Some("claude".to_owned()),
        _ => None,
    };
    screen.delivery = Box::new(RexAgent::from_lookup(env).expect("named").expect_err("unreachable"));
    assert_eq!(screen.send_label(), "Send 1 new ▸ claude · not found (E)");
    keys(&mut screen, "E");
    assert_eq!(
        screen.status.as_deref(),
        Some(
            "no agent to send to (REX_BLOCK is not set, so this pane cannot reach a1d6db) · copied to clipboard · E retry"
        )
    );
    assert_eq!(test_clipboard::copied().len(), 1, "the feedback is copied");
    assert_eq!(screen.store.pending().len(), 1, "the note is still pending");
    assert_eq!(screen.send_label(), "Send 1 new ▸ claude · not found (E)", "never Sent ✓");
    assert!(screen.has_unsent(), "quitting asks first");
}
