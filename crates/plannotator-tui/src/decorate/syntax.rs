//! What `pulldown-cmark` says about a list or quote block's container syntax, by source byte.
//!
//! `tui-markdown` writes its own `- `, `1. `, `[x] ` and `>` for list items, task boxes and
//! quotes. To tell which rendered characters those are, the decoration asks the parser
//! rather than the glyphs: a rendered character is container syntax when its aligned source
//! byte is an item's marker, a task box, or a `>` that no text event covers.

use std::borrow::Cow;
use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::doc::parse_options;

#[derive(Debug)]
pub(super) enum Kind {
    Quote,
    Item {
        /// The `-`, `*`, `+` or `12.` that opens the item.
        marker: Range<usize>,
        /// The item's number in an ordered list.
        number: Option<u64>,
        /// The `[ ]` box and whether it is checked.
        task: Option<(Range<usize>, bool)>,
        /// Lists open around the item, counting its own.
        depth: usize,
    },
}

#[derive(Debug)]
pub(super) struct Container {
    pub(super) range: Range<usize>,
    pub(super) kind: Kind,
    parent: Option<usize>,
}

impl Container {
    pub(super) fn is_quote(&self) -> bool {
        matches!(self.kind, Kind::Quote)
    }
}

/// What a source byte under a rendered character is, when it is container syntax.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Part {
    /// The marker or task box of container `index`.
    Marker(usize),
    Quote,
}

#[derive(Debug, Default)]
pub(crate) struct Syntax {
    /// Quotes and items in source order, so a parent comes before its children.
    pub(super) containers: Vec<Container>,
    /// Text, code, HTML and math: bytes that are content wherever they sit.
    leaves: Vec<Range<usize>>,
}

impl Syntax {
    /// Parse a block's source, whose first byte sits at absolute offset `base`.
    pub(crate) fn parse(source: &str, base: usize) -> Self {
        let mut syntax = Self::default();
        let mut open: Vec<usize> = Vec::new();
        let mut lists: Vec<Option<u64>> = Vec::new();
        for (event, range) in Parser::new_ext(source, parse_options()).into_offset_iter() {
            let local = range.start;
            let range = range.start + base..range.end + base;
            let kind = match event {
                Event::Start(Tag::BlockQuote(_)) => Kind::Quote,
                Event::Start(Tag::Item) => {
                    let number = lists.last_mut().and_then(Option::as_mut).map(|next| {
                        *next += 1;
                        *next - 1
                    });
                    let marker = marker_len(source, local, number.is_some());
                    Kind::Item {
                        marker: range.start..range.start + marker,
                        number,
                        task: None,
                        depth: lists.len(),
                    }
                }
                Event::Start(Tag::List(start)) => {
                    lists.push(start);
                    continue;
                }
                Event::End(TagEnd::List(_)) => {
                    lists.pop();
                    continue;
                }
                Event::End(TagEnd::BlockQuote(_) | TagEnd::Item) => {
                    open.pop();
                    continue;
                }
                Event::TaskListMarker(checked) => {
                    let item = open.last().and_then(|&i| syntax.containers.get_mut(i));
                    if let Some(Container { kind: Kind::Item { task, .. }, .. }) = item {
                        *task = Some((range, checked));
                    }
                    continue;
                }
                Event::Text(_)
                | Event::Code(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::FootnoteReference(_) => {
                    syntax.leaves.push(range);
                    continue;
                }
                _ => continue,
            };
            open.push(syntax.containers.len());
            let parent = open.iter().rev().nth(1).copied();
            syntax.containers.push(Container { range, kind, parent });
        }
        syntax
    }

    /// Whether `offset` is inside text, code, HTML or math.
    pub(super) fn is_leaf(&self, offset: usize) -> bool {
        let at = self.leaves.partition_point(|leaf| leaf.end <= offset);
        self.leaves.get(at).is_some_and(|leaf| leaf.contains(&offset))
    }

    /// The deepest container holding `offset`.
    fn innermost(&self, offset: usize) -> Option<usize> {
        let mut at = self.containers.partition_point(|c| c.range.start <= offset).checked_sub(1);
        while let Some(index) = at {
            let container = self.containers.get(index)?;
            if container.range.contains(&offset) {
                return Some(index);
            }
            at = container.parent;
        }
        None
    }

    /// The containers holding `offset`, outermost first.
    pub(super) fn chain(&self, offset: usize) -> Vec<usize> {
        let mut chain: Vec<usize> = self.ancestors(offset).collect();
        chain.reverse();
        chain
    }

    /// The containers holding `offset`, innermost first.
    fn ancestors(&self, offset: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.innermost(offset), |&i| self.containers.get(i).and_then(|c| c.parent))
    }

