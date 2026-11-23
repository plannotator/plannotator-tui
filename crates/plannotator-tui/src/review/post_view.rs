//! The summary before a post to GitHub, drawn over the review.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use super::Screen;
use super::post::{Confirm, Head, short, summary};
use crate::keys::Action;
use crate::theme::palette;
use crate::wrap::wrap_line;
use plannotator_tui_review::post::Event;

/// The summary's widest.
const WIDTH: u16 = 76;

impl Screen {
    pub(super) fn draw_post(&self, frame: &mut Frame, area: Rect, confirm: &Confirm) {
        let Some(pull) = &self.capture.pull else { return };
        let accent = Style::new().fg(palette().accent).bold();
        let warn = Style::new().fg(palette().comment).bold();
        let mut lines: Vec<Line<'static>> = vec![
            Line::from(vec![
                Span::styled(format!("{}#{}", pull.view.repo.slug(), pull.view.number), accent),
                Span::raw(format!(" \u{b7} {}", pull.view.title)),
            ]),
            Line::from(Span::styled(
                format!("anchored to {}, the commit you reviewed", short(&confirm.plan.commit_id)),
                Style::new().dim(),
            )),
            Line::default(),
        ];
        lines.extend(summary(&confirm.plan).into_iter().map(Line::from));
        match &confirm.head {
            Head::Reviewed => {}
            Head::Moved { now } => {
                let reload = self.keys.labelled(Action::Reload, "reloads it first").unwrap_or_default();
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    format!(
                        "The PR moved on: its head is {} now. Comments stay on {}, the lines you saw; \
                         GitHub shows them as outdated where the code changed since. {reload}",
                        short(now),
                        short(&confirm.plan.commit_id)
                    ),
                    warn,
                )));
            }
            Head::Unknown(why) => {
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(format!("Could not check the PR's head: {why}"), warn)));
            }
        }
        lines.push(Line::default());
        let mut choices = vec![Span::raw("verdict  ")];
        for event in Event::ALL {
            let chosen = event == confirm.event;
            let mark = if chosen { "(\u{2022})" } else { "( )" };
            let style = if chosen { Style::new().bold() } else { Style::new().dim() };
            choices.push(Span::styled(format!("{mark} {}   ", event.label()), style));
        }
        lines.push(Line::from(choices));
        if let Some(failed) = &confirm.failed {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(failed.clone(), Style::new().fg(palette().delete).bold())));
        }
        let verb = match confirm.event {
            Event::Comment => "posts",
            Event::Approve => "posts and approves",
            Event::RequestChanges => "posts and requests changes",
        };
        let moves: Vec<String> = [Action::Left, Action::Right]
            .map(|a| self.keys.hint(a))
            .into_iter()
            .filter(|k| !k.is_empty())
            .collect();
        let help: Vec<String> = [
            self.keys.labelled(Action::Confirm, verb),
            (!moves.is_empty()).then(|| format!("{} verdict", moves.join("/"))),
            self.keys.labelled(Action::Back, "cancels"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let width = area.width.min(WIDTH);
        let inner_width = usize::from(width.saturating_sub(4)).max(1);
        let rows: Vec<Line<'static>> = lines
            .iter()
            .flat_map(|line| {
                let wrapped = wrap_line(line, &[], inner_width);
                if wrapped.is_empty() {
                    vec![Line::default()]
                } else {
                    wrapped.into_iter().map(|r| r.line).collect()
                }
            })
            .collect();
        let height = (rows.len() as u16 + 2).min(area.height);
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
            .title(Span::styled(" post to GitHub ", Style::new().dim()))
            .title_bottom(Span::styled(format!(" {} ", help.join(" \u{b7} ")), Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        let text_area = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
        frame.render_widget(Paragraph::new(rows), text_area);
    }
}
