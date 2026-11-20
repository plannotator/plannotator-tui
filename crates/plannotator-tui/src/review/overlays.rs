//! What opens over the review: the Review menu under its button, and the key help.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use super::Screen;
use super::input::{MENU, MenuRow};
use crate::keys::Action;
use crate::theme::palette;
use crate::ui::hints;

impl Screen {
    pub(super) fn draw_menu(&mut self, frame: &mut Frame, cursor: usize) {
        let area = frame.area();
        let sent = self.store.annotations().iter().filter(|n| !self.store.is_pending(n)).count();
        let rows: Vec<String> = MENU
            .iter()
            .map(|row| {
                let label = match row {
                    MenuRow::ChangeComment => "Comment on the whole change".to_owned(),
                    MenuRow::ResendAll => {
                        format!("Resend all \u{b7} {} notes", self.store.annotations().len())
                    }
                    MenuRow::Finish => format!("Finish review \u{b7} archive {sent} sent"),
                    MenuRow::Undo => "Undo finish".to_owned(),
                };
                format!(" {}  {label} ", self.keys.hint(row.binding()))
            })
            .collect();
        let help = hints::picker(
            "move",
            [self.keys.labelled(Action::Confirm, "run"), self.keys.labelled(Action::Back, "close")],
        );
        let content = rows.iter().map(|r| r.width()).max().unwrap_or(0).max(help.width());
        let width = (content as u16 + 2).min(area.width);
        let height = (rows.len() as u16 + 2).min(area.height);
        let button = self.geometry.review_button.or(self.geometry.send_button);
        let x = button.map_or(area.right(), Rect::right).saturating_sub(width).max(area.x);
        let y = button.map_or(area.y, Rect::bottom).min(area.bottom().saturating_sub(height));
        let rect = Rect { x, y, width, height };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(" review ", Style::new().dim()))
            .title_bottom(Span::styled(help, Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let mut menu_rows = Vec::new();
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .enumerate()
            .take(usize::from(inner.height))
            .map(|(index, text)| {
                menu_rows.push((
                    Rect { x: inner.x, y: inner.y + index as u16, width: inner.width, height: 1 },
                    index,
                ));
                let style = if index == cursor { palette().selection } else { Style::new() };
                Line::from(Span::styled(format!("{text:<pad$}", pad = usize::from(inner.width)), style))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        self.geometry.menu_rows = menu_rows;
    }

    /// `?`: every key, in the user's keymap.
    pub(super) fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let key = |actions: &[Action]| {
            actions.iter().map(|a| self.keys.hint(*a)).filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" ")
        };
        let sections: [(&str, Vec<(String, &str)>); 4] = [
            (
                "anywhere",
                vec![
                    (key(&[Action::CycleFocus]), "diff / notes"),
                    (key(&[Action::ToggleNotes]), "hide the notes, or show them again"),
                    (key(&[Action::Send]), "send new and edited notes"),
                    (key(&[Action::SendAndClose]), "send and close"),
                    (key(&[Action::ChangeComment]), "comment on the whole change"),
                    (
                        key(&[Action::ReviewMenu]),
                        "review menu: whole-change comment, resend all, finish, undo",
                    ),
                    (key(&[Action::PickTarget]), "choose the agent Send goes to"),
                    (
                        key(&[Action::Reload]),
                        "reload the diff (the footer says when it changed); notes follow",
                    ),
                    (key(&[Action::Quit]), "quit"),
                ],
            ),
            (
                "diff",
                vec![
                    (key(&[Action::Down, Action::Up]), "line"),
                    (key(&[Action::NextHunk, Action::PreviousHunk]), "hunk"),
                    (key(&[Action::NextFile, Action::PreviousFile]), "file"),
                    (key(&[Action::ToggleViewed]), "mark the file viewed, or not (shared with the browser)"),
                    (
                        key(&[Action::NextUnviewed, Action::PreviousUnviewed]),
                        "next / previous file not viewed",
                    ),
                    (key(&[Action::Top, Action::Bottom]), "first / last"),
                    (key(&[Action::Left, Action::Right]), "old / new side"),
                    (key(&[Action::Visual]), "select lines (again to finish); shift-click too"),
                    (key(&[Action::Comment]), "comment on the selection or line"),
                    (key(&[Action::Suggest]), "suggest code for new lines"),
                    (key(&[Action::FileComment]), "comment on the file"),
                    (key(&[Action::WholeFile]), "the whole file as it is now, at this line"),
                    (key(&[Action::WholeBase]), "the whole file before the change (base)"),
                    (key(&[Action::Back]), "clear the selection"),
                    (key(&[Action::ToggleLayout]), "split / unified"),
                    (key(&[Action::ToggleWrap]), "wrap long lines"),
                    (key(&[Action::ToggleFiles]), "file list"),
                ],
            ),
            (
                "whole file",
                vec![
                    (
                        key(&[Action::WholeFile, Action::WholeBase, Action::Back]),
                        "back to the diff, where it was",
                    ),
                    (key(&[Action::Comment]), "comment; outside the hunks it is a note not in the diff"),
                    (key(&[Action::NextHunk, Action::PreviousHunk]), "next / previous changed lines"),
                    (key(&[Action::NextFile, Action::PreviousFile]), "next / previous file"),
                    (key(&[Action::ToggleViewed]), "mark this file viewed, or not"),
                ],
            ),
            (
                "notes",
                vec![
                    (key(&[Action::Down, Action::Up]), "next / previous (the diff follows)"),
                    (key(&[Action::Confirm]), "show it; a note not in the diff opens the whole file"),
                    (key(&[Action::Edit]), "edit"),
                    (key(&[Action::Remove]), "remove"),
                ],
            ),
        ];
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (title, rows) in sections {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(Span::styled(
                format!(" {title}"),
                Style::new().fg(palette().accent).bold(),
            )));
            for (keys, what) in rows.into_iter().filter(|(keys, _)| !keys.is_empty()) {
                lines.push(Line::from(vec![
                    Span::raw(format!("  {keys:<9} ")).bold(),
                    Span::raw(what.to_owned()),
                ]));
            }
        }
        let width = (lines.iter().map(Line::width).max().unwrap_or(0) as u16 + 3).min(area.width);
        let height = (lines.len() as u16 + 2).min(area.height);
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let close = self.keys.labelled(Action::Back, "closes").unwrap_or_default();
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(" code review keys ", Style::new().dim()))
            .title_bottom(Span::styled(format!(" {close} "), Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        frame.render_widget(Paragraph::new(lines), inner);
    }
}
