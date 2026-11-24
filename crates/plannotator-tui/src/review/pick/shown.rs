//! What the code review's event loop shows: a review, the picker on its own, or the picker
//! over a review. A choice in the picker becomes a review here; over a review, the new one
//! takes the old one's delivery target, the agent picked for Send, its diff settings, keys
//! and notes rail, and the old one's unsent notes stay saved on their own scope.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;

use anyhow::Result;
use plannotator_tui_review::{Capture, Repo};
use ratatui::Frame;
use ratatui::crossterm::event::{Event, KeyEventKind};

use super::source::{GitSource, Listing, PAGE, Source};
use super::{Outcome, Picker};
use crate::config::review::ReviewConfig;
use crate::delivery::{Delivery, Discard};
use crate::keys::Keymap;
use crate::notes_rail::RailStart;
use crate::review::{Screen, settings};

/// How the review opens: what the session's config says, and whether it is the real thing
/// (the clipboard, the Pi link and the person's delivery target) or a test's.
pub(crate) struct Opener {
    pub(crate) data_dir: PathBuf,
    pub(crate) wake: SyncSender<()>,
    pub(crate) review: ReviewConfig,
    /// The remembered diff settings' file; `None` remembers nothing.
    pub(crate) remembered: Option<PathBuf>,
    pub(crate) notes_rail: RailStart,
    pub(crate) keys: Keymap,
    pub(crate) shift_enter: bool,
    pub(crate) copy_on_select: bool,
    /// What the first review says once, as it opens: a config file that did not load.
    pub(crate) notice: Option<String>,
    pub(crate) delivery: Box<dyn Fn() -> Box<dyn Delivery>>,
    /// The clipboard and the Pi link; off in tests.
    pub(crate) live: bool,
}

impl Opener {
    /// The first review of a session.
    fn screen(&mut self, repo: Repo, capture: Capture) -> Result<Screen> {
        let settings = settings::Settings::load(self.review, self.remembered.clone());
        let mut screen =
            Screen::open_with(repo, capture, &self.data_dir, (self.delivery)(), self.wake.clone(), settings)?;
        screen.start_notes_rail(self.notes_rail);
        screen.set_keys(self.keys.clone(), self.shift_enter);
        if self.live {
            screen.enable_clipboard();
        }
        screen.set_copy_on_select(self.copy_on_select);
        if self.live {
            screen.start_pi_link();
        }
        if let Some(notice) = self.notice.take() {
            screen.add_notice(notice);
        }
        Ok(screen)
    }

    /// The picker on its own, over `repo`, with the first page of commits already listed.
    fn picker(&self, repo: Repo, listing: Option<Listing>, notice: Option<String>) -> Picker {
        let source: Arc<dyn Source> = Arc::new(GitSource { repo, data_dir: self.data_dir.clone() });
        let mut picker = Picker::open(source, listing, None, self.keys.clone(), self.wake.clone());
        if let Some(notice) = notice {
            picker.say(notice);
        }
        picker
    }
}

/// Where the event loop starts.
pub(crate) enum Start {
    Review(Repo, Box<Capture>),
    /// The picker on its own.
    Pick(Repo),
}

pub(crate) struct Shown {
    screen: Option<Screen>,
    picker: Option<Picker>,
    opener: Opener,
    quit: bool,
}

impl Shown {
    /// Open the review, or the picker with its first page of commits listed now, before the
    /// first frame: a quick `git log`.
    pub(crate) fn start(start: Start, mut opener: Opener) -> Result<Self> {
        let (screen, picker) = match start {
            Start::Review(repo, capture) => (Some(opener.screen(repo, *capture)?), None),
            Start::Pick(repo) => {
                let source = GitSource { repo: repo.clone(), data_dir: opener.data_dir.clone() };
                let (listing, failed) = match source.list(0, PAGE) {
                    Ok(listing) => (Some(listing), None),
                    Err(error) => (None, Some(format!("could not list the commits: {error}"))),
                };
                (None, Some(opener.picker(repo, listing, failed)))
            }
        };
        Ok(Self { screen, picker, opener, quit: false })
    }

    /// Show `picker` on its own, for a test.
    #[cfg(test)]
    pub(crate) fn picking(picker: Picker, opener: Opener) -> Self {
        Self { screen: None, picker: Some(picker), opener, quit: false }
    }

