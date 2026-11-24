//! The review's settings (`[review]`), in the viewer's terms: the options it opens with, and
//! bringing an open viewer to them.

use anyhow::Result;
use diffs_rs::engine::{self, Command, Options, SplitRatio};
use diffs_rs::theme::Appearance;

use super::Diff;
use crate::config::review::{Background, DiffTheme, Layout, ReviewConfig, SplitShare};

impl Diff {
    /// Bring the viewer to `settings`: only what differs changes, so this is cheap to call
    /// on every tick. A toggle the viewer cannot take yet (no file has arrived, or nothing is
    /// drawn) is taken on a later call.
    pub(in crate::review) fn apply(&mut self, settings: ReviewConfig, light: bool) -> Result<()> {
        let now = self.engine.options();
        if !self.whole {
            if now.layout != layout(settings.layout) {
                self.engine.set_layout(layout(settings.layout))?;
            }
            let ratio = SplitRatio::from_permille(settings.split_ratio.permille());
            if self.engine.split_ratio() != ratio {
                self.engine.set_split_ratio(ratio)?;
            }
        }
        if now.wrap != settings.wrap {
            self.engine.execute(Command::ToggleWrap)?;
        }
        if now.line_numbers != settings.line_numbers {
            self.engine.execute(Command::ToggleNumbers)?;
        }
        if now.appearance != appearance(settings.theme, light) {
            self.engine.execute(Command::ToggleTheme)?;
        }
        // On the terminal's own background and text colour, as the document screen draws;
        // the changed-line tints come from the light or dark palette.
        let terminal = settings.background == Background::Terminal;
        if self.terminal_background != terminal {
            self.engine.set_terminal_background(terminal)?;
            self.terminal_background = terminal;
        }
        Ok(())
    }

    /// The viewer's options now, for tests.
    #[cfg(test)]
    pub(in crate::review) fn options(&self) -> Options {
        self.engine.options()
    }

    /// The screen column of the line between a split diff's sides, as last drawn.
    pub(in crate::review) fn split_line(&self) -> Option<u16> {
        self.engine.split_line()
    }

    /// The split ratio that puts the line between the sides at screen `column`, within its
    /// limits. `None` while unified or too narrow for the line to move.
    pub(in crate::review) fn split_share_at(&self, column: u16) -> Option<SplitShare> {
        self.engine.split_ratio_at(column).map(|ratio| SplitShare::from_permille(ratio.permille()))
    }
}

/// The viewer's options for `settings`, the layout aside.
pub(super) fn options(settings: ReviewConfig, light: bool) -> Options {
    Options {
        appearance: appearance(settings.theme, light),
        wrap: settings.wrap,
        line_numbers: settings.line_numbers,
        ..Options::default()
    }
}

pub(super) fn layout(layout: Layout) -> engine::Layout {
    match layout {
        Layout::Auto => engine::Layout::Auto,
        Layout::Split => engine::Layout::Split,
        Layout::Unified => engine::Layout::Unified,
    }
}

/// The diff's palette: `auto` is the app's own, `light` when the app is light.
fn appearance(theme: DiffTheme, light: bool) -> Appearance {
    match theme {
        DiffTheme::Auto if light => Appearance::Light,
        DiffTheme::Auto | DiffTheme::Dark => Appearance::Dark,
        DiffTheme::Light => Appearance::Light,
    }
}
