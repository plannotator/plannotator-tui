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
                    MenuRow::Undo => self.undo_label().to_owned(),
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

    /// The key help's sections: a title, then each key with what it does.
    fn help_sections(&self) -> [(&'static str, Vec<(String, &'static str)>); 5] {
        let key = |actions: &[Action]| {
            actions.iter().map(|a| self.keys.hint(*a)).filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" ")
        };
        [
            (
                "anywhere",
                vec![
                    (key(&[Action::CycleFocus]), "diff / notes"),
                    (key(&[Action::ToggleNotes]), "hide the notes, or show them again"),
                    (key(&[Action::Send]), "send new and edited notes"),
                    (key(&[Action::SendAndClose]), "send and close"),
                    (key(&[Action::ChangeComment]), "comment on the whole change"),
                    (
                        key(&[Action::ReviewMenu, Action::UndoFinish]),
                        "review menu: comment, resend, finish / undo a finish or a removal",
                    ),
                    (key(&[Action::PickTarget]), "choose the agent Send goes to"),
                    (
                        key(&[Action::PickChanges]),
                        "pick what to review: uncommitted, unstaged, staged, a commit",
                    ),
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
                    ("drag".to_owned(), "select lines with the mouse, then comment or suggest"),
                    (key(&[Action::CopyLines]), "copy the selected lines"),
                    (key(&[Action::Comment]), "comment on the selection or line"),
                    (key(&[Action::Suggest]), "suggest code for new lines"),
                    (key(&[Action::FileComment]), "comment on the file"),
                    (key(&[Action::Ask]), "ask the agent about the lines; the answer shows under them"),
                ]
                .into_iter()
                .chain(self.box_help())
                .chain([
                    (key(&[Action::WholeFile]), "the whole file as it is now, at this line"),
                    (key(&[Action::WholeBase]), "the whole file before the change (base)"),
                    (key(&[Action::GoToDefinition]), "go to the definition of a word on the line"),
                    (
                        key(&[Action::JumpBack, Action::JumpForward]),
                        "back / forward to where you jumped from",
                    ),
                    (key(&[Action::Back]), "clear the selection"),
                    (key(&[Action::ToggleLayout]), "split / unified (or click the header)"),
                    (key(&[Action::ToggleWrap]), "wrap long lines"),
                    (key(&[Action::ToggleWhitespace]), "hide whitespace-only changes, or show them (no ws)"),
                    (key(&[Action::SplitLeft, Action::SplitRight]), "move the line between the sides"),
                    ("drag the middle".to_owned(), "resize the sides; double-click to even them"),
                    (key(&[Action::DiffSettings]), "diff settings (remembered for next time)"),
                    (key(&[Action::ToggleFiles]), "the file list: open it and move there; there, close it"),
                    (key(&[Action::FilterFiles]), "filter the files by name"),
                    (
                        key(&[Action::FilterKinds]),
                        "show only some kinds: code, tests, docs, config, generated",
                    ),
                    (key(&[Action::HideViewed]), "hide viewed files, or show them again"),
                    (key(&[Action::NarrowFiles, Action::WidenFiles]), "narrow / widen the file list"),
                ])
                .collect(),
            ),
            (
                "file list",
                vec![
                    (key(&[Action::Down, Action::Up]), "next / previous row (the diff follows)"),
                    (key(&[Action::Confirm]), "go to the file; open or close a folder"),
                    (key(&[Action::Left, Action::Right]), "close / open a folder"),
                    (
                        key(&[Action::FilterFiles]),
                        "type a filter: words in the path; enter shows the matches, esc clears",
                    ),
                    (key(&[Action::FilterKinds]), "kinds: space turns one on or off, enter shows it alone"),
                    (key(&[Action::HideViewed]), "hide viewed files"),
                    (key(&[Action::ToggleViewed]), "mark the file viewed, or not"),
                    (key(&[Action::Back]), "clear the name filter, then back to the diff"),
                    ("drag its line".to_owned(), "wider / narrower; double-click it to fit the names again"),
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
                    (key(&[Action::GoToDefinition]), "go to definition; on the base it searches the base"),
                ],
            ),
            (
                "notes",
                vec![
                    (key(&[Action::Down, Action::Up]), "next / previous (the diff follows)"),
                    (key(&[Action::WidenNotes, Action::NarrowNotes]), "widen / narrow the notes"),
                    ("drag its line".to_owned(), "wider / narrower; double-click it to fit again"),
                    (key(&[Action::Confirm]), "show it; a note not in the diff opens the whole file"),
                    (key(&[Action::Edit]), "edit"),
                    (key(&[Action::Remove]), "remove"),
                ],
            ),
        ]
    }

    /// `?`: every key, in the user's keymap.
    pub(super) fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let sections = self.help_sections();
        let blocks: Vec<Vec<Line<'static>>> = sections
            .into_iter()
            .map(|(title, rows)| {
                let heading =
                    Line::from(Span::styled(format!(" {title}"), Style::new().fg(palette().accent).bold()));
                let keys = rows.into_iter().filter(|(keys, _)| !keys.is_empty()).map(|(keys, what)| {
                    Line::from(vec![Span::raw(format!("  {keys:<9} ")).bold(), Span::raw(what.to_owned())])
                });
                std::iter::once(heading).chain(keys).collect()
            })
            .collect();
        let stack = |blocks: &[Vec<Line<'static>>]| -> Vec<Line<'static>> {
            let mut lines = Vec::new();
            for block in blocks {
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.extend(block.iter().cloned());
            }
            lines
        };
        let widest = |lines: &[Line<'static>]| lines.iter().map(Line::width).max().unwrap_or(0) as u16;
        let one = stack(&blocks);
        // Too tall for one column: the diff's keys go in a second, beside the rest.
        let columns = if one.len() as u16 + 2 > area.height
            && let Some((diff, rest)) = blocks.split_at_checked(2).map(|(head, tail)| {
                let (first, diff) = head.split_at(1);
                (diff.to_vec(), [first, tail].concat())
            })
            && widest(&stack(&rest)) + widest(&stack(&diff)) + 4 <= area.width
        {
            vec![stack(&rest), stack(&diff)]
        } else {
            vec![one]
        };
        let column_widths: Vec<u16> = columns.iter().map(|c| widest(c) + 1).collect();
        let width = (column_widths.iter().sum::<u16>() + 2).min(area.width);
        let height = (columns.iter().map(Vec::len).max().unwrap_or(0) as u16 + 2).min(area.height);
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let tallest = columns.iter().map(Vec::len).max().unwrap_or(0);
        let room = tallest.saturating_sub(usize::from(height.saturating_sub(2)));
        self.help_room.set(room);
        let scroll = self.help_scroll.min(room);
        let close = self.keys.labelled(Action::Back, "closes").unwrap_or_default();
        let scrolls: Vec<String> = [Action::Down, Action::Up]
            .map(|a| self.keys.hint(a))
            .into_iter()
            .filter(|k| !k.is_empty())
            .collect();
        let close = if room > 0 && !scrolls.is_empty() {
            format!("{} scroll \u{b7} {close}", scrolls.join("/"))
        } else {
            close
        };
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(" code review keys ", Style::new().dim()))
            .title_bottom(Span::styled(format!(" {close} "), Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let mut x = inner.x;
        for (lines, column_width) in columns.into_iter().zip(column_widths) {
            let area = Rect { x, width: column_width.min(inner.right().saturating_sub(x)), ..inner };
            frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), area);
            x = x.saturating_add(column_width);
        }
    }
}
