use plannotator_tui_hosts::{Message, Role};

use super::answer;

fn prompt(id: &str) -> Message {
    Message { id: id.to_owned(), role: Role::Human, text: format!("prompt {id}"), at: None }
}

fn reply(id: &str, text: &str) -> Message {
    Message { id: id.to_owned(), role: Role::Assistant, text: text.to_owned(), at: None }
}

#[test]
fn the_answer_is_every_reply_after_the_first_prompt_after_the_mark_oldest_first() {
    let newest_first = [
        reply("a3", "and more"),
        reply("a2", "first part"),
        prompt("q"),
        reply("a1", "older"),
        prompt("mark"),
    ];
    assert_eq!(answer(&newest_first, Some("mark")), Some(("first part\n\nand more".to_owned(), false)));
}

#[test]
fn the_next_prompt_ends_the_answer() {
    let newest_first =
        [reply("a3", "to the next"), prompt("next"), reply("a2", "answer"), prompt("q"), prompt("mark")];
    assert_eq!(answer(&newest_first, Some("mark")), Some(("answer".to_owned(), true)));
}

#[test]
fn nothing_answers_until_the_question_is_in_the_transcript() {
    let newest_first = [reply("a1", "about something else"), prompt("mark")];
    assert_eq!(answer(&newest_first, Some("mark")), None);
}

#[test]
fn a_mark_out_of_reach_answers_nothing() {
    let newest_first = [reply("a2", "answer"), prompt("q")];
    assert_eq!(answer(&newest_first, Some("gone")), None);
}

#[test]
fn without_a_mark_the_first_prompt_is_the_question() {
    let newest_first = [reply("a1", "answer"), prompt("q")];
    assert_eq!(answer(&newest_first, None), Some(("answer".to_owned(), false)));
}
