#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use super::*;
use crate::doc::Document;

fn index(source: &str, width: usize) -> (Document, DocLayout, Index) {
    let doc = Document::parse(source.to_owned());
    let layout = DocLayout::build(&doc, width);
    let index = Index::of(&layout);
    (doc, layout, index)
}

/// The source text under each match.
fn found<'a>(doc: &'a Document, matches: &[Range<usize>]) -> Vec<&'a str> {
    matches.iter().map(|range| &doc.source[range.clone()]).collect()
}

/// The highlighted text of every document row that shows part of match `which`.
fn highlighted_rows(layout: &DocLayout, matches: &[Range<usize>], which: usize) -> Vec<String> {
    (0..layout.total_rows)
        .filter_map(|row| layout.row(row))
        .filter_map(|row| {
            let text: String = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
            let hits = row_hits(&row.cells, matches);
            let shown: String =
                text.chars().zip(&hits).filter(|(_, hit)| **hit == Some(which)).map(|(c, _)| c).collect();
            (!shown.is_empty()).then_some(shown)
        })
        .collect()
}

#[test]
fn a_lowercase_query_ignores_case_and_a_capital_makes_it_exact() {
    let (doc, _, index) = index("Login first, then login again. LOGIN.\n", 80);
    assert_eq!(found(&doc, &index.find("login")), ["Login", "login", "LOGIN"]);
    assert_eq!(found(&doc, &index.find("Login")), ["Login"]);
    assert_eq!(found(&doc, &index.find("LOGIN")), ["LOGIN"]);
}

#[test]
fn a_query_is_plain_text_not_a_pattern() {
    let (doc, _, index) = index("abc and a.c and a*c\n", 80);
    assert_eq!(found(&doc, &index.find("a.c")), ["a.c"]);
    assert_eq!(found(&doc, &index.find("a*c")), ["a*c"]);
}

#[test]
fn a_match_finds_the_rendered_text_through_concealed_markup() {
    let (doc, _, index) = index("Ship the **login page** by Friday.\n", 80);
    assert_eq!(found(&doc, &index.find("the login page")), ["the **login page"]);
    assert_eq!(index.find("**login"), [] as [Range<usize>; 0]);
}

#[test]
fn a_match_spans_a_soft_wrap_and_highlights_both_rows() {
    let source = "The rollout plan covers the staging cluster first and production after review.\n";
    let (doc, layout, index) = index(source, 24);
    let matches = index.find("staging cluster first");
    assert_eq!(found(&doc, &matches), ["staging cluster first"]);
    let rows = highlighted_rows(&layout, &matches, 0);
    assert!(rows.len() > 1, "the match should cross a wrap at 24 columns: {rows:?}");
    assert_eq!(rows.concat().replace(' ', ""), "stagingclusterfirst");
}

#[test]
fn a_match_spans_a_soft_line_break_in_the_source() {
    let (doc, layout, index) = index("Ship the login page\nby Friday.\n", 80);
    let matches = index.find("page by");
    assert_eq!(found(&doc, &matches), ["page\nby"]);
    // The space the break became has no source byte, yet sits inside the highlight.
    assert_eq!(highlighted_rows(&layout, &matches, 0), ["page by"]);
}

#[test]
fn a_match_inside_a_list_item_skips_the_bullet() {
    let (doc, layout, index) = index("- item one\n- item two\n  continues here\n", 80);
    let matches = index.find("two continues");
    assert_eq!(found(&doc, &matches), ["two\n  continues"]);
    assert_eq!(highlighted_rows(&layout, &matches, 0), ["two continues"]);
    assert_eq!(found(&doc, &index.find("one item")), Vec::<&str>::new(), "list items are separate lines");
}

#[test]
fn a_match_in_a_table_cell_that_wraps_highlights_each_of_its_rows() {
    let source = "| Item | Note |\n|---|---|\n| A | the quick brown fox jumps over the lazy dog |\n";
    let (doc, layout, index) = index(source, 30);
    let matches = index.find("brown fox jumps over");
    assert_eq!(found(&doc, &matches), ["brown fox jumps over"]);
    let rows = highlighted_rows(&layout, &matches, 0);
    assert!(rows.len() > 1, "the cell should wrap at 30 columns: {rows:?}");
    assert_eq!(rows.concat().replace(' ', ""), "brownfoxjumpsover");
}

#[test]
fn decoration_alone_is_not_a_match() {
    let (_, _, index) = index("- one\n- two\n", 80);
    assert_eq!(index.find("∙"), [] as [Range<usize>; 0]);
}

#[test]
fn no_match_leaves_nothing_to_step_to() {
    let (_, _, index) = index("Nothing to see here.\n", 80);
    let matches = index.find("absent");
    assert_eq!(matches, [] as [Range<usize>; 0]);
    assert_eq!(at_or_after(&matches, 0), None);
    assert_eq!(next(&matches, 0), None);
    assert_eq!(previous(&matches, 0), None);
    assert_eq!(index.find(""), [] as [Range<usize>; 0]);
}

#[test]
fn stepping_wraps_at_both_ends() {
    let matches = [10..12, 20..22, 30..32];
    assert_eq!(next(&matches, 10), Some((1, false)));
    assert_eq!(next(&matches, 30), Some((0, true)));
    assert_eq!(previous(&matches, 20), Some((0, false)));
    assert_eq!(previous(&matches, 10), Some((2, true)));
    assert_eq!(previous(&matches, 5), Some((2, true)));
}

#[test]
fn stepping_from_the_current_match_wraps_at_both_ends() {
    assert_eq!(step(3, 0, true), Some((1, false)));
    assert_eq!(step(3, 2, true), Some((0, true)));
    assert_eq!(step(3, 0, false), Some((2, true)));
    assert_eq!(step(3, 2, false), Some((1, false)));
    assert_eq!(step(1, 0, true), Some((0, true)));
    assert_eq!(step(0, 0, true), None);
}

#[test]
fn typing_lands_on_the_first_match_at_or_after_the_start_and_wraps_past_the_last() {
    let matches = [10..12, 20..22, 30..32];
    assert_eq!(at_or_after(&matches, 0), Some(0));
    assert_eq!(at_or_after(&matches, 20), Some(1));
    assert_eq!(at_or_after(&matches, 21), Some(2));
    assert_eq!(at_or_after(&matches, 31), Some(0));
}

#[test]
fn covering_finds_the_match_under_a_byte() {
    let matches = [10..12, 20..22];
    assert_eq!(covering(&matches, 9), None);
    assert_eq!(covering(&matches, 11), Some(0));
    assert_eq!(covering(&matches, 12), None);
    assert_eq!(covering(&matches, 21), Some(1));
}

#[test]
fn a_character_with_a_longer_lower_case_still_lines_up() {
    // `İ` lower-cases to two characters; the text after it must still map to its source.
    let (doc, _, index) = index("İstanbul and Ankara\n", 80);
    assert_eq!(found(&doc, &index.find("ankara")), ["Ankara"]);
    assert_eq!(found(&doc, &index.find("İstanbul")), ["İstanbul"]);
}
