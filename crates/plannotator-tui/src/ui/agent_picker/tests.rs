#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::delivery::Clipboard;

fn session(name: &str, cwd: &str, pid: u32) -> PiEntry {
    serde_json::from_value(serde_json::json!({
        "v": 1, "host": "pi", "id": name, "pid": pid, "cwd": cwd, "name": name,
        "port": 1, "token": "t", "busy": false, "updated": 0,
    }))
    .expect("entry")
}

fn pane(agent: &str, pane: &str, cwd: Option<&str>, workspace: &str, tab: &str, status: &str) -> Reachable {
    Reachable::Herdr(
        PaneAgent {
            pane: pane.into(),
            agent: agent.into(),
            name: None,
            status: status.into(),
            cwd: cwd.map(PathBuf::from),
            workspace: workspace.into(),
            tab: tab.into(),
            session: None,
        },
        PathBuf::from("herdr"),
    )
}

fn labels(agents: &[Reachable]) -> Vec<String> {
    agents.iter().map(|a| format!("{} {}", a.label(), a.place())).collect()
}

/// A mix of Pi sessions and Herdr panes, some in the review's folder.
fn mixed() -> Vec<Reachable> {
    vec![
        pane("codex", "w2:p1", Some("/elsewhere/web"), "web", "main", "idle"),
        Reachable::Pi(session("web", "/nowhere/repo", 9)),
        pane("claude", "w1:p3", Some("/nowhere/repo/sub"), "api", "tests", "working"),
        pane("claude", "w1:p2", Some("/nowhere/repo"), "api", "main", "idle"),
        Reachable::Pi(session("other", "/elsewhere", 10)),
        pane("claude", "w3:p1", None, "notes", "1", "blocked"),
    ]
}

#[test]
fn sessions_in_the_folder_come_first_then_by_name_whatever_their_activity() {
    let folder = Path::new("/nowhere/repo");
    let mut sessions = vec![
        Reachable::Pi(session("web", "/nowhere/repo", 7)),
        Reachable::Pi(session("other", "/elsewhere", 7)),
        Reachable::Pi(session("api", "/nowhere/repo", 7)),
    ];
    order(&mut sessions, folder);
    let names: Vec<String> = sessions.iter().map(Reachable::label).collect();
    assert_eq!(names, ["pi · api", "pi · web", "pi · other"]);
    // A Pi going busy changes its state; the rows stay put.
    if let Some(Reachable::Pi(web)) = sessions.get_mut(1) {
        web.busy = true;
        web.updated = 99;
    }
    order(&mut sessions, folder);
    assert_eq!(sessions.iter().map(Reachable::label).collect::<Vec<_>>(), names);
}

#[test]
fn herdr_agents_and_pis_share_one_steady_order_folder_first() {
    let folder = Path::new("/nowhere/repo");
    let mut agents = mixed();
    order(&mut agents, folder);
    assert_eq!(
        labels(&agents),
        [
            "claude api › main · w1:p2",
            "claude api › tests · w1:p3",
            "pi · web pid 9",
            "claude notes › 1 · w3:p1",
            "codex web › main · w2:p1",
            "pi · other pid 10",
        ]
    );
    // However they arrive, they sort the same.
    let mut reversed: Vec<Reachable> = mixed().into_iter().rev().collect();
    order(&mut reversed, folder);
    assert_eq!(labels(&reversed), labels(&agents));
}

#[test]
fn picking_a_herdr_agent_moves_send_there_and_back_returns_to_the_start() {
    let mut agents = mixed();
    order(&mut agents, Path::new("/nowhere/repo"));
    let picker = AgentPicker { agents, cursor: 0, copy_row: false };
    let mut delivery: Box<dyn Delivery> = Box::new(Clipboard);
    let mut start = None;
    assert_eq!(picker.rows(false), 6);
    assert_eq!(
        picker.choose(1, &mut delivery, &mut start),
        Picked::Moved("Send now goes to claude in w1:p3".into())
    );
    assert!(delivery.is_agent());
    assert_eq!(delivery.agent_host(), Some("claude"));
    assert_eq!(picker.rows(start.is_some()), 7, "six agents and the way back");
    assert_eq!(
        picker.choose(2, &mut delivery, &mut start),
        Picked::Moved("Send now goes to pi · web".into())
    );
    assert_eq!(
        picker.choose(6, &mut delivery, &mut start),
        Picked::Moved("Send now goes to clipboard".into())
    );
    assert!(start.is_none());
    assert_eq!(picker.choose(6, &mut delivery, &mut start), Picked::Nothing);
}

