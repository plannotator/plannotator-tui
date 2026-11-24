//! The frame: our header and footer (`footer`) around the diff (the viewer draws no chrome),
//! the file list on the left (`files`), the notes rail on the right (`rail`), and the
//! comment box (`compose_at`), menu, picker and help over them (`overlays`). The look
//! is the document review's: the same buttons, hints and boxes from `ui/`.

use anyhow::Result;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use plannotator_tui_review::Side;

use super::diff::Lines;
use super::files::fit;
use super::{Draft, Focus, Geometry, Mode, Screen};
use crate::ui::compose;

const RAIL_WIDTH: u16 = 36;
pub(super) const RAIL_MIN_WIDTH: u16 = 28;
/// Below this the rail is dropped; notes still show as gutter markers.
pub(super) const RAIL_MIN_TOTAL_WIDTH: u16 = 80;

/// The most characters of a PR's title the header shows.
const PR_TITLE_MAX: usize = 48;

impl Screen {
    pub(crate) fn draw(&mut self, frame: &mut Frame) -> Result<()> {
        self.track_current();
        let area = frame.area();
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(self.footer_height(area.width)),
        ])
        .areas(area);
        // A hidden rail gives its columns to the diff, which the viewer reflows as on a resize.
        let mut rail_width = if self.rail_shown() && area.width >= RAIL_MIN_TOTAL_WIDTH {
            let fitted = (area.width * 3 / 10).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH);
            self.dividers.notes_width(fitted, body.width, self.files_wanted())
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
        self.geometry = Geometry { screen: area, diff, rail, files, ..Geometry::default() };
        self.dividers.notes_drawn(rail_width);
        self.dividers.place(files, rail);
        // Before anything asks whether the diff is split: `auto` follows the terminal.
        self.decide_auto(area.width, diff.width)?;
        self.draw_header(frame, header);
        let cursor_selected = self.cursor_selected();
        match &mut self.whole {
            Some(whole) => whole.draw(frame, diff)?,
            None => self.diff.draw(frame, diff)?,
        }
        let (middle, code) = match self.active_diff_mut() {
            Some(shown) => {
                let middle = if shown.split() { shown.split_line() } else { None };
                (middle, (diff.top()..diff.bottom()).map(|y| shown.code_row(y)).collect())
            }
            None => (None, Vec::new()),
        };
        let code = |y: u16| code.get(usize::from(y - diff.y)).copied().unwrap_or(true);
        let selected = super::mouse::tint_selection(frame.buffer_mut(), diff, middle, cursor_selected, code);
        if files.width > 0 {
            let listing = self.listing();
            self.draw_files(frame, files, &listing);
        }
        if rail_width > 0 {
            self.draw_rail(frame, rail);
        }
        self.dividers.draw(frame);
        self.draw_footer(frame, footer);
        if !matches!(self.mode, Mode::Compose(_)) {
            self.compose_at = None;
        }
        match &self.mode {
            Mode::Compose(draft) => {
                let draft = draft.clone();
                self.draw_compose(frame, diff, &draft);
            }
            Mode::Menu(cursor) => self.draw_menu(frame, *cursor),
            Mode::Settings(cursor) => self.draw_settings(frame, *cursor),
            Mode::PickTarget => {
                let root = self.real_root();
                self.picker.draw(frame, area, &self.keys, &root, self.start_delivery.as_deref());
            }
            Mode::Help => self.draw_help(frame, area),
            Mode::Pick(pick) => {
                let pick = pick.clone();
                self.draw_pick(frame, diff, &pick);
            }
            Mode::Browse => self.draw_toolbar(frame, diff, selected),
            Mode::Kinds(cursor) => self.draw_kinds(frame, *cursor),
            Mode::ConfirmQuit | Mode::FilterFiles => {}
        }
        // Last, so whatever was drawn over the line between the sides (the toolbar, a box)
        // keeps its cells and takes their clicks.
        self.draw_split_line(frame, diff);
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

    /// The comment box, next to what it is about (`compose_at`).
    fn draw_compose(&mut self, frame: &mut Frame, diff: Rect, draft: &Draft) {
        let rect = self.compose_rect(diff, draft);
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
        let (text, first) = compose::draw(frame, rect, &self.compose, &title);
        // The text wraps one column short of its area, which keeps a column for the cursor.
        self.geometry.compose_wrap = usize::from(text.width.saturating_sub(1)).max(1);
        self.geometry.compose_text = Some((text, first));
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
