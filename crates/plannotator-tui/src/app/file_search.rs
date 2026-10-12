//! Quick Open (`ctrl+p` in a folder review): find a Markdown file by name and open it.
//! Paths arrive from a background scan in batches; matching never reads file contents.
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;

use anyhow::Result;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use super::{App, Mode};
use crate::file_search::{
    rank,
    scan::{Scan, Update},
};
use crate::theme::palette;

/// Scan batches taken per poll; the channel holds as many, so one poll can drain it.
const BATCHES_PER_POLL: usize = 64;

struct Candidate {
    path: PathBuf,
    name: String,
    relative: String,
}

pub(super) struct FileSearch {
    root: PathBuf,
    scan: Scan,
    query: String,
    files: Vec<Candidate>,
    /// Indices into `files`, best match first.
    results: Vec<usize>,
    cursor: usize,
    /// Set once the user moves the cursor. Until then it stays on the best match while
    /// results stream in, so `Enter` right after typing opens the top result.
    moved: bool,
    done: bool,
    errors: usize,
    error: Option<String>,
}

impl FileSearch {
    fn rematch(&mut self) {
        let selected = self.moved.then(|| self.results.get(self.cursor).copied()).flatten();
        let query = self.query.to_lowercase();
        let mut ranked: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| rank(&file.name, &query).map(|score| (score, index)))
            .collect();
        ranked.sort_by(|(a, i), (b, j)| {
            a.cmp(b).then_with(|| {
                self.files.get(*i).map(|f| &f.relative).cmp(&self.files.get(*j).map(|f| &f.relative))
            })
        });
        self.results = ranked.into_iter().map(|(_, index)| index).collect();
        self.cursor = selected.and_then(|index| self.results.iter().position(|i| *i == index)).unwrap_or(0);
    }

    fn set_query(&mut self, query: String) {
        self.query = query;
        self.moved = false;
        self.rematch();
    }

    fn move_cursor(&mut self, delta: isize) {
        let last = self.results.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
        self.moved = true;
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.results.get(self.cursor).and_then(|i| self.files.get(*i)).map(|f| f.path.clone())
    }
}

impl App {
    /// Open Quick Open on the tree's folder, honouring its hidden-entries setting. Each
    /// opening rescans, so files added or removed since last time are found.
    pub(super) fn start_file_search(&mut self) -> Result<()> {
        let Some(tree) = &self.tree else { return Ok(()) };
        let root = tree.root().to_path_buf();
        self.file_search = Some(FileSearch {
            scan: Scan::start(root.clone(), tree.show_hidden())?,
            root,
            query: String::new(),
            files: Vec::new(),
            results: Vec::new(),
            cursor: 0,
            moved: false,
            done: false,
            errors: 0,
            error: None,
        });
        self.mode = Mode::FileSearch;
        Ok(())
    }

    fn close_file_search(&mut self) {
        self.file_search = None;
        self.mode = Mode::Browse;
    }

    pub(crate) fn file_search_scanning(&self) -> bool {
        self.file_search.as_ref().is_some_and(|search| !search.done)
    }

