//! Colours for fenced code, by language, worked out only for the lines that are drawn.
//!
//! The grammars and the engine are the ones diffs-rs colours its diffs with (`two-face`'s
//! bundled grammars on its `syntect` fork, both pure Rust). The colours are not a syntect
//! theme: each scope maps to a [`Role`], and each role to a colour of the palette in force, so
//! code follows `[colors]`, light and dark terminals, and `NO_COLOR` like the rest of the app.
//!
//! A block is coloured from its first line down to the last line anyone has asked for, and
//! the parser's state is kept between asks, so a long document only pays for the code on
//! screen. A grammar that fails, or a line too long to parse quickly, leaves the rest of its
//! block plain.

use std::ops::Range;
use std::str::FromStr as _;
use std::sync::OnceLock;

use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use syntect::highlighting::ScopeSelectors;
use syntect::highlighting::{
    Color as SyntectColor, HighlightIterator, HighlightState, Highlighter, StyleModifier, Theme, ThemeItem,
    ThemeSettings,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use crate::srcmap::LineOffsets;
use crate::theme::palette;

/// Lines longer than this stop the colouring: a grammar's regexes can take a long time on one.
const LONGEST_LINE: usize = 4096;
/// Lines coloured past the one asked for, so scrolling down a block does not stop each row.
const READ_AHEAD: usize = 64;
/// Lines of each sample [`warm_up`] parses: enough to reach most of a grammar's patterns.
pub(crate) const WARM_LINES: usize = 40;

/// What a piece of code is, as far as its colour goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Keyword,
    String,
    Number,
    Function,
    Type,
    Comment,
}

impl Role {
    const ALL: [Role; 6] =
        [Role::Keyword, Role::String, Role::Number, Role::Function, Role::Type, Role::Comment];

    /// The scopes this role covers. Where two selectors match, the more specific one wins,
    /// whatever the role: `storage.type.struct` is a keyword, `support.type` a type.
    fn scopes(self) -> &'static str {
        match self {
            // Sublime grammars file most declaration keywords (`fn`, `let`, `def`) as storage.
            Role::Keyword => "keyword, storage, keyword.operator.word, entity.name.tag, variable.language",
            Role::String => "string, constant.character.escape",
            Role::Number => "constant.numeric, constant.language, constant.character, constant.other",
            Role::Function => "entity.name.function, support.function, variable.function",
            Role::Type => {
                "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, \
                 entity.name.trait, entity.name.impl, entity.other.inherited-class, support.type, support.class"
            }
            Role::Comment => "comment",
        }
    }

    /// The role a theme colour stands for: its red channel is the role's place in `ALL`, plus one.
    fn from_color(color: SyntectColor) -> Option<Role> {
        usize::from(color.r).checked_sub(1).and_then(|i| Role::ALL.get(i)).copied()
    }

    /// How this role is drawn with the palette in force.
    pub(crate) fn style(self) -> Style {
        let p = palette();
        match self {
            Role::Keyword => Style::new().fg(p.code_keyword),
            Role::String => Style::new().fg(p.code_string),
            Role::Number => Style::new().fg(p.code_number),
            Role::Function => Style::new().fg(p.code_function),
            Role::Type => Style::new().fg(p.code_type),
            Role::Comment => Style::new().fg(p.code_comment).add_modifier(Modifier::ITALIC),
        }
    }
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// A theme whose every colour names a [`Role`] instead of being one.
fn role_theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let scopes = Role::ALL
            .iter()
            .enumerate()
            .filter_map(|(index, role)| {
                let scope = ScopeSelectors::from_str(role.scopes()).ok()?;
                let r = u8::try_from(index + 1).ok()?;
                let foreground = Some(SyntectColor { r, g: 0, b: 0, a: 255 });
                Some(ThemeItem { scope, style: StyleModifier { foreground, ..StyleModifier::default() } })
            })
            .chain(plain_scopes())
            .collect();
        let settings = ThemeSettings {
            foreground: Some(SyntectColor { r: 0, g: 0, b: 0, a: 255 }),
            ..ThemeSettings::default()
        };
        Theme { settings, scopes, ..Theme::default() }
    })
}

