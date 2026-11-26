//! What an Ask box says: its lines in the title, then each question and its answer, each
//! under who wrote it, with a rule between one question and the next.

use std::hash::{Hash as _, Hasher as _};

use plannotator_tui_review::Side;
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};

use super::Thread;
use crate::review::diff::Lines;
use crate::theme::palette;

impl Thread {
    /// `ask · L9`, `ask · L9–12`, `ask · L7 old`, as a note's box names its lines.
    pub(in crate::review) fn label(&self) -> String {
        let Lines { side, first, last, .. } = &self.lines;
        let lines = if first == last { format!("L{first}") } else { format!("L{first}\u{2013}{last}") };
        let old = if *side == Side::Old { " old" } else { "" };
        format!("ask \u{b7} {lines}{old}")
    }

    /// A number that changes whenever what the box shows does.
    pub(in crate::review) fn revision(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for turn in &self.turns {
            (&turn.question, &turn.agent, turn.sent, turn.complete, &turn.answer, &turn.failed)
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The box's text at `width` columns: each question, then its answer as Markdown, all of it.
    pub(in crate::review) fn body(&self, width: usize) -> Vec<Line<'static>> {
        let speaker =
            |who: &str, colour| Line::from(Span::styled(who.to_owned(), Style::new().fg(colour).bold()));
        let mut rows = Vec::new();
        for (index, turn) in self.turns.iter().enumerate() {
            if index > 0 {
                let rule = "\u{254c}".repeat(width);
                rows.push(Line::from(Span::styled(rule, Style::new().fg(palette().idle_fg))));
            }
            rows.push(speaker("You", palette().accent));
            for line in turn.question.lines() {
                rows.extend(wrapped(&Line::from(line.to_owned()), width));
            }
            rows.push(Line::default());
            rows.push(speaker(&turn.agent, palette().heading));
            for line in crate::layout::styled(&turn.answer).lines {
                rows.extend(wrapped(&line, width));
            }
            let state = match (&turn.failed, turn.sent, turn.complete) {
                (Some(why), true, _) => {
                    Some(Span::styled(format!("cannot read the answer: {why}"), palette().delete))
                }
                (Some(why), false, _) => Some(Span::styled(format!("not sent: {why}"), palette().delete)),
                (None, _, _) if !turn.answer.is_empty() => None,
                (None, false, _) => Some(Span::raw("sending\u{2026}").italic()),
                (None, true, false) => Some(Span::raw("waiting for the answer\u{2026}").italic()),
                (None, true, true) => Some(Span::raw("no answer").italic()),
            };
            if let Some(state) = state {
                rows.extend(wrapped(&Line::from(state), width));
            }
        }
        rows
    }
}

fn wrapped(line: &Line<'_>, width: usize) -> impl Iterator<Item = Line<'static>> {
    let chars: usize = line.spans.iter().map(|span| span.content.chars().count()).sum();
    crate::wrap::wrap_line(line, &vec![None; chars], width).into_iter().map(|row| row.line)
}
