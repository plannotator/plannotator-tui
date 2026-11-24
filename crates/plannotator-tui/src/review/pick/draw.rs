//! Drawing the picker: the whole screen when it opens on its own, a box over the review
//! otherwise. Each row is its title, then its size, age, author and badges in columns that
//! line up down the list; a narrow terminal drops the author first, then the age, then the
//! file count, then the badges, and the title is cut with `…` last.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use super::rows::{Kind, Row, Stat, age, file_count};
use super::{More, Picker};
use crate::keys::Action;
use crate::theme::palette;
use crate::ui::hints;

/// The widest the box over a review gets.
const BOX_MAX_WIDTH: u16 = 110;
/// The fewest cells a title keeps before the columns beside it give way: a short sha and
/// about sixteen cells of its subject.
const TITLE_MIN: usize = 24;
/// The widest an author's name is shown.
const AUTHOR_MAX: usize = 16;
const GAP: usize = 2;

/// Which of a row's columns are drawn, and how wide each is.
#[derive(Debug, Clone, Copy, Default)]
struct Columns {
    /// The title's cells: the widest title, or what is left beside the other columns.
    title: usize,
    stat: usize,
    /// The widest `+12 −3`, and `4 files` after it; zero when the file count is left out.
    counts: usize,
    files: usize,
    age: usize,
    author: usize,
    badge: usize,
}

impl Columns {
    fn width(self) -> usize {
        [self.stat, self.age, self.author, self.badge].iter().filter(|w| **w > 0).map(|w| w + GAP).sum()
    }
}

