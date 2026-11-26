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
fn a_question_reaches_the_agent_with_its_lines_and_their_code() {
    let (fixture, mut screen, _transcript) = asking("ask-prompt");
    at(&mut screen, 9);
    keys(&mut screen, "a");
    row_with(&frame(&mut screen, 120, 40), "ask about app.py:9");
    press(&mut screen, KeyCode::Esc);
    let question = ask(&fixture, &mut screen, "why a local?");
    assert!(question.starts_with("A question from the code review of Uncommitted changes in "), "{question}");
    assert!(question.contains(", about `app.py:9`:\n\n```\n    total = a + b\n```\n"), "{question}");
    assert!(question.ends_with("\nwhy a local?"), "{question}");
    assert_eq!(screen.mode, Mode::Browse);
    assert!(screen.store.annotations().is_empty(), "a question is never a note");
}

#[test]
fn its_answer_shows_in_a_box_under_the_lines_under_who_wrote_what() {
    let (fixture, mut screen, transcript) = asking("ask-answer");
    at(&mut screen, 9);
    let question = ask(&fixture, &mut screen, "why a local?");
    let rows = frame(&mut screen, 120, 40);
    row_with(&rows, "ask \u{b7} L9");
    row_with(&rows, "waiting for the answer");
    row_with(&rows, "ask again (\u{23ce} Enter) \u{b7} remove (\u{2326} Delete)");
    append(
        &transcript,
        &[user("p2", Some("r1"), &question), assistant("r2", Some("p2"), "It names the **sum**.")],
    );
    let rows = wait_for(&mut screen, "It names the sum.");
    assert!(!rows.iter().any(|row| row.contains("an earlier reply")), "only the answer to this question");
    let row = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap_or(usize::MAX);
    assert!(
        row("total = a + b") < row("You")
            && row("You") < row("why a local?")
            && row("why a local?") < row("claude")
            && row("claude") < row("It names the sum.")
            && row("It names the sum.") < row("return total"),
        "under line 9, each message under who wrote it:\n{}",
        rows.join("\n")
    );
}

#[test]
fn asking_again_from_the_box_goes_into_it_with_the_question_box_at_its_end() {
    let (fixture, mut screen, transcript) = asking("ask-again");
    at(&mut screen, 9);
    let question = ask(&fixture, &mut screen, "why a local?");
    append(
        &transcript,
        &[user("p2", Some("r1"), &question), assistant("r2", Some("p2"), "It names the sum.")],
    );
    wait_for(&mut screen, "It names the sum.");
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    let rows = frame(&mut screen, 120, 40);
    let end = rows.iter().position(|row| row.contains("ask again")).expect("the answer box's end");
    let asking = rows.iter().position(|row| row.contains("ask about app.py:9")).expect("the question box");
    assert_eq!(asking, end + 1, "the question box opens under the answer box:\n{}", rows.join("\n"));
    type_and_save(&mut screen, "and if b is negative?");
    wait(&mut screen, "the follow-up sent", |_| fixture.sent().len() == 2);
    let rows = frame(&mut screen, 120, 40);
    assert_eq!(rows.iter().filter(|row| row.contains("ask \u{b7} L9")).count(), 1, "one box for the lines");
    row_with(&rows, "and if b is negative?");
}

#[test]
fn the_remove_key_removes_a_box_from_the_box_or_from_its_lines() {
    let (fixture, mut screen, _transcript) = asking("ask-remove");
    let gone = |screen: &mut Screen| !frame(screen, 120, 40).iter().any(|row| row.contains("ask \u{b7} L9"));
    at(&mut screen, 9);
    ask(&fixture, &mut screen, "why?");
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Delete);
    assert!(gone(&mut screen), "removed from the box");
    at(&mut screen, 9);
    keys(&mut screen, "vjv");
    ask(&fixture, &mut screen, "and these?");
    at(&mut screen, 10);
    press(&mut screen, KeyCode::Delete);
    assert!(gone(&mut screen), "removed from one of its lines");
}

#[test]
fn two_questions_waiting_at_once_each_get_their_own_answer() {
    let (fixture, mut screen, transcript) = asking("ask-two");
    at(&mut screen, 9);
    let first = ask(&fixture, &mut screen, "why a local?");
    at(&mut screen, 14);
    let second = ask(&fixture, &mut screen, "what if argv is empty?");
    // The agent was busy: both questions arrive later, in its own order.
    append(
        &transcript,
        &[
            user("p2", Some("r1"), &second),
            assistant("r2", Some("p2"), "Then it raises IndexError."),
            user("p3", Some("r2"), &first),
            assistant("r3", Some("p3"), "It names the sum."),
        ],
    );
    let rows = wait(&mut screen, "both answers", |rows| {
        rows.iter().any(|row| row.contains("It names the sum."))
            && rows.iter().any(|row| row.contains("IndexError"))
    });
    let row = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap_or(usize::MAX);
    assert!(
        row("It names the sum.") < row("greet(sys.argv[1])") && row("greet(sys.argv[1])") < row("IndexError"),
        "each answer in its own box:\n{}",
        rows.join("\n")
    );
}

#[test]
fn an_answer_that_ends_without_any_text_says_so() {
    let (fixture, mut screen, transcript) = asking("ask-empty");
    at(&mut screen, 9);
    let question = ask(&fixture, &mut screen, "why?");
    append(&transcript, &[user("p2", Some("r1"), &question), user("p3", Some("p2"), "something else")]);
    wait_for(&mut screen, "no answer");
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
fn an_ask_box_and_a_notes_box_on_one_line_both_show() {
    let fixture = Fixture::new("ask-beside-note");
    fixture.write_config("[review]\nlayout = \"unified\"\nnotes = \"inline\"\n");
    let transcript = fixture.dir.join("transcript.jsonl");
    append(&transcript, &[user("p1", None, "earlier"), assistant("r1", Some("p1"), "an earlier reply")]);
    let mut screen = fixture.open_configured();
    screen.answer_from(Transcript::at(Host::ClaudeCode, &transcript));
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    at(&mut screen, 9);
    ask(&fixture, &mut screen, "why a local?");
    let rows = frame(&mut screen, 120, 40);
    row_with(&rows, "name the total");
    row_with(&rows, "ask \u{b7} L9");
}