    #[cfg(test)]
    pub(crate) fn screen(&self) -> Option<&Screen> {
        self.screen.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn screen_mut(&mut self) -> Option<&mut Screen> {
        self.screen.as_mut()
    }

    #[cfg(test)]
    pub(crate) fn picker_mut(&mut self) -> Option<&mut Picker> {
        self.picker.as_mut()
    }

    pub(crate) fn wants_quit(&self) -> bool {
        self.quit || self.screen.as_ref().is_some_and(Screen::wants_quit)
    }

    /// Work that finished, in the review and the picker; a capture the picker chose opens.
    /// True when the screen changed.
    pub(crate) fn tick(&mut self) -> Result<bool> {
        let mut changed = match &mut self.screen {
            Some(screen) => screen.tick()?,
            None => false,
        };
        let Some(picker) = &mut self.picker else { return Ok(changed) };
        changed |= picker.tick();
        if let Some((repo, capture)) = picker.take_opened() {
            changed = true;
            match self.open(repo, capture) {
                Ok(()) => self.picker = None,
                Err(error) => {
                    if let Some(picker) = &mut self.picker {
                        picker.say(format!("{error:#}"));
                    }
                }
            }
        }
        Ok(changed)
    }

    /// The chosen capture as the review: the first, or the one after the review shown.
    fn open(&mut self, repo: Repo, capture: Capture) -> Result<()> {
        let Some(old) = &mut self.screen else {
            self.screen = Some(self.opener.screen(repo, capture)?);
            return Ok(());
        };
        let next = old.successor(repo, capture)?;
        let mut old = std::mem::replace(old, next);
        if let Err(error) = old.close()
            && let Some(screen) = &mut self.screen
        {
            screen.add_notice(format!("{error:#}"));
        }
        Ok(())
    }

    pub(crate) fn draw(&mut self, frame: &mut Frame) -> Result<()> {
        if let Some(screen) = &mut self.screen {
            screen.draw(frame)?;
        }
        if let Some(picker) = &mut self.picker {
            picker.draw(frame);
        }
        Ok(())
    }

    pub(crate) fn after_draw(&mut self) {
        if let Some(screen) = &mut self.screen {
            screen.after_draw();
        }
    }

    pub(crate) fn held_at_edge(&self) -> bool {
        self.picker.is_none() && self.screen.as_ref().is_some_and(Screen::held_at_edge)
    }

    pub(crate) fn handle_event(&mut self, event: Event) -> Result<()> {
        if let Some(picker) = &mut self.picker {
            let outcome = match &event {
                Event::Key(key) if key.kind != KeyEventKind::Release => picker.key(key),
                Event::Mouse(mouse) => {
                    picker.mouse(*mouse);
                    Outcome::Stay
                }
                // The review under the box still follows the terminal's size.
                Event::Resize(..) => {
                    if let Some(screen) = &mut self.screen {
                        screen.handle_event(event)?;
                    }
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            };
            match (outcome, &mut self.screen) {
                (Outcome::Stay, _) => {}
                (Outcome::Close, _) => self.picker = None,
                // Over a review, quitting asks first about unsent notes, as the review's `q` does.
                (Outcome::Quit, Some(screen)) => {
                    self.picker = None;
                    screen.request_quit();
                }
                (Outcome::Quit, None) => self.quit = true,
            }
            return Ok(());
        }
        let Some(screen) = &mut self.screen else { return Ok(()) };
        screen.handle_event(event)?;
        if screen.take_pick_request() {
            match screen.pick_source(&self.opener.data_dir) {
                Ok(source) => {
                    let over = Some(screen.capture.scope.clone());
                    let keys = screen.keys.clone();
                    self.picker = Some(Picker::open(source, None, over, keys, self.opener.wake.clone()));
                }
                Err(why) => screen.status = Some(why),
            }
        }
        Ok(())
    }

    /// Stop the review's workers; see [`Screen::close`].
    pub(crate) fn close(&mut self) -> Result<()> {
        self.screen.as_mut().map_or(Ok(()), Screen::close)
    }
}

impl Screen {
    /// What the picker over this review lists: its repository. A pull request fetched into
    /// the cache repository has no working tree of its own to pick from.
    fn pick_source(&self, data_dir: &std::path::Path) -> Result<Arc<dyn Source>, String> {
        if self.capture.pull.as_ref().is_some_and(|pull| pull.clone.is_none()) {
            return Err("this pull request was fetched outside a clone: nothing else here to pick".into());
        }
        Ok(Arc::new(GitSource { repo: self.repo.clone(), data_dir: data_dir.to_path_buf() }))
    }

    /// The review of `capture` that takes this one's place: the same delivery target and
    /// agent picked for Send, diff settings, keys, mouse and clipboard choices, notes rail
    /// and Pi link. The notes this one has not sent stay saved on its scope; the footer says
    /// so.
    fn successor(&mut self, repo: Repo, capture: Capture) -> Result<Self> {
        let mut next = Self::open_with(
            repo,
            capture,
            &self.data_dir,
            Box::new(Discard),
            self.wake.clone(),
            self.settings.clone(),
        )?;
        next.delivery = std::mem::replace(&mut self.delivery, Box::new(Discard));
        next.start_delivery = self.start_delivery.take();
        next.set_keys(self.keys.clone(), self.shift_enter);
        next.clipboard = self.clipboard;
        next.on_release = self.on_release;
        next.notes_rail = std::mem::take(&mut self.notes_rail);
        next.notes_rail.settle(!next.store.annotations().is_empty());
        next.reach = std::mem::take(&mut self.reach);
        next.pi_link = self.pi_link.take();
        let unsent = self.store.pending().len();
        if unsent > 0 && self.capture.scope != next.capture.scope {
            let notes =
                if unsent == 1 { "1 unsent note".to_owned() } else { format!("{unsent} unsent notes") };
            next.add_notice(format!("{notes} kept on {}", self.scope_title()));
        }
        Ok(next)
    }
}
