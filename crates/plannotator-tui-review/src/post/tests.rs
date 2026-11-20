//! The expected bodies follow the browser's `ReviewSubmissionDialog.tsx` builders
//! (`buildAnnotationFileComments`, `buildFileScopedBody`, `buildOutsideDiffSection`).
#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use serde_json::json;

use super::submit::classify;
use super::*;
use crate::patch::parse;

const HEAD: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

/// `app.py`: old 1 one, 2 two, 3 four, 4 five, 5 six; new 1 one, 2 TWO, 3 three, 4 four,
/// 5 five, 6 six. Line 40 of either side is not in the diff.
fn files() -> Vec<FilePatch> {
    parse(
        "diff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1,5 +1,6 @@\n one\n-two\n+TWO\n+three\n four\n five\n six\n",
    )
}

fn repo() -> PrRepo {
    PrRepo { host: "github.com".into(), owner: "o".into(), name: "app".into() }
}

fn note(json: Value) -> CodeAnnotation {
    let mut value = json;
    if let Some(object) = value.as_object_mut() {
        object.entry("createdAt").or_insert(1.into());
        object.entry("type").or_insert("comment".into());
        object.entry("scope").or_insert("line".into());
        object.entry("filePath").or_insert("app.py".into());
        object.entry("side").or_insert("new".into());
    }
    serde_json::from_value(value).expect("a note")
}

fn line(id: &str, side: &str, first: u32, last: u32, text: &str) -> CodeAnnotation {
    note(json!({"id": id, "side": side, "lineStart": first, "lineEnd": last, "text": text}))
}

fn plan_with(notes: &[CodeAnnotation], posting: impl Fn(&CodeAnnotation) -> Posting) -> Plan {
    let repo = repo();
    Plan::build(notes, &files(), Target { repo: &repo, number: 7, reviewed_head: HEAD }, posting)
}

fn plan(notes: &[CodeAnnotation]) -> Plan {
    plan_with(notes, |_| Posting::New)
}

fn request_json(plan: &Plan, event: Event) -> Value {
    serde_json::to_value(plan.request(event)).expect("serializes")
}

#[test]
fn a_one_line_note_is_an_inline_comment_on_the_right_without_start_fields() {
    let plan = plan(&[line("a", "new", 2, 2, "name it")]);
    assert_eq!(
        request_json(&plan, Event::Comment),
        json!({
            "commit_id": HEAD,
            "body": "See inline comments.",
            "event": "COMMENT",
            "comments": [{"path": "app.py", "line": 2, "side": "RIGHT", "body": "name it"}],
        })
    );
    assert_eq!(plan.inline, ["a"]);
}

#[test]
fn a_range_carries_start_line_and_start_side_on_the_same_side() {
    let plan = plan(&[line("a", "new", 2, 3, "split these")]);
    assert_eq!(
        serde_json::to_value(&plan.comments).expect("JSON"),
        json!([{"path": "app.py", "line": 3, "side": "RIGHT", "start_line": 2, "start_side": "RIGHT", "body": "split these"}])
    );
}

#[test]
fn old_side_lines_go_on_the_left() {
    let plan = plan(&[line("a", "old", 2, 2, "why drop two?"), line("b", "old", 1, 2, "and this")]);
    let sides: Vec<_> = plan.comments.iter().map(|c| (c.side, c.start_side, c.line)).collect();
    assert_eq!(sides, [("LEFT", None, 2), ("LEFT", Some("LEFT"), 2)]);
}

#[test]
fn a_suggestion_is_a_suggestion_block_after_its_text() {
    let with_text = note(json!({"id": "s", "type": "suggestion", "lineStart": 2, "lineEnd": 3,
        "text": "fold these", "suggestedCode": "TWO_AND_THREE", "originalCode": "TWO\nthree"}));
    let bare = note(json!({"id": "t", "type": "suggestion", "lineStart": 2, "lineEnd": 2,
        "suggestedCode": "Two"}));
    let plan = plan(&[with_text, bare]);
    let bodies: Vec<&str> = plan.comments.iter().map(|c| c.body.as_str()).collect();
    assert_eq!(bodies, ["fold these\n\n```suggestion\nTWO_AND_THREE\n```", "```suggestion\nTwo\n```"]);
}

