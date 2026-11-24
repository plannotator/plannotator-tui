//! The footer: the status on the left (what happened, the file list's filter, the repo, the
//! notes) and the key hints on the right, in one row on a wide screen and two on a narrow
//! one; hints that do not fit give way, never send, help and quit.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr as _;

use super::diff::Progress;
use super::draw::{RAIL_MIN_TOTAL_WIDTH, count};
use super::{Focus, Mode, Screen};
use crate::keys::Action;

mod fit;
use crate::theme::palette;

/// From this many columns the footer's hints take one row, below it two.
const FOOTER_ONE_ROW: u16 = 120;

/// Columns a diff file heading keeps for its status and counts (` Modified   +12 −3 `).
const HEADING_COUNTS: usize = 24;

impl Screen {
    /// The quit prompt. `y` sends this review's notes only; the other reviews' unsent notes
    /// are named apart, with `esc` to go to them, and those whose review is gone are said
    /// to be kept.
    ///
    /// ` send notes to X before quitting? y send · n quit · esc cancel`, or with others:
    /// ` quit? y send these to X · 1 more unsent on Commit 2efc81b (esc go to it) · n quit`.
    pub(super) fn quit_question(&self) -> String {
        let label = |action, text| self.keys.labelled(action, text);
        let here = self.has_unsent();
        let gone = self.gone_text();
        let target = self.delivery.describe();
        if let Some(more) = self.more_unsent_text() {
            let go = label(Action::Back, "go to it").map(|go| format!(" ({go})")).unwrap_or_default();
            let parts = [
                here.then(|| label(Action::Yes, "send these").map(|yes| format!("{yes} to {target}")))
                    .flatten(),
                Some(if here {
                    format!("{more}{go}")
                } else {
                    format!("{}{go}", self.unsent_elsewhere_text().unwrap_or(more))
                }),
                gone,
                label(Action::No, "quit"),
            ];
            return format!(" quit? {}", parts.into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} "));
        }
        let answers = [(Action::Yes, "send"), (Action::No, "quit"), (Action::Back, "cancel")]
            .into_iter()
            .filter(|(action, _)| here || *action != Action::Yes)
            .filter_map(|(action, text)| label(action, text))
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        match (here, gone) {
            (true, None) => format!(" send notes to {target} before quitting? {answers}"),
            (true, Some(gone)) => format!(" send notes to {target} before quitting? {answers} \u{b7} {gone}"),
            (false, gone) => format!(" {} \u{b7} {answers}", gone.unwrap_or_default()),
        }
    }

    pub(super) fn footer_height(&self, width: u16) -> u16 {
        if self.mode == Mode::ConfirmQuit { 1 } else { self.footer_text(width).1.len().max(1) as u16 }
    }

