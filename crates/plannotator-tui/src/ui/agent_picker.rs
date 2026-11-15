//! The agent picker (`docs/spec-pi-link.md`): choose which agent Send goes to, from the
//! running Pi sessions and, inside Herdr, every pane with an agent. Document and code reviews
//! open the same picker; each keeps its own Send target and hands it here to be swapped.
//!
//! `P` opens it at any time. Send opens it too when nothing names a target and an agent is
//! within reach, with a first row that copies instead, as Send did before.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr as _;

use crate::delivery::{Delivery, HerdrAgent};
use crate::herdr::agents::{self, HerdrAgents, PaneAgent};
use crate::herdr::context::HerdrEnv;
use crate::keys::{self, Action, Keymap};
use crate::pi::target::PiSession;
use crate::pi::{self, PiEntry};
use crate::theme::palette;
use crate::ui::hints;

/// An agent Send can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reachable {
    /// A Pi session, over the Pi link.
    Pi(PiEntry),
    /// An agent in a Herdr pane, through `herdr agent prompt` with this binary.
    Herdr(PaneAgent, PathBuf),
}

impl Reachable {
    /// `pi · api`, `claude`, `claude · reviewer`.
    fn label(&self) -> String {
        match self {
            Self::Pi(entry) => format!("pi · {}", entry.label()),
            Self::Herdr(agent, _) => agent.label(),
        }
    }

    fn cwd(&self) -> Option<&Path> {
        match self {
            Self::Pi(entry) => Some(&entry.cwd),
            Self::Herdr(agent, _) => agent.cwd.as_deref(),
        }
    }

    /// `idle`, `busy`, `working`, `blocked`, ...
    fn state(&self) -> &str {
        match self {
            Self::Pi(entry) if entry.busy => "busy",
            Self::Pi(_) => "idle",
            Self::Herdr(agent, _) => &agent.status,
        }
    }

    /// Where it runs: `pid 7` for a Pi, `workspace › tab · w1:p3` for a Herdr pane.
    fn place(&self) -> String {
        match self {
            Self::Pi(entry) => format!("pid {}", entry.pid),
            Self::Herdr(agent, _) => format!("{} · {}", agent.place(), agent.pane),
        }
    }

    fn delivery(&self) -> Box<dyn Delivery> {
        match self {
            Self::Pi(entry) => Box::new(PiSession::new(entry.clone())),
            Self::Herdr(agent, bin) => {
                Box::new(HerdrAgent::new(bin.clone(), agent.pane.clone(), Some(agent.agent.clone())))
            }
        }
    }

    /// What breaks a tie between two rows with the same label: a Pi's pid, a pane's id.
    fn tiebreak(&self) -> (u8, String, u32) {
        match self {
            Self::Pi(entry) => (0, String::new(), entry.pid),
            Self::Herdr(agent, _) => (1, agent.pane.clone(), 0),
        }
    }
}

/// Where the picker looks for agents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentFinder {
    /// The Pi registry; `None` is [`pi::root`].
    pub(crate) pi_root: Option<PathBuf>,
    /// Herdr's agents, when the review runs inside Herdr.
    pub(crate) herdr: Option<HerdrAgents>,
    /// Send with no target asks which agent first. Only interactive reviews ask.
    pub(crate) ask_on_send: bool,
}

impl AgentFinder {
    /// An interactive review: Pi sessions, Herdr's agents when inside Herdr, and Send asks.
    pub(crate) fn from_env() -> Self {
        let env = HerdrEnv::from_env();
        Self {
            pi_root: None,
            herdr: env.in_herdr.then_some(HerdrAgents { bin: env.bin, own_pane: env.pane_id }),
            ask_on_send: true,
        }
    }

    /// Every agent within reach, in the picker's order for a review of `folder` (a real path).
    /// A Pi that Herdr also lists is shown once, as the Pi session.
    pub(crate) fn find(&self, folder: &Path) -> Vec<Reachable> {
        let root = self.pi_root.clone().unwrap_or_else(pi::root);
        let pis = pi::live_pi_sessions(&root);
        let panes = self.herdr.as_ref().map(HerdrAgents::list).unwrap_or_default();
        let bin = self.herdr.as_ref().map(|h| h.bin.clone()).unwrap_or_default();
        let panes = panes
            .into_iter()
            .filter(|pane| {
                !pis.iter().any(|pi| agents::is_pi_session(pane, &pi.id, pi.transcript.as_deref()))
            })
            .map(|pane| Reachable::Herdr(pane, bin.clone()))
            .collect::<Vec<_>>();
        let mut found: Vec<Reachable> = pis.into_iter().map(Reachable::Pi).chain(panes).collect();
        order(&mut found, folder);
        found
    }
}

