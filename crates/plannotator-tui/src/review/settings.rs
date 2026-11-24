//! The diff's settings: what they are now, changing them from the settings menu (`,`), the
//! header's switches, `u`, `w`, `W`, `(`/`)` and the line between a split diff's sides, and
//! remembering the person's choices for the next review.
//!
//! Three layers: the defaults, `[review]` in the config file (the person's, never written),
//! and the choices remembered in the data folder (`state`). The menu's Reset forgets the
//! choices and goes back to the config file's values.
//!
//! `layout = "auto"` is split or unified by the terminal's width (`auto`), never by the panes
//! beside the diff.
//!
//! The settings here are the truth; the viewers follow them (`Diff::apply`), the main diff
//! and the whole-file view alike. A change is remembered once nothing is being dragged, so a
//! drag of the line between the sides is written once, when it ends.

mod auto;
mod menu;
pub(crate) mod state;
mod whitespace;

use std::path::PathBuf;

use crate::config::review::{
    Background, DiffTheme, InlineChanges, Layout, ReviewConfig, SplitShare, TabWidth, Whitespace,
};
use crate::theme::{Theme, theme};

use super::{Mode, Screen};

/// The split ratio's step for the keys and the menu: five hundredths of the width.
const SPLIT_STEP: u16 = 50;

/// The settings in force, and what they are measured against.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(super) now: ReviewConfig,
    /// `[review]` in the config file: what Reset goes back to.
    config: ReviewConfig,
    /// The settings as this review last read or wrote the state file: a key that differs
    /// from it is one changed here, and only those are written.
    remembered: ReviewConfig,
    /// The state file; `None` remembers nothing.
    path: Option<PathBuf>,
    /// What the review says once when it opens: a state file it could not wholly read.
    notice: Option<String>,
    /// A write failed and the footer said so: later ones stay quiet.
    failed: bool,
    /// What `auto` is for the terminal drawn last (`auto`): split or not; `None` before
    /// the first frame.
    auto_split: Option<bool>,
    /// The whitespace the viewers draw with: `now`'s once the change is taken
    /// (`whitespace`), which waits for a frame or another key.
    shown_whitespace: Whitespace,
    /// How `W` hides whitespace: as it was hidden last in this review, else as the remembered
    /// choice or `[review]` hides it, else as `git diff -w`.
    hide_as: Whitespace,
}

impl Default for Settings {
    fn default() -> Self {
        Self::load(ReviewConfig::DEFAULT, None)
    }
}

impl Settings {
    /// `config` (the config file's `[review]`) with the choices remembered at `path` over it.
    pub(crate) fn load(config: ReviewConfig, path: Option<PathBuf>) -> Self {
        let (now, notice) = path.as_deref().map_or((config, None), |path| state::load(path, config));
        let hide_as = [now.whitespace, config.whitespace]
            .into_iter()
            .find(|w| *w != Whitespace::Show)
            .unwrap_or(Whitespace::IgnoreAll);
        Self {
            now,
            config,
            remembered: now,
            path,
            notice,
            failed: false,
            auto_split: None,
            shown_whitespace: now.whitespace,
            hide_as,
        }
    }

    /// Whether choices are remembered for the next review.
    pub(super) fn remembers(&self) -> bool {
        self.path.is_some()
    }

    /// What to say when the review opens, once.
    pub(super) fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }
}

/// One row of the settings menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    Layout,
    SplitRatio,
    Wrap,
    LineNumbers,
    Whitespace,
    TabWidth,
    InlineChanges,
    Theme,
    Background,
    /// Forget the remembered choices: back to the config file's values and the defaults.
    Reset,
}

/// The menu's rows, in display order.
pub(super) const ROWS: [Row; 10] = [
    Row::Layout,
    Row::SplitRatio,
    Row::Wrap,
    Row::LineNumbers,
    Row::Whitespace,
    Row::TabWidth,
    Row::InlineChanges,
    Row::Theme,
    Row::Background,
    Row::Reset,
];

/// The tab widths the menu steps through; the config file can set any from 1 to 16.
const TAB_WIDTHS: [u8; 3] = [2, 4, 8];

