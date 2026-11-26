//! Asking the agent about lines: the question goes to it as Send goes, the answer is read
//! from its transcript into a box under the lines, and nothing becomes a note.

use std::io::Write as _;

use ratatui::crossterm::event::KeyCode;
use serde_json::json;

use super::*;
use plannotator_tui_hosts::Host;

use crate::last::Transcript;
use crate::review::Mode;

fn user(uuid: &str, parent: Option<&str>, text: &str) -> String {
    json!({"type": "user", "uuid": uuid, "parentUuid": parent, "message": {"role": "user", "content": text}})
        .to_string()
}

fn assistant(uuid: &str, parent: Option<&str>, text: &str) -> String {
    let content = json!([{"type": "text", "text": text}]);
    let message = json!({"role": "assistant", "id": format!("m-{uuid}"), "content": content});
    json!({"type": "assistant", "uuid": uuid, "parentUuid": parent, "message": message}).to_string()
}

fn append(path: &Path, lines: &[String]) {
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).expect("transcript");
    for line in lines {
        writeln!(file, "{line}").expect("append");
    }
}

/// Poll and draw, as the event loop does, until `done` holds for the rows drawn.
fn wait(screen: &mut Screen, what: &str, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let started = Instant::now();
    loop {
        screen.tick().expect("tick");
        let rows = frame(screen, 120, 40);
        if done(&rows) {
            return rows;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "never: {what}\n{}", rows.join("\n"));
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for(screen: &mut Screen, text: &str) -> Vec<String> {
    wait(screen, text, |rows| rows.iter().any(|row| row.contains(text)))
}

/// The review on the fixture, answering from a transcript holding one earlier exchange.
fn asking(tag: &str) -> (Fixture, Screen, PathBuf) {
    let fixture = Fixture::new(tag);
    let transcript = fixture.dir.join("transcript.jsonl");
    append(&transcript, &[user("p1", None, "earlier"), assistant("r1", Some("p1"), "an earlier reply")]);
    let mut screen = fixture.open();
    screen.answer_from(Transcript::at(Host::ClaudeCode, &transcript));
    (fixture, screen, transcript)
}

/// Ask `question` on the lines the cursor or the selection is on, and wait until it is sent.
fn ask(fixture: &Fixture, screen: &mut Screen, question: &str) -> String {
    let before = fixture.sent().len();
    keys(screen, "a");
    type_and_save(screen, question);
    wait(screen, "the question sent", |_| fixture.sent().len() > before);
    fixture.sent().last().cloned().expect("sent")
}

#[test]
fn a_question_about_lines_reaches_the_agent_and_its_answer_shows_in_a_box_under_them() {
    let (fixture, mut screen, transcript) = asking("ask");
    at(&mut screen, 9);
    keys(&mut screen, "a");
    row_with(&frame(&mut screen, 120, 40), "ask about app.py:9");
    press(&mut screen, KeyCode::Esc);
    let question = ask(&fixture, &mut screen, "why a local?");
    assert!(question.starts_with("A question from the code review of Uncommitted changes in "), "{question}");
    assert!(question.contains(", about `app.py:9`:\n\n```\n    total = a + b\n```\n"), "{question}");
    assert!(question.ends_with("\nwhy a local?"), "{question}");
    assert_eq!(screen.mode, Mode::Browse);
    let rows = frame(&mut screen, 120, 40);
    row_with(&rows, "ask \u{b7} L9");
    row_with(&rows, "waiting for the answer");
    row_with(&rows, "ask again (\u{23ce} Enter) \u{b7} remove (\u{232b} Delete)");

    append(
        &transcript,
        &[user("p2", Some("r1"), &question), assistant("r2", Some("p2"), "It names the **sum**.")],
    );
    let rows = wait_for(&mut screen, "It names the sum.");
    assert!(!rows.iter().any(|row| row.contains("an earlier reply")), "only the answer to this question");
    let at_row = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap_or(usize::MAX);
    assert!(
        at_row("You") < at_row("why a local?")
            && at_row("why a local?") < at_row("claude")
            && at_row("claude") < at_row("It names the sum."),
        "each message says who wrote it:\n{}",
        rows.join("\n")
    );
    let row = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap_or(usize::MAX);
    assert!(
        row("total = a + b") < row("It names the sum.") && row("It names the sum.") < row("return total"),
        "the box is under line 9:\n{}",
        rows.join("\n")
    );

    // On the box, Enter asks again about its lines, in a question box at the answer box's end,
    // and the answer goes into the same box.
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    let rows = frame(&mut screen, 120, 40);
    let end = rows.iter().position(|row| row.contains("ask again")).expect("the answer box's end");
    let asking = rows.iter().position(|row| row.contains("ask about app.py:9")).expect("the question box");
    assert_eq!(asking, end + 1, "the question box opens under the answer box:\n{}", rows.join("\n"));
    type_and_save(&mut screen, "and if b is negative?");
    wait(&mut screen, "the follow-up sent", |_| fixture.sent().len() == 2);
    assert!(fixture.sent().last().is_some_and(|sent| sent.ends_with("\nand if b is negative?")));
    let rows = frame(&mut screen, 120, 40);
    assert_eq!(rows.iter().filter(|row| row.contains("ask \u{b7} L9")).count(), 1, "one box for the lines");
    row_with(&rows, "and if b is negative?");
    assert!(screen.store.annotations().is_empty(), "a question is never a note");

    // Backspace (the Mac's delete key) on the box removes it, as Delete does.
    at(&mut screen, 9);
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Backspace);
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|row| row.contains("ask \u{b7} L9")), "{}", rows.join("\n"));
}

