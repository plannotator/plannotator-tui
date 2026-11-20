//! The agent picker (`docs/spec-pi-link.md`): choose which agent Send goes to, from the
//! running Pi sessions and, inside Herdr or Rex, every pane with an agent. Document and code reviews
//! open the same picker; each keeps its own Send target and hands it here to be swapped.
//!
//! `P` opens it at any time. Send opens it too when nothing names a target and an agent is
//! within reach, with a first row that copies instead, as Send did before.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::SyncSender;

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
use crate::rex::agent::RexAgent;
use crate::rex::agents::{BlockAgent, RexAgents};
use crate::rex::command::Rex;
use crate::theme::palette;
use crate::ui::hints;

/// An agent Send can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reachable {
    /// A Pi session, over the Pi link.
    Pi(PiEntry),
    /// An agent in a Herdr pane, through `herdr agent prompt` with this binary.
    Herdr(PaneAgent, PathBuf),
    /// An agent in a Rex block, pasted into with this `rex`.
    Rex(BlockAgent, PathBuf),
    /// A Pi session that runs in a Rex block: sent to over the Pi link, shown where Rex has it.
    PiInRex(PiEntry, BlockAgent),
}

impl Reachable {
    /// `pi · api`, `claude`, `claude · reviewer`.
    fn label(&self) -> String {
        match self {
            Self::Pi(entry) | Self::PiInRex(entry, _) => format!("pi · {}", entry.label()),
            Self::Herdr(agent, _) => agent.label(),
            Self::Rex(agent, _) => agent.label(),
        }
    }

    fn cwd(&self) -> Option<&Path> {
        match self {
            Self::Pi(entry) | Self::PiInRex(entry, _) => Some(&entry.cwd),
            Self::Herdr(agent, _) => agent.cwd.as_deref(),
            Self::Rex(agent, _) => agent.cwd.as_deref(),
        }
    }

    /// `idle`, `busy`, `working`, `blocked`, ...
    fn state(&self) -> &str {
        match self {
            Self::Pi(entry) | Self::PiInRex(entry, _) if entry.busy => "busy",
            Self::Pi(_) | Self::PiInRex(..) => "idle",
            Self::Herdr(agent, _) => &agent.status,
            Self::Rex(agent, _) => &agent.status,
        }
    }

    /// Where it runs: `pid 7` for a Pi, `workspace › tab · w1:p3` for a Herdr pane,
    /// `session › window · a1d6db` for a Rex block, and for a Pi in one.
    fn place(&self) -> String {
        match self {
            Self::Pi(entry) => format!("pid {}", entry.pid),
            Self::Herdr(agent, _) => format!("{} · {}", agent.place(), agent.pane),
            Self::Rex(agent, _) | Self::PiInRex(_, agent) => agent.place(),
        }
    }

    fn delivery(&self) -> Box<dyn Delivery> {
        match self {
            Self::Pi(entry) | Self::PiInRex(entry, _) => Box::new(PiSession::new(entry.clone())),
            Self::Herdr(agent, bin) => {
                Box::new(HerdrAgent::new(bin.clone(), agent.pane.clone(), Some(agent.agent.clone())))
            }
            Self::Rex(agent, bin) => Box::new(RexAgent::picked(
                Rex::new(bin.clone(), agent.session_id.clone()),
                agent.block.clone(),
                agent.agent.clone(),
                agent.pid,
            )),
        }
    }

    /// What breaks a tie between two rows with the same label: a Pi's pid, a pane's id, a
    /// Rex block's place in Rex's order.
    fn tiebreak(&self) -> (u8, String, u32) {
        match self {
            Self::Pi(entry) | Self::PiInRex(entry, _) => (0, String::new(), entry.pid),
            Self::Herdr(agent, _) => (1, agent.pane.clone(), 0),
            Self::Rex(agent, _) => {
                let (session, window, block) = agent.rank;
                (2, format!("{session:05}.{window:05}.{block:05}"), 0)
            }
        }
    }

