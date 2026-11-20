//! The notes rail: a heading per file, a bubble per note, the outdated ones last. Bubbles
//! are drawn like a document review's; the selected one is kept in view, and moving to a
//! note moves the diff to it.

use plannotator_tui_review::{CodeAnnotation, NoteScope, Side};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::notes::{self, Row};
use super::{Focus, Screen};
use crate::theme::palette;
use crate::wrap::wrap_line;
use unicode_width::UnicodeWidthStr as _;

/// Lines of a note's text a bubble shows; the rest is elided.
const BODY_LINES: usize = 4;

/// One drawn piece of the rail, with its height.
enum Piece {
    Heading(String, Style),
    Bubble { index: usize, title: String, body: Vec<Line<'static>>, dim: bool },
}

impl Piece {
    fn height(&self) -> usize {
        match self {
            Self::Heading(..) => 1,
            Self::Bubble { body, .. } => body.len() + 2,
        }
    }
}

impl Screen {
    pub(super) fn rail_order(&self) -> Vec<usize> {
        notes::order(self.store.annotations())
    }

    pub(super) fn note_id(&self, index: usize) -> Option<&str> {
        self.store.annotations().get(index).map(|n| n.id.as_str())
    }

    pub(super) fn selected_note(&self) -> Option<&CodeAnnotation> {
        let index = *self.rail_order().get(self.rail_cursor)?;
        self.store.annotations().get(index)
    }

    pub(super) fn clamp_rail_cursor(&mut self) {
        self.rail_cursor = self.rail_cursor.min(self.store.annotations().len().saturating_sub(1));
    }

    /// Move the rail's cursor by `delta` notes and show the note in the diff.
    pub(super) fn move_rail(&mut self, delta: isize) {
        let count = self.store.annotations().len();
        if count == 0 {
            return;
        }
        self.rail_cursor = self.rail_cursor.saturating_add_signed(delta).min(count - 1);
        self.show_selected_note();
    }

    /// Scroll the diff to the selected note: its first line, or its file's first shown line
    /// for a file note. An outdated note has no place to go.
    pub(super) fn show_selected_note(&mut self) {
        let Some(note) = self.selected_note() else { return };
        let path = note.file_path.clone();
        let place = match note.scope {
            // On no file: the rail is its only place.
            NoteScope::General => return,
            NoteScope::Line if notes::in_diff(note) => Some((note.side, note.line_start)),
            NoteScope::Line if notes::is_outdated(note) => {
                self.status = Some("outdated: its code is gone from the diff".into());
                return;
            }
            _ => self.file(&path).and_then(|file| {
                let new = file.lines(Side::New).next().map(|(n, _)| (Side::New, n));
                new.or_else(|| file.lines(Side::Old).next().map(|(n, _)| (Side::Old, n)))
            }),
        };
        let Some((side, line)) = place else {
            self.status = Some("not in the diff: its lines are unchanged now".into());
            return;
        };
        if let Err(err) = self.diff.reveal(&path, side, line) {
            self.status = Some(format!("{err:#}"));
        }
    }

