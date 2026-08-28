//! Drawing: header, gutter + document, annotation rail, footer, and the floating toolbar
//! and compose box. Pure over `App` except for recording geometry for hit-testing.

use plannotui_schema::Kind;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{App, GUTTER, Geometry, Mode, TOOLBAR, glyph, label};
use crate::wrap::wrap_line;

const RAIL_WIDTH: u16 = 36;
const RAIL_MIN_WIDTH: u16 = 28;
/// Below this the rail is dropped and annotations are only marked in the gutter.
const RAIL_MIN_TOTAL_WIDTH: u16 = 80;
const COMPOSE_WIDTH: u16 = 48;

pub(crate) const COMMENT_BG: Color = Color::Indexed(58);
pub(crate) const APPROVE_BG: Color = Color::Indexed(22);
const BLOCK_BG: Color = Color::Indexed(236);
const TOOLBAR_BG: Color = Color::Indexed(238);

fn accent(kind: Kind) -> Color {
    match kind {
        Kind::Comment => Color::Yellow,
        Kind::LooksGood => Color::Green,
        Kind::Delete => Color::Red,
    }
}

/// Painting precedence when annotations overlap a cell.
fn priority(kind: Kind) -> u8 {
    match kind {
        Kind::Comment => 0,
        Kind::LooksGood => 1,
        Kind::Delete => 2,
    }
}

impl App {
    pub(crate) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [header, body, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);

