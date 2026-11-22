#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use super::*;
use crate::patch::parse;

/// The file every scenario starts from: two functions with the same first line, so a note
/// on `let x = 1;` is only unique through its context.
const V1: &str = "fn a() {\n    let x = 1;\n    check(x);\n}\n\nfn b() {\n    let x = 1;\n    run(x);\n}\n";

/// A patch adding `content` as a new file: every line is shown.
fn added(content: &str) -> FilePatch {
    let lines: Vec<&str> = content.lines().collect();
    let body: String = lines.iter().flat_map(|l| ["+", l, "\n"]).collect();
    let text = format!(
        "diff --git a/src/f.rs b/src/f.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/f.rs\n@@ -0,0 +1,{} @@\n{body}",
        lines.len()
    );
    parse(&text).pop().expect("one file")
}

/// A patch showing only new lines `first..` of `content` (as unchanged context), under a
/// hunk header with function context `func`.
fn showing(content: &str, first: u32, count: usize, func: &str) -> FilePatch {
    let lines: Vec<&str> = content.lines().skip(first as usize - 1).take(count).collect();
    let body: String = lines.iter().flat_map(|l| [" ", l, "\n"]).collect();
    let text = format!(
        "diff --git a/src/f.rs b/src/f.rs\n--- a/src/f.rs\n+++ b/src/f.rs\n@@ -{first},{count} +{first},{count} @@ {func}\n{body}"
    );
    parse(&text).pop().expect("one file")
}

fn note_on(patch: &FilePatch, first: u32, last: u32) -> CodeAnnotation {
    CodeAnnotation::line_comment(patch, Side::New, first, last, "why?".into(), "key1")
}

fn now<'a>(content: &'a str, patch: &'a FilePatch) -> Current<'a> {
    Current { content: Some(content), patch: Some(patch), patch_key: "key2" }
}

#[test]
fn an_unchanged_file_keeps_the_note_where_it_is() {
    let patch = added(V1);
    let note = note_on(&patch, 7, 8);
    assert_eq!(resolve(&note, &now(V1, &patch)), Resolution::StillThere);
}

#[test]
fn lines_inserted_above_move_the_note_to_its_unique_match() {
    let patch = added(V1);
    let note = note_on(&patch, 7, 8);
    let v2 = format!("// header\n// more\n{V1}");
    let moved = added(&v2);
    assert_eq!(resolve(&note, &now(&v2, &moved)), Resolution::Moved { line_start: 9, line_end: 10 });
}

#[test]
fn a_found_note_whose_lines_left_the_diff_is_not_in_the_diff() {
    let patch = added(V1);
    let note = note_on(&patch, 7, 8);
    // Same file, but the patch now shows only fn a.
    let narrow = showing(V1, 1, 4, "");
    assert_eq!(resolve(&note, &now(V1, &narrow)), Resolution::NotInDiff { line_start: 7, line_end: 8 });
    // Moved and out of the diff: the new lines are reported.
    let v2 = format!("\n{V1}");
    let narrow = showing(&v2, 1, 4, "");
    assert_eq!(resolve(&note, &now(&v2, &narrow)), Resolution::NotInDiff { line_start: 8, line_end: 9 });
}

#[test]
fn a_file_no_longer_in_the_diff_still_resolves_by_content() {
    let patch = added(V1);
    let note = note_on(&patch, 3, 3);
    let current = Current { content: Some(V1), patch: None, patch_key: "key2" };
    assert_eq!(resolve(&note, &current), Resolution::NotInDiff { line_start: 3, line_end: 3 });
}

#[test]
fn rewritten_text_makes_the_note_outdated_and_keeps_its_lines() {
    let patch = added(V1);
    let mut note = note_on(&patch, 3, 3);
    let v2 = V1.replace("check(x)", "verify(x)");
    let resolution = resolve(&note, &now(&v2, &added(&v2)));
    assert_eq!(resolution, Resolution::Outdated);
    assert!(apply(&mut note, resolution));
    assert!(note.outdated);
    assert_eq!((note.line_start, note.line_end), (3, 3));
    assert_eq!(note.status(), Some(Status::Outdated));
}

