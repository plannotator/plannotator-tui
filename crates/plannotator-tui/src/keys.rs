//! Key bindings: every key the viewer acts on, by action name, rebindable under `[keys]`.
//!
//! An action keeps its name wherever it applies, so `down = ["n", "Down"]` moves down in the
//! document, the tree, the notes and every picker. A context lists the actions one mode
//! dispatches, in the order it tries them; two actions that read the keyboard at the same
//! time may not share a key, and a config that binds one twice is an error that names both.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Action {
    Up,
    Down,
    Left,
    Right,
    WordForward,
    WordBack,
    WordEnd,
    LineStart,
    LineEnd,
    PageDown,
    PageUp,
    Top,
    Bottom,
    Confirm,
    Back,
    Quit,
    Interrupt,
    CycleFocus,
    Send,
    SendAndClose,
    Approve,
    ToggleTree,
    Reload,
    OpenInEditor,
    PickMessage,
    QuickOpen,
    /// Choose which agent Send goes to: a Pi session or a Herdr agent (`docs/spec-pi-link.md`).
    PickTarget,
    ReviewMenu,
    ResendAll,
    FinishReview,
    UndoFinish,
    Archive,
    ToggleHidden,
    Edit,
    Remove,
    AttachImage,
    RemoveImage,
    Visual,
    VisualLine,
    MoveCursor,
    Comment,
    ClearBlock,
    ClearAll,
    LooksGood,
    Delete,
    /// Copy the waiting selection to the clipboard.
    #[serde(rename = "copy")]
    CopySelection,
    Yes,
    No,
    // Code review (`plannotator-tui review`).
    /// Suggest replacement code for the selected new-side lines.
    Suggest,
    /// Comment on the whole file under the cursor.
    FileComment,
    /// Comment on the whole change: a review-level note on no file.
    ChangeComment,
    NextHunk,
    PreviousHunk,
    NextFile,
    PreviousFile,
    /// Split or unified diff.
    ToggleLayout,
    ToggleWrap,
    /// Show or hide the diff's file list.
    ToggleFiles,
    /// Mark the file under the cursor viewed, or not (`docs/spec-review-progress.md`).
    ToggleViewed,
    /// The next or previous file not marked viewed, wrapping around.
    NextUnviewed,
    PreviousUnviewed,
    Help,
    /// The reading column, by eight columns.
    NarrowColumn,
    WidenColumn,
    /// The whole file under the cursor as it is now, or back to the diff.
    WholeFile,
    /// The whole file as it was before the change (its base), or back to the diff.
    WholeBase,
    /// Hide the notes rail, or show it again, in the document and in the code review.
    ToggleNotes,
    /// Find text in the open document.
    Search,
    /// The next or previous match of the last search, wrapping at either end.
    NextMatch,
    PreviousMatch,
    /// Search for the identifier on the line and go to its definition.
    GoToDefinition,
    /// Back to the place before a jump, and forward again.
    JumpBack,
    JumpForward,
    /// Post the notes to the pull request on GitHub as one review, after a summary.
    PostToGithub,
}

use Action::{
    Approve, Archive, AttachImage, Back, Bottom, ClearAll, ClearBlock, Comment, Confirm, CopySelection,
    CycleFocus, Delete, Down, Edit, FinishReview, Interrupt, Left, LineEnd, LineStart, LooksGood, MoveCursor,
    No, OpenInEditor, PageDown, PageUp, PickMessage, PickTarget, QuickOpen, Quit, Reload, Remove,
    RemoveImage, ResendAll, ReviewMenu, Right, Send, SendAndClose, ToggleHidden, ToggleTree, Top, UndoFinish,
    Up, Visual, VisualLine, WordBack, WordEnd, WordForward, Yes,
};
use Action::{
    ChangeComment, FileComment, Help, NextFile, NextHunk, NextMatch, NextUnviewed, PreviousFile,
    PreviousHunk, PreviousMatch, PreviousUnviewed, Search, Suggest, ToggleFiles, ToggleLayout, ToggleViewed,
    ToggleWrap, WholeBase, WholeFile,
};
use Action::{GoToDefinition, JumpBack, JumpForward, PostToGithub};
use Action::{NarrowColumn, ToggleNotes, WidenColumn};