#[test]
fn file_notes_and_whole_change_notes_go_in_the_body_in_note_order() {
    let notes = [
        note(json!({"id": "f", "scope": "file", "lineStart": 1, "lineEnd": 1, "text": "split this file"})),
        line("a", "new", 2, 2, "name it"),
        note(json!({"id": "g", "scope": "general", "filePath": "", "lineStart": 0, "lineEnd": 0,
            "text": "please add a test"})),
    ];
    let plan = plan(&notes);
    assert_eq!(plan.body, "**app.py:** split this file\n\nplease add a test");
    assert_eq!(request_json(&plan, Event::Comment)["body"], plan.body, "no placeholder beside a body");
    assert_eq!(plan.in_body, [("f".to_owned(), InBody::File), ("g".to_owned(), InBody::Change)]);
    assert_eq!(plan.comments.len(), 1);
}

#[test]
fn notes_outside_the_diff_close_the_body_quoted_and_linked_never_inline() {
    let outside = note(json!({"id": "o", "lineStart": 40, "lineEnd": 41, "text": "Why return silently?",
        "anchorText": "if (!t) {\n  return;", "originalCode": "if (!t) {\n  return;", "outsideDiff": true}));
    let general = note(json!({"id": "g", "scope": "general", "filePath": "", "lineStart": 0, "lineEnd": 0,
        "text": "overall fine"}));
    let plan = plan(&[outside, general]);
    assert_eq!(plan.comments, [], "never inline");
    assert_eq!(
        plan.body,
        format!(
            "overall fine\n\n**Comments outside the diff**\n\n**[app.py (L40-L41)](https://github.com/o/app/blob/{HEAD}/app.py#L40-L41):** Why return silently?\n\n```\nif (!t) {{\n  return;\n```"
        )
    );
    assert_eq!(plan.count_in_body(InBody::OutsideDiff), 1);
}

#[test]
fn quoted_code_outside_the_diff_is_capped_and_fenced_past_its_own_backticks() {
    let code: Vec<String> = (1..=45).map(|n| format!("line {n} ```")).collect();
    let outside = note(json!({"id": "o", "lineStart": 50, "lineEnd": 94, "text": "long",
        "anchorText": code.join("\n"), "outsideDiff": true, "filePath": "src/a b.rs"}));
    let body = plan(&[outside]).body;
    assert!(body.contains("(https://github.com/o/app/blob/"), "{body}");
    assert!(body.contains("/src/a%20b.rs#L50-L94):** long\n\n````\nline 1 ```\n"), "{body}");
    assert!(body.ends_with("line 40 ```\n\u{2026}\n````"), "{body}");
}

#[test]
fn outdated_notes_and_notes_off_the_diff_go_in_the_body_with_their_code() {
    let mut outdated = line("x", "new", 2, 2, "was wrong");
    outdated.outdated = true;
    outdated.anchor_text = Some("TWO".into());
    let mut off = line("y", "old", 40, 40, "unchanged now");
    off.plannotator_tui.resolution = Some(Status::NotInDiff);
    let gone = note(json!({"id": "z", "type": "suggestion", "lineStart": 40, "lineEnd": 41,
        "suggestedCode": "fixed()"}));
    let plan = plan(&[outdated, off, gone]);
    assert_eq!(plan.comments, []);
    assert_eq!(
        plan.body,
        format!(
            "**app.py (L2, new):** {OUTDATED_ANNOTATION_LABEL} was wrong\n\nCommented on:\n```\nTWO\n```\n\n\
             **app.py (L40, old):** unchanged now\n\n\
             **app.py (L40-L41, new):** \n\nSuggested code:\n```\nfixed()\n```"
        )
    );
    assert_eq!(plan.count_in_body(InBody::OffDiff), 3);
}

#[test]
fn posted_notes_are_left_out_and_edited_ones_post_again_as_new() {
    let notes =
        [line("a", "new", 2, 2, "one"), line("b", "new", 3, 3, "two"), line("c", "old", 2, 2, "three")];
    let plan = plan_with(&notes, |n| match n.id.as_str() {
        "a" => Posting::Posted,
        "b" => Posting::Edited,
        _ => Posting::New,
    });
    assert_eq!(plan.inline, ["b", "c"]);
    assert_eq!(plan.skipped, [("a".to_owned(), Skip::AlreadyPosted)]);
    assert_eq!(plan.reposted, ["b"]);
    assert_eq!(plan.ids(), ["b", "c"]);
    let all_posted = plan_with(&notes, |_| Posting::Posted);
    assert_eq!((all_posted.comments.len(), all_posted.body.as_str()), (0, ""), "nothing to post");
    assert_eq!(all_posted.count_skipped(Skip::AlreadyPosted), 3);
}