    pub(super) fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        if self.mode == Mode::ConfirmQuit {
            let question = self.quit_question();
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
        let mut parts: Vec<String> = self.nav.notice().into_iter().chain(self.disk_notice()).collect();
        parts.extend(self.status.iter().cloned());
        if self.whole.as_ref().is_some_and(|w| w.version == super::whole::Version::Base)
            && self.status.as_deref() != Some(super::whole::BASE_READ_ONLY)
        {
            parts.push(super::whole::BASE_READ_ONLY.to_owned());
        }
        match self.diff.progress() {
            Progress::Loading => parts.push("loading the diff\u{2026}".into()),
            Progress::Coloring => parts.push("coloring\u{2026}".into()),
            Progress::SourceFailed => parts.push("source unavailable".into()),
            Progress::Failed(error) => parts.push(format!("diff error: {error}")),
            Progress::Ready => {}
        }
        // The list hidden for want of room still tells what its filter hides, early so a
        // narrow footer keeps it.
        let typing = self.mode == Mode::FilterFiles;
        if self.files.drawn_width == 0 && self.files.filter.is_active() {
            if !typing {
                parts.push(format!(
                    "filter: {} of {} files",
                    self.files.shown().len(),
                    self.capture.files.len()
                ));
            }
            match self.notes_in_hidden_files() {
                0 => {}
                1 => parts.push("1 unsent note in hidden files".into()),
                n => parts.push(format!("{n} unsent notes in hidden files")),
            }
        }
        // Typing a filter the list cannot show: the footer shows it.
        if self.files.drawn_width == 0 && typing {
            parts.insert(0, format!("filter: {}\u{258f}", self.files.filter.query));
        } else if self.files.drawn_width == 0 && self.files_wanted() {
            parts.push(match self.keys.hint(Action::ToggleFiles) {
                _ if self.files.no_room => "no room for the file list".to_owned(),
                key if key.is_empty() => "files hidden: no room".to_owned(),
                key => format!("files hidden: {key} shows them"),
            });
        }
        let notes = self.store.annotations();
        let outdated = notes.iter().filter(|n| super::notes::is_outdated(n)).count();
        if self.cursor_file_viewed() {
            parts.push("\u{2713} viewed".into());
        }
        // A narrow diff cuts its file headings short: the whole path of the file the cursor
        // is in goes here then.
        if let Some(path) = self.files.current.as_deref().filter(|_| self.whole.is_none())
            && path.width() + HEADING_COUNTS > usize::from(self.geometry.diff.width)
        {
            parts.push(path.to_owned());
        }
        parts.push(self.repo_name());
        parts.push(match outdated {
            0 => count(notes.len()),
            n => format!("{} ({n} outdated)", count(notes.len())),
        });
        if let Some(lines) = self.selected_lines() {
            let span = if lines.first == lines.last {
                lines.first.to_string()
            } else {
                format!("{}-{}", lines.first, lines.last)
            };
            parts.push(format!("selected {span} {}", lines.side.as_str()));
        }
        // Notes drawn in the diff: the ones that are not, and where they are.
        let away = self.notes_not_inline();
        if away > 0 {
            let what = if away == 1 {
                "1 note not shown inline".to_owned()
            } else {
                format!("{away} notes not shown inline")
            };
            let key = self.keys.hint(Action::ToggleNotes);
            parts.push(if key.is_empty() || self.rail_shown() {
                what
            } else {
                format!("{what} \u{b7} {key}")
            });
        }
        // Closed by the person, or away since the review opened without notes: the notes'
        // marks are in the gutter, and the footer says where the notes themselves are.
        if self.settings.now.notes == crate::config::review::Notes::Inline {
            if self.rail_shown() && width < RAIL_MIN_TOTAL_WIDTH {
                parts.push("rail hidden: widen to \u{2265}80 cols".into());
            }
        } else if self.notes_rail.closed() || (!self.rail_shown() && !notes.is_empty()) {
            parts.push(match self.keys.hint(Action::ToggleNotes) {
                key if key.is_empty() => "notes hidden".to_owned(),
                key => format!("notes hidden: {key} shows them"),
            });
        } else if self.rail_shown() && width < RAIL_MIN_TOTAL_WIDTH {
            parts.push("rail hidden: widen to \u{2265}80 cols".into());
        }
        let text = format!(" {}", parts.join(" \u{b7} "));
        let room = usize::from(width).saturating_sub(1);
        let beside = room.saturating_sub(text.width() + 1);
        // One row of hints, two on a narrow screen; the ones that do not fit give way.
        let rows = if width >= FOOTER_ONE_ROW { 1 } else { 2 };
        let hints = self.footer_hints();
        let keep = self.footer_tail_len(&hints);
        (text, fit::fit(&hints, keep, room, beside, rows))
    }

    /// `the diff changed on disk · r refreshes`, once a check saw the diff change.
    pub(super) fn disk_notice(&self) -> Option<String> {
        self.watch.changed().then(|| match self.keys.hint(Action::Reload) {
            key if key.is_empty() => "the diff changed on disk".to_owned(),
            key => format!("the diff changed on disk \u{b7} {key} refreshes"),
        })
    }