impl Picker {
    pub(crate) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if self.over.is_some() {
            self.draw_box(frame, area);
        } else {
            self.draw_screen(frame, area);
        }
        if self.help {
            self.draw_help(frame, self.geometry.area);
        }
    }

    /// The whole screen: a title row, the rows, the footer.
    fn draw_screen(&mut self, frame: &mut Frame, area: Rect) {
        let hints = hints::flow(
            &self.hint_list(),
            usize::from(area.width.saturating_sub(2)),
            usize::from(area.width.saturating_sub(2)),
        );
        let footer = self.footer_text();
        let footer_rows =
            u16::from(footer.is_some()).saturating_add(u16::try_from(hints.len()).unwrap_or(u16::MAX));
        let [title, list, foot] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(footer_rows)])
                .areas(area);
        self.geometry.area = area;
        let heading = Line::from(vec![
            Span::styled(" Pick what to review", Style::new().fg(palette().accent).bold()),
            Span::raw(format!(" \u{b7} {}", self.name)).dim(),
        ]);
        frame.render_widget(Paragraph::new(heading), title);
        self.draw_rows(frame, list);
        let room = usize::from(area.width.saturating_sub(1));
        let mut lines: Vec<Line<'static>> =
            footer.into_iter().map(|text| Line::from(format!(" {}", fit(&text, room)))).collect();
        lines.extend(
            hints.into_iter().map(|row| Line::from(Span::raw(format!("{row} ")).dim()).right_aligned()),
        );
        frame.render_widget(Paragraph::new(lines), foot);
    }

    /// The box over a review, its hints in the bottom border and the footer's text inside.
    fn draw_box(&mut self, frame: &mut Frame, area: Rect) {
        let width = BOX_MAX_WIDTH.min(area.width.saturating_sub(4)).max(area.width.min(40));
        let rows = u16::try_from(self.rows.len()).unwrap_or(u16::MAX).saturating_add(u16::from(!self.listed));
        let height = rows.saturating_add(3).min(area.height.saturating_sub(2)).max(area.height.min(5));
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };
        self.geometry.area = rect;
        frame.render_widget(Clear, rect);
        let help = hints::picker("move", self.hint_list().into_iter().map(Some));
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(format!(" pick what to review \u{b7} {} ", self.name), Style::new().dim()))
            .title_bottom(Span::styled(fit(&help, usize::from(width.saturating_sub(2))), Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let [list, foot] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
        self.draw_rows(frame, list);
        if let Some(text) = self.footer_text() {
            let room = usize::from(foot.width.saturating_sub(1));
            frame.render_widget(Paragraph::new(format!(" {}", fit(&text, room))), foot);
        }
    }

    /// The hints the footer shows: what the keys do here, in the person's keymap.
    fn hint_list(&self) -> Vec<String> {
        let back = if self.over.is_some() { self.keys.labelled(Action::Back, "back") } else { None };
        [
            self.keys.labelled(Action::Confirm, "open"),
            self.keys.labelled(Action::Reload, "reload"),
            back,
            self.keys.labelled(Action::Help, "keys"),
            self.keys.labelled(Action::Quit, "quit"),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// What happened, else what the picker knows about the repository as a whole: a clean
    /// tree points at the commits, and only a repository with none says there is nothing.
    fn footer_text(&self) -> Option<String> {
        self.status.clone().or_else(|| {
            self.nothing_to_review().then(|| {
                if self.has_commits() {
                    "No uncommitted changes \u{b7} pick a commit below".to_owned()
                } else {
                    format!("Nothing to review in {}", self.name)
                }
            })
        })
    }

    fn draw_rows(&mut self, frame: &mut Frame, list: Rect) {
        let visible = usize::from(list.height).max(1);
        self.geometry.visible = visible;
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + visible {
            self.scroll = self.cursor + 1 - visible;
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(visible));
        let now = now();
        let head = self.rows.iter().find_map(|row| match &row.kind {
            Kind::Commit(commit) => Some(commit.time),
            _ => None,
        });
        let width = usize::from(list.width);
        let columns = self.columns(width, now, head);
        let mut lines = Vec::new();
        self.geometry.rows.clear();
        for (index, row) in self.rows.iter().enumerate().skip(self.scroll).take(visible) {
            let y = list.y.saturating_add(u16::try_from(index - self.scroll).unwrap_or(u16::MAX));
            self.geometry.rows.push((Rect { x: list.x, y, width: list.width, height: 1 }, index));
            lines.push(self.row_line(row, index, columns, width, now, head));
        }
        if !self.listed && lines.len() < visible {
            lines.push(Line::from(Span::raw("  listing the commits\u{2026}").dim()));
        }
        frame.render_widget(Paragraph::new(lines), list);
    }

    /// The richest columns that leave the title room.
    fn columns(&self, width: usize, now: i64, head: Option<i64>) -> Columns {
        let widest = |cell: &dyn Fn(&Row) -> usize| self.rows.iter().map(cell).max().unwrap_or(0);
        let counts = widest(&|row| match row.stat() {
            Some(stat @ Stat::Lines { .. }) => stat.text(false).width(),
            _ => 0,
        });
        let files = widest(&|row| match row.stat() {
            Some(Stat::Lines { files, .. } | Stat::Files { files }) => file_count(files).width(),
            _ => 0,
        });
        let other = widest(&|row| match row.stat() {
            Some(Stat::Lines { .. } | Stat::Files { .. }) | None => 0,
            Some(stat) => stat.text(false).width(),
        });
        let lines = if files > 0 { counts + " \u{b7} ".width() + files } else { 0 };
        let full = Columns {
            title: 0,
            stat: lines.max(other),
            counts,
            files,
            age: widest(&|row| row.time(head).map_or(0, |t| age(t, now).width())),
            author: widest(&|row| row.author().map_or(0, |a| a.width().min(AUTHOR_MAX))),
            badge: widest(&|row| self.badge(row).width()),
        };
        let narrow_stat = counts.max(other).max(widest(&|row| match row.stat() {
            Some(stat @ Stat::Files { .. }) => stat.text(false).width(),
            _ => 0,
        }));
        let title_room = |c: Columns| width.saturating_sub(2 + c.width());
        let steps = [
            full,
            Columns { author: 0, ..full },
            Columns { author: 0, age: 0, ..full },
            Columns { author: 0, age: 0, files: 0, stat: narrow_stat, ..full },
            Columns { author: 0, age: 0, files: 0, stat: narrow_stat, badge: 0, ..full },
        ];
        let chosen = steps.into_iter().find(|c| title_room(*c) >= TITLE_MIN).unwrap_or(steps[4]);
        let titles = widest(&|row| match row.title() {
            (Some(sha), subject) => sha.width() + 1 + subject.width(),
            (None, title) => title.width(),
        });
        Columns { title: titles.min(title_room(chosen)), ..chosen }
    }

    /// `open` on the row the review shows, `2 unsent` where notes wait to be sent.
    fn badge(&self, row: &Row) -> String {
        let open = self.over.is_some() && row.scope() == self.over;
        let unsent = match row.unsent() {
            0 => None,
            n => Some(format!("{n} unsent")),
        };
        [open.then(|| "open".to_owned()), unsent].into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ")
    }

    fn row_line(
        &self,
        row: &Row,
        index: usize,
        c: Columns,
        width: usize,
        now: i64,
        head: Option<i64>,
    ) -> Line<'static> {
        let selected = index == self.cursor;
        let empty = row.is_empty();
        let p = palette();
        let quiet = |style: Style| if empty && !selected { style.dim() } else { style };
        let mut spans: Vec<(String, Style)> =
            vec![(if selected { "\u{25b8} " } else { "  " }.to_owned(), Style::new())];
        let title_width = c.title;
        let (sha, title) = row.title();
        let title_style = match row.kind {
            Kind::More if self.more == More::Loading => Style::new().dim(),
            Kind::More => Style::new().fg(p.accent),
            _ => Style::new(),
        };
        let title = if row.kind == Kind::More && self.more == More::Loading {
            "loading\u{2026}".to_owned()
        } else {
            title
        };
        if let Some(sha) = sha {
            let sha = fit(&sha, title_width);
            let rest = title_width.saturating_sub(sha.width() + 1);
            let subject = fit(&title, rest);
            let pad = rest.saturating_sub(subject.width());
            spans.push((sha, quiet(Style::new().fg(p.accent))));
            spans.push((format!(" {subject}{}", " ".repeat(pad)), quiet(title_style)));
        } else {
            let title = fit(&title, title_width);
            let pad = title_width.saturating_sub(title.width());
            spans.push((format!("{title}{}", " ".repeat(pad)), quiet(title_style)));
        }
        let gap = " ".repeat(GAP);
        if c.stat > 0 {
            spans.push((gap.clone(), Style::new()));
            match row.stat() {
                Some(stat @ Stat::Lines { added, removed, files }) => {
                    // `+12 −3` right-aligned under each other, the file counts after them.
                    let counts = stat.text(false).width();
                    let tail = if c.files > 0 { " \u{b7} ".width() + c.files } else { 0 };
                    spans
                        .push((" ".repeat(c.stat.saturating_sub(c.counts.max(counts) + tail)), Style::new()));
                    spans.push((" ".repeat(c.counts.saturating_sub(counts)), Style::new()));
                    let count =
                        |n: u64, color| if n == 0 { Style::new().dim() } else { Style::new().fg(color) };
                    spans.push((format!("+{added}"), count(added, p.looks_good)));
                    spans.push((format!(" \u{2212}{removed}"), count(removed, p.delete)));
                    if c.files > 0 {
                        spans.push((
                            format!(" \u{b7} {:<w$}", file_count(files), w = c.files),
                            Style::new().dim(),
                        ));
                    }
                }
                // The file count in its column, where the counts would be blank.
                Some(Stat::Files { files }) if c.files > 0 => {
                    let tail = "   ".width() + c.files;
                    spans.push((" ".repeat(c.stat.saturating_sub(tail)), Style::new()));
                    spans.push((
                        format!("   {:<w$}", file_count(files), w = c.files),
                        quiet(Style::new().dim()),
                    ));
                }
                Some(stat) => {
                    spans.push((format!("{:>w$}", stat.text(false), w = c.stat), Style::new().dim()));
                }
                None => spans.push((" ".repeat(c.stat), Style::new())),
            }
        }
        if c.age > 0 {
            let text = row.time(head).map(|t| age(t, now)).unwrap_or_default();
            spans.push((format!("{gap}{text:>w$}", w = c.age), quiet(Style::new().dim())));
        }
        if c.author > 0 {
            let text = fit(&row.author().unwrap_or_default(), c.author);
            let pad = c.author.saturating_sub(text.width());
            spans.push((format!("{gap}{text}{}", " ".repeat(pad)), quiet(Style::new().dim())));
        }
        if c.badge > 0 {
            let text = self.badge(row);
            let pad = c.badge.saturating_sub(text.width());
            spans.push((format!("{gap}{text}{}", " ".repeat(pad)), Style::new().fg(p.comment)));
        }
        if selected {
            // One style for the whole row: the selection's own colours, nothing over them.
            let text: String = spans.into_iter().map(|(text, _)| text).collect();
            let pad = width.saturating_sub(text.width());
            return Line::from(Span::styled(format!("{text}{}", " ".repeat(pad)), p.selection));
        }
        Line::from(spans.into_iter().map(|(text, style)| Span::styled(text, style)).collect::<Vec<_>>())
    }

    /// `?`: the picker's keys.
    fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let key = |actions: &[Action]| {
            actions.iter().map(|a| self.keys.hint(*a)).filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" ")
        };
        let rows = [
            (key(&[Action::Down, Action::Up]), "next / previous row"),
            (key(&[Action::Top, Action::Bottom]), "first / last row"),
            (key(&[Action::PageDown, Action::PageUp]), "a page down / up"),
            (key(&[Action::Confirm]), "review it; on more\u{2026}, list older commits"),
            ("click".to_owned(), "review the row; the wheel scrolls"),
            (key(&[Action::Reload]), "list the commits and count the changes again"),
            // On the startup picker there is no review to go back to.
            (
                if self.over.is_some() { key(&[Action::Back, Action::PickChanges]) } else { String::new() },
                "back to the review",
            ),
            (key(&[Action::Quit]), "quit"),
        ];
        let rows: Vec<_> = rows.into_iter().filter(|(keys, _)| !keys.is_empty()).collect();
        // The key column as wide as its widest entry (`ctrl+d ctrl+u`).
        let column = rows.iter().map(|(keys, _)| keys.width()).max().unwrap_or(0);
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .map(|(keys, what)| {
                let pad = " ".repeat(column.saturating_sub(keys.width()));
                Line::from(vec![Span::raw(format!(" {keys}{pad}  ")).bold(), Span::raw(what.to_owned())])
            })
            .collect();
        let widest = u16::try_from(lines.iter().map(Line::width).max().unwrap_or(0)).unwrap_or(u16::MAX);
        let width = widest.saturating_add(3).min(area.width);
        let height = u16::try_from(lines.len()).unwrap_or(u16::MAX).saturating_add(2).min(area.height);
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(" picker keys ", Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// The present, in seconds since the epoch.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `text` cut to `width` cells, ending in `…` when cut.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('\u{2026}');
    }
    out
}
