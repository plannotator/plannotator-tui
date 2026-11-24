//! The review's settings (`[review]`), in the viewer's terms: the options it opens with, and
//! bringing an open viewer to them.

use anyhow::Result;
use diffs_rs::engine::{self, Command, Options, SplitRatio, TabWidth};
use diffs_rs::theme::Appearance;

use super::Diff;
use crate::config::review::{
    Background, DiffTheme, InlineChanges, Layout, ReviewConfig, SplitShare, Whitespace,
};

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
        let tab_width = TabWidth::new(settings.tab_width.get());
        if self.engine.tab_width() != tab_width {
            self.engine.set_tab_width(tab_width)?;
        }
        let changes = inline_changes(settings.inline_changes);
        if self.engine.inline_changes() != changes {
            self.engine.set_inline_changes(changes)?;
        }
        // Tens of milliseconds on a huge diff: the screen holds a change until a frame or
        // another key needs it (`settings::whitespace`).
        let hidden = whitespace(settings.whitespace);
        if self.engine.whitespace() != hidden {
            self.engine.set_whitespace(hidden)?;
        }
        Ok(())
    }

    /// The tab width, changed-text marks and whitespace the viewer draws with, for tests.
    #[cfg(test)]
    pub(in crate::review) fn drawn_with(&self) -> (u8, engine::InlineChanges, engine::Whitespace) {
        (self.engine.tab_width().get(), self.engine.inline_changes(), self.engine.whitespace())
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

fn inline_changes(changes: InlineChanges) -> engine::InlineChanges {
    match changes {
        InlineChanges::Word => engine::InlineChanges::Word,
        InlineChanges::WordAlt => engine::InlineChanges::WordAlt,
        InlineChanges::Char => engine::InlineChanges::Char,
        InlineChanges::None => engine::InlineChanges::None,
    }
}

fn whitespace(whitespace: Whitespace) -> engine::Whitespace {
    match whitespace {
        Whitespace::Show => engine::Whitespace::Show,
        Whitespace::IgnoreAll => engine::Whitespace::IgnoreAll,
        Whitespace::IgnoreChange => engine::Whitespace::IgnoreChange,
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