/// Operators and separating punctuation keep the code's own colour, even where a broader
/// role's selector (`keyword`) would take them. The quotes of a string and the `//` of a
/// comment are `punctuation.definition` and stay with their string or comment.
fn plain_scopes() -> Option<ThemeItem> {
    let selectors = "keyword.operator, punctuation.separator, punctuation.terminator, \
                     punctuation.section, punctuation.accessor";
    let scope = ScopeSelectors::from_str(selectors).ok()?;
    let foreground = Some(SyntectColor { r: 0, g: 0, b: 0, a: 255 });
    Some(ThemeItem { scope, style: StyleModifier { foreground, ..StyleModifier::default() } })
}

/// The grammar for a fence's info string (`rust`, `py`, `rust,ignore`, `{.python}`), if one
/// is bundled.
pub(crate) fn syntax_for(info: &str) -> Option<&'static SyntaxReference> {
    let token = info
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()?
        .trim_matches(|c: char| matches!(c, '{' | '}' | '.'));
    if token.is_empty() {
        return None;
    }
    let set = syntaxes();
    set.find_syntax_by_token(token).or_else(|| set.find_syntax_by_token(&token.to_ascii_lowercase()))
}

/// Compile, off the main thread, the grammars a document's first code blocks use, so the
/// first frame does not wait for them. `samples` are a fence's info string and some of its
/// code. A grammar compiles its patterns once per process, the first time a line needs them;
/// parsing a few lines here does that, and the drawing thread finds them ready.
pub(crate) fn warm_up(samples: Vec<(String, String)>) {
    if samples.is_empty() {
        return;
    }
    let warm = move || {
        for (info, code) in samples {
            let Some(syntax) = syntax_for(&info) else { continue };
            let mut parse = ParseState::new(syntax);
            for line in code.split_inclusive('\n').take(WARM_LINES) {
                if parse.parse_line(line, syntaxes()).is_err() {
                    break;
                }
            }
        }
    };
    // Without a thread the first frame compiles them instead, as it would have anyway.
    let _ = std::thread::Builder::new().name("code-grammars".to_owned()).spawn(warm);
}

/// One block's colours so far, and where the parser stopped.
#[derive(Debug)]
pub(crate) struct CodeColors {
    syntax: &'static SyntaxReference,
    state: Option<(ParseState, HighlightState)>,
    /// Lines coloured so far.
    done: usize,
    /// Absolute source ranges and their roles, in source order.
    spans: Vec<(Range<usize>, Role)>,
}

impl CodeColors {
    pub(crate) fn new(syntax: &'static SyntaxReference) -> Self {
        Self { syntax, state: None, done: 0, spans: Vec::new() }
    }

