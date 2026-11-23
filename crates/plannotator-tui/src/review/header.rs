//! The header row: what is reviewed on the left; on the right the layout switch
//! (`Split │ Unified`, the one in force lit), the wrap switch, the Settings and Review menus,
//! and the Send button on the edge. Each is clickable.
//!
//! A narrow header gives way in steps, the richest arrangement that fits winning: first the
//! key hints in the menu buttons, then the wrap switch, the Review button, the viewed count
//! in the title, the title's tail (cut with `…`), the Settings label (`≡`) and last the
//! switch's words (`S │ U`). Send always shows.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use super::Screen;
use crate::keys::Action;
use crate::theme::palette;
use crate::ui::{chips, send_button};

/// The fewest cells a cut title keeps.
const TITLE_MIN: usize = 12;

/// One clickable thing in the header, left of Send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Chip {
    /// `Split │ Unified`, or `S │ U` when `compact`.
    Layout {
        compact: bool,
    },
    Wrap,
    Settings(String),
    Review(String),
}

impl Chip {
    fn width(&self) -> usize {
        match self {
            Self::Layout { compact: false } => " Split ".width() + " Unified ".width(),
            Self::Layout { compact: true } => " S ".width() + " U ".width(),
            Self::Wrap => " Wrap ".width(),
            Self::Settings(label) | Self::Review(label) => label.width(),
        }
    }
}

/// The labels the header can show its menus with, richest first.
pub(super) struct Labels {
    pub(super) settings: [String; 3],
    pub(super) review: [String; 2],
}

/// `text` cut to `width` cells, ending in `…` when cut.
fn ellipsize(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('\u{2026}');
    }
    out
}

/// The title and the chips that fit `width` cells beside a Send button `send` cells wide:
/// the first arrangement, richest first, whose title and chips fit. `layout` is false while
/// the layout switch does not apply (the whole-file view). `short` is the title without its
/// viewed count.
pub(super) fn arrange(
    width: usize,
    send: usize,
    (full, short): (&str, &str),
    labels: &Labels,
    layout: bool,
) -> (String, Vec<Chip>) {
    let [settings, settings_short, settings_tiny] = &labels.settings;
    let [review, review_short] = &labels.review;
    let switch = |compact| layout.then_some(Chip::Layout { compact });
    let set = |label: &String| Some(Chip::Settings(label.clone()));
    let rev = |label: &String| Some(Chip::Review(label.clone()));
    // (title, chips); a title of None is cut to whatever room the chips leave.
    let candidates: [(Option<&str>, [Option<Chip>; 4]); 11] = [
        (Some(full), [switch(false), Some(Chip::Wrap), set(settings), rev(review)]),
        (Some(full), [switch(false), Some(Chip::Wrap), set(settings_short), rev(review_short)]),
        (Some(full), [switch(false), None, set(settings_short), rev(review_short)]),
        (Some(full), [switch(false), None, set(settings_short), None]),
        (Some(short), [switch(false), None, set(settings_short), None]),
        (Some(short), [switch(false), None, set(settings_tiny), None]),
        (None, [switch(false), None, set(settings_short), None]),
        (None, [switch(false), None, set(settings_tiny), None]),
        (None, [switch(true), None, set(settings_tiny), None]),
        (None, [switch(true), None, None, None]),
        (None, [None, None, None, None]),
    ];
    for (title, chips) in candidates {
        let chips: Vec<Chip> = chips.into_iter().flatten().collect();
        let used: usize = chips.iter().map(|chip| chip.width() + 1).sum::<usize>() + send;
        let room = width.saturating_sub(used);
        match title {
            Some(title) if title.width() < room => return (title.to_owned(), chips),
            None if room > TITLE_MIN => return (ellipsize(full, room - 1), chips),
            Some(_) | None => {}
        }
    }
    (ellipsize(full, width.saturating_sub(send + 1)), Vec::new())
}

impl Screen {
    /// The labels of the Settings and Review buttons, with their keys and without.
    fn header_labels(&self) -> Labels {
        let with_key = |name: &str, action| match self.keys.hint(action) {
            key if key.is_empty() => format!(" {name} \u{25be} "),
            key => format!(" {name} \u{25be} ({key}) "),
        };
        Labels {
            settings: [
                with_key("Settings", Action::DiffSettings),
                " Settings \u{25be} ".to_owned(),
                " \u{2261} ".to_owned(),
            ],
            review: [with_key("Review", Action::ReviewMenu), " Review \u{25be} ".to_owned()],
        }
    }

