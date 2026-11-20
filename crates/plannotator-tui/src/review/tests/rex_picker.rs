//! The agent picker inside Rex, in a code review opened by `rex review`, against a fake `rex`
//! that answers from recorded Rex replies (`tests/fixtures/rex-agents`).

use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::review::Mode;
use crate::rex::agent::RexAgent;
use crate::rex::agents::{FIXTURE_OWN_BLOCK, FIXTURE_SESSION, RexAgents, fake_rex};
use crate::rex::command::Rex;
use crate::ui::agent_picker::{AgentFinder, Reach};

#[test]
fn a_code_review_from_one_rex_agent_can_send_to_another() {
    let fixture = Fixture::new("rex-picker");
    let mut screen = fixture.open();
    let fake = fixture.dir.join("fake rex");
    let bin = fake_rex(&fake);
    // As `rex review` opens it beside the Claude in `work › api`.
    let rex = Rex::new(bin.clone(), FIXTURE_SESSION.into());
    let claude = "block:01a1f658-997c-79c0-ae26-1bf77b9ba798".to_owned();
    screen.delivery = Box::new(RexAgent::picked(rex, claude, "claude".into(), 94903));
    let root = screen.real_root();
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
    screen.reach = Reach::start_with(finder, root.clone());
    screen.reach.find(&root);
    assert!(screen.send_label().contains("▸ claude in 9ba798"), "{}", screen.send_label());

    keys(&mut screen, "P");
    assert_eq!(screen.mode, Mode::PickTarget);
    let rows = frame(&mut screen, 120, 40);
    if let Some(dir) = std::env::var_os("REX_PICKER_PROOF_DIR") {
        std::fs::create_dir_all(&dir).expect("proof dir");
        std::fs::write(PathBuf::from(dir).join("code-review-p-from-claude.txt"), rows.join("\n"))
            .expect("proof");
    }
    assert!(row_with(&rows, "work › api · 9ba798").contains("Fix login redirect"));
    assert!(row_with(&rows, "work › api · 31496a").contains("working"));
    assert!(row_with(&rows, "side › Window 122adf3e · fd82e9").contains("release"));

    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(screen.delivery.describe(), "codex in 31496a");
    note_at(&mut screen, 9, "name the total");
    keys(&mut screen, "E");
    let pastes = read(&fake.join("pastes"));
    assert!(pastes.contains("block:01a1f658-99b3-7134-b64a-0b9d9d31496a"), "{pastes}");
    assert!(pastes.contains("name the total"), "{pastes}");
    assert_eq!(screen.status.as_deref(), Some("sent 1 note(s) → codex in 31496a"));
    assert!(!pastes.contains("9ba798"), "the Claude that opened the review got nothing: {pastes}");
}

fn note_at(screen: &mut Screen, line: u32, text: &str) {
    at(screen, line);
    keys(screen, "c");
    type_and_save(screen, text);
}