impl Row {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Layout => "Layout",
            Self::SplitRatio => "Split ratio",
            Self::Wrap => "Wrap",
            Self::LineNumbers => "Line numbers",
            Self::Whitespace => "Whitespace",
            Self::TabWidth => "Tab width",
            Self::InlineChanges => "Changed text",
            Self::Theme => "Colors",
            Self::Background => "Background",
            Self::Reset => "Reset to defaults",
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

/// `from` rounded to the nearest [`SPLIT_STEP`], then a step right (toward the new side's
/// loss) or left.
fn clean_step(from: SplitShare, right: bool) -> SplitShare {
    let rounded = (from.permille() + SPLIT_STEP / 2) / SPLIT_STEP * SPLIT_STEP;
    SplitShare::from_permille(if right { rounded + SPLIT_STEP } else { rounded - SPLIT_STEP })
}

/// The menu's tab width after `now`: the next of 2, 4 and 8 up or down, so a width the config
/// file set between them steps to its neighbours.
fn tab_step(now: TabWidth, back: bool) -> TabWidth {
    let now = now.get();
    let next = if back {
        TAB_WIDTHS.iter().rev().find(|&&w| w < now).or(TAB_WIDTHS.last())
    } else {
        TAB_WIDTHS.iter().find(|&&w| w > now).or(TAB_WIDTHS.first())
    };
    TabWidth::new(next.copied().unwrap_or(now))
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// The whitespace setting as the menu, the footer and the help name it: plain words, then the
/// `git diff` option it matches.
fn whitespace_label(whitespace: Whitespace) -> &'static str {
    match whitespace {
        Whitespace::Show => "show",
        Whitespace::IgnoreAll => "hide (like git -w)",
        Whitespace::IgnoreChange => "hide amount changes (git -b)",
    }
}

/// Whether the app's own palette is the light one, which `auto` colours follow.
fn light() -> bool {
    theme() == Theme::Light
}

impl Screen {
    /// Bring the viewers to the settings; cheap when nothing differs.
    pub(super) fn apply_settings(&mut self) -> anyhow::Result<()> {
        let (settings, light) = (self.settings.for_viewer(), light());
        self.diff.apply(settings, light)?;
        if let Some(diff) = self.whole.as_mut().and_then(super::whole::Whole::diff_mut) {
            diff.apply(settings, light)?;
        }
        Ok(())
    }

    /// The settings changed: the viewers follow now, the state file once nothing is dragged.
    fn settings_changed(&mut self) {
        if let Err(error) = self.apply_settings() {
            self.status = Some(format!("{error:#}"));
        }
    }

    /// Remember the keys changed since the state file was last read or written here, once no
    /// drag of the line between the sides is under way. A failure is said once, in the
    /// footer; the settings still hold for this review.
    pub(super) fn remember_settings(&mut self) {
        let settings = &mut self.settings;
        if settings.now == settings.remembered || self.dividers.dragging_split() {
            return;
        }
        let changes = state::changes(settings.remembered, settings.now);
        settings.remembered = settings.now;
        let Some(path) = &settings.path else { return };
        if let Err(error) = state::write(path, &changes)
            && !std::mem::replace(&mut settings.failed, true)
        {
            self.status = Some(format!("{error:#}; the settings hold for this review"));
        }
    }

    /// The menu's Reset: forget the remembered choices, back to the config file's values.
    fn reset_settings(&mut self) {
        let settings = &mut self.settings;
        settings.now = settings.config;
        settings.remembered = settings.config;
        let forgotten = settings.path.as_deref().map_or(Ok(()), state::forget);
        self.settings_changed();
        self.status = Some(match forgotten {
            Err(error) => format!("{error:#}"),
            Ok(()) if self.settings.config == ReviewConfig::DEFAULT => "settings back to the defaults".into(),
            Ok(()) => "settings back to [review] in the config file".into(),
        });
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
            Row::Whitespace => whitespace_label(now.whitespace).to_owned(),
            Row::TabWidth => format!("{} columns", now.tab_width.get()),
            Row::InlineChanges => match now.inline_changes {
                InlineChanges::Word => "words",
                InlineChanges::WordAlt => "words, joined",
                InlineChanges::Char => "characters",
                InlineChanges::None => "off",
            }
            .to_owned(),
            Row::Theme => match now.theme {
                DiffTheme::Auto => format!("auto \u{b7} {}", if light() { "light" } else { "dark" }),
                DiffTheme::Dark => "dark".to_owned(),
                DiffTheme::Light => "light".to_owned(),
            },
            Row::Background => match now.background {
                Background::Terminal => "terminal".to_owned(),
                Background::Theme => "theme".to_owned(),
            },
            Row::Reset => String::new(),
        }
    }