/// Browsing, before the focused pane gets the key.
pub(crate) const GLOBAL: &[Action] = &[
    Quit,
    Interrupt,
    CycleFocus,
    Send,
    SendAndClose,
    Approve,
    ReviewMenu,
    ResendAll,
    FinishReview,
    UndoFinish,
    Archive,
    ToggleTree,
    Reload,
    OpenInEditor,
    QuickOpen,
    PickMessage,
    PickTarget,
    ToggleNotes,
];
pub(crate) const TREE: &[Action] = &[Down, Up, Confirm, Right, ToggleHidden, Back];
pub(crate) const RAIL: &[Action] = &[Down, Up, Confirm, Edit, Remove, AttachImage, RemoveImage, Back];
/// `attach-image` and `remove-image` act on the selected block's note here.
pub(crate) const DOCUMENT: &[Action] = &[
    Back,
    Visual,
    VisualLine,
    MoveCursor,
    Down,
    Up,
    Left,
    Right,
    PageDown,
    PageUp,
    Top,
    Bottom,
    Comment,
    Confirm,
    ClearBlock,
    ClearAll,
    AttachImage,
    RemoveImage,
    NarrowColumn,
    WidenColumn,
    Search,
    NextMatch,
    PreviousMatch,
];
/// Marking the selected block, tried only when no `DOCUMENT` action claims the key. They sit
/// outside `SHARED` so a config that gave `a` or `d` to a document action keeps loading.
pub(crate) const BLOCK_MARKS: &[Action] = &[LooksGood, Delete];
/// Cursor motions, in visual mode and while moving the cursor (`move-cursor`).
pub(crate) const MOTION: &[Action] =
    &[Left, Right, Down, Up, WordForward, WordBack, WordEnd, LineStart, LineEnd];
/// While a keyboard selection grows; any other key is a motion.
pub(crate) const VISUAL: &[Action] = &[Back, Confirm, Visual, VisualLine];
/// While a finished selection waits for its kind.
pub(crate) const TOOLBAR: &[Action] = &[LooksGood, Comment, Delete, CopySelection];
pub(crate) const CONFIRM_QUIT: &[Action] = &[Yes, Confirm, No, Back];
/// Before `clear-all` empties the document. Only `yes` clears: `confirm` would let a slip
/// followed by Enter wipe every note.
pub(crate) const CONFIRM_CLEAR: &[Action] = &[Yes, No, Back];
pub(crate) const MENU: &[Action] =
    &[Down, Up, Confirm, Back, ReviewMenu, Interrupt, ResendAll, FinishReview, UndoFinish, Archive];
pub(crate) const PICK: &[Action] = &[Down, Up, PageDown, PageUp, Top, Bottom, Confirm, Back, Quit];
/// The agent picker: up and down as in the message picker; `pick-target` again closes it.
pub(crate) const PICK_TARGET: &[Action] = &[Down, Up, Confirm, Back, PickTarget, Interrupt];
pub(crate) const ARCHIVE: &[Action] = &[Down, Up, Confirm, Back, Archive, Quit, Interrupt];