#[test]
fn the_picker_send_opens_offers_copy_first_with_the_cursor_on_the_first_agent() {
    let picker = AgentPicker::for_send(mixed());
    assert_eq!(picker.cursor, 1);
    assert_eq!(picker.rows(false), 7);
    let mut delivery: Box<dyn Delivery> = Box::new(Clipboard);
    let mut start = None;
    assert_eq!(picker.choose(0, &mut delivery, &mut start), Picked::Copy);
    assert_eq!(delivery.describe(), "clipboard");
    assert!(start.is_none());
    assert_eq!(
        picker.choose(1, &mut delivery, &mut start),
        Picked::Moved("Send now goes to codex in w2:p1".into())
    );
}

#[test]
fn only_a_fallback_clipboard_in_an_interactive_review_asks() {
    let none =
        AgentFinder { pi_root: Some(PathBuf::from("/nowhere/registry")), herdr: None, ask_on_send: true };
    let reach = Reach { finder: none, found: Arc::new(AtomicBool::new(true)) };
    assert!(reach.asks(&Clipboard));
    assert!(!reach.asks(&crate::delivery::Caller));
    assert!(!reach.asks(&crate::delivery::Discard));
    // Nothing within reach now: Send copies, and the button stops asking.
    assert!(reach.picker_for_send(&Clipboard, Path::new("/nowhere")).is_none());
    assert!(!reach.asks(&Clipboard));
    assert!(!Reach::default().asks(&Clipboard), "headless and test reviews never ask");
}

/// The picker drawn at `cols`×`rows`, as text.
pub(crate) fn render(picker: &AgentPicker, start: Option<&dyn Delivery>, cols: u16, rows: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("terminal");
    terminal
        .draw(|frame| {
            let area = frame.area();
            picker.draw(frame, area, &Keymap::default(), Path::new("/nowhere/repo"), start);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    (0..rows)
        .map(|y| {
            let line: String =
                (0..cols).filter_map(|x| buffer.cell((x, y))).map(ratatui::buffer::Cell::symbol).collect();
            line.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Save a render for the proof folder when `AGENT_PICKER_PROOF_DIR` names one.
fn proof(name: &str, text: &str) {
    if let Some(dir) = std::env::var_os("AGENT_PICKER_PROOF_DIR") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("proof dir");
        std::fs::write(dir.join(name), format!("{text}\n")).expect("proof");
    }
}

#[test]
fn rows_name_the_agent_its_folder_state_and_where_it_runs() {
    let mut agents = mixed();
    order(&mut agents, Path::new("/nowhere/repo"));
    let p = AgentPicker { agents, cursor: 0, copy_row: false };
    let text = render(&p, None, 90, 12);
    proof("p-picker.txt", &text);
    assert!(text.contains("send to which agent?"), "{text}");
    let words: Vec<String> =
        text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    for row in [
        "│ claude . idle api › main · w1:p2 │",
        "│ claude ./sub working api › tests · w1:p3 │",
        "│ pi · web . idle pid 9 │",
        "│ claude ? blocked notes › 1 · w3:p1 │",
        "│ codex /elsewhere/web idle web › main · w2:p1 │",
        "│ pi · other /elsewhere idle pid 10 │",
    ] {
        assert!(words.iter().any(|l| l.ends_with(row)), "missing {row:?} in\n{text}");
    }
    // The columns line up: every row's state starts at the same column.
    let states: Vec<usize> = text
        .lines()
        .filter_map(|l| {
            let at =
                ["idle", "working", "blocked"].iter().filter_map(|s| l.find(&format!("  {s} "))).min()?;
            l.get(..at).map(|before| before.chars().count())
        })
        .collect();
    assert_eq!(states.len(), 6, "{text}");
    assert!(states.windows(2).all(|w| w[0] == w[1]), "{text}");
    let opened_by_send = AgentPicker::for_send(p.agents.clone());
    let text = render(&opened_by_send, None, 90, 12);
    proof("send-picker.txt", &text);
    let lines: Vec<&str> = text.lines().filter(|l| l.contains('│')).collect();
    assert!(lines[0].contains("Copy to clipboard"), "{text}");
    assert!(lines[1].contains("claude") && lines[1].contains("w1:p2"), "{text}");
    let moved: Box<dyn Delivery> = Box::new(Clipboard);
    let text = render(&p, Some(moved.as_ref()), 90, 12);
    proof("p-picker-after-pick.txt", &text);
    assert!(text.contains("back to clipboard"), "{text}");
}
