//! Rendering through the real pipeline: parse, render, align, decorate, wrap, draw colours.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use ratatui::style::{Color, Modifier};

use super::code::BAR;
use super::*;
use crate::highlight::Role;

#[test]
fn rendered_text_strips_markup_and_joins_blocks_without_separator() {
    let doc = Document::parse("Ship the **login page**\nby Friday.\n\nNext para.\n".to_owned());
    let layout = DocLayout::build(&doc, 80);
    let whole = 0..doc.source.len();
    assert_eq!(layout.rendered_in_range(&doc.source, &whole), "Ship the login page by Friday.Next para.");
    let bold = doc.source.find("**login").expect("present");
    let bold_range = bold..bold + "**login page**".len();
    assert_eq!(layout.rendered_in_range(&doc.source, &bold_range), "login page");
}

#[test]
fn every_cell_of_a_long_table_maps_to_its_source() {
    let doc = Document::parse(
        "| Item | Status | Next step |\n|---|---|---|\n\
| A. Four changes from 3/7 (release checklist handoff, the 3/4 planning call, the 3/6 search demo, \
the 3/4-3/7 progress review and rollout advice) | Scope partly confirmed; never added to the \
draft | 1.0d design + 1.0d build + 1.0d review + 2.5d rollout = **5.5d** |\n\
| B. 3/14 cache troubleshooting and the `api.example.com` proxy | Owner and priority not set | Your call |\n\
| C. 3/19-20 export access in the admin panel (read and write) | Agreed at 1.0d, done once it's \
tested; it's live, but QA hasn't finished acceptance testing | Close it now, or wait for the next sprint |\n\
| D. 3/21 lookup and report tools in the CLI; 3/23 plugin consolidation | Marked \"not confirmed\" | Your call |\n"
            .to_owned(),
    );
    let layout = DocLayout::build(&doc, 100);
    let table = layout.blocks.iter().find(|b| b.kind == BlockKind::Table).expect("a table");
    let mut cells = 0;
    for row in &table.rows {
        let text: String = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
        for (ch, offset) in text.chars().zip(&row.cells) {
            if ch.is_alphanumeric() {
                let source = offset.and_then(|o| doc.source.get(o..));
                assert!(source.is_some_and(|s| s.starts_with(ch)), "{ch:?} is unmapped in {text:?}");
                cells += 1;
            }
        }
    }
    assert!(cells > 300, "only {cells} cells checked");
}

fn rows(layout: &DocLayout, block: usize) -> Vec<String> {
    layout.blocks[block].rows.iter().map(|r| r.line.to_string()).collect()
}

const RUST: &str = "Intro.\n\n```rust\nfn main() {\n    let n = 42; // answer\n}\n```\n\nAfter.\n";

#[test]
fn a_code_block_drops_its_fences_and_draws_a_bar_and_its_language() {
    let doc = Document::parse(RUST.to_owned());
    let layout = DocLayout::build(&doc, 40);
    assert_eq!(
        rows(&layout, 1),
        [
            format!("{BAR} fn main() {{{}rust", " ".repeat(40 - 2 - 11 - 4)),
            format!("{BAR}     let n = 42; // answer"),
            format!("{BAR} }}"),
        ]
    );
    // The bar and the label are drawn, not written: no cell of theirs has a source byte, and
    // every code character maps to its own.
    for row in &layout.blocks[1].rows {
        assert_eq!(row.cells[..2], [None, None]);
        let text: String = row.line.to_string();
        for (ch, cell) in text.chars().zip(&row.cells).skip(2) {
            if let Some(offset) = cell {
                assert!(doc.source[*offset..].starts_with(ch), "{ch:?} in {text:?}");
            }
        }
    }
    let label = layout.blocks[1].rows[0].cells.iter().rev().take(4);
    assert!(label.into_iter().all(Option::is_none));
}

#[test]
fn the_text_under_a_whole_code_block_is_its_code_not_its_fences() {
    let doc = Document::parse(RUST.to_owned());
    let layout = DocLayout::build(&doc, 40);
    let block = doc.blocks[1].range.clone();
    let code = "fn main() {\n    let n = 42; // answer\n}";
    assert_eq!(layout.rendered_in_range(&doc.source, &block), code);
    let cells = layout.blocks[1].rows.iter().flat_map(|r| r.cells.iter().flatten());
    let (lo, hi) = cells.fold((usize::MAX, 0), |(lo, hi), &o| (lo.min(o), hi.max(o)));
    assert_eq!(&doc.source[lo..=hi], code);
}

