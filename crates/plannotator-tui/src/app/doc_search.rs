//! `/` in the document: a one-line prompt in the footer that searches as it is typed,
//! highlights every match, and `n` / `N` to step through them. Matching itself is
//! `crate::doc_search`; this module moves the cursor and draws.
//!
//! A search moves the keyboard cursor, the selected block and the view, and nothing else:
//! a selection waiting in the toolbar stays, and no annotation is touched.

use std::ops::Range;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr as _;

use super::{App, Mode};
use crate::doc_search::{self, Index};
use crate::keys::Action;
use crate::theme::palette;

/// The search the document shows.
#[derive(Debug, Clone)]
pub(super) struct Search {
    query: String,
    /// Source ranges, in document order.
    matches: Vec<Range<usize>>,
    /// The match the cursor was put on.
    current: Option<usize>,
    /// Where the cursor was put; a cursor still there steps on from `current`.
    landed: Option<(usize, usize)>,
}

/// The prompt being typed, and what Esc puts back.
#[derive(Debug)]
pub(super) struct Prompt {
    query: String,
    /// The source byte the search starts from: the cursor's when `/` was pressed.
    from: usize,
    scroll: usize,
    cursor: (usize, usize),
    selected: usize,
    previous: Option<Search>,
}

impl App {
    /// Open the prompt. The first search of a document indexes its rendered text here, so
    /// typing costs only the matching. A search covers the whole document, so a long one
    /// still being laid out in the background is waited for first.
    pub(super) fn start_search(&mut self) {
        let open = &mut self.open;
        if open.search_index.is_none() {
            open.layout.finish();
            open.search_index = Some(Index::of(&open.layout));
        }
        self.search_prompt = Some(Prompt {
            query: String::new(),
            from: self.cursor_offset(),
            scroll: self.scroll,
            cursor: self.cursor,
            selected: self.selected,
            previous: self.open.search.clone(),
        });
        self.mode = Mode::Search;
    }

