//! `CommonMark` emphasis that works next to CJK punctuation.
//!
//! `CommonMark`'s flanking rule refuses `*` that has punctuation on its inner side and a letter
//! on its outer side, so `改造成**“最强妖怪”**。` stays literal: Chinese has no spaces to
//! satisfy the rule, and the leftover delimiters can then pair up with the wrong partners.
//! `tui-markdown` parses internally and has no option for this, so the repair happens in its
//! input: a zero-width mark on the inner side of such a delimiter makes the rule pass, and
//! [`strip_marks`] removes the mark from the rendered text. Source offsets are unaffected
//! because alignment runs against the original source.

use std::borrow::Cow;
use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};
use ratatui::text::{Line, Span, Text};
use unicode_width::UnicodeWidthChar as _;

use crate::doc::parse_options;

/// U+200B: not Unicode whitespace or punctuation, so `CommonMark` treats it as a letter.
const MARK: char = '\u{200b}';

#[derive(Clone, Copy, PartialEq, Eq)]
enum Repair {
    MarkBefore,
    MarkAfter,
}

/// Bytes whose `*` is literal text: code, math and raw HTML.
fn literal_ranges(source: &str) -> Vec<Range<usize>> {
    Parser::new_ext(source, parse_options())
        .into_offset_iter()
        .filter(|(event, _)| {
            matches!(
                event,
                Event::Code(_)
                    | Event::Html(_)
                    | Event::InlineHtml(_)
                    | Event::InlineMath(_)
                    | Event::DisplayMath(_)
                    | Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock)
            )
        })
        .map(|(_, range)| range)
        .collect()
}

fn is_cjk(ch: char) -> bool {
    ch.width() == Some(2)
}

fn is_punctuation(ch: char) -> bool {
    ch.is_ascii_punctuation()
        || matches!(ch,
            '\u{a1}'..='\u{bf}'
            | '\u{2010}'..='\u{2027}'
            | '\u{2030}'..='\u{205e}'
            | '\u{3001}'..='\u{303f}'
            | '\u{fe10}'..='\u{fe19}'
            | '\u{fe30}'..='\u{fe6f}'
            | '\u{ff01}'..='\u{ff0f}'
            | '\u{ff1a}'..='\u{ff20}'
            | '\u{ff3b}'..='\u{ff40}'
            | '\u{ff5b}'..='\u{ff65}')
}

/// Which side of a delimiter run needs a mark for it to open or close emphasis. `None` at the
/// ends of the text counts as whitespace, as it does in `CommonMark`.
fn repair_for(before: Option<char>, after: Option<char>) -> Option<Repair> {
    let punctuated = |ch: Option<char>| ch.is_some_and(is_punctuation);
    let cjk_letter = |ch: Option<char>| ch.is_some_and(|ch| is_cjk(ch) && !is_punctuation(ch));

    // A letter outside and punctuation inside: the only shape the flanking rule refuses.
    if punctuated(after) && cjk_letter(before) {
        Some(Repair::MarkAfter)
    } else if punctuated(before) && cjk_letter(after) {
        Some(Repair::MarkBefore)
    } else {
        None
    }
}

/// The source with emphasis delimiters made parseable. Borrowed when nothing needs repair.
pub(crate) fn protect(source: &str) -> Cow<'_, str> {
    let literal = literal_ranges(source);
    let mut out = String::new();
    let mut copied = 0usize;
    let mut chars = source.char_indices().peekable();

    while let Some((start, ch)) = chars.next() {
        if ch == '\\' {
            chars.next();
            continue;
        }
        if ch != '*' {
            continue;
        }
        let mut end = start + ch.len_utf8();
        while chars.next_if(|&(_, next)| next == '*').is_some() {
            end += '*'.len_utf8();
        }
        if literal.iter().any(|range| range.contains(&start)) {
            continue;
        }
        let before = source.get(..start).and_then(|text| text.chars().next_back());
        let after = source.get(end..).and_then(|text| text.chars().next());
        let at = match repair_for(before, after) {
            Some(Repair::MarkBefore) => start,
            Some(Repair::MarkAfter) => end,
            None => continue,
        };
        out.push_str(source.get(copied..at).unwrap_or_default());
        out.push(MARK);
        copied = at;
    }

    if out.is_empty() {
        return Cow::Borrowed(source);
    }
    out.push_str(source.get(copied..).unwrap_or_default());
    Cow::Owned(out)
}

