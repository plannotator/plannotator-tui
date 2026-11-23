//! The diff's settings (`[review]` in the config file): what they are now, changing them from
//! the settings menu (`,`), the header's switches, `u`, `w`, `(`/`)` and the line between a
//! split diff's sides, and writing every change back to the config file so the next review
//! opens the same way.
//!
//! The settings here are the truth; the viewers follow them (`Diff::apply`), the main diff
//! and the whole-file view alike. A change is written once nothing is being dragged, so a
//! drag of the line between the sides writes once, when it ends.

mod menu;

use std::path::PathBuf;

use crate::config::review::{self, Background, DiffTheme, Layout, ReviewConfig, SplitShare};
use crate::theme::{Theme, theme};

use super::{Mode, Screen};

/// The split ratio's step for the keys and the menu: five hundredths of the width.
const SPLIT_STEP: u16 = 50;

/// The settings in force, the ones last written, and where they are written.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(super) now: ReviewConfig,
    saved: ReviewConfig,
    /// The config file; `None` writes nothing.
    path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new(ReviewConfig::DEFAULT, None)
    }
}

impl Settings {
    /// `review` as read from the config file at `path`.
    pub(crate) fn new(review: ReviewConfig, path: Option<PathBuf>) -> Self {
        Self { now: review, saved: review, path }
    }

    /// The config file's name, for the menu: `config.toml`.
    pub(super) fn file_name(&self) -> Option<String> {
        let name = self.path.as_ref()?.file_name()?;
        Some(name.to_string_lossy().into_owned())
    }
}

/// One row of the settings menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    Layout,
    SplitRatio,
    Wrap,
    LineNumbers,
    Theme,
    Background,
}

/// The menu's rows, in display order.
pub(super) const ROWS: [Row; 6] =
    [Row::Layout, Row::SplitRatio, Row::Wrap, Row::LineNumbers, Row::Theme, Row::Background];

impl Row {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Layout => "Layout",
            Self::SplitRatio => "Split ratio",
            Self::Wrap => "Wrap",
            Self::LineNumbers => "Line numbers",
            Self::Theme => "Colors",
            Self::Background => "Background",
        }
    }
}

/// How a row's value moves: back (left), on (right), or Enter's move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Back,
    On,
    /// The next value; on the split ratio, back to even.
    Enter,
}

