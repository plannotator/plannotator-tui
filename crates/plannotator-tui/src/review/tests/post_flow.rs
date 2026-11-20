//! Posting a pull request review's notes to GitHub, against the simulated GitHub of
//! `pull_flow`: the `gh` stand-in answers `gh pr view` from `view.json` and records every
//! `gh api` call (its arguments, and the JSON on its stdin) instead of reaching GitHub.

use std::os::unix::fs::PermissionsExt as _;

use plannotator_tui_review::CodeAnnotation;
use ratatui::crossterm::event::KeyModifiers;

use super::pull_flow::{Pull, URL, git_in};
use super::*;
use crate::review::Mode;

const REVIEW_URL: &str = "https://github.com/o/app/pull/7#pullrequestreview-42";

impl Pull {
    /// Make the `gh` stand-in answer `gh api` with `stdout`, `stderr` and `exit`, recording
    /// each call's arguments in `api-args` and its stdin as one line of `api-requests.jsonl`.
    fn answer_api(&self, stdout: &str, stderr: &str, exit: i32) {
        let dir = self.dir.display().to_string();
        std::fs::write(self.dir.join("api-stdout"), stdout).expect("stdout");
        std::fs::write(self.dir.join("api-stderr"), stderr).expect("stderr");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = api ]; then\n  printf '%s\\n' \"$@\" >> '{dir}/api-args'\n  cat >> '{dir}/api-requests.jsonl'\n  echo >> '{dir}/api-requests.jsonl'\n  cat '{dir}/api-stdout'\n  cat '{dir}/api-stderr' >&2\n  exit {exit}\nfi\ncat '{dir}/view.json'\n"
        );
        let gh = self.dir.join("gh");
        std::fs::write(&gh, script).expect("gh");
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    fn answer_created(&self) {
        let reply = serde_json::json!({"id": 42, "html_url": REVIEW_URL, "state": "COMMENTED"});
        self.answer_api(&reply.to_string(), "", 0);
    }

    /// Every review `gh api` was handed, in order.
    fn requests(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.dir.join("api-requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("JSON"))
            .collect()
    }
}

fn ctrl_g(screen: &mut Screen) {
    screen.handle_event(Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))).expect("key");
}

fn comment_on(screen: &mut Screen, side: Side, line: u32, text: &str) {
    screen.diff.reveal("app.py", side, line).expect("in the diff");
    keys(screen, "c");
    type_and_save(screen, text);
}

/// A line comment, a range, an old-side comment, a suggestion, a file note, a whole-change
/// note and a note outside the diff.
fn every_kind_of_note(screen: &mut Screen) {
    comment_on(screen, Side::New, 5, "say hello properly");
    at(screen, 9);
    keys(screen, "vjv");
    keys(screen, "c");
    type_and_save(screen, "name the total");
    comment_on(screen, Side::Old, 10, "why drop the default?");
    at(screen, 14);
    keys(screen, "s");
    press(screen, KeyCode::End);
    type_and_save(screen, "  # from argv");
    keys(screen, "C");
    type_and_save(screen, "split this file");
    keys(screen, "M");
    type_and_save(screen, "please add a test");
    let outside = CodeAnnotation::outside_diff_comment(
        "app.py",
        15,
        15,
        "    print(add(1, 2))".into(),
        "is this still needed?".into(),
        &screen.capture.patch_key,
    );
    screen.store.add(outside).expect("adds");
}