/// A code review, before the focused pane gets the key.
pub(crate) const REVIEW_GLOBAL: &[Action] = &[
    Quit,
    Interrupt,
    CycleFocus,
    Send,
    SendAndClose,
    ReviewMenu,
    ResendAll,
    FinishReview,
    UndoFinish,
    ChangeComment,
    PostToGithub,
    Reload,
    PickTarget,
    Help,
    ToggleNotes,
];
/// The diff. `left` and `right` pick the old and the new side.
pub(crate) const DIFF: &[Action] = &[
    Back,
    Visual,
    VisualLine,
    Comment,
    Suggest,
    FileComment,
    Down,
    Up,
    PageDown,
    PageUp,
    Top,
    Bottom,
    NextHunk,
    PreviousHunk,
    NextFile,
    PreviousFile,
    Left,
    Right,
    ToggleLayout,
    ToggleWrap,
    ToggleFiles,
    ToggleViewed,
    NextUnviewed,
    PreviousUnviewed,
    WholeFile,
    WholeBase,
    GoToDefinition,
    JumpBack,
    JumpForward,
    Confirm,
];
/// The code review's notes rail.
pub(crate) const NOTES: &[Action] = &[Down, Up, Confirm, Edit, Remove, Back];
pub(crate) const REVIEW_MENU: &[Action] = &[
    Down,
    Up,
    Confirm,
    Back,
    ReviewMenu,
    Interrupt,
    ChangeComment,
    PostToGithub,
    ResendAll,
    FinishReview,
    UndoFinish,
];
/// The summary before a post to GitHub: `left`/`right` choose the verdict, `confirm` posts,
/// `back` cancels.
pub(crate) const POST_REVIEW: &[Action] = &[Left, Right, Confirm, Back, Interrupt];
/// The code review's key help: the motions scroll it when it does not fit; the rest close it.
pub(crate) const HELP: &[Action] = &[Back, Help, Quit, Interrupt, Down, Up, PageDown, PageUp];

/// Contexts that read the same keypress: the global keys come first while browsing, and
/// roaming tries the motions before the document's own keys.
const SHARED: &[&[&[Action]]] = &[
    &[GLOBAL, TREE],
    &[GLOBAL, RAIL],
    &[GLOBAL, DOCUMENT],
    &[GLOBAL, MOTION],
    &[GLOBAL, VISUAL, MOTION],
    &[GLOBAL, TOOLBAR],
    &[CONFIRM_QUIT],
    &[CONFIRM_CLEAR],
    &[MENU],
    &[PICK],
    &[ARCHIVE],
    &[REVIEW_GLOBAL, DIFF],
    &[REVIEW_GLOBAL, NOTES],
    &[REVIEW_MENU],
    &[POST_REVIEW],
    &[HELP],
];

/// Named keys, spelled as the config file writes them.
const NAMED: [(&str, KeyCode); 13] = [
    ("Up", KeyCode::Up),
    ("Down", KeyCode::Down),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("Enter", KeyCode::Enter),
    ("Esc", KeyCode::Esc),
    ("Tab", KeyCode::Tab),
    ("Delete", KeyCode::Delete),
    ("Backspace", KeyCode::Backspace),
];

/// One binding: a character or a named key, optionally with Ctrl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Key {
    code: KeyCode,
    ctrl: bool,
}

const fn ch(c: char) -> Key {
    Key { code: KeyCode::Char(c), ctrl: false }
}

const fn ctrl(c: char) -> Key {
    Key { code: KeyCode::Char(c), ctrl: true }
}

const fn named(code: KeyCode) -> Key {
    Key { code, ctrl: false }
}

impl Key {
    /// A Ctrl binding needs Ctrl alone. A plain binding allows Shift, as a shifted letter
    /// arrives with Shift set, but not Ctrl; Ctrl with Alt still counts as plain, because
    /// Windows reports `AltGr` that way and `AltGr` types ordinary characters.
    /// A terminal that reports Shift+e as `e` with Shift (a kitty `CSI 101;2u`) means `E`.
    fn matches(self, key: &KeyEvent) -> bool {
        let ctrl_or_alt = key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT);
        let held_ctrl = ctrl_or_alt == KeyModifiers::CONTROL;
        let code = match key.code {
            KeyCode::Char(c) if c.is_ascii_lowercase() && key.modifiers.contains(KeyModifiers::SHIFT) => {
                KeyCode::Char(c.to_ascii_uppercase())
            }
            code => code,
        };
        self.code == code && if self.ctrl { key.modifiers == KeyModifiers::CONTROL } else { !held_ctrl }
    }

    /// How the in-app hints spell the key: `j`, `G`, `enter`, `ctrl+d`.
    fn hint(self) -> String {
        match self.code {
            KeyCode::Char(_) => self.to_string(),
            _ => self.to_string().to_lowercase(),
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("ctrl+")?;
        }
        match (self.code, NAMED.iter().find(|(_, code)| *code == self.code)) {
            (_, Some((name, _))) => f.write_str(name),
            (KeyCode::Char(c), None) => write!(f, "{c}"),
            (other, None) => write!(f, "{other:?}"),
        }
    }
}