    /// The syntax at absolute `offset` of `source` (the block's source, at `base`), if any.
    pub(super) fn part_at(&self, source: &str, base: usize, offset: usize) -> Option<Part> {
        if self.is_leaf(offset) {
            return None;
        }
        let inner = self.innermost(offset)?;
        if let Some(Container { kind: Kind::Item { marker, task, .. }, .. }) = self.containers.get(inner)
            && (marker.contains(&offset) || task.as_ref().is_some_and(|(range, _)| range.contains(&offset)))
        {
            return Some(Part::Marker(inner));
        }
        let byte = offset.checked_sub(base).and_then(|i| source.as_bytes().get(i));
        let quoted =
            || self.ancestors(offset).any(|i| self.containers.get(i).is_some_and(Container::is_quote));
        (byte == Some(&b'>') && quoted()).then_some(Part::Quote)
    }

    /// `source` with each marker spelled the way `tui-markdown` writes it (`-`, `3.`, `[x]`),
    /// so alignment maps the rendered marker to its source bytes. Byte offsets are unchanged.
    pub(crate) fn aligned_source<'a>(&self, source: &'a str, base: usize) -> Cow<'a, str> {
        let mut bytes = source.as_bytes().to_vec();
        let mut set = |offset: usize, text: &[u8]| {
            let at = offset.saturating_sub(base);
            if let Some(target) = bytes.get_mut(at..at + text.len())
                && target.is_ascii()
            {
                target.copy_from_slice(text);
            }
        };
        for container in &self.containers {
            let Kind::Item { marker, number, task, .. } = &container.kind else { continue };
            match number {
                None => set(marker.start, b"-"),
                Some(n) => {
                    let spelled = format!("{n}.");
                    if spelled.len() == marker.len() {
                        set(marker.start, spelled.as_bytes());
                    }
                }
            }
            if let Some((range, checked)) = task {
                set(range.start, if *checked { b"[x]" } else { b"[ ]" });
            }
        }
        if bytes == source.as_bytes() {
            return Cow::Borrowed(source);
        }
        String::from_utf8(bytes).map_or(Cow::Borrowed(source), Cow::Owned)
    }
}

/// Bytes of the list marker at `at`: one for a bullet, the digits and `.` or `)` for a number.
fn marker_len(source: &str, at: usize, ordered: bool) -> usize {
    if !ordered {
        return 1;
    }
    let rest = source.get(at..).unwrap_or_default();
    rest.bytes().take_while(u8::is_ascii_digit).count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_and_task_boxes_are_found_where_the_parser_puts_them() {
        let source = "> - a\n>   1) [X] b\n>   2) c";
        let syntax = Syntax::parse(source, 100);
        let byte = |text: &str| 100 + source.find(text).unwrap_or_default();
        assert!(matches!(syntax.part_at(source, 100, byte("- a")), Some(Part::Marker(_))));
        assert!(matches!(syntax.part_at(source, 100, byte(") [X]")), Some(Part::Marker(_))));
        assert!(matches!(syntax.part_at(source, 100, byte("X]")), Some(Part::Marker(_))));
        assert_eq!(syntax.part_at(source, 100, byte(">   2")), Some(Part::Quote));
        assert_eq!(syntax.part_at(source, 100, byte("a")), None);
        assert_eq!(syntax.aligned_source(source, 100), "> - a\n>   1. [x] b\n>   2. c");
    }

    #[test]
    fn escaped_and_inline_syntax_is_content() {
        let source = "- a  \n  \\- b > c\n\n  `- d`";
        let syntax = Syntax::parse(source, 0);
        for text in ["- b", "> c", "- d"] {
            assert_eq!(syntax.part_at(source, 0, source.find(text).unwrap_or_default()), None, "{text}");
        }
    }
}
