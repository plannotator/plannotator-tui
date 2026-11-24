//! The change picker: what to review, from a fixed list whose rows never move. Uncommitted,
//! Unstaged and Staged first, the branch against its base when it has moved past it, then
//! the last 20 commits newest first and `more…` for 20 older ones.
//!
//! It opens on its own (`review --pick`, or a bare `review` with nothing to show), or over a
//! review (`pick-changes`, `p`). The first frame needs no git at all beyond the commit list
//! taken before it; each row's size (`+12 −3 · 4 files`, or `(no changes)`) and its unsent
//! notes fill in from background threads, the rows on screen first ([`source`]). Until the
//! person moves it, the cursor sits on the first row not known to be empty, so with a clean
//! tree after the agent committed it lands on that commit.
//!
//! Enter captures the row's scope exactly ([`plannotator_tui_review::Repo::capture`]: no
//! fallback to another scope), also in the background; what fails says so in the footer and
//! the picker stays. At most two jobs run at once ([`source::AT_ONCE`]), so at most two git
//! commands; a reload, a choice or closing the picker kills what runs. [`shown`] turns a capture into a review, carrying the open review's
//! delivery, settings, keys and chosen agent over.

mod draw;
mod rows;
pub(crate) mod shown;
pub(crate) mod source;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::mpsc::SyncSender;

use plannotator_tui_review::{Capture, Repo, Scope};
use ratatui::crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::keys::{self, Action, Keymap};
use rows::{Kind, Row, Sized};
use source::{Done, Job, Listing, PAGE, Source, Work};

/// What the footer says while the list is read again.
const RELOADING: &str = "reloading\u{2026}";

/// What a key or click asks of whoever shows the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Stay,
    /// Back to the review it was opened over.
    Close,
    Quit,
}

/// Older commits: none, ready to list, or being listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum More {
    None,
    Ready,
    Loading,
}

/// Where the rows were drawn last frame, for the mouse and for which sizes come first.
#[derive(Debug, Default, Clone)]
struct Geometry {
    rows: Vec<(Rect, usize)>,
    /// The whole picker: the screen, or the box over the review.
    area: Rect,
    /// Rows that fit.
    visible: usize,
}

pub(crate) struct Picker {
    work: Work,
    name: String,
    rows: Vec<Row>,
    /// The first page of commits arrived.
    listed: bool,
    more: More,
    cursor: usize,
    /// The person moved the cursor: it no longer follows the first row with changes.
    pinned: bool,
    scroll: usize,
    status: Option<String>,
    /// The scope of the review it was opened over; `None` on its own.
    over: Option<Scope>,
    help: bool,
    /// Moves on with every reload, so answers to an older round are dropped.
    generation: u64,
    /// The row being captured.
    opening: Option<usize>,
    opened: Option<(Repo, Capture)>,
    /// The row to keep the cursor on across a reload.
    keep: Option<Scope>,
    keys: Keymap,
    geometry: Geometry,
}