    /// Change one setting from the menu, and say what it is now.
    pub(super) fn change_setting(&mut self, row: Row, step: Step) {
        if row == Row::Reset {
            self.reset_settings();
            return;
        }
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
            Row::Whitespace => {
                let order = [Whitespace::Show, Whitespace::IgnoreAll, Whitespace::IgnoreChange];
                now.whitespace = cycle(&order, now.whitespace, back);
            }
            Row::TabWidth => now.tab_width = tab_step(now.tab_width, back),
            Row::InlineChanges => {
                let order =
                    [InlineChanges::Word, InlineChanges::WordAlt, InlineChanges::Char, InlineChanges::None];
                now.inline_changes = cycle(&order, now.inline_changes, back);
            }
            Row::Theme => {
                now.theme = cycle(&[DiffTheme::Auto, DiffTheme::Dark, DiffTheme::Light], now.theme, back);
            }
            Row::Background => {
                now.background = cycle(&[Background::Terminal, Background::Theme], now.background, back);
            }
            Row::Reset => {}
        }
        self.settings_changed();
        self.status = Some(match row {
            Row::Whitespace => self.whitespace_status(),
            _ => format!("{} {}", row.label().to_lowercase(), self.setting_value(row)),
        });
    }

    /// The split ratio one step further, in clean steps: from the ratio drawn (which a narrow
    /// pane can hold nearer even than the one set, else the one set), rounded to the nearest
    /// step, then one step on: 0.52 goes to 0.55 or 0.45, 0.55 to 0.60 or 0.50.
    fn split_stepped(&self, right: bool) -> SplitShare {
        let drawn = self.diff.split_line().and_then(|line| self.diff.split_share_at(line));
        clean_step(drawn.unwrap_or(self.settings.now.split_ratio), right)
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
    use super::{SplitShare, TabWidth, clean_step, cycle, tab_step};

    #[test]
    fn the_split_keys_step_by_clean_twentieths() {
        let step = |from, right| clean_step(SplitShare::from_permille(from), right).permille();
        assert_eq!((step(500, true), step(550, true), step(600, false)), (550, 600, 550));
        assert_eq!((step(520, true), step(520, false)), (550, 450));
        assert_eq!((step(208, true), step(208, false)), (250, 150));
        assert_eq!((step(575, true), step(574, true)), (650, 600), "half way rounds up");
        assert_eq!((step(900, true), step(100, false)), (900, 100), "within the limits");
    }

    #[test]
    fn the_tab_width_steps_through_2_4_and_8_and_a_width_between_steps_to_its_neighbours() {
        let step = |from, back| tab_step(TabWidth::new(from), back).get();
        assert_eq!((step(2, false), step(4, false), step(8, false)), (4, 8, 2));
        assert_eq!((step(2, true), step(4, true), step(8, true)), (8, 2, 4));
        assert_eq!((step(3, false), step(3, true)), (4, 2));
        assert_eq!((step(16, false), step(16, true), step(1, true)), (2, 8, 8));
    }

    #[test]
    fn a_cycle_wraps_both_ways() {
        let values = [1, 2, 3];
        assert_eq!(cycle(&values, 1, false), 2);
        assert_eq!(cycle(&values, 3, false), 1);
        assert_eq!(cycle(&values, 1, true), 3);
        assert_eq!(cycle(&values, 2, true), 1);
    }
}
