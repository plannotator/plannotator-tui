//! Document model: the source text split into top-level Markdown blocks with byte ranges.
//!
//! The parser is `pulldown-cmark`; this module only walks its event stream at depth zero
//! and records where each block starts and ends. Everything downstream (rendering,
//! anchoring, hit-testing) is keyed by block index and byte range. Nothing here knows what
//! a heading or a list *is*.

use std::ops::Range;

use plannotator_tui_schema::{DocumentSource, Provenance};
use pulldown_cmark::{Event, Options, Parser, Tag};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Heading,
    Paragraph,
    List,
    CodeBlock,
    BlockQuote,
    Table,
    Rule,
    Html,
    Other,
}

impl BlockKind {
    /// Code and tables keep their columns; everything else word-wraps.
    pub(crate) fn preserves_columns(self) -> bool {
        matches!(self, BlockKind::CodeBlock | BlockKind::Table)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub(crate) range: Range<usize>,
    pub(crate) kind: BlockKind,
}

#[derive(Debug)]
pub(crate) struct Document {
    pub(crate) source: String,
    pub(crate) blocks: Vec<Block>,
}

/// The option set `tui-markdown` enables, so block boundaries agree with its renderer.
pub(crate) fn parse_options() -> Options {
    Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT
        | Options::ENABLE_MATH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_GFM
        | Options::ENABLE_TABLES
}

impl Document {
    /// A file's text: YAML front matter at the top is metadata and is not shown.
    pub(crate) fn parse(source: String) -> Self {
        let blocks = split_blocks(&source, true);
        Self { source, blocks }
    }

    /// The document for `source`. Only a file or a Workspaces document can carry front
    /// matter; an agent's message or piped text is all content, even when it opens with `---`.
    pub(crate) fn of(source: &DocumentSource) -> Self {
        let front_matter =
            matches!(source.provenance, Provenance::File { .. } | Provenance::Workspace { .. });
        let blocks = split_blocks(&source.content, front_matter);
        Self { source: source.content.clone(), blocks }
    }

    /// Source text of block `index`. Empty for an out-of-range index.
    pub(crate) fn block_text(&self, index: usize) -> &str {
        self.blocks.get(index).map_or("", |b| &self.source[b.range.clone()])
    }

    /// The block whose range contains `offset`.
    pub(crate) fn block_containing(&self, offset: usize) -> Option<usize> {
        self.blocks.iter().position(|b| b.range.contains(&offset))
    }
}

fn kind_of(tag: &Tag<'_>) -> BlockKind {
    match tag {
        Tag::Heading { .. } => BlockKind::Heading,
        Tag::Paragraph => BlockKind::Paragraph,
        Tag::List(_) => BlockKind::List,
        Tag::CodeBlock(_) => BlockKind::CodeBlock,
        Tag::BlockQuote(_) => BlockKind::BlockQuote,
        Tag::Table(_) => BlockKind::Table,
        Tag::HtmlBlock => BlockKind::Html,
        _ => BlockKind::Other,
    }
}

/// Bytes of the YAML front matter that opens `source`, or 0 when it has none. Front matter
/// starts on the first line and must close; `pulldown-cmark` takes a metadata block at any
/// unindented `---` line, so it is only asked about the first one.
fn front_matter_len(source: &str) -> usize {
    if !source.starts_with("---") {
        return 0;
    }
    match Parser::new_ext(source, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS).into_offset_iter().next() {
        Some((Event::Start(Tag::MetadataBlock(_)), range)) if range.start == 0 => range.end,
        _ => 0,
    }
}

/// Walk the event stream and cut the source at depth-zero block boundaries. Front matter,
/// when `front_matter` allows it, is never a block; every later `---` is Markdown.
fn split_blocks(source: &str, front_matter: bool) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    let mut open: Option<(usize, BlockKind)> = None;