    /// Whether it runs in the Rex window (tab) the review runs in.
    fn in_this_window(&self) -> bool {
        match self {
            Self::Rex(agent, _) | Self::PiInRex(_, agent) => agent.here,
            Self::Pi(_) | Self::Herdr(..) => false,
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
    /// Rex's agents, when the review runs inside Rex.
    pub(crate) rex: Option<RexAgents>,
    /// Send with no target asks which agent first. Only interactive reviews ask.
    pub(crate) ask_on_send: bool,
}

impl AgentFinder {
    /// An interactive review: Pi sessions, Herdr's or Rex's agents when inside one, and
    /// Send asks.
    pub(crate) fn from_env() -> Self {
        let env = HerdrEnv::from_env();
        Self {
            pi_root: None,
            herdr: env.in_herdr.then_some(HerdrAgents { bin: env.bin, own_pane: env.pane_id }),
            rex: RexAgents::from_lookup(|k| std::env::var(k).ok()),
            ask_on_send: true,
        }
    }

    /// Every agent within reach, in the picker's order for a review of `folder` (a real path).
    /// A Pi that Herdr or Rex also lists is shown once, as the Pi session. Inside Rex, the
    /// agents in the review's own window come first.
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
        let blocks = self.rex.as_ref().map(RexAgents::list).unwrap_or_default();
        let rex = self.rex.as_ref().map(|r| r.bin.clone()).unwrap_or_default();
        let mut found = with_rex(pis, blocks, &rex);
        found.extend(panes);
        order(&mut found, folder);
        // Stable: within each group the order above holds.
        found.sort_by_key(|agent| !agent.in_this_window());
        found
    }
}

/// The Pi sessions and the Rex blocks with agents, as rows: a Pi that runs in a Rex block
/// (the same process) is one row, sent to over the Pi link and placed where Rex has it.
fn with_rex(pis: Vec<PiEntry>, blocks: Vec<BlockAgent>, rex: &Path) -> Vec<Reachable> {
    let (pi_blocks, blocks): (Vec<BlockAgent>, Vec<BlockAgent>) =
        blocks.into_iter().partition(|block| pis.iter().any(|pi| pi.pid == block.pid));
    let pis = pis.into_iter().map(|pi| match pi_blocks.iter().find(|block| block.pid == pi.pid) {
        Some(block) => Reachable::PiInRex(pi, block.clone()),
        None => Reachable::Pi(pi),
    });
    pis.chain(blocks.into_iter().map(|block| Reachable::Rex(block, rex.to_path_buf()))).collect()
}

/// Whether Send asks which agent first, and whether there is any to ask about. Shared by
/// both reviews.
#[derive(Debug, Default)]
pub(crate) struct Reach {
    pub(crate) finder: AgentFinder,
    /// Whether the last look found an agent. The first look runs on a thread at start, so
    /// a slow Pi or Herdr never holds up the first frame; until it answers, Send copies.
    found: Arc<AtomicBool>,
    /// Where the first look is: [`SHOWN`] (or never started), [`LOOKING`], or [`ANSWERED`]
    /// until [`Reach::poll`] hands the answer to the screen.
    first_look: Arc<AtomicU8>,
}

/// The first look is not running, or the screen has its answer.
const SHOWN: u8 = 0;
/// The first look is running.
const LOOKING: u8 = 1;
/// The first look answered; the screen has not redrawn for it yet.
const ANSWERED: u8 = 2;

impl Reach {
    /// An interactive review of `folder`: look for agents once, in the background. `wake`
    /// is the event loop's, when it has one: the look pokes it on answering.
    pub(crate) fn start(folder: PathBuf, wake: Option<SyncSender<()>>) -> Self {
        Self::start_with(AgentFinder::from_env(), folder, wake)
    }

    pub(crate) fn start_with(finder: AgentFinder, folder: PathBuf, wake: Option<SyncSender<()>>) -> Self {
        let found = Arc::new(AtomicBool::new(false));
        let first_look = Arc::new(AtomicU8::new(if finder.ask_on_send { LOOKING } else { SHOWN }));
        if finder.ask_on_send {
            let (finder, found, answered) = (finder.clone(), Arc::clone(&found), Arc::clone(&first_look));
            let spawned = std::thread::Builder::new().name("agent-finder".into()).spawn(move || {
                let any = !finder.find(&folder).is_empty();
                found.store(any, Ordering::Relaxed);
                answered.store(ANSWERED, Ordering::Release);
                if let Some(wake) = wake {
                    let _ = wake.try_send(());
                }
            });
            if spawned.is_err() {
                first_look.store(SHOWN, Ordering::Release);
            }
        }
        Self { finder, found, first_look }
    }