impl FromStr for Key {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let (ctrl, rest) = s.strip_prefix("ctrl+").map_or((false, s), |rest| (true, rest));
        if let Some((_, code)) = NAMED.iter().find(|(name, _)| name.eq_ignore_ascii_case(rest)) {
            return Ok(Self { code: *code, ctrl });
        }
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Ok(Self { code: KeyCode::Char(c), ctrl });
        }
        let names: Vec<&str> = NAMED.iter().map(|(name, _)| *name).collect();
        anyhow::bail!(
            "unknown key {s:?}; expected one character, ctrl+<character>, or one of {}",
            names.join(", ")
        )
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

const DEFAULTS: &[(Action, &[Key])] = &[
    (Up, &[ch('k'), named(KeyCode::Up)]),
    (Down, &[ch('j'), named(KeyCode::Down)]),
    (Left, &[ch('h'), named(KeyCode::Left)]),
    (Right, &[ch('l'), named(KeyCode::Right)]),
    (WordForward, &[ch('w')]),
    (WordBack, &[ch('b')]),
    (WordEnd, &[ch('e')]),
    (LineStart, &[ch('0'), named(KeyCode::Home)]),
    (LineEnd, &[ch('$'), named(KeyCode::End)]),
    (PageDown, &[ctrl('d'), named(KeyCode::PageDown)]),
    (PageUp, &[ctrl('u'), named(KeyCode::PageUp)]),
    (Top, &[ch('g'), named(KeyCode::Home)]),
    (Bottom, &[ch('G'), named(KeyCode::End)]),
    (Confirm, &[named(KeyCode::Enter)]),
    (Back, &[named(KeyCode::Esc)]),
    (Quit, &[ch('q')]),
    (Interrupt, &[ctrl('c')]),
    (CycleFocus, &[named(KeyCode::Tab)]),
    (Send, &[ch('E')]),
    (SendAndClose, &[ch('S')]),
    (Approve, &[ch('A')]),
    (ToggleTree, &[ch('t')]),
    (Reload, &[ch('r')]),
    (OpenInEditor, &[ch('o')]),
    (PickMessage, &[ch('p')]),
    (QuickOpen, &[ctrl('p')]),
    (PickTarget, &[ch('P')]),
    (ReviewMenu, &[ch('m')]),
    (ResendAll, &[ch('R')]),
    (FinishReview, &[ch('F')]),
    (UndoFinish, &[ch('U')]),
    (Archive, &[ch('H')]),
    (ToggleHidden, &[ch('.')]),
    (Edit, &[ch('e')]),
    (Remove, &[ch('x'), named(KeyCode::Delete)]),
    (AttachImage, &[ch('I')]),
    (RemoveImage, &[ch('D')]),
    (Visual, &[ch('v')]),
    (VisualLine, &[ch('V')]),
    (MoveCursor, &[ch('i')]),
    (Comment, &[ch('c')]),
    (ClearBlock, &[ch('x')]),
    (ClearAll, &[ch('X')]),
    (LooksGood, &[ch('a')]),
    (Delete, &[ch('d')]),
    (CopySelection, &[ch('y')]),
    (Yes, &[ch('y'), ch('Y')]),
    (No, &[ch('n'), ch('N')]),
    (Suggest, &[ch('s')]),
    (FileComment, &[ch('C')]),
    (ChangeComment, &[ch('M')]),
    (NextHunk, &[ch(']')]),
    (PreviousHunk, &[ch('[')]),
    (NextFile, &[ch('n')]),
    (PreviousFile, &[ch('N')]),
    (ToggleLayout, &[ch('u')]),
    (ToggleWrap, &[ch('w')]),
    (ToggleFiles, &[ch('f')]),
    (ToggleViewed, &[ch('x')]),
    // `n` with Ctrl: the next file, skipping viewed ones. Ctrl+Shift+N reaches most
    // terminals as Ctrl+N, so the backward jump has no default key.
    (NextUnviewed, &[ctrl('n')]),
    (PreviousUnviewed, &[]),
    (Help, &[ch('?')]),
    (NarrowColumn, &[ch('<')]),
    (WidenColumn, &[ch('>')]),
    (WholeFile, &[ch('o')]),
    (WholeBase, &[ch('O')]),
    (ToggleNotes, &[ch('z')]),
    (Search, &[ch('/')]),
    (NextMatch, &[ch('n')]),
    (PreviousMatch, &[ch('N')]),
    (GoToDefinition, &[ch('d')]),
    (JumpBack, &[ctrl('o')]),
    // Ctrl+I is Tab in terminals that do not tell them apart; there `cycle-focus` gets it.
    (JumpForward, &[ctrl('i')]),
    // G for GitHub; free in every code review context.
    (PostToGithub, &[ctrl('g')]),
];

