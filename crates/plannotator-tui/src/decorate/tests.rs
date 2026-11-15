//! Decoration through the real pipeline: parse, render, align, decorate, wrap.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use unicode_width::UnicodeWidthChar as _;

use super::{BULLETS, QUOTE_BAR, RULE, TASK_DONE, TASK_OPEN};
use crate::doc::Document;
use crate::layout::DocLayout;

fn layout(source: &str, width: usize) -> (Document, DocLayout) {
    let doc = Document::parse(source.to_owned());
    let layout = DocLayout::build(&doc, width);
    (doc, layout)
}

fn rows(source: &str, width: usize) -> Vec<String> {
    let (_, layout) = layout(source, width);
    layout.blocks.iter().flat_map(|b| b.rows.iter()).map(|r| r.line.to_string()).collect()
}

#[test]
fn bullets_follow_nesting_whatever_the_source_marker() {
    assert_eq!(
        rows("* a\n  + b\n    - c\n      * d\n\n3) e\n3) f", 40),
        ["∙ a", "  ◦ b", "    ▪ c", "      ∙ d", "3. e", "4. f"]
    );
}

#[test]
fn a_task_box_replaces_the_bullet() {
    assert_eq!(rows("- [ ] open\n- [X] done\n\n1. [x] first", 40), ["☐ open", "☑ done", "1. ☑ first"]);
}

#[test]
fn quote_bars_nest_and_repeat_on_wrapped_rows() {
    assert_eq!(
        rows("> one two three four\n>\n> > nested\n> - item", 14),
        ["╎ one two", "╎ three four", "╎", "╎ ╎ nested", "╎", "╎ ∙ item"]
    );
}

#[test]
fn escaped_and_entity_markers_stay_literal() {
    assert_eq!(
        rows("- a  \n  \\- b\n\n> \\> c\n\n> &gt; d\n\n- &gt; e", 40),
        ["∙ a", "  - b", "╎ > c", "╎ > d", "∙ > e"]
    );
}

#[test]
fn a_short_fence_inside_a_longer_one_is_code() {
    let rows = rows("- item\n\n  ````\n  ```\n  - not an item\n  ```\n  ````\n- next", 40);
    assert!(rows.iter().any(|row| row.contains("- not an item")), "{rows:#?}");
    assert_eq!(rows.iter().filter(|row| row.contains('∙')).count(), 2, "{rows:#?}");
    assert_eq!(rows.last().map(String::as_str), Some("∙ next"));
}

#[test]
fn wrapped_list_rows_hang_under_the_text_and_the_indent_has_no_source() {
    let (doc, layout) = layout("- alpha beta gamma delta\n  - nested words wrap too", 14);
    let rows = &layout.blocks[0].rows;
    let text: Vec<String> = rows.iter().map(|r| r.line.to_string()).collect();
    assert_eq!(text, ["∙ alpha beta", "  gamma delta", "  ◦ nested", "    words wrap", "    too"]);
    for row in rows {
        let indent = row.line.to_string().chars().take_while(|c| !c.is_alphanumeric()).count();
        assert!(row.cells.iter().take(indent).all(Option::is_none), "{:?}", row.line.to_string());
        let start = row.cells.iter().flatten().next().copied().expect("text has source");
        let first = row.line.to_string().chars().nth(indent).expect("text");
        assert!(doc.source[start..].starts_with(first));
    }
}

#[test]
fn a_rule_fills_the_width_in_one_column_glyphs() {
    let (_, layout) = layout("a\n\n***\n\nb", 20);
    let rule = &layout.blocks[1].rows[0];
    assert_eq!(rule.line.to_string(), RULE.to_string().repeat(20));
    // Ambiguous-width glyphs are drawn two columns wide in CJK locales; none is used here.
    for glyph in [RULE, QUOTE_BAR, TASK_OPEN, TASK_DONE].into_iter().chain(BULLETS) {
        assert_eq!(glyph.width_cjk(), Some(1), "{glyph:?}");
    }
}

#[test]
fn rendered_text_across_items_has_no_decoration() {
    let (doc, layout) = layout("- one\n- [ ] two\n  - three\n\n> four", 40);
    let whole = 0..doc.source.len();
    assert_eq!(layout.rendered_in_range(&doc.source, &whole), "one two threefour");
}