    let body = if front_matter { front_matter_len(source) } else { 0 };
    let mut options = parse_options();
    options.remove(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    for (event, range) in Parser::new_ext(&source[body..], options).into_offset_iter() {
        let range = range.start + body..range.end + body;
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    open = Some((range.start, kind_of(&tag)));
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Some((start, kind)) = open.take()
                {
                    blocks.push(Block { range: start..range.end, kind });
                }
            }
            Event::Rule if depth == 0 => blocks.push(Block { range, kind: BlockKind::Rule }),
            // Any other depth-zero leaf (rare: stray html/text) becomes its own block.
            _ if depth == 0 => blocks.push(Block { range, kind: BlockKind::Other }),
            _ => {}
        }
    }

    // Trailing newlines are not part of a block's text: quotes stay stable across files
    // that differ only in final-newline conventions.
    for block in &mut blocks {
        let text = &source[block.range.clone()];
        let trimmed = text.trim_end_matches(['\n', '\r']);
        block.range.end = block.range.start + trimmed.len();
    }
    blocks.retain(|b| !b.range.is_empty());
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_top_level_blocks_with_ranges() {
        let src = "# Title\n\nPara one\nstill one.\n\n- a\n- b\n\n```rs\nfn x() {}\n```\n\n---\n";
        let doc = Document::parse(src.to_owned());
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [
                BlockKind::Heading,
                BlockKind::Paragraph,
                BlockKind::List,
                BlockKind::CodeBlock,
                BlockKind::Rule
            ]
        );
        assert_eq!(doc.block_text(0), "# Title");
        assert_eq!(doc.block_text(1), "Para one\nstill one.");
        assert_eq!(doc.block_text(3), "```rs\nfn x() {}\n```");
    }

    #[test]
    fn front_matter_is_dropped_and_nested_lists_stay_one_block() {
        let doc = Document::parse("---\ntitle: X\n---\n\n- a\n  - nested\n- b\n".to_owned());
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks.first().map(|b| b.kind), Some(BlockKind::List));
    }

    fn texts(doc: &Document) -> Vec<&str> {
        (0..doc.blocks.len()).map(|i| doc.block_text(i)).collect()
    }

    /// Issue #162: a pair of rules mid-document is two rules, not front matter.
    #[test]
    fn rule_pair_mid_document_keeps_the_text_between() {
        let src = "# Title\n\nIntro paragraph.\n\n---\n**Draft**\n\nThis paragraph is not shown.\n\n- neither is this list\n\n---\n\nClosing paragraph.\n";
        let doc = Document::parse(src.to_owned());
        assert_eq!(
            texts(&doc),
            [
                "# Title",
                "Intro paragraph.",
                "---",
                "**Draft**",
                "This paragraph is not shown.",
                "- neither is this list",
                "---",
                "Closing paragraph."
            ]
        );
    }

    #[test]
    fn front_matter_at_the_top_is_dropped_and_later_rules_stay() {
        let src = "---\ntitle: X\n---\n# Heading\n\n---\nkept: yes\n---\n";
        let doc = Document::parse(src.to_owned());
        assert_eq!(texts(&doc), ["# Heading", "---", "kept: yes\n---"]);
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, [BlockKind::Heading, BlockKind::Rule, BlockKind::Heading]);
        // Ranges index the whole source, front matter included.
        assert_eq!(doc.blocks.first().map(|b| b.range.clone()), Some(17..26));
        assert_eq!(src.find("# Heading"), Some(17));
        assert_eq!(doc.block_containing(3), None);
    }

    #[test]
    fn unclosed_rule_at_the_top_is_content() {
        let doc = Document::parse("---\ntitle: X\n\nBody.\n".to_owned());
        assert_eq!(texts(&doc), ["---", "title: X", "Body."]);
        // A blank second line is not front matter either, and does not move it further down.
        let doc = Document::parse("---\n\nBody.\n\n---\nmiddle\n---\n".to_owned());
        assert_eq!(texts(&doc), ["---", "Body.", "---", "middle\n---"]);
    }

    #[test]
    fn rule_under_a_paragraph_is_a_setext_heading() {
        let doc = Document::parse("Intro\n---\nbody\n\nMore\n---\n".to_owned());
        assert_eq!(texts(&doc), ["Intro\n---", "body", "More\n---"]);
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, [BlockKind::Heading, BlockKind::Paragraph, BlockKind::Heading]);
    }

    #[test]
    fn an_agent_message_has_no_front_matter() {
        let text = "---\ntitle: X\n---\nReply.\n";
        let message = |provenance| DocumentSource::new(text.to_owned(), "m", true, provenance);
        let reply = Document::of(&message(Provenance::AgentMessage {
            host: "claude-code".into(),
            session: None,
            message_id: None,
        }));
        assert_eq!(texts(&reply), ["---", "title: X\n---", "Reply."]);
        assert_eq!(texts(&Document::of(&message(Provenance::Stdin))), ["---", "title: X\n---", "Reply."]);
        let file = Document::of(&DocumentSource::file("a.md".into(), text.to_owned()));
        assert_eq!(texts(&file), ["Reply."]);
    }
}
