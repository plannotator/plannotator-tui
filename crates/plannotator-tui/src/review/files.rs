//! The changed files, on the left of the diff (`f`): a folder tree whose width fits its
//! names, and a filter by name (`/`), by kind (`t`) and by viewed mark (`H`).
//!
//! While a filter is on, the diff shows only the files it lets through, in the list's
//! order, so `n`/`N`, `]`/`[` and `ctrl+n` move among them. Notes are never filtered: the
//! rail keeps them all, Send sends them all, and the list says how many unsent ones sit in
//! hidden files.
//!
//! - [`kind`]: what kind of file a path is. [`filter`]: what passes.
//! - [`tree`]: the rows. [`fit`]: the width, and cutting long names.
//! - `shown`: what the diff shows, and the kinds; `attrs`: `.gitattributes`.
//! - `nav`: moving the diff from file to file.
//! - `draw` and `row`: the list on screen; `input`: its keys; `kinds`: the kind picker.

mod attrs;
mod draw;
pub(super) mod filter;
pub(super) mod fit;
mod input;
pub(super) mod kind;
mod kinds;
mod nav;
mod row;
mod shown;
pub(super) mod tree;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use super::Screen;
use draw::Listing;
use filter::Filter;
use kind::Kind;

/// The file list's state.
#[derive(Debug, Default)]
pub(super) struct FileList {
    /// Closed with `toggle-files`; `toggle-files` opens it again.
    pub(super) closed: bool,
    /// The filter, as typed so far.
    pub(super) filter: Filter,
    /// Where the focus was when `/` started typing, to go back to on `Esc`.
    filter_from: Option<super::Focus>,
    /// The paths the diff shows, in the list's order: the filter as last applied.
    shown: Vec<String>,
    /// The same paths, to look up.
    shown_set: HashSet<String>,
    /// Lines added and removed in the files shown.
    shown_totals: (u32, u32),
    /// Each capture file's kind, in capture order.
    kinds: Vec<Kind>,
    /// Git's `linguist-generated` answer for the capture, until it comes, with the
    /// version it is for.
    attributes: Option<(u64, std::sync::mpsc::Receiver<HashMap<String, bool>>)>,
    /// Each capture file's index, by path; two for a path the patch names twice.
    index: HashMap<String, Vec<usize>>,
    /// The capture's files in the list's order, every folder open.
    order: Vec<usize>,
    /// Folders closed in the list, by row key.
    collapsed: BTreeSet<String>,
    /// The row under the list's cursor, by key.
    pub(super) cursor: Option<String>,
    /// First list row drawn.
    scroll: usize,
    /// The cursor the list last scrolled to show.
    scrolled_to: Option<String>,
    /// The file the diff's cursor is in.
    pub(super) current: Option<String>,
    /// The width chosen with `narrow-files` and `widen-files`.
    chosen_width: Option<u16>,
    /// The width drawn last frame; 0 when hidden.
    pub(super) drawn_width: u16,
    /// Whether the last frame was too narrow for the list even with the whole width.
    pub(super) no_room: bool,
    /// A file to show once the diff, replaced for a new filter, has it.
    pending: Option<String>,
    /// Counts up when the capture, its kinds or the viewed marks change, so the rows built
    /// for the last frame are known to be stale.
    version: u64,
    /// The rows last built, for the filter as typed, and what they were built from.
    rows: RefCell<Option<(Built, Rc<Listing>)>>,
    /// Counts up when the files shown change.
    shown_version: u64,
    /// The width the rows of the files shown need, and the versions it was worked out for.
    shown_width: Cell<Option<(u64, u64, u16)>>,
}

/// What a listing was built from: when all of it is the same, so are the rows.
#[derive(Debug)]
struct Built {
    version: u64,
    filter: Filter,
    collapsed: BTreeSet<String>,
}

impl FileList {
    pub(super) fn chosen_width(&self) -> Option<u16> {
        self.chosen_width
    }

    /// Set the list's width (`None`: fit its names again), and show it.
    pub(super) fn set_width(&mut self, width: Option<u16>) {
        self.chosen_width = width;
        self.closed = false;
    }

    /// The paths the diff shows, in the list's order.
    pub(super) fn shown(&self) -> &[String] {
        &self.shown
    }

    pub(super) fn is_shown(&self, path: &str) -> bool {
        self.shown_set.contains(path)
    }

    /// Lines added and removed in the files the diff shows.
    pub(super) fn shown_totals(&self) -> (u32, u32) {
        self.shown_totals
    }

    /// A viewed mark changed: rows built before are stale.
    pub(super) fn viewed_changed(&mut self) {
        self.version += 1;
    }

    /// The list was drawn `width` wide (0: hidden). A chosen width the room could not give
    /// comes down to what it gave, so the next `narrow-files` narrows what is seen.
    pub(super) fn drawn(&mut self, width: u16) {
        self.drawn_width = width;
        if width > 0 {
            self.chosen_width = self.chosen_width.map(|chosen| chosen.min(width));
        }
    }

    /// Whether git's answer about `.gitattributes` is in (or will not come).
    #[cfg(test)]
    pub(super) fn attributes_read(&self) -> bool {
        self.attributes.is_none()
    }

    /// The kind of the capture's file at `path`.
    #[cfg(test)]
    pub(super) fn kind_of(&self, path: &str) -> Option<Kind> {
        self.index.get(path).and_then(|v| v.first()).and_then(|&i| self.kinds.get(i)).copied()
    }

    /// After the diff was replaced for a new capture: come back to the file it was on.
    pub(super) fn retarget(&mut self) {
        self.pending = self.current.clone().filter(|p| self.shown_set.contains(p));
    }
}

impl Screen {
    /// Whether the list wants to show: not closed, and more than one file or a filter.
    pub(super) fn files_wanted(&self) -> bool {
        !self.files.closed
            && (self.capture.files.len() > 1
                || self.focus == super::Focus::Files
                || self.files.filter.is_active())
    }

    /// `Files · 340`, or `Files · 12 of 340` while a filter hides some.
    pub(super) fn files_heading(&self, listed: usize) -> String {
        let total = self.capture.files.len();
        if listed == total {
            format!("Files \u{b7} {total}")
        } else {
            format!("Files \u{b7} {listed} of {total}")
        }
    }
}
