//! The header row: what is reviewed on the left; on the right the layout switch
//! (`Split │ Unified`, the one in force lit), the wrap switch, the `no ws` chip while
//! whitespace changes are hidden, the Settings and Review menus, and the Send button on the
//! edge. Each is clickable.
//!
//! A narrow header gives way in steps, the richest arrangement that fits winning. What stays
//! longest: Send, then the `no ws` chip (nobody should review with changes hidden without
//! seeing it), the layout switch, the Review menu, the Settings menu, the wrap switch and last
//! the key hints in the menus. On the way the Settings label shrinks to `≡`, the
//! title loses its viewed count and then its tail (cut with `…`), and the switch can shrink
//! to `S │ U`.

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
    /// `no ws`: whitespace changes are hidden; a click shows them.
    Whitespace,
    Settings(String),
    Review(String),
}

/// The chip that says whitespace changes are hidden.
const NO_WS: &str = " no ws ";

impl Chip {
    fn width(&self) -> usize {
        match self {
            Self::Layout { compact: false } => " Split ".width() + " Unified ".width(),
            Self::Layout { compact: true } => " S ".width() + " U ".width(),
            Self::Wrap => " Wrap ".width(),
            Self::Whitespace => NO_WS.width(),
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
/// the layout switch does not apply (the whole-file view), `hidden` true while whitespace
/// changes are hidden. `short` is the title without its viewed count.
pub(super) fn arrange(
    width: usize,
    send: usize,
    (full, short): (&str, &str),
    labels: &Labels,
    (layout, hidden): (bool, bool),
) -> (String, Vec<Chip>) {
    let [settings, settings_short, settings_tiny] = &labels.settings;
    let [review, review_short] = &labels.review;
    let switch = |compact| layout.then_some(Chip::Layout { compact });
    let wrap = Some(Chip::Wrap);
    let set = |label: &String| Some(Chip::Settings(label.clone()));
    let rev = Some(Chip::Review(review_short.clone()));
    let (full_switch, compact) = (switch(false), switch(true));
    // (title, chips); a title of None is cut to whatever room the chips leave. Each step
    // gives up the least wanted thing first: every chip shown outranks the ones gone.
    let candidates: [(Option<&str>, [Option<Chip>; 4]); 16] = [
        (Some(full), [full_switch.clone(), wrap.clone(), set(settings), Some(Chip::Review(review.clone()))]),
        (Some(full), [full_switch.clone(), wrap, set(settings_short), rev.clone()]),
        (Some(full), [full_switch.clone(), None, set(settings_short), rev.clone()]),
        (Some(full), [full_switch.clone(), None, set(settings_tiny), rev.clone()]),
        (Some(short), [full_switch.clone(), None, set(settings_short), rev.clone()]),
        (Some(short), [full_switch.clone(), None, set(settings_tiny), rev.clone()]),
        (Some(full), [full_switch.clone(), None, None, rev.clone()]),
        (Some(short), [full_switch.clone(), None, None, rev.clone()]),
        (Some(short), [compact.clone(), None, set(settings_tiny), rev.clone()]),
        (Some(short), [compact.clone(), None, None, rev.clone()]),
        (None, [full_switch.clone(), None, set(settings_tiny), rev.clone()]),
        (None, [full_switch.clone(), None, None, rev.clone()]),
        (None, [compact.clone(), None, None, rev]),
        (None, [full_switch, None, None, None]),
        (None, [compact, None, None, None]),
        (None, [None, None, None, None]),
    ];
    for (title, chips) in candidates {
        let mut chips: Vec<Chip> = chips.into_iter().flatten().collect();
        // Beside the wrap switch, or after the layout switch when the wrap switch gave way.
        if hidden {
            let at = chips.iter().take_while(|c| matches!(c, Chip::Layout { .. } | Chip::Wrap)).count();
            chips.insert(at, Chip::Whitespace);
        }
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
            (self.whole.is_none(), self.settings.now.whitespace != crate::config::review::Whitespace::Show),
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
                Chip::Whitespace => vec![(NO_WS.to_owned(), chips::chips().whitespace_hidden)],
                Chip::Settings(label) | Chip::Review(label) => vec![(label.clone(), menu)],
            };
            for (index, (text, style)) in parts.into_iter().enumerate() {
                let rect = Rect { x, y: area.y, width: text.width() as u16, height: 1 };
                buf.set_span(rect.x, rect.y, &Span::styled(text, style), rect.width);
                match &chip {
                    Chip::Layout { .. } => self.geometry.layout_switch.push((rect, index == 0)),
                    Chip::Wrap => self.geometry.wrap_switch = Some(rect),
                    Chip::Whitespace => self.geometry.whitespace_chip = Some(rect),
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

    const FULL: &str = " Uncommitted · 2 files +14 −5 · 0/2 viewed ";
    const SHORT: &str = " Uncommitted · 2 files +14 −5 ";
    /// ` Copy 0 new (E) ` and ` Send 1 new ▸ test agent (E) `, each with its gap.
    const SENDS: [usize; 2] = [17, 30];

    fn shown(width: usize, send: usize) -> (String, Vec<Chip>) {
        arrange(width, send, (FULL, SHORT), &labels(), (true, false))
    }

    /// Each chip's rank: what stays longest first.
    fn rank(chip: &Chip) -> usize {
        match chip {
            Chip::Layout { .. } => 0,
            Chip::Review(_) => 1,
            Chip::Settings(_) => 2,
            Chip::Wrap => 3,
            Chip::Whitespace => usize::MAX,
        }
    }

    #[test]
    fn a_wide_header_shows_everything_and_a_narrow_one_gives_way_by_rank() {
        let (title, chips) = shown(200, 17);
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
        for send in SENDS {
            for width in 30..=200 {
                let (title, chips) = shown(width, send);
                let used = title.width() + chips.iter().map(|c| c.width() + 1).sum::<usize>() + send;
                assert!(used <= width, "{width}: {title:?} {chips:?}");
                // Whatever shows, everything that outranks it shows too.
                let ranks: Vec<usize> = chips.iter().map(rank).collect();
                let mut sorted = ranks.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..ranks.len()).collect::<Vec<_>>(), "{width}/{send}: {chips:?}");
                // A key hint only while everything else is there.
                let hinted =
                    chips.iter().any(|c| matches!(c, Chip::Settings(l) | Chip::Review(l) if l.contains('(')));
                assert!(!hinted || chips.len() == 4, "{width}/{send}: {chips:?}");
            }
        }
    }

    #[test]
    fn at_60_80_100_and_120_columns_the_switch_and_review_stay() {
        let review = |chips: &[Chip]| chips.iter().any(|c| matches!(c, Chip::Review(_)));
        let switch = |chips: &[Chip]| chips.iter().any(|c| matches!(c, Chip::Layout { .. }));
        for send in SENDS {
            for width in [60, 80, 100, 120] {
                let (title, chips) = shown(width, send);
                assert!(switch(&chips), "{width}/{send}: {chips:?}");
                assert!(title.width() >= TITLE_MIN, "{width}/{send}: {title:?}");
                if width >= 80 || send == 17 {
                    assert!(review(&chips), "{width}/{send}: {title:?} {chips:?}");
                }
            }
        }
        // At 80 with a long Send label the title keeps its counts: the switch shrinks first.
        let (full, short) = (" Uncommitted · 1 file +7 −3 · 0/1 viewed ", " Uncommitted · 1 file +7 −3 ");
        let (title, chips) = arrange(80, 30, (full, short), &labels(), (true, false));
        assert_eq!(title, short);
        assert_eq!(chips, [Chip::Layout { compact: true }, Chip::Review(" Review ▾ ".into())]);
        let (_, chips) = shown(120, 17);
        assert_eq!(chips.len(), 4, "everything at 120: {chips:?}");
    }

    #[test]
    fn the_whole_file_view_has_no_layout_switch() {
        let (_, chips) = arrange(200, 24, (FULL, FULL), &labels(), (false, false));
        assert!(!chips.iter().any(|c| matches!(c, Chip::Layout { .. })), "{chips:?}");
        assert!(chips.contains(&Chip::Wrap));
        let (_, chips) = arrange(200, 24, (FULL, FULL), &labels(), (false, true));
        assert_eq!(chips.get(1), Some(&Chip::Whitespace), "the whole file's diff hides it: {chips:?}");
    }

    #[test]
    fn hidden_whitespace_shows_beside_wrap_and_stays_longest_down_to_60_columns() {
        let hidden = |width, send| arrange(width, send, (FULL, SHORT), &labels(), (true, true));
        let (_, chips) = hidden(200, 17);
        assert_eq!(chips.get(1..3), Some(&[Chip::Wrap, Chip::Whitespace][..]), "{chips:?}");
        for send in SENDS {
            for width in 60..=200 {
                let (title, chips) = hidden(width, send);
                let used = title.width() + chips.iter().map(|c| c.width() + 1).sum::<usize>() + send;
                assert!(used <= width, "{width}: {title:?} {chips:?}");
                assert!(chips.contains(&Chip::Whitespace), "{width}/{send}: {chips:?}");
            }
            // At 60 columns the layout switch stays beside it.
            let (_, chips) = hidden(60, send);
            assert!(chips.iter().any(|c| matches!(c, Chip::Layout { .. })), "{send}: {chips:?}");
        }
    }

    #[test]
    fn a_cut_title_ends_in_an_ellipsis_within_its_width() {
        assert_eq!(ellipsize("abcdef", 6), "abcdef");
        assert_eq!(ellipsize("abcdef", 4), "abc…");
        assert_eq!(ellipsize("ab", 0), "");
    }
}