#[test]
fn changed_context_alone_makes_the_note_outdated() {
    let patch = added(V1);
    let note = note_on(&patch, 2, 2); // `let x = 1;` in fn a, before `check(x);`
    let v2 = V1.replace("check(x)", "audit(x)");
    assert_eq!(resolve(&note, &now(&v2, &added(&v2))), Resolution::Outdated);
}

#[test]
fn a_common_line_moves_only_to_the_place_whose_context_matches() {
    let patch = added(V1);
    let note = note_on(&patch, 7, 7); // `let x = 1;` in fn b: the text alone is not unique
    let v2 = format!("fn z() {{}}\n{V1}");
    assert_eq!(resolve(&note, &now(&v2, &added(&v2))), Resolution::Moved { line_start: 8, line_end: 8 });
}

#[test]
fn two_full_matches_are_ambiguous_and_never_moved() {
    let patch = added(V1);
    let note = note_on(&patch, 6, 9); // all of fn b
    // fn b now exists twice, neither at the note's lines.
    let fn_b = "fn b() {\n    let x = 1;\n    run(x);\n}\n";
    let v2 = format!("// top\n{V1}\n{fn_b}");
    assert_eq!(resolve(&note, &now(&v2, &added(&v2))), Resolution::Outdated);
}

#[test]
fn null_context_lines_match_anything() {
    // Made on a hunk that showed only lines 2 and 3: the browser records `null` for the
    // lines around them that the patch did not show.
    let patch = showing(V1, 2, 2, "");
    let note = note_on(&patch, 2, 2);
    let recorded = note.anchor_context.clone().expect("context");
    assert_eq!(recorded.before, [None, None]);
    assert_eq!(recorded.after, [Some("    check(x);".into()), None]);
    let v2 = format!("use std::io;\n{V1}");
    assert_eq!(resolve(&note, &now(&v2, &added(&v2))), Resolution::Moved { line_start: 3, line_end: 3 });
}

#[test]
fn a_different_hunk_function_context_blocks_a_match_in_the_diff() {
    let patch = showing(V1, 6, 4, "mod outer {");
    let note = note_on(&patch, 7, 8);
    assert_eq!(note.anchor_context.as_ref().and_then(|c| c.hunk.as_deref()), Some("mod outer {"));
    let other_hunk = showing(V1, 6, 4, "mod renamed {");
    assert_eq!(resolve(&note, &now(V1, &other_hunk)), Resolution::Outdated);
    // Out of the diff there is no current hunk to compare, so the text and context decide.
    let elsewhere = showing(V1, 1, 3, "");
    assert_eq!(resolve(&note, &now(V1, &elsewhere)), Resolution::NotInDiff { line_start: 7, line_end: 8 });
}

#[test]
fn a_crlf_working_file_matches_the_lf_patch() {
    let patch = added(V1);
    let note = note_on(&patch, 3, 4);
    let crlf = V1.replace('\n', "\r\n");
    assert_eq!(resolve(&note, &now(&crlf, &patch)), Resolution::StillThere);
}

#[test]
fn a_missing_file_makes_line_notes_outdated() {
    let patch = added(V1);
    let note = note_on(&patch, 3, 3);
    let gone = Current { content: None, patch: None, patch_key: "key2" };
    assert_eq!(resolve(&note, &gone), Resolution::Outdated);
}

#[test]
fn a_note_without_anchor_text_is_trusted_only_on_its_own_patch() {
    let patch = added(V1);
    let mut note = note_on(&patch, 3, 3);
    note.anchor_text = None;
    note.anchor_context = None;
    let same = Current { content: Some(V1), patch: Some(&patch), patch_key: "key1" };
    assert_eq!(resolve(&note, &same), Resolution::StillThere);
    assert_eq!(resolve(&note, &now(V1, &patch)), Resolution::Outdated);
}

