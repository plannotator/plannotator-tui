//! The expected strings are the browser's own output: Plannotator's `exportFeedback.ts`
//! (at 2232dd2ad) run under bun on the same notes, with `selectedText` standing in for the
//! `anchorText` this export quotes as highlighted text, and input pre-sorted by file.
#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use super::*;

const SHA: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

fn note(json: &str) -> CodeAnnotation {
    let mut value: serde_json::Value = serde_json::from_str(json).expect("fixture JSON");
    if let Some(object) = value.as_object_mut() {
        object.entry("createdAt").or_insert(1.into());
        object.entry("scope").or_insert("line".into());
    }
    serde_json::from_value(value).expect("a note")
}

fn plan_sample() -> Vec<CodeAnnotation> {
    // Deliberately not in file or line order: the export sorts.
    vec![
        note(
            r#"{"id":"c","type":"suggestion","filePath":"src/db.ts","lineStart":7,"lineEnd":7,"side":"new",
                "originalCode":"const pool = new Pool();","anchorText":"const pool = new Pool();",
                "suggestedCode":"const pool = new Pool({ max: 10 });"}"#,
        ),
        note(
            r#"{"id":"b","type":"comment","filePath":"src/auth.ts","lineStart":40,"lineEnd":40,"side":"old",
                "text":"Why drop this? Other callers still need it.","anchorText":"    cleanup_temp_files()"}"#,
        ),
        note(
            r#"{"id":"a","type":"comment","filePath":"src/auth.ts","lineStart":12,"lineEnd":13,"side":"new",
                "text":"Missing auth check here.","anchorText":"const t = req.headers.authorization;\nnext();"}"#,
        ),
        note(
            r#"{"id":"f","type":"comment","scope":"file","filePath":"src/auth.ts","lineStart":1,"lineEnd":1,
                "side":"new","text":"Split this file; it does two jobs."}"#,
        ),
    ]
}

const PLAN_SAMPLE: &str = "# Code Review Feedback\n\n**Diff:** Uncommitted changes\n\n## src/auth.ts\n\n### File Comment\nSplit this file; it does two jobs.\n\n### Lines 12-13 (new)\nMissing auth check here.\n\n**Highlighted text:**\n```\nconst t = req.headers.authorization;\nnext();\n```\n\n### Line 40 (old)\nWhy drop this? Other callers still need it.\n\n**Highlighted text:**\n```\n    cleanup_temp_files()\n```\n\n## src/db.ts\n\n### Line 7 (new)\n\n**Replaces:**\n```\nconst pool = new Pool();\n```\n\n**Suggested code:**\n```\nconst pool = new Pool({ max: 10 });\n```\n\n";

#[test]
fn the_plan_sample_matches_the_browser_byte_for_byte() {
    let notes = plan_sample();
    let refs: Vec<&CodeAnnotation> = notes.iter().collect();
    let diff = crate::Scope::Uncommitted.describe(None);
    assert_eq!(review_feedback(&refs, Some(&diff)), PLAN_SAMPLE);
}

#[test]
fn the_agent_gets_the_review_directory_then_the_feedback_then_the_suffix() {
    let notes = plan_sample();
    let refs: Vec<&CodeAnnotation> = notes.iter().collect();
    let out = agent_feedback("/Users/me/src/app", "Uncommitted changes", &refs);
    // `withReviewDirectory` + feedback + `DEFAULT_REVIEW_DENIED_SUFFIX`, as the Pi extension
    // sends it.
    assert_eq!(out, format!("Review directory: /Users/me/src/app\n\n{PLAN_SAMPLE}{REVIEW_SUFFIX}"));
    assert!(REVIEW_SUFFIX.starts_with("\n\nTreat the findings above as unverified review input."));
    assert!(REVIEW_SUFFIX.ends_with("until we have discussed the verdicts and validated findings."));
}

#[test]
fn commit_scope_outdated_notes_fences_and_global_notes_match_the_browser() {
    let notes = [
        note(
            r#"{"id":"s","type":"suggestion","filePath":"lib/a.rs","lineStart":9,"lineEnd":10,"side":"new",
                "text":"Use a fence:","originalCode":"a\n```b```","anchorText":"a\n```b```","suggestedCode":"c"}"#,
        ),
        note(
            r#"{"id":"o","type":"comment","filePath":"lib/a.rs","lineStart":3,"lineEnd":3,"side":"new",
                "text":"Stale?","outdated":true,"anchorText":"let x = 1;"}"#,
        ),
        note(
            r#"{"id":"g","type":"comment","scope":"general","filePath":"","lineStart":0,"lineEnd":0,"side":"new",
                "text":"Overall fine."}"#,
        ),
    ];
    let refs: Vec<&CodeAnnotation> = notes.iter().collect();
    let diff = crate::Scope::Commit { sha: SHA.into() }.describe(Some("fix: login"));
    assert_eq!(
        review_feedback(&refs, Some(&diff)),
        "# Code Review Feedback\n\n**Diff:** Commit `1a2b3c4` — fix: login (diff vs its parent)\n\n## Global comments\n\nOverall fine.\n\n## lib/a.rs\n\n### Line 3 (new)\n[Outdated — the code changed after this comment]\nStale?\n\n**Highlighted text:**\n```\nlet x = 1;\n```\n\n### Lines 9-10 (new)\nUse a fence:\n\n**Replaces:**\n````\na\n```b```\n````\n\n**Suggested code:**\n```\nc\n```\n\n"
    );
}