#[test]
fn a_known_language_is_coloured_only_when_drawn_and_names_keep_the_terminal_colour() {
    let doc = Document::parse(RUST.to_owned());
    let layout = DocLayout::build(&doc, 40);
    let first = layout.blocks[1].first_row;
    let colour = |row: usize, text: &str| {
        let line = layout.drawn_line(row).expect("a row");
        line.spans.iter().find(|s| s.content.contains(text)).map(|s| s.style.patch(Style::default()))
    };
    assert_eq!(colour(first, "fn").and_then(|s| s.fg), Role::Keyword.style().fg);
    assert_eq!(colour(first, "main").and_then(|s| s.fg), Role::Function.style().fg);
    assert_eq!(colour(first + 1, "42").and_then(|s| s.fg), Role::Number.style().fg);
    let comment = colour(first + 1, "answer").expect("comment");
    assert_eq!(comment.fg, Role::Comment.style().fg);
    assert!(comment.add_modifier.contains(Modifier::ITALIC));
    assert_eq!(colour(first + 1, "n").and_then(|s| s.fg), Some(Color::Reset));
    // The bar keeps its own colour.
    assert_eq!(colour(first, &BAR.to_string()).and_then(|s| s.fg), Some(palette().code_comment));
    // Laid-out rows never carry the colours, so a reflow cannot lose or repeat them.
    assert!(layout.blocks[1].rows[0].line.spans.iter().all(|s| s.style.fg != Role::Keyword.style().fg));
}

#[test]
fn an_unknown_language_keeps_the_one_code_colour_and_its_label() {
    let doc = Document::parse("```klingon\nqapla' batlh\n```\n".to_owned());
    let layout = DocLayout::build(&doc, 40);
    assert_eq!(rows(&layout, 0), [format!("{BAR} qapla' batlh{}klingon", " ".repeat(40 - 2 - 12 - 7))]);
    let line = layout.drawn_line(0).expect("a row");
    assert_eq!(*line, layout.blocks[0].rows[0].line, "drawn as laid out");
    assert_eq!(line.style.fg, Some(Color::LightYellow));
}

#[test]
fn a_narrow_column_drops_the_label_and_wraps_long_code_beside_the_bar() {
    let doc = Document::parse(RUST.to_owned());
    let layout = DocLayout::build(&doc, 14);
    assert_eq!(
        rows(&layout, 1),
        [
            format!("{BAR} fn main() {{"),
            format!("{BAR}     let n ="),
            format!("{BAR} 42; // answe"),
            format!("{BAR} r"),
            format!("{BAR} }}"),
        ]
    );
    assert!(layout.blocks.iter().flat_map(|b| &b.rows).all(|r| r.cells.len() <= 14));
    // Nothing is lost: the rows still carry every byte of the code.
    let block = doc.blocks[1].range.clone();
    assert_eq!(layout.rendered_in_range(&doc.source, &block), "fn main() {\n    let n = 42; // answer\n}");
    // Colour still follows the source, on the continuation rows too.
    let first = layout.blocks[1].first_row;
    let line = layout.drawn_line(first + 2).expect("a row");
    let number = line.spans.iter().find(|s| s.content.contains("42")).expect("42");
    assert_eq!(number.style.fg, Role::Number.style().fg);
}

#[test]
fn a_title_has_a_rule_under_it_and_sections_get_room_above() {
    let doc = Document::parse("# Title\n\nText.\n\n## Section\n\n### Sub\n".to_owned());
    let layout = DocLayout::build(&doc, 20);
    assert_eq!(rows(&layout, 0), ["Title".to_owned(), TITLE_RULE.to_string().repeat(20)]);
    assert_eq!(layout.blocks[0].rows[1].cells, vec![None; 20], "the rule has no source");
    // Title, its rule, a gap, the text; two blank rows before a section; one before a sub-heading.
    let starts: Vec<usize> = layout.blocks.iter().map(|b| b.first_row).collect();
    assert_eq!(starts, [0, 3, 6, 8]);
    let row = &layout.blocks[0].rows[0].line;
    let title = row.style.patch(row.spans[0].style);
    assert!(title.add_modifier.contains(Modifier::BOLD) && title.fg == Some(palette().heading));
}

