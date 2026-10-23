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
    ToggleTree,
    Reload,
    PickMessage,
    ReviewMenu,
    ResendAll,
    FinishReview,
    UndoFinish,
    Archive,
    ToggleHidden,
    Edit,
    Remove,
    Visual,
    MoveCursor,
    Comment,
    ClearBlock,
    LooksGood,
    Delete,
    Yes,
    No,
}

use Action::{
    Archive, Back, Bottom, ClearBlock, Comment, Confirm, CycleFocus, Delete, Down, Edit, FinishReview,
    Interrupt, Left, LineEnd, LineStart, LooksGood, MoveCursor, No, PageDown, PageUp, PickMessage, Quit,
    Reload, Remove, ResendAll, ReviewMenu, Right, Send, SendAndClose, ToggleHidden, ToggleTree, Top,
    UndoFinish, Up, Visual, WordBack, WordForward, Yes,
};

/// Browsing, before the focused pane gets the key.
pub(crate) const GLOBAL: &[Action] = &[
    Quit,
    Interrupt,
    CycleFocus,
    Send,
    SendAndClose,
    ReviewMenu,
    ResendAll,
    FinishReview,
    UndoFinish,
    Archive,
    ToggleTree,
    Reload,
    PickMessage,
];
pub(crate) const TREE: &[Action] = &[Down, Up, Confirm, Right, ToggleHidden, Back];
pub(crate) const RAIL: &[Action] = &[Down, Up, Confirm, Edit, Remove, Back];
pub(crate) const DOCUMENT: &[Action] = &[
    Back, Visual, MoveCursor, Down, Up, Left, Right, PageDown, PageUp, Top, Bottom, Comment, Confirm,
    ClearBlock,
];
/// Cursor motions, in visual mode and while moving the cursor (`move-cursor`).
pub(crate) const MOTION: &[Action] = &[Left, Right, Down, Up, WordForward, WordBack, LineStart, LineEnd];
/// While a keyboard selection grows; any other key is a motion.
pub(crate) const VISUAL: &[Action] = &[Back, Confirm, Visual];
/// While a finished selection waits for its kind.
pub(crate) const TOOLBAR: &[Action] = &[LooksGood, Comment, Delete];
pub(crate) const CONFIRM_QUIT: &[Action] = &[Yes, Confirm, No, Back];
pub(crate) const MENU: &[Action] =
    &[Down, Up, Confirm, Back, ReviewMenu, Interrupt, ResendAll, FinishReview, UndoFinish, Archive];
pub(crate) const PICK: &[Action] = &[Down, Up, Confirm, Back, Quit];
pub(crate) const ARCHIVE: &[Action] = &[Down, Up, Confirm, Back, Archive, Quit, Interrupt];

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
    &[MENU],
    &[PICK],
    &[ARCHIVE],
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
    /// A plain binding ignores modifiers, as a shifted letter arrives with Shift set; a Ctrl
    /// binding needs Ctrl alone.
    fn matches(self, key: &KeyEvent) -> bool {
        self.code == key.code && (!self.ctrl || key.modifiers == KeyModifiers::CONTROL)
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
    (ToggleTree, &[ch('t')]),
    (Reload, &[ch('r')]),
    (PickMessage, &[ch('p')]),
    (ReviewMenu, &[ch('m')]),
    (ResendAll, &[ch('R')]),
    (FinishReview, &[ch('F')]),
    (UndoFinish, &[ch('U')]),
    (Archive, &[ch('H')]),
    (ToggleHidden, &[ch('.')]),
    (Edit, &[ch('e')]),
    (Remove, &[ch('x'), named(KeyCode::Delete)]),
    (Visual, &[ch('v')]),
    (MoveCursor, &[ch('i')]),
    (Comment, &[ch('c')]),
    (ClearBlock, &[ch('x')]),
    (LooksGood, &[ch('a')]),
    (Delete, &[ch('d')]),
    (Yes, &[ch('y'), ch('Y')]),
    (No, &[ch('n'), ch('N')]),
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
        keymap.0.extend(BTreeMap::<Action, Vec<Key>>::deserialize(deserializer)?);
        keymap.check().map_err(serde::de::Error::custom)?;
        Ok(keymap)
    }
}

impl Keymap {
    /// The first action in `context` bound to `key`. A plain binding also matches the key
    /// with Ctrl held, so an exact Ctrl binding is tried first: `page-up = ["ctrl+k"]` must
    /// not lose `ctrl+k` to `up`'s plain `k` just because `up` comes first.
    pub(crate) fn action(&self, key: &KeyEvent, context: &[Action]) -> Option<Action> {
        let exact_ctrl = |action| self.keys(action).iter().any(|k| k.ctrl && k.matches(key));
        context
            .iter()
            .copied()
            .find(|&action| exact_ctrl(action))
            .or_else(|| context.iter().copied().find(|&action| self.is(action, key)))
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
            Self::ToggleTree => "toggle-tree",
            Self::Reload => "reload",
            Self::PickMessage => "pick-message",
            Self::ReviewMenu => "review-menu",
            Self::ResendAll => "resend-all",
            Self::FinishReview => "finish-review",
            Self::UndoFinish => "undo-finish",
            Self::Archive => "archive",
            Self::ToggleHidden => "toggle-hidden",
            Self::Edit => "edit",
            Self::Remove => "remove",
            Self::Visual => "visual",
            Self::MoveCursor => "move-cursor",
            Self::Comment => "comment",
            Self::ClearBlock => "clear-block",
            Self::LooksGood => "looks-good",
            Self::Delete => "delete",
            Self::Yes => "yes",
            Self::No => "no",
        }
    }
}

#[cfg(test)]
mod tests;