#[test]
fn ctrl_g_shows_a_summary_and_posts_one_review_only_on_enter() {
    let pull = Pull::new("post");
    pull.answer_created();
    let mut screen = pull.open();
    every_kind_of_note(&mut screen);
    let head = git_in(&pull.work(), &["rev-parse", "feature"]);

    ctrl_g(&mut screen);
    assert!(matches!(screen.mode, Mode::Post(_)), "{:?}", screen.status);
    let rows = frame(&mut screen, 120, 40);
    if let Some(dir) = std::env::var_os("POST_GH_PROOF_DIR") {
        std::fs::create_dir_all(&dir).expect("proof dir");
        std::fs::write(PathBuf::from(dir).join("summary.txt"), rows.join("\n")).expect("proof");
    }
    row_with(&rows, "post to GitHub");
    assert!(row_with(&rows, "o/app#7").contains("feat: greet from argv"));
    row_with(&rows, &format!("anchored to {}, the commit you reviewed", &head[..7]));
    row_with(&rows, "4 inline comments");
    row_with(&rows, "3 in the review body:");
    row_with(&rows, "  1 whole-change note");
    row_with(&rows, "  1 file note");
    row_with(&rows, "  1 note outside the diff, quoted with a link");
    assert!(row_with(&rows, "verdict").contains("(\u{2022}) Comment"));
    row_with(&rows, "enter posts");
    assert_eq!(pull.requests().len(), 0, "nothing leaves before Enter");

    press(&mut screen, KeyCode::Enter);
    assert_eq!(screen.mode, Mode::Browse);
    let [request] = pull.requests().try_into().expect("one request");
    let args = read(&pull.dir.join("api-args"));
    assert_eq!(args, "api\n--method\nPOST\nrepos/o/app/pulls/7/reviews\n--input\n-\n");
    assert_eq!(request["commit_id"], head.as_str());
    assert_eq!(request["event"], "COMMENT");
    assert_eq!(
        request["comments"],
        serde_json::json!([
            {"path": "app.py", "line": 5, "side": "RIGHT", "body": "say hello properly"},
            {"path": "app.py", "line": 10, "side": "RIGHT", "start_line": 9, "start_side": "RIGHT", "body": "name the total"},
            {"path": "app.py", "line": 10, "side": "LEFT", "body": "why drop the default?"},
            {"path": "app.py", "line": 14, "side": "RIGHT", "body": "```suggestion\n    greet(sys.argv[1])  # from argv\n```"},
        ])
    );
    let body = request["body"].as_str().expect("a body");
    assert!(
        body.starts_with(
            "**app.py:** split this file\n\nplease add a test\n\n**Comments outside the diff**\n\n"
        ),
        "{body}"
    );
    assert!(body.contains(&format!("[app.py (L15)]({URL}/blob/{head}/app.py#L15)")), "{body}");

    let status = screen.status.clone().unwrap_or_default();
    assert_eq!(status, format!("posted to GitHub: 4 inline, 3 in the body \u{b7} {REVIEW_URL}"));
    let rows = frame(&mut screen, 120, 40);
    assert!(rows[39].contains(REVIEW_URL) || rows[38].contains(REVIEW_URL), "the footer shows the link");
    row_with(&rows, "5 new \u{b7} posted");
    let record: serde_json::Value = serde_json::from_str(&read(screen.store.path())).expect("JSON");
    assert_eq!(record["posted"][0]["url"], REVIEW_URL);
    assert_eq!(record["posted"][0]["commitId"], head.as_str());
    assert_eq!(record["posted"][0]["annotationIds"].as_array().map(Vec::len), Some(7));
    assert_eq!(pull.agent.sent.borrow().len(), 0, "posting is not sending to the agent");
}

#[test]
fn esc_on_the_summary_posts_nothing() {
    let pull = Pull::new("post-esc");
    pull.answer_created();
    let mut screen = pull.open();
    comment_on(&mut screen, Side::New, 9, "name the total");
    ctrl_g(&mut screen);
    keys(&mut screen, "l");
    let Mode::Post(confirm) = &screen.mode else { panic!("the summary") };
    assert_eq!(confirm.event, plannotator_tui_review::Event::Approve, "an explicit choice");
    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(pull.requests().len(), 0);
    assert_eq!(screen.status.as_deref(), Some("nothing posted to GitHub"));
    assert!(!screen.store.is_posted(&screen.store.annotations()[0]));
}