/// Actions added after configs were written: `approve` (live only in a gated review), `copy`
/// (on a waiting selection), `word-end`, `visual-line`, the code review's own, the reading
/// column's, `toggle-notes` and search.
const LATE: &[Action] = &[
    Approve,
    CopySelection,
    WordEnd,
    VisualLine,
    Suggest,
    FileComment,
    NextHunk,
    PreviousHunk,
    NextFile,
    PreviousFile,
    ToggleLayout,
    ToggleWrap,
    ToggleFiles,
    ToggleViewed,
    NextUnviewed,
    PreviousUnviewed,
    Help,
    ChangeComment,
    NarrowColumn,
    WidenColumn,
    WholeFile,
    WholeBase,
    ToggleNotes,
    Search,
    NextMatch,
    PreviousMatch,
    GoToDefinition,
    JumpBack,
    JumpForward,
    PostToGithub,
];

/// The keys bound to each action. A `[keys]` entry replaces that action's keys and leaves the
/// others at their defaults; an empty list unbinds the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Keymap(BTreeMap<Action, Vec<Key>>);

impl Default for Keymap {
    fn default() -> Self {
        Self(DEFAULTS.iter().map(|(action, keys)| (*action, keys.to_vec())).collect())
    }
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut keymap = Self::default();
        let entries = BTreeMap::<Action, Vec<Key>>::deserialize(deserializer)?;
        let user_keys: Vec<Key> = entries.values().flatten().copied().collect();
        let unset: Vec<Action> = LATE.iter().copied().filter(|late| !entries.contains_key(late)).collect();
        keymap.0.extend(entries);
        // Their default keys yield to a config that gives those keys to another action, so
        // a config that loaded before they existed still loads.
        for late in unset {
            let taken =
                keymap.0.get(&late).is_some_and(|keys| keys.iter().any(|key| user_keys.contains(key)));
            if taken && keymap.check().is_err() {
                keymap.0.insert(late, Vec::new());
            }
        }
        keymap.check().map_err(serde::de::Error::custom)?;
        Ok(keymap)
    }
}

impl Keymap {
    /// The first action in `context` bound to `key`.
    pub(crate) fn action(&self, key: &KeyEvent, context: &[Action]) -> Option<Action> {
        context.iter().copied().find(|&action| self.is(action, key))
    }

    pub(crate) fn is(&self, action: Action, key: &KeyEvent) -> bool {
        self.keys(action).iter().any(|k| k.matches(key))
    }

    /// The action's first key as the hints spell it; empty when it is unbound.
    pub(crate) fn hint(&self, action: Action) -> String {
        self.keys(action).first().map(|key| key.hint()).unwrap_or_default()
    }

    /// `key label` for a hint line, or nothing when the action is unbound, so an unbound
    /// action leaves no bare label behind.
    pub(crate) fn labelled(&self, action: Action, label: &str) -> Option<String> {
        let key = self.hint(action);
        (!key.is_empty()).then(|| format!("{key} {label}"))
    }