#[test]
fn a_note_without_context_is_checked_in_place_but_never_moved() {
    let patch = added(V1);
    let mut note = note_on(&patch, 3, 3);
    note.anchor_context = None;
    assert_eq!(resolve(&note, &now(V1, &patch)), Resolution::StillThere);
    let v2 = format!("\n{V1}");
    assert_eq!(resolve(&note, &now(&v2, &added(&v2))), Resolution::Outdated);
}

#[test]
fn anchor_text_that_disagrees_with_the_line_span_is_outdated() {
    let patch = added(V1);
    let mut note = note_on(&patch, 3, 3);
    note.line_end = 4;
    assert_eq!(resolve(&note, &now(V1, &patch)), Resolution::Outdated);
}

#[test]
fn file_and_review_notes_never_go_outdated() {
    let patch = added(V1);
    let file_note = CodeAnnotation::file_comment(&patch, "split it".into(), "key1");
    assert_eq!(resolve(&file_note, &now(V1, &patch)), Resolution::StillThere);
    let gone = Current { content: None, patch: None, patch_key: "key2" };
    assert_eq!(resolve(&file_note, &gone), Resolution::NotInDiff { line_start: 1, line_end: 1 });
    let mut general = file_note;
    general.scope = NoteScope::General;
    assert_eq!(resolve(&general, &gone), Resolution::StillThere);
}

#[test]
fn applying_a_move_changes_lines_but_not_the_edit_time() {
    let patch = added(V1);
    let mut note = note_on(&patch, 7, 8);
    note.outdated = true;
    let updated = note.updated_at();
    assert!(apply(&mut note, Resolution::Moved { line_start: 9, line_end: 10 }));
    assert_eq!((note.line_start, note.line_end, note.outdated), (9, 10, false));
    assert_eq!(note.status(), Some(Status::Moved));
    assert_eq!(note.updated_at(), updated, "a move is not an edit");
    assert!(!apply(&mut note, Resolution::Moved { line_start: 9, line_end: 10 }), "nothing left to change");
}

#[test]
fn a_suggestion_records_the_lines_it_replaces() {
    let patch = added(V1);
    let note = CodeAnnotation::suggestion(&patch, 3, 3, "    verify(x);".into(), None, "key1");
    assert_eq!(note.original_code.as_deref(), Some("    check(x);"));
    assert_eq!(note.anchor_text, note.original_code);
    assert_eq!(note.text, None);
    assert_eq!(resolve(&note, &now(V1, &patch)), Resolution::StillThere);
}

#[test]
fn a_note_outside_the_diff_stays_not_in_the_diff_and_follows_its_text() {
    // Written on `run(x);` (line 8), which no hunk shows: the patch shows only fn a.
    let patch = showing(V1, 1, 4, "");
    let note = CodeAnnotation::outside_diff_comment(
        "src/f.rs",
        8,
        8,
        "    run(x);".into(),
        "why run?".into(),
        "key1",
    );
    assert_eq!(resolve(&note, &now(V1, &patch)), Resolution::NotInDiff { line_start: 8, line_end: 8 });
    // Lines added above: its text is in one place only, so it moves with it.
    let v2 = format!("// header\n{V1}");
    let shifted = showing(&v2, 1, 2, "");
    assert_eq!(resolve(&note, &now(&v2, &shifted)), Resolution::NotInDiff { line_start: 9, line_end: 9 });
    // Its code rewritten: gone, so outdated, as for any note.
    let v3 = V1.replace("run(x)", "walk(x)");
    assert_eq!(resolve(&note, &now(&v3, &patch)), Resolution::Outdated);
    // A line note made in the diff, with no context recorded, still does not move.
    let mut plain = note.clone();
    plain.outside_diff = false;
    assert_eq!(resolve(&plain, &now(&v2, &shifted)), Resolution::Outdated);
}