/// Remove the marks [`protect`] added from rendered text.
pub(crate) fn strip_marks(text: Text<'static>) -> Text<'static> {
    let lines = text
        .lines
        .into_iter()
        .map(|line| {
            let spans = line.spans.into_iter().map(|span| {
                let content = span.content.replace(MARK, "");
                Span::styled(content, span.style)
            });
            Line::from(spans.collect::<Vec<_>>()).style(line.style)
        })
        .collect::<Vec<_>>();
    Text::from(lines).style(text.style)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;
    use tui_markdown::from_str;

    use super::*;

    /// Render through the real parser: the visible text, and the visible text that is bold.
    fn render(source: &str) -> (String, String) {
        let protected = protect(source);
        let text = strip_marks(crate::layout::own(from_str(&protected)));
        let spans = || text.lines.iter().flat_map(|line| line.spans.iter());
        let plain = spans().map(|span| span.content.as_ref()).collect();
        let bold = spans()
            .filter(|span| span.style.add_modifier.contains(Modifier::BOLD))
            .map(|span| span.content.as_ref())
            .collect();
        (plain, bold)
    }

    #[test]
    fn bold_opens_after_a_letter_and_before_cjk_punctuation() {
        let (plain, bold) = render("按**【宇宙】、【地球】**四大板块");
        assert_eq!(plain, "按【宇宙】、【地球】四大板块");
        assert_eq!(bold, "【宇宙】、【地球】");
    }

    #[test]
    fn bold_closes_after_punctuation_and_before_a_letter() {
        let (plain, bold) = render("灭亡了**巴莫拉（Vamola）**母星");
        assert_eq!(plain, "灭亡了巴莫拉（Vamola）母星");
        assert_eq!(bold, "巴莫拉（Vamola）");
    }

    #[test]
    fn bold_wrapped_in_quotes_inside_a_sentence() {
        let (plain, bold) = render("化名**“三丈目老师”**潜伏，改造成**“最强妖怪”**。");
        assert_eq!(plain, "化名“三丈目老师”潜伏，改造成“最强妖怪”。");
        assert_eq!(bold, "“三丈目老师”“最强妖怪”");
    }

    #[test]
    fn single_star_emphasis_is_repaired_too() {
        let (plain, _) = render("化名*“斜体”*潜伏");
        assert_eq!(plain, "化名“斜体”潜伏");
    }

    #[test]
    fn ascii_emphasis_keeps_commonmark_behaviour() {
        assert!(matches!(protect("a **b** c and foo**\"bar\"**baz"), Cow::Borrowed(_)));
        assert_eq!(render("a **b** c").1, "b");
        assert_eq!(render("foo**\"bar\"**baz").0, "foo**\"bar\"**baz");
    }

    #[test]
    fn code_math_and_escapes_stay_literal() {
        for source in [
            "`成**“甲”**乙`",
            "```\n成**“甲”**乙\n```",
            r"成\*\*“甲”\*\*乙",
            "$成**“甲”**乙$",
            "<b title=\"成**“甲”**乙\">",
        ] {
            assert!(matches!(protect(source), Cow::Borrowed(_)), "changed: {source}");
        }
    }

    #[test]
    fn a_list_marker_star_is_not_touched() {
        assert!(matches!(protect("* 甲\n* “乙”"), Cow::Borrowed(_)));
    }
}