    pub(super) fn search_key(&mut self, key: KeyEvent) {
        let Some(prompt) = &self.search_prompt else {
            self.mode = Mode::Browse;
            return;
        };
        match key.code {
            KeyCode::Esc => self.cancel_search(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => self.cancel_search(),
            KeyCode::Enter => {
                // An empty search clears; any other keeps its highlights and its place.
                if prompt.query.is_empty() {
                    self.open.search = None;
                }
                self.search_prompt = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Backspace => {
                let mut query = prompt.query.clone();
                query.pop();
                self.set_search_query(query);
            }
            KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.search_paste(&ch.to_string());
            }
            _ => {}
        }
    }

    /// A paste joins the query; a query is one line, so line breaks drop out.
    pub(super) fn search_paste(&mut self, text: &str) {
        let Some(prompt) = &self.search_prompt else { return };
        let query = prompt.query.chars().chain(text.chars().filter(|ch| !ch.is_control())).collect();
        self.set_search_query(query);
    }

    /// Esc: the view, the cursor and the search shown go back to how `/` found them.
    fn cancel_search(&mut self) {
        let Some(prompt) = self.search_prompt.take() else { return };
        self.restore(&prompt);
        self.open.search = prompt.previous;
        self.mode = Mode::Browse;
    }

    fn restore(&mut self, prompt: &Prompt) {
        self.scroll = prompt.scroll;
        self.cursor = prompt.cursor;
        self.selected = prompt.selected;
    }

    /// Search as typed: highlight every match and put the cursor on the first at or after
    /// where `/` was pressed. With nothing found, the view goes back to that place.
    fn set_search_query(&mut self, query: String) {
        let Some(mut prompt) = self.search_prompt.take() else { return };
        let matches = match &self.open.search_index {
            Some(index) => index.find(&query),
            None => Vec::new(),
        };
        let current = doc_search::at_or_after(&matches, prompt.from);
        self.open.search = (!query.is_empty()).then(|| Search {
            query: query.clone(),
            matches,
            current: None,
            landed: None,
        });
        match current {
            Some(index) => self.land(index),
            None => self.restore(&prompt),
        }
        prompt.query = query;
        self.search_prompt = Some(prompt);
    }

    /// `n` (forward) and `N`: the next or previous match from the cursor, wrapping.
    pub(super) fn step_search(&mut self, forward: bool) {
        let Some(search) = &self.open.search else {
            let key = self.keys.hint(Action::Search);
            self.status = Some(if key.is_empty() {
                "no search".to_owned()
            } else {
                format!("no search: {key} starts one")
            });
            return;
        };
        // Still on the match it landed on: step by one, so a match that shares a cell with
        // its neighbour cannot be stepped over. Moved away: step from the cursor.
        let step = if let Some(current) = search.current.filter(|_| search.landed == Some(self.cursor)) {
            doc_search::step(search.matches.len(), current, forward)
        } else if forward {
            doc_search::next(&search.matches, self.cursor_offset())
        } else {
            doc_search::previous(&search.matches, self.cursor_offset())
        };
        let Some((index, wrapped)) = step else {
            self.status = Some(format!("no matches for \u{201c}{}\u{201d}", search.query));
            return;
        };
        self.land(index);
        self.status = wrapped.then(|| {
            if forward { "search wrapped to the top" } else { "search wrapped to the bottom" }.to_owned()
        });
    }

    /// Esc after a search: drop its highlights, and any word about it in the footer. False
    /// when there was none to drop.
    pub(super) fn clear_search(&mut self) -> bool {
        let cleared = self.open.search.take().is_some();
        if cleared {
            self.status = None;
        }
        cleared
    }

    /// Put the cursor on match `index`, select its block, and bring it into view: centred
    /// when it was off screen, so the lines after it show too.
    fn land(&mut self, index: usize) {
        let Some(search) = &mut self.open.search else { return };
        let Some(range) = search.matches.get(index).cloned() else { return };
        search.current = Some(index);
        let block = self.open.doc.blocks.partition_point(|b| b.range.end <= range.start);
        // A match clipped off a code line has no cell; its block's first row stands in.
        let position = self
            .open
            .layout
            .first_cell_in_range(block, &range)
            .or_else(|| self.open.layout.blocks.get(block).map(|b| (b.first_row, 0)));
        let Some(position) = position else { return };
        search.landed = Some(position);
        self.cursor = position;
        if block < self.open.doc.blocks.len() {
            self.selected = block;
        }
        let height = usize::from(self.geometry.doc.height.max(1));
        if position.0 < self.scroll || position.0 >= self.scroll + height {
            self.scroll = position.0.saturating_sub(height / 2);
            self.scroll_by(0);
        }
    }

    /// The source byte at the cursor, or the next one shown after it; past the last, the
    /// end of the document.
    fn cursor_offset(&self) -> usize {
        let layout = &self.open.layout;
        let (row, col) = self.cursor;
        (row..layout.total_rows)
            .filter_map(|r| layout.row(r).map(|cells| (r, cells)))
            .find_map(|(r, found)| {
                let skip = if r == row { col } else { 0 };
                found.cells.iter().skip(skip).find_map(|cell| *cell)
            })
            .unwrap_or(self.open.doc.source.len())
    }

    /// Per column of a row, how the search paints it; `None` when there is nothing to paint.
    pub(super) fn search_styles(&self, cells: &[Option<usize>]) -> Option<Vec<Option<Style>>> {
        let search = self.open.search.as_ref().filter(|s| !s.matches.is_empty())?;
        let hits = doc_search::row_hits(cells, &search.matches);
        Some(
            hits.into_iter()
                .map(|hit| {
                    hit.map(|index| {
                        if Some(index) == search.current {
                            palette().search_current
                        } else {
                            palette().search
                        }
                    })
                })
                .collect(),
        )
    }

    /// `/query 3/17` for the footer, while a search is shown and the prompt is closed.
    pub(super) fn search_label(&self) -> Option<String> {
        if self.mode == Mode::Search {
            return None;
        }
        let search = self.open.search.as_ref()?;
        Some(format!("/{} {}", search.query, self.search_count()?))
    }

    /// `3/17`, `no matches`; nothing before anything is typed.
    fn search_count(&self) -> Option<String> {
        let search = self.open.search.as_ref()?;
        Some(match (search.matches.len(), search.current) {
            (0, _) => "no matches".to_owned(),
            (total, Some(current)) => format!("{}/{total}", current + 1),
            (total, None) => format!("{total} matches"),
        })
    }

    pub(super) fn has_search(&self) -> bool {
        self.open.search.is_some()
    }

    /// The prompt owns the footer's one row: the query on the left with the terminal's
    /// cursor after it, the count and the keys on the right.
    pub(super) fn draw_search_prompt(&self, frame: &mut Frame, area: Rect) {
        let Some(prompt) = &self.search_prompt else { return };
        let count = self.search_count();
        let right =
            count.into_iter().chain(["enter keep".to_owned(), "esc cancel".to_owned()]).collect::<Vec<_>>();
        let right = format!("{} ", right.join(" \u{b7} "));
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(area);
        let text = format!(" /{}", prompt.query);
        // A query wider than its room shows its end, where the typing is.
        let room = usize::from(left_area.width.saturating_sub(1));
        let shown: String = if text.width() > room {
            let chars: Vec<char> = text.chars().collect();
            let mut used = 0;
            let start = chars
                .iter()
                .rposition(|c| {
                    used += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
                    used > room
                })
                .map_or(0, |i| i + 1);
            chars.get(start..).map(|tail| tail.iter().collect()).unwrap_or_default()
        } else {
            text
        };
        let x = left_area.x + (shown.width() as u16).min(left_area.width.saturating_sub(1));
        frame.render_widget(Paragraph::new(Line::from(Span::raw(shown).bold())), left_area);
        frame.render_widget(Paragraph::new(Line::from(Span::raw(right).dim()).right_aligned()), right_area);
        frame.set_cursor_position((x, area.y));
    }
}

#[cfg(test)]
mod tests;
