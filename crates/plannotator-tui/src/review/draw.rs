//! The frame: our header and footer around the diff (the viewer draws no chrome), the
//! notes rail on the right, and the comment box, menu, picker and help over them. The look
//! is the document review's: the same buttons, hints and boxes from `ui/`.

use anyhow::Result;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use super::diff::Progress;
use super::input::{MENU, MenuRow};
use super::{Draft, Focus, Geometry, Mode, Screen};
use crate::keys::Action;
use crate::theme::palette;
use crate::ui::{compose, hints, send_button};

const RAIL_WIDTH: u16 = 36;
const RAIL_MIN_WIDTH: u16 = 28;
/// Below this the rail is dropped; notes still show as gutter markers.
const RAIL_MIN_TOTAL_WIDTH: u16 = 80;
/// The comment box's widest; code suggestions want the room.
pub(super) const COMPOSE_MAX_WIDTH: u16 = 84;

impl Screen {
    pub(crate) fn draw(&mut self, frame: &mut Frame) -> Result<()> {
        let area = frame.area();
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(self.footer_height(area.width)),
        ])
        .areas(area);
        let rail_width = if area.width >= RAIL_MIN_TOTAL_WIDTH {
            (area.width * 3 / 10).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH)
        } else {
            0
        };
        let [diff, _gap, rail] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(u16::from(rail_width > 0)),
            Constraint::Length(rail_width),
        ])
        .areas(body);
        self.geometry = Geometry { diff, rail, ..Geometry::default() };
        self.draw_header(frame, header);
        self.diff.draw(frame, diff)?;
        if rail_width > 0 {
            self.draw_rail(frame, rail);
        }
        self.draw_footer(frame, footer);
        match &self.mode {
            Mode::Compose(draft) => {
                let draft = draft.clone();
                self.draw_compose(frame, diff, &draft);
            }
            Mode::Menu(cursor) => self.draw_menu(frame, *cursor),
            Mode::PickTarget => {
                let root = crate::ui::pi_picker::real(self.root());
                self.picker.draw(frame, area, &self.keys, &root, self.start_delivery.as_deref());
            }
            Mode::Help => self.draw_help(frame, area),
            Mode::Browse | Mode::ConfirmQuit => {}
        }
        Ok(())
    }

    /// `Uncommitted · 4 files +120 −30` on the left; on the right the Review menu and the
    /// Send button, on the edge.
    fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let (added, removed) = self.capture.totals();
        let files = self.capture.files.len();
        let title = format!(
            " {} \u{b7} {files} file{} +{added} \u{2212}{removed} ",
            self.capture.scope.title(),
            if files == 1 { "" } else { "s" }
        );
        let send = format!(" {} ", self.send_label());
        let review = match self.keys.hint(Action::ReviewMenu) {
            key if key.is_empty() => " Review \u{25be} ".to_owned(),
            key => format!(" Review \u{25be} ({key}) "),
        };
        let buf = frame.buffer_mut();
        buf.set_span(
            area.x,
            area.y,
            &Span::styled(title.as_str(), Style::new().fg(palette().accent).bold()),
            area.width,
        );
        let send_width = (send.width() as u16).min(area.width);
        let send_rect = Rect { x: area.right() - send_width, y: area.y, width: send_width, height: 1 };
        let style = send_button::style(&self.send_state, self.store.pending().len());
        buf.set_span(send_rect.x, send_rect.y, &Span::styled(send, style), send_rect.width);
        self.geometry.send_button = Some(send_rect);
        let review_width = review.width() as u16;
        let room = area.width.saturating_sub(send_width + 1 + title.width() as u16);
        if review_width <= room {
            let rect = Rect { x: send_rect.x - 1 - review_width, y: area.y, width: review_width, height: 1 };
            let style = Style::new().fg(palette().accent).bg(palette().toolbar_bg);
            buf.set_span(rect.x, rect.y, &Span::styled(review, style), rect.width);
            self.geometry.review_button = Some(rect);
        }
    }

    fn footer_height(&self, width: u16) -> u16 {
        if self.mode == Mode::ConfirmQuit { 1 } else { self.footer_text(width).1.len().max(1) as u16 }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        if self.mode == Mode::ConfirmQuit {
            let answers = [(Action::Yes, "send"), (Action::No, "quit"), (Action::Back, "cancel")]
                .into_iter()
                .filter_map(|(action, label)| self.keys.labelled(action, label))
                .collect::<Vec<_>>()
                .join(" \u{b7} ");
            let question = format!(" send notes to {} before quitting? {answers}", self.delivery.describe());
            frame.render_widget(Paragraph::new(Line::from(Span::raw(question).bold())), area);
            return;
        }
        let (text, rows) = self.footer_text(area.width);
        for (index, (hints, y)) in rows.iter().rev().zip((area.y..area.bottom()).rev()).enumerate() {
            let row = Rect { y, height: 1, ..area };
            let hints = if hints.is_empty() { String::new() } else { format!("{hints} ") };
            let help_area = if index == 0 {
                let [text_area, help_area] =
                    Layout::horizontal([Constraint::Min(0), Constraint::Length(hints.width() as u16)])
                        .areas(row);
                frame.render_widget(Paragraph::new(Line::from(Span::raw(text.as_str()).dim())), text_area);
                help_area
            } else {
                row
            };
            frame
                .render_widget(Paragraph::new(Line::from(Span::raw(hints).dim()).right_aligned()), help_area);
        }
    }

    /// The footer's left text (status first, as in document reviews) and its hint rows.
    fn footer_text(&self, width: u16) -> (String, Vec<String>) {
        let mut parts: Vec<String> = self.status.iter().cloned().collect();
        match self.diff.progress() {
            Progress::Loading => parts.push("loading the diff\u{2026}".into()),
            Progress::Coloring => parts.push("coloring\u{2026}".into()),
            Progress::SourceFailed => parts.push("source unavailable".into()),
            Progress::Failed(error) => parts.push(format!("diff error: {error}")),
            Progress::Ready => {}
        }
        let notes = self.store.annotations();
        let outdated = notes.iter().filter(|n| super::notes::is_outdated(n)).count();
        parts.push(self.repo_name());
        parts.push(match outdated {
            0 => format!("{} notes", notes.len()),
            n => format!("{} notes ({n} outdated)", notes.len()),
        });
        if let Some(lines) = self.diff.selection() {
            let span = if lines.first == lines.last {
                lines.first.to_string()
            } else {
                format!("{}-{}", lines.first, lines.last)
            };
            parts.push(format!("selected {span} {}", lines.side.as_str()));
        }
        if width < RAIL_MIN_TOTAL_WIDTH {
            parts.push("rail hidden: widen to \u{2265}80 cols".into());
        }
        let text = format!(" {}", parts.join(" \u{b7} "));
        let room = usize::from(width).saturating_sub(1);
        let beside = room.saturating_sub(text.width() + 1);
        (text, hints::flow(&self.footer_hints(), room, beside))
    }

    fn footer_hints(&self) -> Vec<String> {
        if matches!(self.mode, Mode::Compose(_)) {
            return Vec::new();
        }
        let key = |action, label| self.keys.labelled(action, label);
        let pair = |a: Action, b: Action, label: &str| {
            let keys: Vec<String> =
                [a, b].map(|x| self.keys.hint(x)).into_iter().filter(|k| !k.is_empty()).collect();
            (!keys.is_empty()).then(|| format!("{} {label}", keys.join("/")))
        };
        let tail = [key(Action::Send, "send"), key(Action::Help, "help"), key(Action::Quit, "quit")];
        let hints: Vec<Option<String>> = match self.focus {
            Focus::Diff if self.diff.selection().is_some() => vec![
                self.diff.selecting().then(|| key(Action::Visual, "end selection")).flatten(),
                key(Action::Comment, "comment"),
                key(Action::Suggest, "suggest"),
                key(Action::Back, "clear"),
            ],
            Focus::Diff => [
                key(Action::Visual, "select"),
                key(Action::Comment, "comment"),
                key(Action::Suggest, "suggest"),
                key(Action::FileComment, "file note"),
                pair(Action::NextHunk, Action::PreviousHunk, "hunk"),
                pair(Action::NextFile, Action::PreviousFile, "file"),
                key(Action::Reload, "reload"),
                (!self.store.annotations().is_empty()).then(|| key(Action::CycleFocus, "notes")).flatten(),
            ]
            .into_iter()
            .chain(tail)
            .collect(),
            Focus::Notes => [
                pair(Action::Down, Action::Up, "move"),
                key(Action::Edit, "edit"),
                key(Action::Remove, "remove"),
                key(Action::CycleFocus, "diff"),
            ]
            .into_iter()
            .chain(tail)
            .collect(),
        };
        hints.into_iter().flatten().collect()
    }

    /// The comment box, along the bottom of the diff, as wide as code needs.
    fn draw_compose(&mut self, frame: &mut Frame, diff: Rect, draft: &Draft) {
        let width = diff.width.min(COMPOSE_MAX_WIDTH);
        let wrap_width = usize::from(width.saturating_sub(compose::CHROME));
        self.geometry.compose_wrap = wrap_width;
        let height = compose::height(&self.compose, wrap_width).min(diff.height);
        let rect = Rect {
            x: diff.x + (diff.width - width) / 2,
            y: diff.bottom().saturating_sub(height),
            width,
            height,
        };
        let verb = match draft {
            Draft::Comment(lines) => {
                format!("comment on {}", lines_label(&lines.path, lines.first, lines.last))
            }
            Draft::Suggestion(lines) => {
                format!("suggest code for {}", lines_label(&lines.path, lines.first, lines.last))
            }
            Draft::File(path) => format!("note on {path}"),
            Draft::Edit(_) => "edit".to_owned(),
        };
        let title = compose::title(&verb, self.shift_enter);
        compose::draw(frame, rect, &self.compose, &title, Style::new().dim());
    }

    fn draw_menu(&mut self, frame: &mut Frame, cursor: usize) {
        let area = frame.area();
        let sent = self.store.annotations().iter().filter(|n| !self.store.is_pending(n)).count();
        let rows: Vec<String> = MENU
            .iter()
            .map(|row| {
                let label = match row {
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
    fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let key = |actions: &[Action]| {
            actions.iter().map(|a| self.keys.hint(*a)).filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" ")
        };
        let sections: [(&str, Vec<(String, &str)>); 3] = [
            (
                "anywhere",
                vec![
                    (key(&[Action::CycleFocus]), "diff / notes"),
                    (key(&[Action::Send]), "send new and edited notes"),
                    (key(&[Action::SendAndClose]), "send and close"),
                    (key(&[Action::ReviewMenu]), "review menu: resend all, finish, undo"),
                    (key(&[Action::PickTarget]), "send to a Pi session"),
                    (key(&[Action::Reload]), "reload the diff; notes follow their lines"),
                    (key(&[Action::Quit]), "quit"),
                ],
            ),
            (
                "diff",
                vec![
                    (key(&[Action::Down, Action::Up]), "line"),
                    (key(&[Action::NextHunk, Action::PreviousHunk]), "hunk"),
                    (key(&[Action::NextFile, Action::PreviousFile]), "file"),
                    (key(&[Action::Top, Action::Bottom]), "first / last"),
                    (key(&[Action::Left, Action::Right]), "old / new side"),
                    (key(&[Action::Visual]), "select lines (again to finish); shift-click too"),
                    (key(&[Action::Comment]), "comment on the selection or line"),
                    (key(&[Action::Suggest]), "suggest code for new lines"),
                    (key(&[Action::FileComment]), "comment on the file"),
                    (key(&[Action::Back]), "clear the selection"),
                    (key(&[Action::ToggleLayout]), "split / unified"),
                    (key(&[Action::ToggleWrap]), "wrap long lines"),
                    (key(&[Action::ToggleFiles]), "file list"),
                ],
            ),
            (
                "notes",
                vec![
                    (key(&[Action::Down, Action::Up]), "next / previous (the diff follows)"),
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

/// `src/a.rs:12` or `src/a.rs:12-14`.
fn lines_label(path: &str, first: u32, last: u32) -> String {
    if first == last { format!("{path}:{first}") } else { format!("{path}:{first}-{last}") }
}
