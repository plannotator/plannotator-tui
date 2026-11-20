//! Top-level code blocks: drawn without their fences, beside a quiet bar, with the language
//! at the top right when the first line leaves room, and coloured by language.
//!
//! The rows are laid out like any other block: one per code line, and a line too long for
//! the column continues on the next row rather than being cut off. Colour is not part of
//! them: it is worked out the first time a row is drawn ([`Code::paint`]) and kept, so a
//! document full of code only pays for the code on screen. Every cell keeps the source byte
//! the renderer aligned it to; the bar, the padding and the label have none.

use std::cell::RefCell;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use crate::highlight::{CodeColors, syntax_for};
use crate::srcmap::LineOffsets;
use crate::theme::palette;
use crate::wrap::{Row, split_line};

/// Drawn down the left of a code block. One column wide even where ambiguous-width
/// characters are drawn wide.
#[cfg(test)]
pub(crate) const BAR: char = '▐';
/// The bar and the space after it, starting each row.
const BAR_PREFIX: &str = "▐ ";
const BAR_WIDTH: usize = 2;
/// Fence words that name no language worth labelling.
const UNLABELLED: [&str; 4] = ["text", "txt", "plain", "plaintext"];

#[derive(Debug)]
pub(crate) struct Code {
    /// The first word of the fence's info string; empty for an indented block or a bare fence.
    lang: String,
    /// What the first row shows at its right: the language, when it is worth naming.
    label: Option<String>,
    colors: RefCell<Colors>,
}

#[derive(Debug)]
enum Colors {
    /// Nothing has been drawn yet, so nobody has looked the language up.
    Unresolved,
    /// No bundled grammar: the block keeps the one code colour.
    Plain,
    Known(Box<CodeColors>),
}

impl Code {
    /// A code block whose opening fence carried `info` (empty for an indented block).
    pub(crate) fn new(info: &str) -> Self {
        let lang = info.split(|c: char| c.is_whitespace() || c == ',').next().unwrap_or_default();
        let label = lang.trim_matches(|c: char| matches!(c, '{' | '}' | '.'));
        let named = !label.is_empty() && !UNLABELLED.iter().any(|word| word.eq_ignore_ascii_case(label));
        Self {
            lang: lang.to_owned(),
            label: named.then(|| label.to_owned()),
            colors: RefCell::new(Colors::Unresolved),
        }
    }

    /// The block's rows, `width` columns wide: one per code line, and more for a line too long
    /// to fit. The label goes at the right of the first row when that row leaves room for it.
    pub(crate) fn rows(&self, lines: &[Line<'_>], offsets: &[LineOffsets], width: usize) -> Vec<Row> {
        let bar = Span::styled(BAR_PREFIX, Style::new().fg(palette().code_comment));
        let mut rows = Vec::with_capacity(lines.len());
        for (line, offsets) in lines.iter().zip(offsets) {
            split_line(&mut rows, &bar, line, offsets, width);
        }
        if rows.is_empty() {
            rows.push(Row { line: Line::from(bar), cells: vec![None; BAR_WIDTH] });
        }
        if let (Some(label), Some(first)) = (&self.label, rows.first_mut()) {
            let used = first.cells.len();
            // One blank column between the code and the label, at least.
            if used + 1 + label.width() <= width {
                let pad = width - label.width() - used;
                first.line.spans.push(Span::raw(" ".repeat(pad)));
                first.line.spans.push(Span::styled(label.clone(), Style::new().fg(palette().code_comment)));
                first.cells.extend(std::iter::repeat_n(None, pad + label.width()));
            }
        }
        rows
    }

    /// `row`, the block's row `index`, in colour; `None` to draw it as laid out.
    pub(crate) fn paint(
        &self,
        lines: &[Line<'_>],
        offsets: &[LineOffsets],
        index: usize,
        row: &Row,
    ) -> Option<Line<'static>> {
        let mut colors = self.colors.borrow_mut();
        if matches!(*colors, Colors::Unresolved) {
            *colors = match syntax_for(&self.lang) {
                Some(syntax) => Colors::Known(Box::new(CodeColors::new(syntax))),
                None => Colors::Plain,
            };
        }
        let Colors::Known(colors) = &mut *colors else { return None };
        colors.advance(lines, offsets, index);
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut column = 0;
        for span in &row.line.spans {
            for ch in span.content.chars() {
                let style = match row.cells.get(column).copied().flatten() {
                    Some(offset) => match colors.role_at(offset) {
                        Some(role) => span.style.patch(role.style()),
                        // Plain code reads in the terminal's own colour beside coloured code.
                        None => span.style.fg(Color::Reset),
                    },
                    None => span.style,
                };
                column += ch.width().unwrap_or(0);
                match spans.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push(ch),
                    _ => spans.push(Span::styled(ch.to_string(), style)),
                }
            }
        }
        Some(Line::from(spans).style(row.line.style))
    }
}