/// The value after `now` in `values`, or before it, wrapping around.
fn cycle<T: Copy + PartialEq>(values: &[T], now: T, back: bool) -> T {
    let at = values.iter().position(|v| *v == now).unwrap_or(0);
    let next = if back { at + values.len() - 1 } else { at + 1 };
    values.get(next % values.len()).copied().unwrap_or(now)
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// Whether the app's own palette is the light one, which `auto` colours follow.
fn light() -> bool {
    theme() == Theme::Light
}

impl Screen {
    /// Bring the viewers to the settings; cheap when nothing differs.
    pub(super) fn apply_settings(&mut self) -> anyhow::Result<()> {
        let (settings, light) = (self.settings.now, light());
        self.diff.apply(settings, light)?;
        if let Some(diff) = self.whole.as_mut().and_then(super::whole::Whole::diff_mut) {
            diff.apply(settings, light)?;
        }
        Ok(())
    }

    /// The settings changed: the viewers follow now, the config file once nothing is dragged.
    fn settings_changed(&mut self) {
        if let Err(error) = self.apply_settings() {
            self.status = Some(format!("{error:#}"));
        }
    }

    /// Write the settings to the config file if they changed since last written and no drag
    /// of the line between the sides is under way. A failure is said once, in the footer.
    pub(super) fn remember_settings(&mut self) {
        let settings = &mut self.settings;
        if settings.now == settings.saved || self.dividers.dragging_split() {
            return;
        }
        settings.saved = settings.now;
        let Some(path) = &settings.path else { return };
        if let Err(error) = review::save(path, settings.now) {
            self.status = Some(format!("{error:#}"));
        }
    }

    /// The row's value as the menu shows it.
    pub(super) fn setting_value(&self, row: Row) -> String {
        let now = &self.settings.now;
        let split = self.diff.split();
        match row {
            Row::Layout => match now.layout {
                Layout::Auto => format!("auto \u{b7} {} now", if split { "split" } else { "unified" }),
                Layout::Split => "split".to_owned(),
                Layout::Unified => "unified".to_owned(),
            },
            Row::SplitRatio if split => now.split_ratio.to_string(),
            Row::SplitRatio => format!("{} \u{b7} when split", now.split_ratio),
            Row::Wrap => on_off(now.wrap).to_owned(),
            Row::LineNumbers => on_off(now.line_numbers).to_owned(),
            Row::Theme => match now.theme {
                DiffTheme::Auto => format!("auto \u{b7} {}", if light() { "light" } else { "dark" }),
                DiffTheme::Dark => "dark".to_owned(),
                DiffTheme::Light => "light".to_owned(),
            },
            Row::Background => match now.background {
                Background::Terminal => "terminal".to_owned(),
                Background::Theme => "theme".to_owned(),
            },
        }
    }

    /// Change one setting from the menu, and say what it is now.
    pub(super) fn change_setting(&mut self, row: Row, step: Step) {
        let back = step == Step::Back;
        let stepped = self.split_stepped(step == Step::On);
        let now = &mut self.settings.now;
        match row {
            Row::Layout => {
                now.layout = cycle(&[Layout::Auto, Layout::Split, Layout::Unified], now.layout, back);
            }
            Row::SplitRatio => {
                now.split_ratio = match step {
                    Step::Enter => SplitShare::EVEN,
                    Step::Back | Step::On => stepped,
                };
            }
            Row::Wrap => now.wrap = !now.wrap,
            Row::LineNumbers => now.line_numbers = !now.line_numbers,
            Row::Theme => {
                now.theme = cycle(&[DiffTheme::Auto, DiffTheme::Dark, DiffTheme::Light], now.theme, back);
            }
            Row::Background => {
                now.background = cycle(&[Background::Terminal, Background::Theme], now.background, back);
            }
        }
        self.settings_changed();
        self.status = Some(format!("{} {}", row.label().to_lowercase(), self.setting_value(row)));
    }

    /// The split ratio one step further: from the ratio drawn, which a narrow pane can hold
    /// nearer even than the one set, else from the one set.
    fn split_stepped(&self, right: bool) -> SplitShare {
        let drawn = self.diff.split_line().and_then(|line| self.diff.split_share_at(line));
        let from = drawn.unwrap_or(self.settings.now.split_ratio).permille();
        SplitShare::from_permille(if right {
            from.saturating_add(SPLIT_STEP)
        } else {
            from.saturating_sub(SPLIT_STEP)
        })
    }

    /// `u`: split if the diff is unified now, else unified.
    pub(super) fn toggle_layout(&mut self) {
        let split = !self.diff.split();
        self.pick_layout(split);
    }

    /// The header's switch: `split` or unified, whatever `auto` would pick. The layout in
    /// force already does nothing.
    pub(super) fn pick_layout(&mut self, split: bool) {
        if self.diff.split() == split {
            return;
        }
        self.settings.now.layout = if split { Layout::Split } else { Layout::Unified };
        self.settings_changed();
    }

    /// `w`, or the header's wrap switch.
    pub(super) fn toggle_wrap(&mut self) {
        self.settings.now.wrap = !self.settings.now.wrap;
        self.settings_changed();
    }

    /// `(`/`)`: the line between the sides, one step left or right.
    pub(super) fn step_split(&mut self, right: bool) {
        if !self.diff.split() {
            self.status = Some("the split ratio applies to the split layout".into());
            return;
        }
        self.settings.now.split_ratio = self.split_stepped(right);
        self.settings_changed();
    }

    /// The line between the sides dragged to screen `column`.
    pub(super) fn drag_split(&mut self, column: u16) {
        if let Some(share) = self.diff.split_share_at(column) {
            self.settings.now.split_ratio = share;
            self.settings_changed();
        }
    }

    /// A double click on the line between the sides: even again.
    pub(super) fn even_split(&mut self) {
        self.settings.now.split_ratio = SplitShare::EVEN;
        self.settings_changed();
    }

    /// `,` or the header's Settings button.
    pub(super) fn open_settings(&mut self) {
        self.mode = Mode::Settings(0);
    }
}

#[cfg(test)]
mod tests {
    use super::cycle;

    #[test]
    fn a_cycle_wraps_both_ways() {
        let values = [1, 2, 3];
        assert_eq!(cycle(&values, 1, false), 2);
        assert_eq!(cycle(&values, 3, false), 1);
        assert_eq!(cycle(&values, 1, true), 3);
        assert_eq!(cycle(&values, 2, true), 1);
    }
}