impl std::fmt::Debug for Picker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picker")
            .field("name", &self.name)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl Picker {
    /// The picker over `source`. `listing` is the first page when it was taken before the
    /// first frame; without it the picker asks in the background. `over` is the scope of the
    /// review it opens over. `wake` is the event loop's.
    pub(crate) fn open(
        source: Arc<dyn Source>,
        listing: Option<Listing>,
        over: Option<Scope>,
        keys: Keymap,
        wake: SyncSender<()>,
    ) -> Self {
        let name = rows::clean(&source.name());
        let mut picker = Self {
            work: Work::new(source, wake),
            name,
            rows: [Kind::Uncommitted, Kind::Unstaged, Kind::Staged].map(Row::new).into(),
            listed: false,
            more: More::None,
            cursor: 0,
            pinned: false,
            scroll: 0,
            status: None,
            // Over a review, the cursor starts on the row it shows.
            keep: over.clone(),
            over,
            help: false,
            generation: 0,
            opening: None,
            opened: None,
            keys,
            geometry: Geometry::default(),
        };
        if let Some(listing) = listing {
            picker.take_listing(0, Ok(listing));
        } else {
            picker.keep_cursor(false);
            picker.ask(Job::List { skip: 0, count: PAGE });
        }
        picker.ask_sizes();
        picker
    }

    /// Say `status` in the footer.
    pub(crate) fn say(&mut self, status: String) {
        self.status = Some(status);
    }

    /// Take the answers that arrived and ask for the next sizes. True when the picker changed.
    pub(crate) fn tick(&mut self) -> bool {
        let answers = self.work.take();
        let changed = !answers.is_empty();
        for done in answers {
            self.take(done);
        }
        if !self.work.pump() {
            self.status = Some("could not start a thread to ask git".into());
        }
        self.ask_sizes();
        self.settle_cursor();
        changed
    }

    /// The capture Enter asked for, once it is ready.
    pub(crate) fn take_opened(&mut self) -> Option<(Repo, Capture)> {
        self.opened.take()
    }

    fn ask(&mut self, job: Job) {
        if !self.work.ask(self.generation, job) {
            self.status = Some("could not start a thread to ask git".into());
        }
    }

    fn take(&mut self, done: Done) {
        match done {
            Done::List { generation, skip, listing } if generation == self.generation => {
                self.take_listing(skip, listing);
            }
            Done::Size { generation, row, size } if generation == self.generation => {
                if let Some(row) = self.rows.get_mut(row) {
                    row.size = match size {
                        Ok(size) => Sized::Known(size),
                        Err(error) => Sized::Failed(error),
                    };
                }
            }
            Done::Capture { generation, row, captured } if generation == self.generation => {
                if self.opening != Some(row) {
                    return;
                }
                self.opening = None;
                let name = self.rows.get(row).map(Row::name).unwrap_or_default();
                match *captured {
                    Ok((_, capture)) if capture.is_empty() => {
                        self.status = Some(format!("{name} has no changes"));
                    }
                    Ok(opened) => {
                        self.status = None;
                        self.opened = Some(opened);
                    }
                    Err(error) => {
                        // A commit pruned or rewritten since the list was read.
                        let stale = self
                            .rows
                            .get(row)
                            .is_some_and(|row| matches!(row.kind, Kind::Commit(_) | Kind::Branch(_)))
                            && !error.contains("too large");
                        let key = self.keys.hint(Action::Reload);
                        self.status = Some(if stale && !key.is_empty() {
                            format!("{name}: {error} \u{b7} press {key} to reload")
                        } else {
                            format!("{name}: {error}")
                        });
                    }
                }
            }
            // An answer to the round before a reload.
            Done::List { .. } | Done::Size { .. } | Done::Capture { .. } => {}
        }
    }

    /// Add a page of commits: the first page (with the branch) after the fixed rows, an older
    /// one where `more…` was.
    fn take_listing(&mut self, skip: usize, listing: Result<Listing, String>) {
        let listing = match listing {
            Ok(listing) => listing,
            Err(error) => {
                self.status = Some(format!("could not list the commits: {error}"));
                if skip > 0 {
                    self.more = More::Ready;
                }
                self.listed = true;
                return;
            }
        };
        if skip == 0 {
            if self.status.as_deref() == Some(RELOADING) {
                self.status = None;
            }
            self.rows.truncate(3);
            self.rows.extend(listing.branch.map(|scope| Row::new(Kind::Branch(scope))));
        } else if self.rows.last().is_some_and(|row| row.kind == Kind::More) {
            self.rows.pop();
        }
        self.rows.extend(listing.commits.into_iter().map(|commit| Row::new(Kind::Commit(commit))));
        self.more = if listing.more { More::Ready } else { More::None };
        if listing.more {
            self.rows.push(Row::new(Kind::More));
        }
        self.listed = true;
        self.keep_cursor(true);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    /// Put the cursor on the row of `scope` once it is listed, instead of the review's own.
    pub(crate) fn keep_at(&mut self, scope: Scope) {
        self.keep = Some(scope);
        self.keep_cursor(self.listed);
    }

    /// Put the cursor on the row it was asked to stay on ([`Self::keep`]), once that row is
    /// listed; after the first page (`listed`), give up on one that is not.
    fn keep_cursor(&mut self, listed: bool) {
        let Some(keep) = &self.keep else { return };
        if let Some(at) = self.rows.iter().position(|row| row.scope().as_ref() == Some(keep)) {
            self.cursor = at;
            self.pinned = true;
            self.keep = None;
        } else if listed {
            self.keep = None;
        }
    }

    /// Ask for the sizes not known yet, the rows on screen first, while the work has room and
    /// no capture waits for it.
    fn ask_sizes(&mut self) {
        let shown = self.scroll..self.scroll + self.geometry.visible.max(1);
        while self.work.has_room() && self.opening.is_none() {
            let waiting = |row: &Row| row.size == Sized::Waiting && row.kind != Kind::More;
            let next = shown
                .clone()
                .find(|&i| self.rows.get(i).is_some_and(waiting))
                .or_else(|| self.rows.iter().position(waiting));
            let Some(index) = next else { return };
            let Some(row) = self.rows.get_mut(index) else { return };
            let Some(scope) = row.scope() else { return };
            row.size = Sized::Asking;
            self.ask(Job::Size { row: index, scope });
        }
    }

    /// Until the person moves it, the cursor is on the first row not known to be empty.
    fn settle_cursor(&mut self) {
        if self.pinned {
            return;
        }
        let first = self.rows.iter().position(|row| row.kind != Kind::More && !row.is_empty());
        self.cursor = first.unwrap_or(0);
    }

    /// Nothing but commits to review: the working tree and the branch are known empty.
    fn nothing_to_review(&self) -> bool {
        self.listed
            && self
                .rows
                .iter()
                .filter(|row| !matches!(row.kind, Kind::Commit(_) | Kind::More))
                .all(Row::is_empty)
    }

    /// Commits are listed.
    fn has_commits(&self) -> bool {
        self.rows.iter().any(|row| matches!(row.kind, Kind::Commit(_)))
    }

    fn move_to(&mut self, index: usize) {
        self.pinned = true;
        self.cursor = index.min(self.rows.len().saturating_sub(1));
    }

    pub(crate) fn key(&mut self, key: &KeyEvent) -> Outcome {
        let action = self.keys.action(key, keys::PICK_CHANGES);
        if self.help {
            match action {
                Some(Action::Quit | Action::Interrupt) => return Outcome::Quit,
                Some(_) => self.help = false,
                None => {}
            }
            return Outcome::Stay;
        }
        let page = self.geometry.visible.saturating_sub(1).max(1);
        let last = self.rows.len().saturating_sub(1);
        match action {
            Some(Action::Down) => self.move_to(self.cursor + 1),
            Some(Action::Up) => self.move_to(self.cursor.saturating_sub(1)),
            Some(Action::PageDown) => self.move_to(self.cursor + page),
            Some(Action::PageUp) => self.move_to(self.cursor.saturating_sub(page)),
            Some(Action::Top) => self.move_to(0),
            Some(Action::Bottom) => self.move_to(last),
            Some(Action::Confirm) => return self.choose(self.cursor),
            Some(Action::Reload) => self.reload(),
            Some(Action::Help) => self.help = true,
            Some(Action::Back | Action::PickChanges) => {
                if self.over.is_some() {
                    return Outcome::Close;
                }
                self.status = self.keys.labelled(Action::Quit, "quits");
            }
            Some(Action::Quit | Action::Interrupt) => return Outcome::Quit,
            _ => {}
        }
        Outcome::Stay
    }

    pub(crate) fn mouse(&mut self, mouse: MouseEvent) -> Outcome {
        // The keys box takes the mouse: a click closes it, as the review's own help does, and
        // nothing under it is opened or scrolled.
        if self.help {
            if matches!(mouse.kind, MouseEventKind::Down(_)) {
                self.help = false;
            }
            return Outcome::Stay;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self.geometry.rows.iter().find(|(rect, _)| {
                    mouse.row == rect.y && mouse.column >= rect.x && mouse.column < rect.right()
                });
                if let Some(&(_, index)) = hit {
                    self.move_to(index);
                    return self.choose(index);
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let visible = self.geometry.visible.max(1);
                let most = self.rows.len().saturating_sub(visible);
                self.scroll = if mouse.kind == MouseEventKind::ScrollDown {
                    (self.scroll + 3).min(most)
                } else {
                    self.scroll.saturating_sub(3)
                };
                self.pinned = true;
                self.cursor = self
                    .cursor
                    .clamp(self.scroll, (self.scroll + visible).saturating_sub(1))
                    .min(self.rows.len().saturating_sub(1));
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// Enter on a row: review it, list older commits, or say it has nothing to review. The
    /// row of the review it was opened over just goes back to that review, untouched.
    fn choose(&mut self, index: usize) -> Outcome {
        let Some(row) = self.rows.get(index) else { return Outcome::Stay };
        if row.kind == Kind::More {
            if self.more == More::Ready {
                self.more = More::Loading;
                let skip = self.rows.iter().filter(|row| matches!(row.kind, Kind::Commit(_))).count();
                self.ask(Job::List { skip, count: PAGE });
            }
            return Outcome::Stay;
        }
        if self.over.is_some() && row.scope() == self.over {
            return Outcome::Close;
        }
        let name = row.name();
        if row.is_empty() {
            self.status = Some(format!("{name} has no changes"));
            return Outcome::Stay;
        }
        let Some(scope) = row.scope() else { return Outcome::Stay };
        if self.opening == Some(index) {
            return Outcome::Stay;
        }
        // The capture takes the room the sizes had; they are asked again if it fails.
        self.work.stop_captures();
        self.work.stop_sizes();
        for row in &mut self.rows {
            if row.size == Sized::Asking {
                row.size = Sized::Waiting;
            }
        }
        self.opening = Some(index);
        self.status = Some(format!("opening {name}\u{2026}"));
        self.ask(Job::Capture { row: index, scope });
        Outcome::Stay
    }

    /// `r`: list the commits again and work every size out afresh, keeping the cursor's row.
    /// What still runs from before is stopped; until it has ended it counts against the room
    /// the new round has.
    fn reload(&mut self) {
        self.generation += 1;
        self.work.stop_all();
        self.opening = None;
        self.keep = self.rows.get(self.cursor).and_then(Row::scope).filter(|_| self.pinned);
        let count = self.rows.iter().filter(|row| matches!(row.kind, Kind::Commit(_))).count().max(PAGE);
        // The commits are listed again; until then only the working tree's rows show, so a
        // size can never land on a row that moved.
        self.rows = [Kind::Uncommitted, Kind::Unstaged, Kind::Staged].map(Row::new).into();
        self.listed = false;
        self.more = More::None;
        self.ask(Job::List { skip: 0, count });
        self.status = Some(RELOADING.into());
        self.ask_sizes();
    }
}