    /// The four cursor directions run together the way the hints show them: `hjkl`.
    pub(crate) fn directions_hint(&self) -> String {
        [Left, Down, Up, Right].map(|action| self.hint(action)).concat()
    }

    fn keys(&self, action: Action) -> &[Key] {
        self.0.get(&action).map_or(&[], Vec::as_slice)
    }

    /// No key may mean two actions where both read the same keypress.
    fn check(&self) -> Result<(), String> {
        for contexts in SHARED {
            let mut seen: Vec<(Key, Action)> = Vec::new();
            for &action in contexts.iter().copied().flatten() {
                for &key in self.keys(action) {
                    match seen.iter().find(|(k, _)| *k == key) {
                        Some(&(_, other)) if other != action => {
                            return Err(format!(
                                "key {:?} is bound to both {} and {}",
                                key.to_string(),
                                other.as_str(),
                                action.as_str()
                            ));
                        }
                        Some(_) => {}
                        None => seen.push((key, action)),
                    }
                }
            }
        }
        Ok(())
    }
}

impl Action {
    /// The action's name as `[keys]` writes it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::WordForward => "word-forward",
            Self::WordBack => "word-back",
            Self::WordEnd => "word-end",
            Self::LineStart => "line-start",
            Self::LineEnd => "line-end",
            Self::PageDown => "page-down",
            Self::PageUp => "page-up",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Confirm => "confirm",
            Self::Back => "back",
            Self::Quit => "quit",
            Self::Interrupt => "interrupt",
            Self::CycleFocus => "cycle-focus",
            Self::Send => "send",
            Self::SendAndClose => "send-and-close",
            Self::Approve => "approve",
            Self::ToggleTree => "toggle-tree",
            Self::Reload => "reload",
            Self::OpenInEditor => "open-in-editor",
            Self::PickMessage => "pick-message",
            Self::QuickOpen => "quick-open",
            Self::PickTarget => "pick-target",
            Self::ReviewMenu => "review-menu",
            Self::ResendAll => "resend-all",
            Self::FinishReview => "finish-review",
            Self::UndoFinish => "undo-finish",
            Self::Archive => "archive",
            Self::ToggleHidden => "toggle-hidden",
            Self::Edit => "edit",
            Self::Remove => "remove",
            Self::AttachImage => "attach-image",
            Self::RemoveImage => "remove-image",
            Self::Visual => "visual",
            Self::VisualLine => "visual-line",
            Self::MoveCursor => "move-cursor",
            Self::Comment => "comment",
            Self::ClearBlock => "clear-block",
            Self::ClearAll => "clear-all",
            Self::LooksGood => "looks-good",
            Self::Delete => "delete",
            Self::CopySelection => "copy",
            Self::Yes => "yes",
            Self::No => "no",
            Self::Suggest => "suggest",
            Self::FileComment => "file-comment",
            Self::ChangeComment => "change-comment",
            Self::NextHunk => "next-hunk",
            Self::PreviousHunk => "previous-hunk",
            Self::NextFile => "next-file",
            Self::PreviousFile => "previous-file",
            Self::ToggleLayout => "toggle-layout",
            Self::ToggleWrap => "toggle-wrap",
            Self::ToggleFiles => "toggle-files",
            Self::ToggleViewed => "toggle-viewed",
            Self::NextUnviewed => "next-unviewed",
            Self::PreviousUnviewed => "previous-unviewed",
            Self::Help => "help",
            Self::NarrowColumn => "narrow-column",
            Self::WidenColumn => "widen-column",
            Self::WholeFile => "whole-file",
            Self::WholeBase => "whole-base",
            Self::ToggleNotes => "toggle-notes",
            Self::Search => "search",
            Self::NextMatch => "next-match",
            Self::PreviousMatch => "previous-match",
            Self::GoToDefinition => "go-to-definition",
            Self::JumpBack => "jump-back",
            Self::JumpForward => "jump-forward",
            Self::PostToGithub => "post-to-github",
        }
    }
}

#[cfg(test)]
mod tests;
