//! The frame: our header and footer around the diff (the viewer draws no chrome), the
//! notes rail on the right (`rail`), and the comment box, menu, picker and help over them
//! (`overlays`). The look
//! is the document review's: the same buttons, hints and boxes from `ui/`.

use anyhow::Result;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr as _;

use plannotator_tui_review::Side;

use super::diff::{Lines, Progress};
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

/// The most characters of a PR's title the header shows.
const PR_TITLE_MAX: usize = 48;

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
                let root = self.real_root();
                self.picker.draw(frame, area, &self.keys, &root, self.start_delivery.as_deref());
            }
            Mode::Help => self.draw_help(frame, area),
            Mode::Browse | Mode::ConfirmQuit => {}
        }
        Ok(())
    }

    /// The header's name for what is reviewed: `Uncommitted`, `Commit 1a2b3c4`, or for a pull
    /// request `PR #123 · title · base ← head`, its title cut to [`PR_TITLE_MAX`] characters.
    pub(super) fn scope_title(&self) -> String {
        let Some(pull) = &self.capture.pull else { return self.capture.scope.title() };
        let view = &pull.view;
        let mut title: String = view.title.chars().take(PR_TITLE_MAX).collect();
        if view.title.chars().count() > PR_TITLE_MAX {
            title.push('\u{2026}');
        }
        let title = if title.is_empty() { String::new() } else { format!(" \u{b7} {title}") };
        format!("PR #{}{title} \u{b7} {} \u{2190} {}", view.number, view.base_ref, view.head_ref)
    }

    /// `Uncommitted · 4 files +120 −30` on the left; on the right the Review menu and the
    /// Send button, on the edge.
    fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let (added, removed) = self.capture.totals();
        let files = self.capture.files.len();
        let title = format!(
            " {} \u{b7} {files} file{} +{added} \u{2212}{removed} \u{b7} {} ",
            self.scope_title(),
            if files == 1 { "" } else { "s" },
            self.viewed_label()
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
                // The disk notice stands out from the rest of the dim text.
                let notice = self.disk_notice().map(|n| format!(" {n}"));
                let line = match notice.as_deref().and_then(|n| Some((n, text.strip_prefix(n)?))) {
                    Some((notice, rest)) => Line::from(vec![
                        Span::styled(notice.to_owned(), Style::new().fg(palette().accent).bold()),
                        Span::raw(rest).dim(),
                    ]),
                    None => Line::from(Span::raw(text.as_str()).dim()),
                };
                frame.render_widget(Paragraph::new(line), text_area);
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
        let mut parts: Vec<String> = self.disk_notice().into_iter().collect();
        parts.extend(self.status.iter().cloned());
        match self.diff.progress() {
            Progress::Loading => parts.push("loading the diff\u{2026}".into()),
            Progress::Coloring => parts.push("coloring\u{2026}".into()),
            Progress::SourceFailed => parts.push("source unavailable".into()),
            Progress::Failed(error) => parts.push(format!("diff error: {error}")),
            Progress::Ready => {}
        }
        let notes = self.store.annotations();
        let outdated = notes.iter().filter(|n| super::notes::is_outdated(n)).count();
        if self.cursor_file_viewed() {
            parts.push("\u{2713} viewed".into());
        }
        parts.push(self.repo_name());
        parts.push(match outdated {
            0 => count(notes.len()),
            n => format!("{} ({n} outdated)", count(notes.len())),
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

    /// `the diff changed on disk · r refreshes`, once a check saw the diff change.
    fn disk_notice(&self) -> Option<String> {
        self.watch.changed().then(|| match self.keys.hint(Action::Reload) {
            key if key.is_empty() => "the diff changed on disk".to_owned(),
            key => format!("the diff changed on disk \u{b7} {key} refreshes"),
        })
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
                key(Action::ToggleViewed, "viewed"),
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
                format!("comment on {}", lines_label(lines))
            }
            Draft::Suggestion(lines) => {
                format!("suggest code for {}", lines_label(lines))
            }
            Draft::File(path) => format!("note on {path}"),
            Draft::Change => "comment on the whole change".to_owned(),
            Draft::Edit(_) => "edit".to_owned(),
        };
        let title = self.compose.heading(&verb, self.shift_enter, &self.keys);
        compose::draw(frame, rect, &self.compose, &title);
    }
}

/// `src/a.rs:12`, `src/a.rs:12-14`, or `src/a.rs:7 old` for lines of the old file.
fn lines_label(lines: &Lines) -> String {
    let Lines { path, side, first, last } = lines;
    let at = if first == last { format!("{path}:{first}") } else { format!("{path}:{first}-{last}") };
    if *side == Side::Old { format!("{at} old") } else { at }
}

/// `1 note`, `3 notes`.
fn count(notes: usize) -> String {
    if notes == 1 { "1 note".to_owned() } else { format!("{notes} notes") }
}