    /// True once, when the first look has answered since the last call: the Send button may
    /// read differently now, so the screen redraws.
    pub(crate) fn poll(&self) -> bool {
        self.first_look.compare_exchange(ANSWERED, SHOWN, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// Whether the first look is still running. A loop that cannot be woken checks often
    /// until it answers.
    pub(crate) fn looking(&self) -> bool {
        self.first_look.load(Ordering::Acquire) == LOOKING
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

    /// The rows as text, columns aligned: agent, folder, state, where. Rows wider than
    /// `room` shorten the folders from the left, then the names from the right, so the state
    /// and where an agent runs, which tell two agents apart, stay on screen in a narrow pane.
    fn lines(&self, folder: &Path, start: Option<&dyn Delivery>, room: usize) -> Vec<String> {
        let mut cells: Vec<[String; 4]> = self
            .agents
            .iter()
            .map(|agent| {
                let cwd = agent.cwd().map_or_else(|| "?".to_owned(), |cwd| short_path(cwd, folder));
                [agent.label(), cwd, agent.state().to_owned(), agent.place()]
            })
            .collect();
        let width = |cells: &[[String; 4]], column: usize| {
            cells.iter().filter_map(|row| row.get(column)).map(|cell| cell.width()).max().unwrap_or(0)
        };
        let wide = |cells: &[[String; 4]]| (0..4).map(|column| width(cells, column)).sum::<usize>() + 8;
        if wide(&cells) > room {
            let fit = width(&cells, 1).saturating_sub(wide(&cells) - room).max(FOLDER_MIN);
            for [_, cwd, _, _] in &mut cells {
                *cwd = keep_end(cwd, fit);
            }
        }
        if wide(&cells) > room {
            let fit = width(&cells, 0).saturating_sub(wide(&cells) - room).max(LABEL_MIN);
            for [label, _, _, _] in &mut cells {
                *label = keep_start(label, fit);
            }
        }
        let widths = [width(&cells, 0), width(&cells, 1), width(&cells, 2)];
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
        let mut rows = self.lines(folder, start, usize::from(area.width.saturating_sub(2)));
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

/// The narrowest a folder gets when rows do not fit: `…` and a few characters.
const FOLDER_MIN: usize = 8;

/// The narrowest a name gets when rows still do not fit: the agent and a little more.
const LABEL_MIN: usize = 12;

/// `text`, or as much of its start as fits in `width` columns and `…`.
fn keep_start(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut used = 1;
    let mut head: String = text
        .chars()
        .take_while(|c| {
            used += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
            used <= width
        })
        .collect();
    head.push('…');
    head
}

/// `text`, or `…` and as much of its end as fits in `width` columns.
fn keep_end(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        tail.push(c);
    }
    std::iter::once('…').chain(tail.into_iter().rev()).collect()
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

/// A fake `rex` or `herdr` that answers nothing while the file `gate` exists, then runs
/// `bin`: a test holds the first look open until it lets it answer.
#[cfg(all(test, unix))]
#[allow(clippy::expect_used, reason = "test fixtures assert by panicking")]
pub(crate) fn gated(bin: &Path, gate: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(gate, "").expect("gate");
    let name = bin.file_name().expect("name").to_string_lossy();
    let wrapper = bin.with_file_name(format!("gated {name}"));
    let script = format!(
        "#!/bin/sh\nwhile [ -e '{gate}' ]; do sleep 0.01; done\nexec '{bin}' \"$@\"\n",
        gate = gate.display(),
        bin = bin.display()
    );
    std::fs::write(&wrapper, script).expect("script");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    wrapper
}

#[cfg(test)]
mod tests;