#[test]
fn comments_stay_on_the_reviewed_commit_even_when_the_pr_moved_on() {
    // The plan only knows the commit the diff was captured at; a newer head on GitHub does
    // not move a comment onto lines the person never saw.
    let outside = note(json!({"id": "o", "lineStart": 40, "lineEnd": 40, "text": "here",
        "anchorText": "x", "outsideDiff": true}));
    let plan = plan(&[line("a", "new", 2, 2, "name it"), outside]);
    assert_eq!(plan.request(Event::Comment).commit_id, HEAD);
    assert!(plan.body.contains(&format!("/blob/{HEAD}/app.py#L40)")), "{}", plan.body);
    assert!(matches!(
        classify(
            "{\"message\":\"Unprocessable Entity\",\"errors\":[\"commit_id is not part of the pull request\"]}",
            "gh: Unprocessable Entity (HTTP 422)"
        ),
        PostError::HeadGone { .. }
    ));
}

#[test]
fn empty_notes_are_skipped_and_an_approval_needs_no_placeholder() {
    let plan = plan(&[line("a", "new", 2, 2, ""), line("b", "new", 3, 3, "ok")]);
    assert_eq!(plan.skipped, [("a".to_owned(), Skip::Empty)]);
    assert_eq!(request_json(&plan, Event::Approve)["body"], "");
    assert_eq!(request_json(&plan, Event::Approve)["event"], "APPROVE");
    assert_eq!(request_json(&plan, Event::RequestChanges)["body"], BODY_PLACEHOLDER);
    assert_eq!(request_json(&plan, Event::RequestChanges)["event"], "REQUEST_CHANGES");
}

#[test]
fn a_browser_note_keeps_its_conventional_comment_prefix() {
    let mut browser = line("a", "new", 2, 2, "missing check");
    browser.other.insert("conventionalLabel".into(), "issue".into());
    browser.other.insert("decorations".into(), json!(["blocking", "security"]));
    let plan = plan(&[browser]);
    assert_eq!(plan.comments[0].body, "**issue (blocking, security):** missing check");
}

#[test]
fn gh_failures_are_named_for_the_person() {
    let cases = [
        ("", "To get started with GitHub CLI, please run:  gh auth login", "NotLoggedIn"),
        ("{\"message\":\"Bad credentials\"}", "gh: Bad credentials (HTTP 401)", "NotLoggedIn"),
        (
            "{\"message\":\"Resource not accessible by integration\"}",
            "gh: Resource not accessible by integration (HTTP 403)",
            "NoPermission",
        ),
        ("{\"message\":\"Not Found\"}", "gh: Not Found (HTTP 404)", "NoPermission"),
        (
            "{\"message\":\"API rate limit exceeded for user ID 1.\"}",
            "gh: API rate limit exceeded (HTTP 403)",
            "RateLimited",
        ),
        (
            "",
            "error connecting to api.github.com\ncheck your internet connection or https://githubstatus.com",
            "Offline",
        ),
        (
            "{\"message\":\"Unprocessable Entity\",\"errors\":[\"Line could not be resolved\"]}",
            "gh: Unprocessable Entity (HTTP 422)",
            "Positions",
        ),
        (
            "{\"message\":\"Unprocessable Entity\",\"errors\":[\"Can not approve your own pull request\"]}",
            "gh: Unprocessable Entity (HTTP 422)",
            "OwnPullRequest",
        ),
        ("", "boom", "Other"),
    ];
    for (stdout, stderr, want) in cases {
        let got = classify(stdout, stderr);
        assert!(format!("{got:?}").starts_with(want), "{stderr}: {got:?}");
    }
    let positions = classify(
        "{\"message\":\"Unprocessable Entity\",\"errors\":[\"Line could not be resolved\"],\"status\":\"422\"}",
        "gh: Unprocessable Entity (HTTP 422)",
    );
    assert_eq!(
        positions.to_string(),
        "GitHub refused the place of an inline comment (Unprocessable Entity: Line could not be resolved)"
    );
}