        let rail_width = if area.width >= RAIL_MIN_TOTAL_WIDTH {
            (area.width * 3 / 10).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH)
        } else {
            0
        };
        let [gutter, doc, _gap, rail] = Layout::horizontal([
            Constraint::Length(GUTTER),
            Constraint::Min(20),
            Constraint::Length(u16::from(rail_width > 0)),
            Constraint::Length(rail_width),
        ])
        .areas(body);
        self.geometry = Geometry { doc, toolbar: None };

        if self.layout.width != usize::from(doc.width) {
            self.layout.reflow(usize::from(doc.width));
            self.clear_selection();
            self.scroll_by(0);
        }

        self.draw_header(frame, header);
        self.draw_document(frame, gutter, doc);
        if rail_width > 0 {
            self.draw_rail(frame, rail);
        }
        self.draw_footer(frame, footer);
        if self.mode == Mode::Compose {
            self.draw_compose(frame);
        } else if self.pending.is_some() {
            self.draw_toolbar(frame);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let title = Line::from(vec![
            Span::raw(" ").dim(),
            Span::styled(self.source.name.clone(), Style::new().bold()),
        ]);
        let right = Line::from(Span::raw("plannotui ").dim()).right_aligned();
        frame.render_widget(Paragraph::new(title), area);
        frame.render_widget(Paragraph::new(right), area);
    }

    fn draw_document(&self, frame: &mut Frame, gutter: Rect, doc: Rect) {
        let placed = self.store.placed();
        let text_selection_active = self.selection.is_some();
        let buf = frame.buffer_mut();

        for y in 0..doc.height {
            let row_index = self.scroll + usize::from(y);
            let Some(block) = self.layout.block_at_row(row_index) else { continue };
            let Some(row) = self.layout.row(row_index) else { continue };
            let screen_y = doc.y + y;
            buf.set_line(doc.x, screen_y, &row.line, doc.width);

            if block == self.selected && !text_selection_active && self.pending.is_none() {
                buf.set_style(
                    Rect { x: doc.x, y: screen_y, width: doc.width, height: 1 },
                    Style::new().bg(BLOCK_BG),
                );
            }

            let mut row_has_annotation = false;
            for (col, cell) in row.cells.iter().enumerate().take(usize::from(doc.width)) {
                let Some(offset) = cell else { continue };
                let kind = placed
                    .iter()
                    .filter(|p| p.range.contains(offset))
                    .map(crate::store::Placed::kind)
                    .max_by_key(|&k| priority(k));
                let Some(kind) = kind else { continue };
                row_has_annotation = true;
                let style = match kind {
                    Kind::Comment => Style::new().bg(COMMENT_BG),
                    Kind::LooksGood => Style::new().bg(APPROVE_BG),
                    Kind::Delete => {
                        Style::new().fg(Color::Red).add_modifier(Modifier::CROSSED_OUT | Modifier::DIM)
                    }
                };
                buf.set_style(Rect { x: doc.x + col as u16, y: screen_y, width: 1, height: 1 }, style);
            }

            if let Some(cols) = self.selection.and_then(|s| s.columns_on(row_index, row.cells.len().max(1))) {
                let start = cols.start.min(usize::from(doc.width)) as u16;
                let end = cols.end.min(usize::from(doc.width)) as u16;
                if end > start {
                    let rect = Rect { x: doc.x + start, y: screen_y, width: end - start, height: 1 };
                    buf.set_style(rect, Style::new().add_modifier(Modifier::REVERSED));
                }
            }

            let marker = match (block == self.selected, row_has_annotation) {
                (true, _) => Span::styled("▍", Style::new().fg(Color::Cyan)),
                (false, true) => Span::styled("▍", Style::new().fg(Color::Yellow)),
                (false, false) => Span::raw(" "),
            };
            buf.set_span(gutter.x, screen_y, &marker, 1);
        }
    }

    /// Screen position for a floating widget anchored at the pending selection: one row
    /// above its first row when there is room, else just below its last row.
    fn float_origin(&self, height: u16, width: u16) -> Option<Rect> {
        let pending = self.pending.as_ref()?;
        let doc = self.geometry.doc;
        let (row, col) = pending.at;
        if row < self.scroll || row >= self.scroll + usize::from(doc.height) {
            return None;
        }
        let screen_row = doc.y + (row - self.scroll) as u16;
        let width = width.min(doc.width);
        let x = (doc.x + col as u16).min(doc.right().saturating_sub(width));
        let y = if screen_row >= doc.y + height {
            screen_row - height
        } else {
            let last_row = self.selection.map_or(row, |s| s.ordered().1.0);
            let below = doc.y + last_row.saturating_sub(self.scroll) as u16 + 1;
            below.min(doc.bottom().saturating_sub(height))
        };
        Some(Rect { x, y, width, height })
    }

    fn draw_toolbar(&mut self, frame: &mut Frame) {
        let labels: Vec<String> = TOOLBAR.iter().map(|(g, l, k, _)| format!(" {g} {l} ({k}) ")).collect();
        let width: u16 = labels.iter().map(|l| l.width() as u16).sum::<u16>() + 1;
        let Some(rect) = self.float_origin(1, width) else { return };
        frame.render_widget(Clear, rect);
        let buf = frame.buffer_mut();
        buf.set_style(rect, Style::new().bg(TOOLBAR_BG));
        let mut x = rect.x + 1;
        let mut spans = [0..0, 0..0, 0..0];
        for ((label, item), span) in labels.iter().zip(TOOLBAR.iter()).zip(spans.iter_mut()) {
            let w = label.width() as u16;
            let style = Style::new().fg(accent(item.3)).bg(TOOLBAR_BG).bold();
            buf.set_span(x, rect.y, &Span::styled(label.as_str(), style), w);
            *span = x..x + w;
            x += w;
        }
        self.geometry.toolbar = Some((rect, spans));
    }

    fn draw_compose(&self, frame: &mut Frame) {
        let Some(rect) = self.float_origin(3, COMPOSE_WIDTH) else { return };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Yellow))
            .title(Span::styled(" comment · enter saves · esc cancels ", Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let width = usize::from(inner.width.saturating_sub(1));
        let scroll = self.input.visual_scroll(width);
        let value: String = self.input.value().chars().skip(scroll).collect();
        let text_area = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
        frame.render_widget(Paragraph::new(Line::from(value)), text_area);
        let cursor_x = inner.x + 1 + self.input.visual_cursor().saturating_sub(scroll) as u16;
        frame.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), inner.y));
    }

    fn draw_rail(&self, frame: &mut Frame, rail: Rect) {
        let view_end = self.scroll + usize::from(rail.height);
        let mut next_y = rail.y;
        for placed in self.store.placed() {
            let Some(block) = self.doc.block_containing(placed.range.start) else { continue };
            let Some(rendered) = self.layout.blocks.get(block) else { continue };
            let anchor_row =
                self.layout.first_row_in_range(block, placed.range).unwrap_or(rendered.first_row);
            if anchor_row + 1 < self.scroll.saturating_sub(2)
                || anchor_row >= view_end
                || next_y >= rail.bottom()
            {
                continue;
            }
            let anchored_y = rail.y + anchor_row.saturating_sub(self.scroll) as u16;
            let y = anchored_y.max(next_y);
            let kind = placed.kind();
            let body = if placed.annotation.body.is_empty() {
                label(kind).to_owned()
            } else {
                placed.annotation.body.clone()
            };
            let inner_width = usize::from(rail.width.saturating_sub(4));
            let lines: Vec<Line<'static>> =
                wrap_line(&Line::from(body.as_str()), &[], inner_width).into_iter().map(|r| r.line).collect();
            let height = (lines.len() as u16 + 2).min(rail.bottom().saturating_sub(y));
            if height < 3 {
                break;
            }
            let border = if block == self.selected {
                Style::new().fg(accent(kind))
            } else {
                Style::new().fg(Color::DarkGray)
            };
            let title = Span::styled(
                format!(" {} {} ", glyph(kind), short_id(&placed.annotation.id)),
                Style::new().fg(accent(kind)),
            );
            let bubble = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border)
                .title(title);
            let rect = Rect { x: rail.x, y, width: rail.width, height };
            let inner = bubble.inner(rect);
            frame.render_widget(bubble, rect);
            let body_style =
                if placed.annotation.body.is_empty() { Style::new().dim().italic() } else { Style::new() };
            let text_area = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
            frame.render_widget(Paragraph::new(lines).style(body_style), text_area);
            next_y = y + height;
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let orphans = self.store.orphans();
        let mut parts = vec![
            format!("{} blocks", self.doc.blocks.len()),
            format!(
                "{} annotations{}",
                self.store.len(),
                if orphans > 0 { format!(" ({orphans} orphaned)") } else { String::new() }
            ),
            match &self.pending {
                Some(p) => format!(
                    "selected {} chars",
                    self.doc.source.get(p.range.clone()).map_or(0, |s| s.chars().count())
                ),
                None => format!("block {}/{}", self.selected + 1, self.doc.blocks.len()),
            },
            format!("frame {:.2}ms (max {:.1})", self.frame_ms, self.frame_max_ms),
        ];
        if let Some(status) = &self.status {
            parts.push(status.clone());
        }
        if frame.area().width < RAIL_MIN_TOTAL_WIDTH {
            parts.push("rail hidden: widen to ≥80 cols".into());
        }
        let help = if self.pending.is_some() {
            "a looks good · c comment · d delete · esc clear · q quit "
        } else {
            "drag to select · j/k block · c comment block · x clear block · r reload · q quit "
        };
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Min(10), Constraint::Length(help.len() as u16)]).areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(format!(" {}", parts.join(" · "))).dim())),
            left_area,
        );
        frame.render_widget(Paragraph::new(Line::from(Span::raw(help).dim()).right_aligned()), right_area);
    }
}

/// The tail of an id, enough to tell bubbles apart: `anno_…F0123` → `F0123`.
fn short_id(id: &str) -> String {
    let tail: String = id.chars().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect();
    tail
}
