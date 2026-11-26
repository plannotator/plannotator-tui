use plannotator_tui_hosts::{Host, Message, Role};

use super::{Found, answer};
use crate::last::Transcript;

fn prompt(id: &str, text: &str) -> Message {
    Message { id: id.to_owned(), role: Role::Human, text: text.to_owned(), at: None }
}

fn reply(id: &str, text: &str) -> Message {
    Message { id: id.to_owned(), role: Role::Assistant, text: text.to_owned(), at: None }
}

fn said(text: &str, complete: bool) -> Found {
    Found::Answer { text: text.to_owned(), complete }
}

const MARKS: [&str; 2] = ["about `a.rs:9`", "why?"];

#[test]
fn the_answer_is_every_reply_after_the_question_oldest_first() {
    let newest_first = [
        reply("a3", "and more"),
        reply("a2", "first part"),
        prompt("q", "about `a.rs:9`: why?"),
        reply("a1", "older"),
        prompt("mark", "earlier"),
    ];
    assert_eq!(answer(&newest_first, Some("mark"), &MARKS), said("first part\n\nand more", false));
}

#[test]
fn the_next_prompt_ends_the_answer() {
    let newest_first = [
        reply("a3", "to the next"),
        prompt("next", "else"),
        reply("a2", "answer"),
        prompt("q", "about `a.rs:9`: why?"),
    ];
    assert_eq!(answer(&newest_first, None, &MARKS), said("answer", true));
}

#[test]
fn a_prompt_typed_meanwhile_or_another_waiting_question_is_not_taken_for_the_question() {
    let newest_first = [
        reply("a2", "the answer"),
        prompt("q", "about `a.rs:9`: why?"),
        reply("a1", "to the other"),
        prompt("other", "about `a.rs:3`: what?"),
        prompt("mark", "earlier"),
    ];
    assert_eq!(answer(&newest_first, Some("mark"), &MARKS), said("the answer", false));
}

#[test]
fn nothing_answers_until_the_question_is_in_the_transcript() {
    let newest_first =
        [reply("a1", "about something else"), prompt("other", "a prompt"), prompt("mark", "earlier")];
    assert_eq!(answer(&newest_first, Some("mark"), &MARKS), Found::Waiting);
}

#[test]
fn a_noted_prompt_out_of_reach_loses_the_question() {
    let newest_first = [reply("a2", "answer"), prompt("q", "about `a.rs:9`: why?")];
    assert_eq!(answer(&newest_first, Some("gone"), &MARKS), Found::Lost);
}

#[test]
fn a_copilot_session_is_watched_by_the_file_it_appends_to() {
    let dir = std::path::Path::new("/sessions/abc");
    assert_eq!(Transcript::at(Host::Copilot, dir).file(), dir.join("events.jsonl"));
    let file = std::path::Path::new("/projects/x/1.jsonl");
    assert_eq!(Transcript::at(Host::ClaudeCode, file).file(), file);
}