#[test]
fn a_definition_list_indents_its_definitions_under_their_terms() {
    let doc = Document::parse(
        "Term\n: A definition long enough to wrap onto a second row.\n\nOther\n: Short.\n".to_owned(),
    );
    assert_eq!(doc.blocks[0].kind, BlockKind::DefinitionList);
    let layout = DocLayout::build(&doc, 30);
    assert_eq!(
        rows(&layout, 0),
        ["Term", "    A definition long enough", "    to wrap onto a second row.", "Other", "    Short."]
    );
    let block = &layout.blocks[0];
    for row in &block.rows[1..3] {
        assert_eq!(row.cells[..4], [None; 4], "the indent has no source");
    }
    let whole = 0..doc.source.len();
    assert_eq!(
        layout.rendered_in_range(&doc.source, &whole),
        "Term A definition long enough to wrap onto a second row. Other Short."
    );
    let term = &block.rows[0].line;
    assert!(term.spans.iter().all(|s| term.style.patch(s.style).add_modifier.contains(Modifier::BOLD)));
}

#[test]
fn the_bar_and_the_title_rule_are_one_column_wide_even_in_cjk_terminals() {
    use unicode_width::UnicodeWidthChar as _;
    for glyph in [BAR, TITLE_RULE] {
        assert_eq!(glyph.width_cjk(), Some(1), "{glyph:?}");
    }
}

fn all_rows(layout: &DocLayout) -> Vec<(usize, String, Vec<Option<usize>>)> {
    layout
        .blocks
        .iter()
        .flat_map(|b| b.rows.iter().map(|r| (b.first_row, r.line.to_string(), r.cells.clone())))
        .collect()
}

/// A long document opens on its first screens only, and the rest, laid out in the
/// background (here for a width the first frame changed), is exactly a full build: section
/// gaps, title rules and code bars included.
#[test]
fn a_long_document_lays_out_its_first_screens_first_and_the_rest_matches_a_full_build() {
    let section = "# Part\n\n## Step\n\nSome **bold** words that wrap at a narrow width, and more.\n\n\
- one\n- two\n\n| a | b |\n|---|---|\n| x | y |\n\n```\ncode\n```\n\n```rust\nlet n = 42;\n```\n\n\
Term\n: Its definition.\n\n---\n\n";
    let doc = Document::parse(section.repeat(400));
    let mut lazy = DocLayout::start(&doc, 70);
    assert!(!lazy.is_complete());
    assert!(lazy.blocks.len() < doc.blocks.len() / 10, "laid out {} blocks up front", lazy.blocks.len());
    assert!(lazy.total_rows >= FIRST_ROWS);
    lazy.reflow(50);
    lazy.finish();
    assert!(lazy.is_complete());
    let full = DocLayout::build(&doc, 50);
    assert_eq!(lazy.blocks.len(), doc.blocks.len());
    assert_eq!(lazy.total_rows, full.total_rows);
    assert!(all_rows(&lazy) == all_rows(&full), "rows differ from a full build");
}

/// Code laid out on a background thread is coloured when drawn, like code on the first screen.
#[test]
fn code_laid_out_in_the_background_is_coloured_when_drawn() {
    let section = "Some words before the code, enough to make a long document.\n\n```rust\nlet n = 42;\n```\n\n";
    let doc = Document::parse(section.repeat(1000));
    let mut layout = DocLayout::start(&doc, 60);
    let first = layout.blocks.len();
    layout.finish();
    let last = layout.blocks.len() - 1;
    assert!(last > first, "the last block came from the background");
    let row = layout.blocks[last].first_row;
    let line = layout.drawn_line(row).expect("the code row");
    let keyword = line.spans.iter().find(|span| span.content == "let").expect("let is a span of its own");
    assert_eq!(keyword.style.fg, Role::Keyword.style().fg);
}