/// Whether Send asks which agent first, and whether there is any to ask about. Shared by
/// both reviews.
#[derive(Debug, Default)]
pub(crate) struct Reach {
    pub(crate) finder: AgentFinder,
    /// Whether the last look found an agent. The first look runs on a thread at start, so
    /// a slow Pi or Herdr never holds up the first frame; until it answers, Send copies.
    found: Arc<AtomicBool>,
}

impl Reach {
    /// An interactive review of `folder`: look for agents once, in the background.
    pub(crate) fn start(folder: PathBuf) -> Self {
        Self::start_with(AgentFinder::from_env(), folder)
    }

    pub(crate) fn start_with(finder: AgentFinder, folder: PathBuf) -> Self {
        let found = Arc::new(AtomicBool::new(false));
        if finder.ask_on_send {
            let (finder, found) = (finder.clone(), Arc::clone(&found));
            let _ = std::thread::Builder::new().name("agent-finder".into()).spawn(move || {
                let any = !finder.find(&folder).is_empty();
                found.store(any, Ordering::Relaxed);
            });
        }
        Self { finder, found }
    }

    /// The agents within reach now, remembering whether there were any.
    pub(crate) fn find(&self, folder: &Path) -> Vec<Reachable> {
        let agents = self.finder.find(folder);
        self.found.store(!agents.is_empty(), Ordering::Relaxed);
        agents
    }

    /// Whether Send with `delivery` would ask which agent: nothing named a target, the
    /// review is interactive, and the last look found one. The Send button says so.
    pub(crate) fn asks(&self, delivery: &dyn Delivery) -> bool {
        self.finder.ask_on_send && delivery.is_fallback() && self.found.load(Ordering::Relaxed)
    }

    /// On Send: the picker to show first, when nothing named a target and an agent is
    /// within reach now. `None` sends as before.
    pub(crate) fn picker_for_send(&self, delivery: &dyn Delivery, folder: &Path) -> Option<AgentPicker> {
        if !self.finder.ask_on_send || !delivery.is_fallback() {
            return None;
        }
        let agents = self.find(folder);
        (!agents.is_empty()).then(|| AgentPicker::for_send(agents))
    }
}

/// The picker's state while it is open. The list is read once, on open: it never changes
/// under the cursor.
#[derive(Debug, Default)]
pub(crate) struct AgentPicker {
    pub(crate) agents: Vec<Reachable>,
    pub(crate) cursor: usize,
    /// Opened by Send with no target: the first row copies to the clipboard instead, and
    /// the cursor starts on the first agent.
    pub(crate) copy_row: bool,
}

/// What a key did in the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerKey {
    Stay,
    Close,
    /// The row under the cursor was confirmed.
    Chose(usize),
}

/// What confirming a row did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Picked {
    /// The copy row: send to the clipboard this once.
    Copy,
    /// Send now goes elsewhere; the status says where.
    Moved(String),
    Nothing,
}

/// A row of the picker.
enum Row<'a> {
    Copy,
    Agent(&'a Reachable),
    /// Back to the target the review started with.
    Back,
}

/// The agents in `folder` first, then by name, then by pid or pane, so the order holds
/// while agents work.
fn order(agents: &mut [Reachable], folder: &Path) {
    // Pi records its cwd as it was given (on macOS `/var/…` for `/private/var/…`).
    agents.sort_by_cached_key(|agent| {
        let inside = agent.cwd().is_some_and(|cwd| {
            let cwd = real(cwd);
            cwd.starts_with(folder) || folder.starts_with(&cwd)
        });
        (!inside, agent.label(), agent.tiebreak())
    });
}

impl AgentPicker {
    /// `P`: the agents within reach, the ones in `folder` (a real path) first, then by name.
    /// Not by activity: that changes each time an agent goes busy or idle, and rows would
    /// swap under the cursor.
    pub(crate) fn open(reach: &Reach, folder: &Path) -> Self {
        Self { agents: reach.find(folder), cursor: 0, copy_row: false }
    }

    /// Send with no target: `agents`, below a row that copies instead.
    pub(crate) fn for_send(agents: Vec<Reachable>) -> Self {
        let cursor = usize::from(!agents.is_empty());
        Self { agents, cursor, copy_row: true }
    }