    /// `Uncommitted · 4 files +120 −30 · 2/4 viewed` on the left, the switches and menus on
    /// the right, Send on the edge.
    pub(super) fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let files = self.capture.files.len();
        // While a filter hides files, the counts are of the files the diff shows.
        let (added, removed) =
            if self.files.shown().len() == files { self.capture.totals() } else { self.files.shown_totals() };
        let count = match self.files.shown().len() {
            shown if shown == files => format!("{files} file{}", if files == 1 { "" } else { "s" }),
            shown => format!("{shown} of {files} files"),
        };
        let (full, short) = if let Some(whole) = self.whole_title() {
            (format!(" {whole} "), format!(" {whole} "))
        } else {
            let head = format!(" {} \u{b7} {count} +{added} \u{2212}{removed}", self.scope_title());
            (format!("{head} \u{b7} {} ", self.viewed_label()), format!("{head} "))
        };
        let send = format!(" {} ", self.send_label());
        let send_width = (send.width() as u16).min(area.width);
        let labels = self.header_labels();
        let (title, chips) = arrange(
            usize::from(area.width),
            usize::from(send_width) + 1,
            (&full, &short),
            &labels,
            self.whole.is_none(),
        );
        let buf = frame.buffer_mut();
        buf.set_span(
            area.x,
            area.y,
            &Span::styled(title.as_str(), Style::new().fg(palette().accent).bold()),
            area.width,
        );
        let send_rect = Rect { x: area.right() - send_width, y: area.y, width: send_width, height: 1 };
        let style = send_button::style(&self.send_state, self.store.pending().len());
        buf.set_span(send_rect.x, send_rect.y, &Span::styled(send, style), send_rect.width);
        self.geometry.send_button = Some(send_rect);
        let chips_width: usize = chips.iter().map(|chip| chip.width() + 1).sum();
        let mut x = send_rect.x.saturating_sub(chips_width as u16);
        let on = chips::chips().switch_on;
        let off = Style::new().fg(chips::chips().switch_off_fg).bg(palette().toolbar_bg);
        let menu = Style::new().fg(chips::chips().menu_fg).bg(palette().toolbar_bg);
        let split = self.diff.split();
        for chip in chips {
            let parts: Vec<(String, Style)> = match &chip {
                Chip::Layout { compact } => {
                    let (a, b) = if *compact { (" S ", " U ") } else { (" Split ", " Unified ") };
                    vec![
                        (a.to_owned(), if split { on } else { off }),
                        (b.to_owned(), if split { off } else { on }),
                    ]
                }
                Chip::Wrap => vec![(" Wrap ".to_owned(), if self.settings.now.wrap { on } else { off })],
                Chip::Settings(label) | Chip::Review(label) => vec![(label.clone(), menu)],
            };
            for (index, (text, style)) in parts.into_iter().enumerate() {
                let rect = Rect { x, y: area.y, width: text.width() as u16, height: 1 };
                buf.set_span(rect.x, rect.y, &Span::styled(text, style), rect.width);
                match &chip {
                    Chip::Layout { .. } => self.geometry.layout_switch.push((rect, index == 0)),
                    Chip::Wrap => self.geometry.wrap_switch = Some(rect),
                    Chip::Settings(_) => self.geometry.settings_button = Some(rect),
                    Chip::Review(_) => self.geometry.review_button = Some(rect),
                }
                x += rect.width;
            }
            x += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Labels {
        Labels {
            settings: [" Settings ▾ (,) ".into(), " Settings ▾ ".into(), " ≡ ".into()],
            review: [" Review ▾ (m) ".into(), " Review ▾ ".into()],
        }
    }

    const FULL: &str = " Uncommitted · 4 files +120 −30 · 2/4 viewed ";
    const SHORT: &str = " Uncommitted · 4 files +120 −30 ";

    fn shown(width: usize, send: usize) -> (String, Vec<Chip>) {
        arrange(width, send, (FULL, SHORT), &labels(), true)
    }

    #[test]
    fn a_wide_header_shows_everything_and_a_narrow_one_gives_way_in_order() {
        let (title, chips) = shown(200, 24);
        assert_eq!(title, FULL);
        assert_eq!(
            chips,
            [
                Chip::Layout { compact: false },
                Chip::Wrap,
                Chip::Settings(" Settings ▾ (,) ".into()),
                Chip::Review(" Review ▾ (m) ".into())
            ]
        );
        let mut seen = Vec::new();
        for width in (40..=200).rev() {
            let (title, chips) = shown(width, 24);
            let used = title.width() + chips.iter().map(|c| c.width() + 1).sum::<usize>() + 24;
            assert!(used <= width, "{width}: {title:?} {chips:?}");
            if !seen.contains(&chips) {
                seen.push(chips.clone());
            }
            // The switch outlasts every other chip.
            if !chips.is_empty() {
                assert!(matches!(chips.first(), Some(Chip::Layout { .. })), "{width}: {chips:?}");
            }
        }
        assert!(seen.len() >= 6, "{seen:?}");
    }

    #[test]
    fn at_80_columns_the_title_keeps_its_counts_and_at_60_the_switch_still_shows() {
        // A long Send label: `Send 1 new ▸ test agent (E)`.
        let (full, short) = (" Uncommitted · 1 file +7 −3 · 0/1 viewed ", " Uncommitted · 1 file +7 −3 ");
        let (title, chips) = arrange(80, 30, (full, short), &labels(), true);
        assert_eq!(title, short);
        assert_eq!(chips.first(), Some(&Chip::Layout { compact: false }));
        let (title, chips) = shown(60, 30);
        assert!(title.ends_with('…') && title.width() > TITLE_MIN, "{title:?}");
        assert!(matches!(chips.first(), Some(Chip::Layout { .. })), "{chips:?}");
        let (_, chips) = shown(60, 17);
        assert_eq!(chips, [Chip::Layout { compact: false }, Chip::Settings(" Settings ▾ ".into())]);
    }

    #[test]
    fn the_whole_file_view_has_no_layout_switch() {
        let (_, chips) = arrange(200, 24, (FULL, FULL), &labels(), false);
        assert!(!chips.iter().any(|c| matches!(c, Chip::Layout { .. })), "{chips:?}");
        assert!(chips.contains(&Chip::Wrap));
    }

    #[test]
    fn a_cut_title_ends_in_an_ellipsis_within_its_width() {
        assert_eq!(ellipsize("abcdef", 6), "abcdef");
        assert_eq!(ellipsize("abcdef", 4), "abc…");
        assert_eq!(ellipsize("ab", 0), "");
    }
}