    /// Colour the block's `lines`, whose characters come from source bytes `offsets`, up to
    /// and including line `upto`, plus a little more.
    pub(crate) fn advance(&mut self, lines: &[Line<'_>], offsets: &[LineOffsets], upto: usize) {
        if upto < self.done {
            return;
        }
        let highlighter = Highlighter::new(role_theme());
        let (mut parse, mut highlight) = match self.state.take() {
            Some(state) => state,
            None if self.done == 0 => {
                (ParseState::new(self.syntax), HighlightState::new(&highlighter, ScopeStack::new()))
            }
            // A line already failed: the rest of the block stays plain.
            None => return,
        };
        let last = upto + READ_AHEAD;
        let todo = lines.iter().zip(offsets).skip(self.done).take(last + 1 - self.done);
        for (line, offsets) in todo {
            let mut line: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
            if line.len() > LONGEST_LINE {
                return;
            }
            line.push('\n');
            let Ok(ops) = parse.parse_line(&line, syntaxes()) else { return };
            let mut chars = offsets.iter();
            for (style, piece) in HighlightIterator::new(&mut highlight, &ops, &line, &highlighter) {
                let role = Role::from_color(style.foreground);
                for (ch, offset) in piece.chars().filter(|&ch| ch != '\n').zip(chars.by_ref()) {
                    let (Some(role), Some(offset)) = (role, *offset) else { continue };
                    self.push(offset..offset + ch.len_utf8(), role);
                }
            }
            self.done += 1;
        }
        self.state = Some((parse, highlight));
    }

    fn push(&mut self, range: Range<usize>, role: Role) {
        match self.spans.last_mut() {
            Some((last, last_role)) if *last_role == role && last.end == range.start => last.end = range.end,
            _ => self.spans.push((range, role)),
        }
    }

    /// The role of the code at source byte `offset`, if it has one.
    pub(crate) fn role_at(&self, offset: usize) -> Option<Role> {
        let at = self.spans.partition_point(|(range, _)| range.end <= offset);
        self.spans.get(at).filter(|(range, _)| range.contains(&offset)).map(|&(_, role)| role)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn colored(lang: &str, code: &str) -> (CodeColors, Vec<Line<'static>>, Vec<LineOffsets>) {
        let syntax = syntax_for(lang).expect("bundled grammar");
        let mut at = 0;
        let (lines, offsets): (Vec<Line<'static>>, Vec<LineOffsets>) = code
            .split('\n')
            .map(|line| {
                let offsets = line.char_indices().map(|(i, _)| Some(at + i)).collect();
                at += line.len() + 1;
                (Line::from(line.to_owned()), offsets)
            })
            .unzip();
        let mut colors = CodeColors::new(syntax);
        colors.advance(&lines, &offsets, 0);
        (colors, lines, offsets)
    }

    #[test]
    fn keywords_strings_and_comments_get_their_roles() {
        let code = "fn main() {\n    let s: String = \"hi\"; // note\n    s.len() + 42\n}";
        let (colors, _, _) = colored("rust", code);
        let at = |text: &str| code.find(text).expect("present");
        assert_eq!(colors.role_at(at("fn")), Some(Role::Keyword));
        assert_eq!(colors.role_at(at("let")), Some(Role::Keyword));
        assert_eq!(colors.role_at(at("main")), Some(Role::Function));
        assert_eq!(colors.role_at(at("String")), Some(Role::Type));
        assert_eq!(colors.role_at(at("hi")), Some(Role::String));
        assert_eq!(colors.role_at(at("42")), Some(Role::Number));
        assert_eq!(colors.role_at(at("note")), Some(Role::Comment));
        assert_eq!(colors.role_at(at("// note")), Some(Role::Comment), "the comment's marker too");
        assert_eq!(colors.role_at(at("\"hi")), Some(Role::String), "and the string's quotes");
        // Names and operators keep the terminal's colour.
        assert_eq!(colors.role_at(at("s:")), None);
        assert_eq!(colors.role_at(at(" = ") + 1), None);
    }

    #[test]
    fn info_strings_name_a_grammar_by_their_first_word() {
        for info in ["rust", "rs", "Rust", "rust,ignore", "python title=x.py", "{.python}", "py"] {
            assert!(syntax_for(info).is_some(), "{info}");
        }
        for info in ["", "not-a-language", "   "] {
            assert!(syntax_for(info).is_none(), "{info}");
        }
    }

    #[test]
    fn colouring_stops_where_it_was_asked_to_and_resumes_from_there() {
        let code = (0..300).fold(String::new(), |code, i| code + &format!("x{i} = {i}\n"));
        let (mut colors, lines, offsets) = colored("python", &code);
        assert_eq!(colors.done, READ_AHEAD + 1);
        colors.advance(&lines, &offsets, 200);
        assert_eq!(colors.done, 200 + READ_AHEAD + 1);
        assert_eq!(colors.role_at(code.rfind("299").expect("present")), None, "not asked for yet");
        colors.advance(&lines, &offsets, 299);
        assert_eq!(colors.done, lines.len());
        let last = code.rfind("299").expect("present");
        assert_eq!(colors.role_at(last), Some(Role::Number));
    }
}