    pub(super) fn draw_rail(&mut self, frame: &mut Frame, rail: Rect) {
        let notes = self.store.annotations();
        if notes.is_empty() {
            let hint =
                self.keys.labelled(crate::keys::Action::Comment, "on a line to add one").unwrap_or_default();
            let text = vec![
                Line::from(Span::styled(" No notes yet", Style::new().dim())),
                Line::from(Span::styled(format!(" {hint}"), Style::new().dim())),
            ];
            frame.render_widget(Paragraph::new(text), rail);
            self.geometry.bubbles = Vec::new();
            return;
        }
        let inner_width = usize::from(rail.width.saturating_sub(4));
        let order = notes::order(notes);
        let selected = order.get(self.rail_cursor).copied();
        let pieces: Vec<Piece> = notes::rows(notes)
            .into_iter()
            .filter_map(|row| match row {
                Row::Change => {
                    Some(Piece::Heading(" Whole change".to_owned(), Style::new().fg(palette().accent).bold()))
                }
                Row::File(path) => Some(Piece::Heading(
                    format!(" {}", tail(path, usize::from(rail.width).saturating_sub(1))),
                    Style::new().fg(palette().accent).bold(),
                )),
                Row::Outdated => {
                    Some(Piece::Heading(" Outdated".to_owned(), Style::new().fg(palette().delete).bold()))
                }
                Row::Note(index) => notes.get(index).map(|note| self.bubble(index, note, inner_width)),
            })
            .collect();
        // Keep the selected bubble on screen: scroll it into view, never past the end.
        let tops: Vec<usize> =
            pieces.iter().scan(0, |y, p| Some(std::mem::replace(y, *y + p.height()))).collect();
        let total: usize = pieces.iter().map(Piece::height).sum();
        let height = usize::from(rail.height);
        if let Some((top, piece)) = tops
            .iter()
            .zip(&pieces)
            .find(|(_, p)| matches!(p, Piece::Bubble { index, .. } if Some(*index) == selected))
        {
            // The file heading above the first note of a file comes along when it fits.
            let top_with_heading = top.saturating_sub(1);
            if top_with_heading < self.rail_scroll {
                self.rail_scroll = top_with_heading;
            } else if top + piece.height() > self.rail_scroll + height {
                self.rail_scroll = (top + piece.height()).saturating_sub(height);
            }
        }
        self.rail_scroll = self.rail_scroll.min(total.saturating_sub(height));
        let mut bubbles = Vec::new();
        for (top, piece) in tops.iter().zip(pieces) {
            let Some(y) = top.checked_sub(self.rail_scroll) else { continue };
            if y + piece.height() > height {
                break;
            }
            let y = rail.y + y as u16;
            match piece {
                Piece::Heading(text, style) => {
                    let row = Rect { y, height: 1, ..rail };
                    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), row);
                }
                Piece::Bubble { index, title, body, dim } => {
                    let rect = Rect { y, height: body.len() as u16 + 2, ..rail };
                    let is_selected = Some(index) == selected;
                    let colour = if dim { Color::DarkGray } else { palette().comment };
                    let border = match (is_selected, self.focus == Focus::Notes) {
                        (true, true) => Style::new().fg(colour).bold(),
                        (true, false) => Style::new().fg(colour),
                        (false, _) => Style::new().fg(Color::DarkGray),
                    };
                    let bubble = Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(border)
                        .title(Span::styled(title, Style::new().fg(colour)));
                    let inner = bubble.inner(rect);
                    frame.render_widget(bubble, rect);
                    let text_area = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
                    let style = if dim { Style::new().dim() } else { Style::new() };
                    frame.render_widget(Paragraph::new(body).style(style), text_area);
                    bubbles.push((rect, index));
                }
            }
        }
        self.geometry.bubbles = bubbles;
    }

    fn bubble(&self, index: usize, note: &CodeAnnotation, width: usize) -> Piece {
        let mut tags = vec![notes::place(note)];
        let author = note.author_name.as_deref().filter(|n| !n.is_empty());
        if let Some(name) = author {
            tags.push(name.to_owned());
        }
        let off_diff = !notes::is_outdated(note) && note.scope == NoteScope::Line && !notes::in_diff(note);
        if off_diff {
            tags.push("not in diff".to_owned());
        }
        let sent = !self.store.is_pending(note);
        if sent {
            tags.push("sent".to_owned());
        }
        let mut title = format!(" {} {} ", notes::glyph(note), tags.join(" \u{b7} "));
        let mut text = notes::body(note);
        // A title wider than the bubble's top border would cut the credit off; it moves into
        // the body instead. The border spans `width` plus the padding and two corners.
        if let Some(name) = author
            && title.width() > width + 2
        {
            tags.retain(|tag| tag != name);
            title = format!(" {} {} ", notes::glyph(note), tags.join(" \u{b7} "));
            text = format!("From: {name}\n{text}");
        }
        let mut body: Vec<Line<'static>> = text
            .split('\n')
            .flat_map(|line| {
                wrap_line(&Line::from(line.to_owned()), &[], width.max(1)).into_iter().map(|r| r.line)
            })
            .collect();
        if body.len() > BODY_LINES {
            body.truncate(BODY_LINES);
            if let Some(last) = body.last_mut() {
                last.spans.push(Span::raw(" \u{2026}"));
            }
        }
        if body.is_empty() {
            body.push(Line::default());
        }
        Piece::Bubble { index, title, body, dim: sent || notes::is_outdated(note) }
    }
}

/// The last `width` columns of a path, with `…` when it was cut.
fn tail(path: &str, width: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= width || width < 2 {
        return path.to_owned();
    }
    let kept: String = chars.iter().skip(chars.len() + 1 - width).collect();
    format!("\u{2026}{kept}")
}