    fn row(&self, index: usize, has_start: bool) -> Option<Row<'_>> {
        let index = if self.copy_row {
            match index.checked_sub(1) {
                Some(index) => index,
                None => return Some(Row::Copy),
            }
        } else {
            index
        };
        match self.agents.get(index) {
            Some(agent) => Some(Row::Agent(agent)),
            None if index == self.agents.len() && has_start && !self.copy_row => Some(Row::Back),
            None => None,
        }
    }

    /// Rows: the copy row when Send opened the picker, each agent, then the target the
    /// review started with when Send has moved off it.
    pub(crate) fn rows(&self, has_start: bool) -> usize {
        usize::from(self.copy_row) + self.agents.len() + usize::from(has_start && !self.copy_row)
    }

    pub(crate) fn key(&mut self, keymap: &Keymap, key: &KeyEvent, has_start: bool) -> PickerKey {
        match keymap.action(key, keys::PICK_TARGET) {
            Some(Action::Down) => {
                self.cursor = (self.cursor + 1).min(self.rows(has_start).saturating_sub(1));
                PickerKey::Stay
            }
            Some(Action::Up) => {
                self.cursor = self.cursor.saturating_sub(1);
                PickerKey::Stay
            }
            Some(Action::Confirm) => PickerKey::Chose(self.cursor),
            Some(Action::Back | Action::PickTarget | Action::Interrupt) => PickerKey::Close,
            _ => PickerKey::Stay,
        }
    }

    /// Act on `row`: make an agent the Send target, go back to where the review started,
    /// or copy this once.
    pub(crate) fn choose(
        &self,
        row: usize,
        delivery: &mut Box<dyn Delivery>,
        start: &mut Option<Box<dyn Delivery>>,
    ) -> Picked {
        match self.row(row, start.is_some()) {
            Some(Row::Copy) => return Picked::Copy,
            Some(Row::Agent(agent)) => {
                let previous = std::mem::replace(delivery, agent.delivery());
                // Keep the first target only: picking an agent after an agent drops the earlier one.
                start.get_or_insert(previous);
            }
            Some(Row::Back) => match start.take() {
                Some(first) => *delivery = first,
                None => return Picked::Nothing,
            },
            None => return Picked::Nothing,
        }
        Picked::Moved(format!("Send now goes to {}", delivery.describe()))
    }

    /// The rows as text, columns aligned: agent, folder, state, where.
    fn lines(&self, folder: &Path, start: Option<&dyn Delivery>) -> Vec<String> {
        let cells: Vec<[String; 4]> = self
            .agents
            .iter()
            .map(|agent| {
                let cwd = agent.cwd().map_or_else(|| "?".to_owned(), |cwd| short_path(cwd, folder));
                [agent.label(), cwd, agent.state().to_owned(), agent.place()]
            })
            .collect();
        let width = |column: usize| {
            cells.iter().filter_map(|row| row.get(column)).map(|cell| cell.width()).max().unwrap_or(0)
        };
        let widths = [width(0), width(1), width(2)];
        let mut rows: Vec<String> = Vec::new();
        if self.copy_row {
            rows.push(" Copy to clipboard ".to_owned());
        }
        rows.extend(cells.iter().map(|[label, cwd, state, place]| {
            let [a, b, c] = widths;
            format!(" {label:<a$}  {cwd:<b$}  {state:<c$}  {place} ")
        }));
        if let Some(start) = start.filter(|_| !self.copy_row) {
            rows.push(format!(" back to {} ", start.describe()));
        }
        rows
    }

    /// Draw the picker centred in `area`. `folder` shortens the agents' folders.
    pub(crate) fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        keymap: &Keymap,
        folder: &Path,
        start: Option<&dyn Delivery>,
    ) {
        let mut rows = self.lines(folder, start);
        let empty = rows.is_empty();
        if empty {
            rows.push(
                " No agents found. For Pi, run `plannotator-tui pi install`, then restart Pi. ".to_owned(),
            );
        }
        let help = hints::picker(
            "move",
            [keymap.labelled(Action::Confirm, "send here"), keymap.labelled(Action::Back, "close")],
        );
        let content = rows.iter().map(|row| row.width()).max().unwrap_or(0).max(help.width());
        let width = (content as u16 + 2).min(area.width);
        let height = (rows.len() as u16 + 2).min(area.height);
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let boxed = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(palette().accent))
            .title(Span::styled(" send to which agent? ", Style::new().dim()))
            .title_bottom(Span::styled(help, Style::new().dim()));
        let inner = boxed.inner(rect);
        frame.render_widget(boxed, rect);
        // Keep the cursor's row on screen when there are more rows than fit.
        let visible = usize::from(inner.height);
        let skip = (self.cursor + 1).saturating_sub(visible);
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .enumerate()
            .skip(skip)
            .take(visible)
            .map(|(index, text)| {
                let style = if empty {
                    Style::new().dim()
                } else if index == self.cursor {
                    palette().selection
                } else {
                    Style::new()
                };
                Line::from(Span::styled(format!("{text:<pad$}", pad = usize::from(inner.width)), style))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// `path` relative to `folder` when it is inside it, else under `~`, else as is.
fn short_path(path: &Path, folder: &Path) -> String {
    if path == folder {
        return ".".to_owned();
    }
    let path = &real(path);
    if path == folder {
        return ".".to_owned();
    }
    if let Ok(rest) = path.strip_prefix(folder) {
        return format!("./{}", rest.display());
    }
    match std::env::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `folder` as a real path, for comparing with the folders agents report.
pub(crate) fn real(folder: &Path) -> PathBuf {
    std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
}

#[cfg(test)]
mod tests;
