//! Keyboard and mouse handling.

use anyhow::Result;
use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};

use super::compose::ComposeAction;
use super::menu::ReviewAction;
use super::selection::Selection;
use super::send::SendState;
use super::{App, Focus, GUTTER, Mode, Pending, TOOLBAR};
use crate::delivery::Delivery as _;
use crate::keys::{self, Action};

impl App {
    pub(crate) fn handle_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match &self.mode {
                Mode::Browse => self.browse_key(*key),
                Mode::ConfirmQuit => self.confirm_quit_key(*key),
                Mode::Pick => self.pick_key(*key),
                Mode::Archive => {
                    self.archive_key(*key);
                    Ok(())
                }
                Mode::ReviewMenu => self.menu_key(*key),
                Mode::Compose | Mode::Edit(_) => self.text_key(*key),
            },
            // A paste lands in the comment box verbatim, newlines included; anywhere else
            // it is ignored rather than replayed as keystrokes.
            Event::Paste(text) if matches!(self.mode, Mode::Compose | Mode::Edit(_)) => {
                self.compose.insert_text(text);
                Ok(())
            }
            Event::Mouse(mouse) if self.mode == Mode::Browse => self.mouse(*mouse),
            Event::Mouse(mouse) if self.mode == Mode::Pick => self.pick_mouse(*mouse),
            Event::Mouse(mouse) if self.mode == Mode::ReviewMenu => self.menu_mouse(*mouse),
            Event::Mouse(mouse) if self.mode == Mode::Archive => {
                self.archive_mouse(*mouse);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn browse_key(&mut self, key: KeyEvent) -> Result<()> {
        // Global keys first.
        match self.keys.action(&key, keys::GLOBAL) {
            Some(Action::Quit | Action::Interrupt) => {
                self.request_quit();
                return Ok(());
            }
            Some(Action::CycleFocus) => {
                self.cycle_focus();
                return Ok(());
            }
            Some(Action::Send) => return self.send_feedback(),
            // A reply review sends every note, so one the agent already has closes like `q`.
            Some(Action::SendAndClose) if !self.is_file_review() => {
                if self.send_state == SendState::Sent {
                    self.request_quit();
                    return Ok(());
                }
                return self.send_and_quit();
            }
            Some(Action::ReviewMenu) if self.is_file_review() => {
                self.open_review_menu();
                return Ok(());
            }
            Some(Action::ResendAll) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::ResendAll);
            }
            Some(Action::FinishReview) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::Finish);
            }
            Some(Action::UndoFinish) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::Undo);
            }
            Some(Action::Archive) if self.is_file_review() => {
                return self.run_review_action(ReviewAction::Archive);
            }
            Some(Action::ToggleTree) => {
                self.toggle_tree(self.geometry.doc.width + self.geometry.tree.width + GUTTER);
                return Ok(());
            }
            Some(Action::Reload) => return self.reload(),
            Some(Action::PickMessage) => {
                self.reopen_picker();
                return Ok(());
            }
            _ => {}
        }
        match self.focus {
            Focus::Tree => self.tree_key(key),
            Focus::Document => self.document_key(key),
            Focus::Rail => self.rail_key(key),
        }
    }

    /// The quit confirmation: send first, quit anyway, or stay.
    fn confirm_quit_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.keys.action(&key, keys::CONFIRM_QUIT) {
            Some(Action::Yes | Action::Confirm) => {
                self.mode = Mode::Browse;
                self.send_and_quit()?;
            }
            Some(Action::No) => {
                self.mode = Mode::Browse;
                self.quit = true;
            }
            Some(Action::Back) => self.mode = Mode::Browse,
            _ => {}
        }
        Ok(())
    }

    fn cycle_focus(&mut self) {
        let has_tree = self.tree.is_some();
        let has_rail = !self.open.store.placed().is_empty();
        self.focus = match self.focus {
            Focus::Document if has_rail => Focus::Rail,
            Focus::Document | Focus::Rail if has_tree => Focus::Tree,
            Focus::Tree | Focus::Document | Focus::Rail => Focus::Document,
        };
    }

    fn tree_key(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.tree_len();
        match self.keys.action(&key, keys::TREE) {
            Some(Action::Down) => {
                self.tree_cursor = (self.tree_cursor + 1).min(len.saturating_sub(1));
            }
            Some(Action::Up) => self.tree_cursor = self.tree_cursor.saturating_sub(1),
            Some(Action::Confirm | Action::Right) => self.open_tree_selection()?,
            Some(Action::ToggleHidden) => self.toggle_tree_hidden()?,
            Some(Action::Back) => self.focus = Focus::Document,
            _ => {}
        }
        // The tree's height is the last frame's; the first frame has not drawn yet, but its cursor is row 0.
        self.keep_tree_cursor_visible(usize::from(self.geometry.tree.height));
        Ok(())
    }

    fn rail_key(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.open.store.placed().len();
        match self.keys.action(&key, keys::RAIL) {
            Some(Action::Down) => {
                self.rail_cursor = (self.rail_cursor + 1).min(len.saturating_sub(1));
            }
            Some(Action::Up) => self.rail_cursor = self.rail_cursor.saturating_sub(1),
            Some(Action::Confirm | Action::Edit) => self.edit_selected_annotation(),
            Some(Action::Remove) => self.remove_selected_annotation()?,
            Some(Action::Back) => self.focus = Focus::Document,
            _ => {}
        }
        if let Some(target) = self.open.store.placed().get(self.rail_cursor)
            && let Some(block) = self.open.doc.block_containing(target.range.start)
        {
            self.selected = block;
            self.ensure_selected_visible();
        }
        Ok(())
    }

    fn document_key(&mut self, key: KeyEvent) -> Result<()> {
        let page = i64::from(self.geometry.doc.height.max(1));
        // With a selection waiting, the toolbar keys win.
        if self.pending.is_some()
            && let Some(action) = self.keys.action(&key, keys::TOOLBAR)
            && let Some(&(_, _, _, kind)) = TOOLBAR.iter().find(|item| item.2 == action)
        {
            return self.act(kind);
        }
        // Visual mode: the selection follows the cursor.
        if self.selection.is_some_and(|s| s.dragging) {
            self.visual_key(key);
            return Ok(());
        }
        // Roaming (`i`): the cursor moves with nothing selected yet, so `v` can start
        // anywhere. Other keys fall through to their block-mode meaning.
        if self.roam && self.selection.is_none() {
            if self.keys.is(Action::Back, &key) {
                self.roam = false;
                self.status = None;
                return Ok(());
            }
            if self.motion_key(key) {
                return Ok(());
            }
        }
        match self.keys.action(&key, keys::DOCUMENT) {
            Some(Action::Back) => {
                if self.pending.is_some() || self.selection.is_some() {
                    self.clear_selection();
                } else {
                    self.request_quit();
                }
            }
            Some(Action::Visual) => {
                self.clear_selection();
                self.selection = Some(Selection::start(self.cursor));
                self.status = Some(format!(
                    "visual: move to extend, {} to select, {} to cancel",
                    self.keys.hint(Action::Confirm),
                    self.keys.hint(Action::Back)
                ));
            }
            Some(Action::MoveCursor) => self.start_roaming(),
            Some(Action::Down) => self.select_block(self.selected + 1),
            Some(Action::Up) => self.select_block(self.selected.saturating_sub(1)),
            // A cursor that moves must be visible, so a column move in block mode is a
            // roaming move: the same key, with the cursor drawn.
            Some(Action::Left | Action::Right) => {
                self.start_roaming();
                self.motion_key(key);
            }
            Some(Action::PageDown) => self.page_by(page / 2),
            Some(Action::PageUp) => self.page_by(-page / 2),
            Some(Action::Top) => self.select_block(0),
            Some(Action::Bottom) => {
                self.select_block(self.open.doc.blocks.len().saturating_sub(1));
            }
            Some(Action::Comment | Action::Confirm) => {
                // No selection: comment on the whole selected block.
                if let (Some(block), Some(rendered)) =
                    (self.open.doc.blocks.get(self.selected), self.open.layout.blocks.get(self.selected))
                {
                    self.pending = Some(Pending { range: block.range.clone(), at: (rendered.first_row, 0) });
                    self.act(Kind::Comment)?;
                }
            }
            Some(Action::ClearBlock) => {
                let removed = self.open.store.remove_in_block(&self.open.doc, self.selected)?;
                if removed > 0 {
                    self.mark_unsent();
                    self.sync_tree_counts();
                }
                self.status = Some(format!("removed {removed} annotation(s) on block"));
            }
            _ => {}
        }
        Ok(())
    }

    /// Keys while a keyboard selection is being extended.
    fn visual_key(&mut self, key: KeyEvent) {
        match self.keys.action(&key, keys::VISUAL) {
            Some(Action::Back) => self.clear_selection(),
            Some(Action::Confirm | Action::Visual) => self.finish_selection(),
            _ => {
                self.motion_key(key);
            }
        }
        if let Some(sel) = self.selection.as_mut() {
            sel.set_head(self.cursor);
        }
    }

    /// Enter roaming: the visual-mode motions move the cursor with nothing selected.
    fn start_roaming(&mut self) {
        self.clear_selection();
        self.roam = true;
        let hint = |action| self.keys.hint(action);
        self.status = Some(format!(
            "move: {} {} {} {} {} · {} select · {} back to blocks",
            self.keys.directions_hint(),
            hint(Action::WordForward),
            hint(Action::WordBack),
            hint(Action::LineStart),
            hint(Action::LineEnd),
            hint(Action::Visual),
            hint(Action::Back)
        ));
    }

    /// The cursor motions shared by visual and roaming modes. True when `key` was one.
    fn motion_key(&mut self, key: KeyEvent) -> bool {
        match self.keys.action(&key, keys::MOTION) {
            Some(Action::Left) => self.move_cursor(0, -1),
            Some(Action::Right) => self.move_cursor(0, 1),
            Some(Action::Down) => self.move_cursor(1, 0),
            Some(Action::Up) => self.move_cursor(-1, 0),
            Some(Action::WordForward) => self.move_word(1),
            Some(Action::WordBack) => self.move_word(-1),
            Some(Action::LineStart) => self.cursor.1 = 0,
            Some(Action::LineEnd) => {
                self.cursor.1 =
                    self.open.layout.row(self.cursor.0).map_or(0, |r| r.cells.len().saturating_sub(1));
            }
            _ => return false,
        }
        self.ensure_cursor_visible();
        true
    }

    /// Move the keyboard cursor by rows/columns, skipping gap rows and clamping to text.
    fn move_cursor(&mut self, rows: i64, cols: i64) {
        let total = self.open.layout.total_rows;
        if total == 0 {
            return;
        }
        let mut row = self.cursor.0;
        for _ in 0..rows.unsigned_abs() {
            let next = if rows > 0 { row + 1 } else { row.saturating_sub(1) };
            if next >= total || next == row {
                break;
            }
            row = next;
            if self.open.layout.row(row).is_none() {
                // gap row: keep going in the same direction
                let after = if rows > 0 { row + 1 } else { row.saturating_sub(1) };
                if after < total && self.open.layout.row(after).is_some() {
                    row = after;
                }
            }
        }
        let width = self.open.layout.row(row).map_or(0, |r| r.cells.len());
        let col = (self.cursor.1 as i64 + cols).clamp(0, width.saturating_sub(1) as i64) as usize;
        self.cursor = (row, col);
        if let Some(block) = self.open.layout.block_at_row(row) {
            self.selected = block;
        }
        self.ensure_cursor_visible();
    }

    /// Jump to the next (+1) or previous (-1) word start on the current row.
    fn move_word(&mut self, direction: i64) {
        let Some(row) = self.open.layout.row(self.cursor.0) else { return };
        let text = row.line.to_string();
        let chars: Vec<char> = text.chars().collect();
        let is_boundary = |i: usize| {
            let here = chars.get(i).is_some_and(|c| !c.is_whitespace());
            let before = i == 0 || chars.get(i - 1).is_some_and(char::is_ascii_whitespace);
            here && before
        };
        let col = self.cursor.1;
        let next = if direction > 0 {
            (col + 1..chars.len()).find(|&i| is_boundary(i)).unwrap_or(chars.len().saturating_sub(1))
        } else {
            (0..col).rev().find(|&i| is_boundary(i)).unwrap_or(0)
        };
        self.cursor.1 = next;
    }

    fn text_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.compose.handle_key(key) {
            ComposeAction::Cancel => self.mode = Mode::Browse,
            ComposeAction::Save => {
                let body = self.compose.value().trim().to_owned();
                match std::mem::replace(&mut self.mode, Mode::Browse) {
                    Mode::Edit(id) => {
                        if body.is_empty() {
                            self.status = Some("edit cancelled: empty".into());
                        } else if self.open.store.edit_body(&id, body)? {
                            self.mark_unsent();
                            self.status = Some("annotation updated".into());
                        }
                    }
                    Mode::Compose
                    | Mode::Browse
                    | Mode::ConfirmQuit
                    | Mode::Pick
                    | Mode::Archive
                    | Mode::ReviewMenu => {
                        if !body.is_empty()
                            && let Some(pending) = self.pending.take()
                        {
                            self.annotate(pending.range, Kind::Comment, body)?;
                            self.status = Some("comment saved".into());
                        }
                        self.clear_selection();
                    }
                }
            }
            ComposeAction::Edited => {}
        }
        Ok(())
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        match mouse.kind {
            MouseEventKind::ScrollDown if self.in_tree(mouse.column, mouse.row) => self.tree_scroll_by(3),
            MouseEventKind::ScrollUp if self.in_tree(mouse.column, mouse.row) => self.tree_scroll_by(-3),
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.send_button_hit(mouse.column, mouse.row) {
                    return self.send_feedback();
                }
                let hit = |rect: Option<ratatui::layout::Rect>| {
                    rect.is_some_and(|r| mouse.row == r.y && mouse.column >= r.x && mouse.column < r.right())
                };
                if hit(self.geometry.review_button) {
                    self.open_review_menu();
                    return Ok(());
                }
                if hit(self.geometry.undo_button) {
                    self.undo_finish_review();
                    return Ok(());
                }
                if let Some(kind) = self.toolbar_hit(mouse.column, mouse.row) {
                    return self.act(kind);
                }
                if let Some(index) = self.tree_hit(mouse.row, mouse.column) {
                    self.tree_cursor = index;
                    self.focus = Focus::Tree;
                    return self.open_tree_selection();
                }
                if let Some(index) = self.bubble_hit(mouse.column, mouse.row) {
                    self.rail_cursor = index;
                    self.focus = Focus::Rail;
                    return Ok(());
                }
                let now = std::time::Instant::now();
                let double = self.last_click.take().is_some_and(|(at, col, row)| {
                    col == mouse.column
                        && row == mouse.row
                        && now.duration_since(at) <= std::time::Duration::from_millis(400)
                });
                self.last_click = Some((now, mouse.column, mouse.row));
                let Some(pos) = self.doc_position(mouse.column, mouse.row, false) else { return Ok(()) };
                self.focus = Focus::Document;
                if double
                    && let Some(block) = self.open.layout.block_at_row(pos.0)
                    && let Some(range) = self.open.doc.blocks.get(block).map(|b| b.range.clone())
                {
                    // A double-click selects the whole block and offers the toolbar for it.
                    self.selected = block;
                    self.selection = None;
                    self.cursor = pos;
                    self.status = None;
                    self.pending = Some(Pending { range, at: pos });
                    return Ok(());
                }
                if let Some(block) = self.open.layout.block_at_row(pos.0) {
                    self.selected = block;
                }
                self.cursor = pos;
                self.pending = None;
                self.selection = Some(Selection::start(pos));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if !self.selection.is_some_and(|s| s.dragging) {
                    return Ok(());
                }
                let doc = self.geometry.doc;
                if mouse.row < doc.y {
                    self.scroll_by(-1);
                } else if mouse.row >= doc.bottom() {
                    self.scroll_by(1);
                }
                if let Some(pos) = self.doc_position(mouse.column, mouse.row, true)
                    && let Some(sel) = self.selection.as_mut()
                {
                    sel.set_head(pos);
                }
            }
            MouseEventKind::Up(MouseButton::Left) if self.selection.is_some_and(|s| s.dragging) => {
                self.finish_selection();
            }
            _ => {}
        }
        Ok(())
    }

    fn send_button_hit(&self, column: u16, row: u16) -> bool {
        self.geometry
            .send_button
            .is_some_and(|rect| row == rect.y && column >= rect.x && column < rect.right())
    }

    fn toolbar_hit(&self, column: u16, row: u16) -> Option<Kind> {
        let (rect, spans) = self.geometry.toolbar.as_ref()?;
        if row != rect.y || column < rect.x || column >= rect.right() {
            return None;
        }
        spans.iter().zip(TOOLBAR.iter()).find(|(span, _)| span.contains(&column)).map(|(_, item)| item.3)
    }

    /// Whether the screen cell is inside the drawn tree pane.
    fn in_tree(&self, column: u16, row: u16) -> bool {
        let tree = self.geometry.tree;
        tree.width > 0 && column >= tree.x && column < tree.right() && row >= tree.y && row < tree.bottom()
    }

    /// The tree row under the screen cell, accounting for the tree's scroll offset.
    fn tree_hit(&self, row: u16, column: u16) -> Option<usize> {
        self.in_tree(column, row)
            .then(|| self.tree_scroll + usize::from(row - self.geometry.tree.y))
            .filter(|&i| i < self.tree_len())
    }

    fn bubble_hit(&self, column: u16, row: u16) -> Option<usize> {
        let id = self
            .geometry
            .bubbles
            .iter()
            .find(|(rect, _)| {
                column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
            })
            .map(|(_, id)| id.clone())?;
        self.open.store.placed().iter().position(|p| p.annotation.id == id)
    }

    /// Screen cell -> document (row, column). With `clamp`, positions outside the
    /// document area snap to its nearest edge so a drag can leave the area.
    fn doc_position(&self, column: u16, row: u16, clamp: bool) -> Option<(usize, usize)> {
        let doc = self.geometry.doc;
        let inside = column >= doc.x && column < doc.right() && row >= doc.y && row < doc.bottom();
        if !inside && !clamp {
            let in_gutter = column >= doc.x.saturating_sub(GUTTER)
                && column < doc.x
                && row >= doc.y
                && row < doc.bottom();
            if !in_gutter {
                return None;
            }
        }
        let col = column.clamp(doc.x, doc.right().saturating_sub(1)) - doc.x;
        let row = row.clamp(doc.y, doc.bottom().saturating_sub(1)) - doc.y;
        let doc_row = (self.scroll + usize::from(row)).min(self.open.layout.total_rows.saturating_sub(1));
        Some((doc_row, usize::from(col)))
    }

    /// Turn the selection into a source range; it may span blocks.
    pub(super) fn finish_selection(&mut self) {
        let Some(sel) = self.selection.as_mut() else { return };
        sel.dragging = false;
        if sel.is_empty() {
            self.selection = None;
            return;
        }
        let sel = *sel;
        let (a, b) = sel.ordered();
        let mut bounds: Option<(usize, usize)> = None;
        for row in a.0..=b.0 {
            let Some(r) = self.open.layout.row(row) else { continue };
            let cols = sel.columns_on(row, r.cells.len()).unwrap_or(0..0);
            for offset in r.cells.iter().skip(cols.start).take(cols.len()).flatten() {
                bounds =
                    Some(bounds.map_or((*offset, *offset), |(lo, hi)| (lo.min(*offset), hi.max(*offset))));
            }
        }
        let Some((lo, hi)) = bounds else {
            self.selection = None;
            return;
        };
        let source = &self.open.doc.source;
        let end_char = source.get(hi..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
        let range = lo..hi + end_char;
        if self.clipboard
            && let Some(text) = source.get(range.clone())
        {
            let _ = crate::delivery::Clipboard.deliver(text);
        }
        if let Some(block) = self.open.doc.block_containing(range.start) {
            self.selected = block;
        }
        self.status = None;
        self.pending = Some(Pending { range, at: a });
    }
}