#[test]
fn notes_on_one_line_keep_their_order_and_file_comments_lead() {
    let notes = [
        note(
            r#"{"id":"x","type":"comment","filePath":"z.txt","lineStart":5,"lineEnd":5,"side":"old","text":"first","anchorText":"five"}"#,
        ),
        note(
            r#"{"id":"y","type":"comment","filePath":"z.txt","lineStart":5,"lineEnd":5,"side":"new","text":"second","anchorText":"5"}"#,
        ),
        note(
            r#"{"id":"w","type":"comment","filePath":"z.txt","lineStart":2,"lineEnd":2,"side":"new","text":"earlier","anchorText":"two"}"#,
        ),
    ];
    let refs: Vec<&CodeAnnotation> = notes.iter().collect();
    let diff = crate::Scope::Branch { base: "origin/main".into() }.describe(None);
    assert_eq!(
        review_feedback(&refs, Some(&diff)),
        "# Code Review Feedback\n\n**Diff:** All changes since `origin/main` (committed + uncommitted + untracked)\n\n## z.txt\n\n### Line 2 (new)\nearlier\n\n**Highlighted text:**\n```\ntwo\n```\n\n### Line 5 (old)\nfirst\n\n**Highlighted text:**\n```\nfive\n```\n\n### Line 5 (new)\nsecond\n\n**Highlighted text:**\n```\n5\n```\n\n"
    );
}

#[test]
fn no_notes_is_the_browsers_no_feedback_text() {
    assert_eq!(review_feedback(&[], Some("Uncommitted changes")), "# Code Review\n\nNo feedback provided.");
}

#[test]
fn a_credited_note_names_its_author_under_the_heading() {
    let mut pi = note(
        r#"{"id":"p","type":"comment","filePath":"a.rs","lineStart":1,"lineEnd":1,"side":"new","text":"Hm.","outdated":true}"#,
    );
    pi.author = Some("pi".into());
    pi.author_name = Some("Pi".into());
    let mut file = note(
        r#"{"id":"q","type":"comment","scope":"file","filePath":"a.rs","lineStart":1,"lineEnd":1,"side":"new","text":"Big."}"#,
    );
    file.author_name = Some("Pi".into());
    assert_eq!(
        review_feedback(&[&pi, &file], None),
        format!(
            "# Code Review Feedback\n\n## a.rs\n\n### File Comment\nFrom: Pi\nBig.\n\n\
             ### Line 1 (new)\n{OUTDATED_ANNOTATION_LABEL}\nFrom: Pi\nHm.\n\n"
        )
    );
}

#[test]
fn highlighted_text_stops_at_forty_lines_and_says_how_many_are_left() {
    let code: Vec<String> = (1..=45).map(|n| format!("line {n}")).collect();
    let mut long = note(
        r#"{"id":"l","type":"comment","filePath":"a.rs","lineStart":1,"lineEnd":45,"side":"old","text":"Why?"}"#,
    );
    long.anchor_text = Some(code.join("\n"));
    let out = review_feedback(&[&long], None);
    let shown = code.get(..HIGHLIGHT_MAX_LINES).expect("45 lines").join("\n");
    assert!(
        out.ends_with(&format!("**Highlighted text:**\n```\n{shown}\n```\n_(5 more lines not shown)_\n\n")),
        "{out}"
    );
    assert!(!out.contains("line 41"));
}

#[test]
fn a_pull_request_opens_with_the_browsers_pr_header() {
    // exportReviewFeedback(notes, prMeta) under bun, for a GitHub PR.
    let notes = [
        note(
            r#"{"id":"b","type":"comment","filePath":"src/auth.ts","lineStart":40,"lineEnd":40,"side":"old",
                "text":"Why drop this?","anchorText":"    cleanup()"}"#,
        ),
        note(
            r#"{"id":"g","type":"comment","scope":"general","filePath":"","lineStart":0,"lineEnd":0,
                "side":"new","text":"Overall fine."}"#,
        ),
        note(
            r#"{"id":"a","type":"comment","filePath":"src/auth.ts","lineStart":12,"lineEnd":12,"side":"new",
                "text":"Missing check.","anchorText":"next();"}"#,
        ),
    ];
    let refs: Vec<&CodeAnnotation> = notes.iter().collect();
    let pull = PrView {
        repo: crate::PrRepo { host: "github.com".into(), owner: "o".into(), name: "r".into() },
        number: 12,
        url: "https://github.com/o/r/pull/12".into(),
        title: "feat: login".into(),
        base_ref: "main".into(),
        head_ref: "feat/login".into(),
        base_sha: SHA.into(),
        head_sha: SHA.into(),
    };
    let browser = "# PR Review: o/r#12\n\n**feat: login**\nBranch: `feat/login` → `main`\nhttps://github.com/o/r/pull/12\n\n## Global comments\n\nOverall fine.\n\n## src/auth.ts\n\n### Line 12 (new)\nMissing check.\n\n**Highlighted text:**\n```\nnext();\n```\n\n### Line 40 (old)\nWhy drop this?\n\n**Highlighted text:**\n```\n    cleanup()\n```\n\n";
    assert_eq!(pull_feedback(&pull, &refs), browser);
    assert_eq!(pull_agent_feedback(&pull, None, &refs), format!("{browser}{REVIEW_SUFFIX}"));
    assert_eq!(
        pull_agent_feedback(&pull, Some("/src/r"), &refs),
        format!("Review directory: /src/r\n\n{browser}{REVIEW_SUFFIX}")
    );
    assert_eq!(pull_feedback(&pull, &[]), "# Code Review\n\nNo feedback provided.");
}