    /// How many of `hints`, from the end, are the ones that always stay: send, help, quit.
    fn footer_tail_len(&self, hints: &[String]) -> usize {
        let tail: Vec<String> = [(Action::Send, "send"), (Action::Help, "help"), (Action::Quit, "quit")]
            .into_iter()
            .filter_map(|(action, label)| self.keys.labelled(action, label))
            .collect();
        hints.iter().rev().take_while(|hint| tail.contains(hint)).count()
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
        if let (Some(whole), Focus::Diff) = (&self.whole, self.focus) {
            let current = whole.version == super::whole::Version::Current;
            let back: Vec<String> = [Action::WholeFile, Action::WholeBase, Action::Back]
                .map(|x| self.keys.hint(x))
                .into_iter()
                .filter(|k| !k.is_empty())
                .collect();
            let hints = [
                (!back.is_empty()).then(|| format!("{} back to the diff", back.join("/"))),
                current.then(|| key(Action::Visual, "select")).flatten(),
                current.then(|| key(Action::Comment, "comment")).flatten(),
                (self.on_release == super::mouse::OnRelease::Wait && self.selected_lines().is_some())
                    .then(|| key(Action::CopyLines, "copy"))
                    .flatten(),
                key(Action::GoToDefinition, "definition"),
                pair(Action::NextHunk, Action::PreviousHunk, "change"),
                pair(Action::NextFile, Action::PreviousFile, "file"),
                (!self.store.annotations().is_empty()).then(|| key(Action::CycleFocus, "notes")).flatten(),
            ];
            return hints.into_iter().chain(tail).flatten().collect();
        }
        if self.mode == Mode::FilterFiles {
            return [
                key(Action::Confirm, "show"),
                key(Action::Back, "clear"),
                Some("\u{2191}\u{2193} move".to_owned()),
            ]
            .into_iter()
            .flatten()
            .collect();
        }
        if matches!(self.mode, Mode::Kinds(_)) {
            return Vec::new();
        }
        let hints: Vec<Option<String>> = match self.focus {
            Focus::Diff if self.cursor_note().is_some() => {
                let besides = |action| self.keys.hint_besides(action, crate::keys::DIFF);
                let edit: Vec<String> = [self.keys.hint(Action::Confirm), besides(Action::Edit)]
                    .into_iter()
                    .filter(|k| !k.is_empty())
                    .collect();
                let remove = besides(Action::Remove);
                [
                    (!edit.is_empty()).then(|| format!("{} edit", edit.join("/"))),
                    (!remove.is_empty()).then(|| format!("{remove} remove")),
                    pair(Action::Down, Action::Up, "move"),
                    key(Action::ToggleViewed, "viewed"),
                    (!self.store.annotations().is_empty())
                        .then(|| key(Action::CycleFocus, "notes"))
                        .flatten(),
                ]
                .into_iter()
                .chain(tail)
                .collect()
            }
            Focus::Diff if self.diff.selection().is_some() => vec![
                self.diff.selecting().then(|| key(Action::Visual, "end selection")).flatten(),
                key(Action::Comment, "comment"),
                key(Action::Suggest, "suggest"),
                (self.on_release == super::mouse::OnRelease::Wait)
                    .then(|| key(Action::CopyLines, "copy"))
                    .flatten(),
                key(Action::Back, "clear"),
            ],
            Focus::Diff => [
                key(Action::Visual, "select"),
                key(Action::Comment, "comment"),
                key(Action::Suggest, "suggest"),
                key(Action::FileComment, "file note"),
                pair(Action::WholeFile, Action::WholeBase, "whole file"),
                key(Action::GoToDefinition, "definition"),
                pair(Action::NextHunk, Action::PreviousHunk, "hunk"),
                pair(Action::NextFile, Action::PreviousFile, "file"),
                key(Action::FilterFiles, "filter files"),
                key(Action::ToggleViewed, "viewed"),
                key(Action::Reload, "reload"),
                (!self.store.annotations().is_empty()).then(|| key(Action::CycleFocus, "notes")).flatten(),
            ]
            .into_iter()
            .chain(tail)
            .collect(),
            Focus::Files => [
                pair(Action::Down, Action::Up, "move"),
                key(Action::Confirm, "open"),
                pair(Action::Left, Action::Right, "fold"),
                key(Action::FilterFiles, "filter"),
                key(Action::FilterKinds, "kinds"),
                key(Action::HideViewed, "hide viewed"),
                key(Action::ToggleViewed, "viewed"),
                pair(Action::NarrowFiles, Action::WidenFiles, "width"),
                key(Action::ToggleFiles, "close"),
                key(Action::Back, "diff"),
            ]
            .into_iter()
            .chain(tail)
            .collect(),
            Focus::Notes => [
                pair(Action::Down, Action::Up, "move"),
                key(Action::Edit, "edit"),
                key(Action::Remove, "remove"),
                pair(Action::WidenNotes, Action::NarrowNotes, "width"),
                key(Action::ToggleNotes, "hide"),
                key(Action::CycleFocus, "diff"),
            ]
            .into_iter()
            .chain(tail)
            .collect(),
        };
        hints.into_iter().flatten().collect()
    }
}