#[test]
fn a_later_post_carries_only_new_and_edited_notes() {
    let pull = Pull::new("post-again");
    pull.answer_created();
    let mut screen = pull.open();
    comment_on(&mut screen, Side::New, 5, "say hello properly");
    comment_on(&mut screen, Side::New, 9, "name the total");
    ctrl_g(&mut screen);
    press(&mut screen, KeyCode::Enter);
    ctrl_g(&mut screen);
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(screen.status.as_deref(), Some("nothing new to post to GitHub (2 already posted)"));

    // Edit the note on line 9 and add one on line 14.
    let edited = screen.store.annotations().iter().find(|n| n.line_start == 9).expect("note").id.clone();
    screen
        .store
        .edit(&edited, |n| {
            n.text = Some("call it total".into());
            true
        })
        .expect("edits");
    comment_on(&mut screen, Side::New, 14, "check argv");
    ctrl_g(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    row_with(&rows, "2 inline comments");
    row_with(&rows, "1 edited since posted: posted again as new comments");
    row_with(&rows, "skipped: 1 already posted");
    press(&mut screen, KeyCode::Enter);
    let requests = pull.requests();
    let [_, second] = requests.as_slice() else { panic!("two posts: {requests:?}") };
    let bodies: Vec<&str> =
        second["comments"].as_array().expect("comments").iter().filter_map(|c| c["body"].as_str()).collect();
    assert_eq!(bodies, ["call it total", "check argv"]);
}

#[test]
fn a_refused_post_posts_nothing_and_names_the_comments() {
    let pull = Pull::new("post-refused");
    pull.answer_api(
        r#"{"message":"Unprocessable Entity","errors":["Line could not be resolved"],"status":"422"}"#,
        "gh: Unprocessable Entity (HTTP 422)\n",
        1,
    );
    let mut screen = pull.open();
    comment_on(&mut screen, Side::New, 9, "name the total");
    comment_on(&mut screen, Side::Old, 10, "why drop the default?");
    ctrl_g(&mut screen);
    press(&mut screen, KeyCode::Enter);
    let Mode::Post(confirm) = &screen.mode else { panic!("the summary stays open") };
    let failed = confirm.failed.clone().expect("a reason");
    assert_eq!(
        failed,
        "Nothing was posted: GitHub refused the place of an inline comment (Unprocessable Entity: Line could \
         not be resolved). The inline comments, each checked against the diff you reviewed, were on app.py:9 \
         new, app.py:10 old."
    );
    let rows = frame(&mut screen, 120, 40);
    row_with(&rows, "Nothing was posted");
    assert!(screen.store.posts().is_empty());
    assert!(screen.store.annotations().iter().all(|n| !screen.store.is_posted(n)));
    assert_eq!(screen.status.as_deref(), Some("nothing posted to GitHub: GitHub refused a comment's place"));
}

#[test]
fn gh_errors_are_named_and_post_nothing() {
    let cases = [
        ("", "To get started with GitHub CLI, please run:  gh auth login\n", "gh is not logged in to GitHub"),
        (
            r#"{"message":"Resource not accessible by integration"}"#,
            "gh: Resource not accessible by integration (HTTP 403)\n",
            "your GitHub login cannot review this pull request",
        ),
        (
            r#"{"message":"API rate limit exceeded for user ID 1."}"#,
            "gh: API rate limit exceeded for user ID 1. (HTTP 403)\n",
            "GitHub's rate limit was reached",
        ),
        ("", "error connecting to api.github.com\n", "could not reach GitHub"),
    ];
    for (stdout, stderr, says) in cases {
        let pull = Pull::new("post-errors");
        pull.answer_api(stdout, stderr, 1);
        let mut screen = pull.open();
        comment_on(&mut screen, Side::New, 9, "name the total");
        ctrl_g(&mut screen);
        press(&mut screen, KeyCode::Enter);
        let Mode::Post(confirm) = &screen.mode else { panic!("the summary stays open") };
        let failed = confirm.failed.clone().unwrap_or_default();
        assert!(failed.starts_with(&format!("Nothing was posted: {says}")), "{failed}");
        assert!(screen.store.posts().is_empty());
    }
}

#[test]
fn when_the_pr_moved_on_the_summary_says_so_and_comments_stay_on_the_reviewed_commit() {
    let pull = Pull::new("post-moved");
    pull.answer_created();
    let mut screen = pull.open();
    let reviewed = git_in(&pull.work(), &["rev-parse", "feature"]);
    comment_on(&mut screen, Side::New, 9, "name the total");
    pull.push_head(&format!("# greeter\n{AFTER}"));
    let now = git_in(&pull.work(), &["rev-parse", "feature"]);
    ctrl_g(&mut screen);
    let rows = frame(&mut screen, 120, 40);
    let moved = rows.iter().find(|r| r.contains("The PR moved on")).expect("the warning");
    assert!(moved.contains(&now[..7]), "{moved}");
    press(&mut screen, KeyCode::Enter);
    let [request] = pull.requests().try_into().expect("one request");
    assert_eq!(request["commit_id"], reviewed.as_str());
    assert_eq!(request["comments"][0]["line"], 9);
}

#[test]
fn the_review_menu_offers_it_and_outside_a_pr_it_says_why_not() {
    let pull = Pull::new("post-menu");
    pull.answer_created();
    let mut screen = pull.open();
    comment_on(&mut screen, Side::New, 9, "name the total");
    keys(&mut screen, "m");
    let rows = frame(&mut screen, 120, 40);
    assert!(row_with(&rows, "Post to GitHub\u{2026} \u{b7} 1 new").contains("ctrl+g"));
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    assert!(matches!(screen.mode, Mode::Post(_)));

    let fixture = Fixture::new("post-local");
    let mut local = fixture.open();
    keys(&mut local, "m");
    let rows = frame(&mut local, 120, 40);
    row_with(&rows, "Post to GitHub\u{2026} \u{b7} pull requests only");
    press(&mut local, KeyCode::Esc);
    ctrl_g(&mut local);
    assert_eq!(local.mode, Mode::Browse);
    assert_eq!(
        local.status.as_deref(),
        Some(
            "Post to GitHub needs a pull request review (plannotator-tui review <PR>); this one is Uncommitted"
        )
    );
}

#[test]
fn a_note_both_sent_and_posted_says_so_even_when_its_title_has_no_room() {
    let pull = Pull::new("post-both");
    pull.answer_created();
    let mut screen = pull.open();
    let outside = CodeAnnotation::outside_diff_comment(
        "app.py",
        14,
        15,
        "    greet(sys.argv[1])\n    print(add(1, 2))".into(),
        "both lines?".into(),
        &screen.capture.patch_key,
    );
    screen.store.add(outside).expect("adds");
    keys(&mut screen, "E");
    ctrl_g(&mut screen);
    press(&mut screen, KeyCode::Enter);
    let rows = frame(&mut screen, 120, 40);
    let title = row_with(&rows, "app.py 14-15 new");
    assert!(title.trim_end().ends_with('\u{256e}'), "the border is whole: {title}");
    let below = rows.iter().position(|r| r.contains("app.py 14-15 new")).map(|i| &rows[i + 1]).expect("row");
    assert!(below.contains("sent \u{b7} posted"), "{rows:#?}");
}
