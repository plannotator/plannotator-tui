//! The frame: our header and footer (`footer`) around the diff (the viewer draws no chrome),
//! the file list on the left (`files`), the notes rail on the right (`rail`), and the
//! comment box, menu, picker and help over them (`overlays`). The look
//! is the document review's: the same buttons, hints and boxes from `ui/`.

use anyhow::Result;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr as _;

use plannotator_tui_review::Side;

use super::diff::Lines;
use super::files::fit;
use super::{Draft, Focus, Geometry, Mode, Screen};
use crate::keys::Action;
use crate::theme::palette;
use crate::ui::{compose, send_button};

const RAIL_WIDTH: u16 = 36;
const RAIL_MIN_WIDTH: u16 = 28;
/// Below this the rail is dropped; notes still show as gutter markers.
pub(super) const RAIL_MIN_TOTAL_WIDTH: u16 = 80;
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
        // A hidden rail gives its columns to the diff, which the viewer reflows as on a resize.
        let mut rail_width = if self.rail_shown() && area.width >= RAIL_MIN_TOTAL_WIDTH {
            (area.width * 3 / 10).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH)
        } else {
            0
        };
        // The file list on the left, as wide as its names need within its share; without
        // room it hides, unless it has the focus, when the notes rail makes way for it.
        // A list that does not show builds no rows.
        let focused = self.focus == Focus::Files;
        let wanted = self.files_wanted();
        let content = if wanted { self.shown_width() } else { 0 };
        let fits = |room: u16, focused| fit::width(content, room, self.files.chosen_width(), focused);
        let mut files_width = None;
        if wanted {
            files_width = fits(body.width.saturating_sub(rail_width + u16::from(rail_width > 0)), focused);
            if files_width.is_none() && focused && rail_width > 0 {
                rail_width = 0;
                files_width = fits(body.width, focused);
            }
        }
        let [main, _gap, rail] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(u16::from(rail_width > 0)),
            Constraint::Length(rail_width),
        ])
        .areas(body);
        // Not even the whole width fits the list at its narrowest: `f` and the footer say so.
        self.files.no_room = wanted && fits(body.width, true).is_none();
        if files_width.is_none() && focused {
            self.focus = Focus::Diff;
        }
        self.files.drawn(files_width.unwrap_or(0));
        let (files, diff) = match files_width {
            Some(width) => {
                let [files, _divider, diff] = Layout::horizontal([
                    Constraint::Length(width),
                    Constraint::Length(1),
                    Constraint::Min(1),
                ])
                .areas(main);
                (files, diff)
            }
            None => (Rect::default(), main),
        };
        self.geometry = Geometry { diff, rail, files, ..Geometry::default() };
        self.draw_header(frame, header);
        match &mut self.whole {
            Some(whole) => whole.draw(frame, diff)?,
            None => self.diff.draw(frame, diff)?,
        }
        if files.width > 0 {
            let listing = self.listing();
            self.draw_files(frame, files, &listing);
        }
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
            Mode::Pick(pick) => {
                let pick = pick.clone();
                self.draw_pick(frame, diff, &pick);
            }
            Mode::Kinds(cursor) => self.draw_kinds(frame, *cursor),
            Mode::Browse | Mode::ConfirmQuit | Mode::FilterFiles => {}
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
        let files = self.capture.files.len();
        // While a filter hides files, the counts are of the files the diff shows.
        let (added, removed) =
            if self.files.shown().len() == files { self.capture.totals() } else { self.files.shown_totals() };
        let count = match self.files.shown().len() {
            shown if shown == files => format!("{files} file{}", if files == 1 { "" } else { "s" }),
            shown => format!("{shown} of {files} files"),
        };
        let title = match self.whole_title() {
            Some(whole) => format!(" {whole} "),
            None => format!(
                " {} \u{b7} {count} +{added} \u{2212}{removed} \u{b7} {} ",
                self.scope_title(),
                self.viewed_label()
            ),
        };
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
            Draft::Outside(lines, _) => format!("comment on {} (not in the diff)", lines_label(lines)),
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
pub(super) fn count(notes: usize) -> String {
    if notes == 1 { "1 note".to_owned() } else { format!("{notes} notes") }
}
