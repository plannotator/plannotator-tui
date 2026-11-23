//! The kind picker (`t`): which kinds of file the list and the diff show, with how many of
//! each the change has, and whether viewed files are hidden. Space turns a row on or off,
//! Enter shows that kind alone; the list follows at once, the diff when the picker closes.

use anyhow::Result;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use super::kind::{KINDS, Kind};
use crate::keys::{self, Action};
use crate::review::{Mode, Screen};
use crate::theme::palette;
use crate::ui::hints;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::review) enum PickerRow {
    All,
    Kind(Kind),
    HideViewed,
}

const ROWS: [PickerRow; KINDS.len() + 2] = [
    PickerRow::All,
    PickerRow::Kind(Kind::Code),
    PickerRow::Kind(Kind::Test),
    PickerRow::Kind(Kind::Docs),
    PickerRow::Kind(Kind::Config),
    PickerRow::Kind(Kind::Generated),
    PickerRow::Kind(Kind::Other),
    PickerRow::HideViewed,
];

impl Screen {
    /// How many of the change's files each row covers.
    fn picker_count(&self, row: PickerRow) -> usize {
        match row {
            PickerRow::All => self.capture.files.len(),
            PickerRow::Kind(kind) => self.files.kinds.iter().filter(|k| **k == kind).count(),
            PickerRow::HideViewed => {
                self.capture.files.iter().filter(|f| self.progress.is_viewed(&f.path)).count()
            }
        }
    }

    fn picker_checked(&self, row: PickerRow) -> bool {
        let filter = &self.files.filter;
        match row {
            PickerRow::All => filter.all_kinds(),
            PickerRow::Kind(kind) => filter.shows(kind),
            PickerRow::HideViewed => filter.hide_viewed,
        }
    }

    /// Space on a row: turn it on or off. `All` turns every kind on.
    fn picker_toggle(&mut self, row: PickerRow) {
        let filter = &mut self.files.filter;
        match row {
            PickerRow::All => filter.kinds = [true; KINDS.len()],
            PickerRow::Kind(kind) => {
                if let Some(shown) = filter.kinds.get_mut(kind.index()) {
                    *shown = !*shown;
                }
            }
            PickerRow::HideViewed => filter.hide_viewed = !filter.hide_viewed,
        }
    }

    pub(in crate::review) fn kinds_key(&mut self, key: KeyEvent, cursor: usize) -> Result<()> {
        let row = ROWS.get(cursor).copied().unwrap_or(PickerRow::All);
        if key.code == KeyCode::Char(' ') && !key.modifiers.contains(KeyModifiers::CONTROL) {
            self.picker_toggle(row);
            return Ok(());
        }
        match self.keys.action(&key, keys::KINDS) {
            Some(Action::Down) => self.mode = Mode::Kinds((cursor + 1).min(ROWS.len() - 1)),
            Some(Action::Up) => self.mode = Mode::Kinds(cursor.saturating_sub(1)),
            Some(Action::Confirm) => {
                match row {
                    PickerRow::Kind(kind) => {
                        self.files.filter.kinds = [false; KINDS.len()];
                        if let Some(shown) = self.files.filter.kinds.get_mut(kind.index()) {
                            *shown = true;
                        }
                    }
                    other => self.picker_toggle(other),
                }
                return self.close_kinds();
            }
            Some(Action::Back | Action::FilterKinds | Action::Interrupt) => return self.close_kinds(),
            _ => {}
        }
        Ok(())
    }

    /// Close the picker; the diff takes the kinds picked.
    fn close_kinds(&mut self) -> Result<()> {
        self.mode = Mode::Browse;
        self.apply_filter(None)
    }

    /// The picker under the list's heading, or over the diff when the list is hidden.
    pub(in crate::review) fn draw_kinds(&self, frame: &mut Frame, cursor: usize) {
        let area = frame.area();
        let lines: Vec<(String, String, bool)> = ROWS
            .iter()
            .map(|row| {
                let label = match row {
                    PickerRow::All => "All kinds".to_owned(),
                    PickerRow::Kind(kind) => kind.label().to_owned(),
                    PickerRow::HideViewed => "Hide viewed".to_owned(),
                };
                (label, self.picker_count(*row).to_string(), self.picker_checked(*row))
            })
            .collect();
        let help = hints::picker(
            "move",
            [
                Some("space on/off".to_owned()),
                self.keys.labelled(Action::Confirm, "only"),
                self.keys.labelled(Action::Back, "close"),
            ],
        );
        let label_width = lines.iter().map(|(l, _, _)| l.width()).max().unwrap_or(0);
        let count_width = lines.iter().map(|(_, c, _)| c.width()).max().unwrap_or(0);
        let content = (4 + label_width + 2 + count_width + 1).max(help.width());
        let anchor = if self.geometry.files.width > 0 { self.geometry.files } else { self.geometry.diff };
        // Over the list it covers the list's width, so no row shows half beside it.
        let under = if self.geometry.files.width > 0 { self.geometry.files.width } else { 0 };
        let width = (content as u16 + 2).max(under).min(area.width);
        let height = (ROWS.len() as u16 + 2).min(area.height);
        let x = anchor.x.min(area.right().saturating_sub(width));
        let y = (anchor.y + 1).min(area.bottom().saturating_sub(height));
        let rect = Rect { x, y, width, height };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(" show files ", Style::new().dim()))
            .title_bottom(Span::styled(help, Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let rows: Vec<Line<'static>> = lines
            .into_iter()
            .enumerate()
            .map(|(index, (label, count, checked))| {
                let mark = if checked { "\u{2713}" } else { " " };
                // The count at the right edge, under the others.
                let left = format!(" {mark}  {label}");
                let right = format!("{count:>count_width$} ");
                let gap = usize::from(inner.width).saturating_sub(left.width() + right.width());
                let text = format!("{left}{}{right}", " ".repeat(gap));
                let style = if index == cursor {
                    palette().selection
                } else if checked {
                    Style::new()
                } else {
                    Style::new().dim()
                };
                Line::from(Span::styled(text, style))
            })
            .collect();
        frame.render_widget(Paragraph::new(rows), inner);
    }
}