#[test]
fn a_question_on_lines_inside_a_box_goes_into_that_box() {
    let (fixture, mut screen, _transcript) = asking("ask-inside");
    at(&mut screen, 9);
    keys(&mut screen, "vjv");
    ask(&fixture, &mut screen, "what do these two do?");
    row_with(&frame(&mut screen, 120, 40), "ask \u{b7} L9\u{2013}10");
    at(&mut screen, 10);
    let question = ask(&fixture, &mut screen, "and this one?");
    assert!(question.contains("about `app.py:10`"), "the question names its own line: {question}");
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|row| row.contains("ask \u{b7} L10 ")), "no second box:\n{}", rows.join("\n"));
    row_with(&rows, "and this one?");
    // With the cursor on one of its lines, not on the box, the remove key removes it too.
    at(&mut screen, 10);
    press(&mut screen, KeyCode::Delete);
    let rows = frame(&mut screen, 120, 40);
    assert!(!rows.iter().any(|row| row.contains("ask \u{b7} L9")), "{}", rows.join("\n"));
}

#[test]
fn a_question_with_no_transcript_to_answer_from_is_not_sent_and_says_why() {
    let fixture = Fixture::new("ask-no-transcript");
    let mut screen = fixture.open();
    screen.answer_from(Transcript::at(Host::ClaudeCode, &fixture.dir.join("missing.jsonl")));
    at(&mut screen, 9);
    keys(&mut screen, "a");
    type_and_save(&mut screen, "why?");
    wait_for(&mut screen, "not sent: cannot read the agent's");
    assert!(fixture.sent().is_empty(), "no question without a way to read its answer");
    assert!(
        screen.status.as_deref().is_some_and(|status| status.starts_with("not asked: ")),
        "{:?}",
        screen.status
    );
}

/// A pi transcript line: a message of `role` with `text`, after entry `parent`.
fn pi(id: &str, parent: Option<&str>, role: &str, text: &str) -> String {
    let message = json!({"role": role, "content": [{"type": "text", "text": text}]});
    json!({"type": "message", "id": id, "parentId": parent, "message": message}).to_string()
}

#[test]
fn a_pi_agent_answers_in_the_box_too() {
    let fixture = Fixture::new("ask-pi");
    let transcript = fixture.dir.join("pi.jsonl");
    let header = json!({"type": "session", "version": 3, "id": "01a04968-90e4-745a-a8d3-6903d935f391", "cwd": "/work"});
    append(
        &transcript,
        &[
            header.to_string(),
            pi("p1", None, "user", "earlier"),
            pi("r1", Some("p1"), "assistant", "an earlier reply"),
        ],
    );
    let mut screen = fixture.open();
    screen.answer_from(Transcript::at(Host::Pi, &transcript));
    at(&mut screen, 9);
    let question = ask(&fixture, &mut screen, "why a local?");
    append(
        &transcript,
        &[
            pi("p2", Some("r1"), "user", &question),
            pi("r2", Some("p2"), "assistant", "Pi says it names the sum."),
        ],
    );
    let rows = wait_for(&mut screen, "Pi says it names the sum.");
    assert!(!rows.iter().any(|row| row.contains("an earlier reply")), "only the answer to this question");
}

#[test]
fn a_codex_agent_is_not_asked_as_its_transcript_has_no_prompts_to_find_the_question_by() {
    let fixture = Fixture::new("ask-codex");
    let transcript = fixture.dir.join("rollout.jsonl");
    append(&transcript, &[]);
    let mut screen = fixture.open();
    screen.answer_from(Transcript::at(Host::Codex, &transcript));
    at(&mut screen, 9);
    keys(&mut screen, "a");
    type_and_save(&mut screen, "why?");
    wait_for(&mut screen, "not supported for codex");
    assert!(fixture.sent().is_empty(), "nothing sent to an agent whose answer cannot be read");
}

#[test]
fn a_question_after_an_answer_longer_than_the_pane_opens_at_its_end() {
    let (fixture, mut screen, transcript) = asking("ask-long");
    at(&mut screen, 9);
    let question = ask(&fixture, &mut screen, "explain it all");
    let long = (1..=40).map(|n| format!("answer line {n}")).collect::<Vec<_>>().join("\n\n");
    append(&transcript, &[user("p2", Some("r1"), &question), assistant("r2", Some("p2"), &long)]);
    wait_for(&mut screen, "answer line 1");
    at(&mut screen, 9);
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    let rows = frame(&mut screen, 120, 40);
    let end = rows.iter().position(|row| row.contains("ask again")).expect("the answer box's end on screen");
    let asking = rows.iter().position(|row| row.contains("ask about app.py:9")).expect("the question box");
    assert_eq!(asking, end + 1, "the question box opens under the answer's end:\n{}", rows.join("\n"));
}