    /// Take what the scan has found so far. The event loop calls this on idle ticks too, so
    /// results appear while the user is not typing. Returns whether anything changed.
    pub(crate) fn poll_file_search(&mut self) -> bool {
        let Some(search) = &mut self.file_search else { return false };
        if search.done {
            return false;
        }
        let mut changed = false;
        let mut added = false;
        for _ in 0..BATCHES_PER_POLL {
            match search.scan.updates.try_recv() {
                Ok(Update::Files(paths)) => {
                    for path in paths {
                        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        let relative =
                            path.strip_prefix(&search.root).unwrap_or(&path).to_string_lossy().into_owned();
                        search.files.push(Candidate { path, name, relative });
                    }
                    added = true;
                }
                Ok(Update::Error(error)) => {
                    search.errors += 1;
                    search.error = Some(error);
                }
                Ok(Update::Done) => search.done = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    search.done = true;
                    search.errors += 1;
                    search.error = Some("file search stopped unexpectedly".into());
                }
            }
            changed = true;
            if search.done {
                break;
            }
        }
        if added {
            search.rematch();
        }
        changed
    }

    pub(super) fn file_search_paste(&mut self, text: &str) {
        if let Some(search) = &mut self.file_search {
            let query = search.query.chars().chain(text.chars().filter(|ch| !ch.is_control())).collect();
            search.set_query(query);
        }
    }

    pub(super) fn file_search_key(&mut self, key: KeyEvent) {
        let Some(search) = &mut self.file_search else {
            self.mode = Mode::Browse;
            return;
        };
        match key.code {
            KeyCode::Esc => self.close_file_search(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => self.close_file_search(),
            KeyCode::Enter => self.open_file_search_result(),
            KeyCode::Up => search.move_cursor(-1),
            KeyCode::Down => search.move_cursor(1),
            KeyCode::Backspace => {
                let mut query = search.query.clone();
                query.pop();
                search.set_query(query);
            }
            KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.file_search_paste(&ch.to_string());
            }
            _ => {}
        }
    }

    pub(super) fn file_search_mouse(&mut self, mouse: MouseEvent) {
        let Some(search) = &mut self.file_search else { return };
        match mouse.kind {
            MouseEventKind::ScrollDown => search.move_cursor(1),
            MouseEventKind::ScrollUp => search.move_cursor(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self.geometry.search_rows.iter().find(|(rect, _)| {
                    mouse.row == rect.y && mouse.column >= rect.x && mouse.column < rect.right()
                });
                if let Some(&(_, row)) = hit {
                    search.cursor = row;
                    search.moved = true;
                    self.open_file_search_result();
                }
            }
            _ => {}
        }
    }

    /// Open the selected result and reveal it in the tree. A file that cannot be read keeps
    /// the popup open with the error, and the document that was open stays open.
    fn open_file_search_result(&mut self) {
        let Some(path) = self.file_search.as_ref().and_then(FileSearch::selected_path) else { return };
        if let Err(error) = self.open_file_path(&path) {
            if let Some(search) = &mut self.file_search {
                search.error = Some(format!("{error:#}"));
            }
            return;
        }
        self.close_file_search();
        self.reveal_in_tree(&path);
    }

    pub(super) fn draw_file_search(&mut self, frame: &mut Frame) {
        let Some(search) = &self.file_search else { return };
        let area = frame.area();
        let width = area.width.saturating_sub(4).clamp(1, 90).min(area.width);
        let height = area.height.saturating_sub(2).clamp(1, 18).min(area.height);
        let rect = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Cyan))
            .title(Span::styled(" open a Markdown file ", Style::new().dim()))
            .title_bottom(Span::styled(" ↑↓ select · enter/click open · esc cancel ", Style::new().dim()));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // Query on the first row, the scan's progress on the last, results between.
        let prompt = format!("> {}", search.query);
        let prompt_width = prompt.width() as u16;
        frame.render_widget(Paragraph::new(prompt), Rect { height: 1, ..inner });
        frame.set_cursor_position((inner.x + prompt_width.min(inner.width.saturating_sub(1)), inner.y));

        let list_height = usize::from(inner.height.saturating_sub(2));
        let start = search.cursor.saturating_sub(list_height.saturating_sub(1));
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        for (row, index) in search.results.iter().enumerate().skip(start).take(list_height) {
            let Some(file) = search.files.get(*index) else { continue };
            let style = if row == search.cursor { palette().selection } else { Style::new() };
            let text = tail(&file.relative, usize::from(inner.width).saturating_sub(1));
            lines.push(Line::from(Span::styled(format!(" {text}"), style)));
            let y = inner.y + 1 + (row - start) as u16;
            rows.push((Rect { x: inner.x, y, width: inner.width, height: 1 }, row));
        }
        if search.results.is_empty() && list_height > 0 {
            let empty = match (search.done, search.files.is_empty()) {
                (false, _) => " searching…",
                (true, true) => " no Markdown files in this folder",
                (true, false) => " no matching files",
            };
            lines.push(Line::from(Span::styled(empty, Style::new().dim())));
        }
        let list = Rect { y: inner.y + 1, height: inner.height.saturating_sub(2), ..inner };
        frame.render_widget(Paragraph::new(lines), list);

        let mut parts = vec![format!(" {} of {} files", search.results.len(), search.files.len())];
        if !search.done {
            parts.push("scanning…".to_owned());
        }
        if search.errors > 0 {
            parts.push(format!("{} unreadable", search.errors));
        }
        let status = parts.join(" · ");
        let status = match &search.error {
            Some(error) => Line::from(vec![
                Span::styled(status, Style::new().dim()),
                Span::styled(format!(" · {error}"), Style::new().fg(Color::Yellow)),
            ]),
            None => Line::from(Span::styled(status, Style::new().dim())),
        };
        if inner.height > 1 {
            frame.render_widget(Paragraph::new(status), Rect { y: inner.bottom() - 1, height: 1, ..inner });
        }
        self.geometry.search_rows = rows;
    }
}

/// The end of `text` in at most `width` columns, so a deep path keeps its file name.
fn tail(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut kept = Vec::new();
    let mut used = 1; // the leading ellipsis
    for ch in text.chars().rev() {
        used += ch.width().unwrap_or(0);
        if used > width {
            break;
        }
        kept.push(ch);
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

#[cfg(test)]
mod tests;
