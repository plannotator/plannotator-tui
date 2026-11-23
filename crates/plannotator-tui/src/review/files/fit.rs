//! Fitting the file list to the screen: how wide it is, and how a name too long for it is
//! cut. A file's name keeps its end and extension (`ReviewAgentJ…Panel.tsx`); a folder row
//! keeps its last folders (`packages/…/components/`). The status letter and the `+12 −3`
//! counts give way before the name does.

use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr};

/// The narrowest list worth drawing.
pub(crate) const MIN_WIDTH: u16 = 20;
/// The diff keeps at least this many columns beside the list.
pub(crate) const DIFF_MIN_WIDTH: u16 = 48;
/// While the list has the focus it may squeeze the diff to this, the least the viewer draws.
const DIFF_MIN_FOCUSED: u16 = 40;

/// The list's width, divider excluded, in `room` columns shared with the diff (and a one
/// column divider): what its rows need (`content`), or the width chosen with `<` and `>`,
/// within two fifths of the room and never leaving the diff less than [`DIFF_MIN_WIDTH`].
/// `None` when there is no room; with the focus the list shows anyway, beside a narrower
/// diff.
pub(crate) fn width(content: u16, room: u16, chosen: Option<u16>, focused: bool) -> Option<u16> {
    let most = (room * 2 / 5).min(room.saturating_sub(1 + DIFF_MIN_WIDTH));
    if most >= MIN_WIDTH {
        let ceiling = match chosen {
            // A chosen width may pass the share, as long as the diff keeps its minimum.
            Some(_) => room.saturating_sub(1 + DIFF_MIN_WIDTH),
            None => most,
        };
        return Some(chosen.unwrap_or(content).clamp(MIN_WIDTH, ceiling));
    }
    let squeezed = room.saturating_sub(1 + DIFF_MIN_FOCUSED);
    (focused && squeezed >= MIN_WIDTH).then(|| chosen.unwrap_or(content).min(squeezed).max(1))
}

/// `name` in at most `width` columns, cut in the middle so its start, its end and its
/// extension stay readable: `ReviewAgentJ…Panel.tsx`.
pub(crate) fn middle(name: &str, width: usize) -> String {
    if name.width() <= width {
        return name.to_owned();
    }
    if width <= 1 {
        return "\u{2026}".repeat(width);
    }
    let room = width - 1;
    // The extension, when there is one worth keeping, stays whole if it can.
    let ext = name.rfind('.').filter(|&at| at > 0).map_or(0, |at| name.len() - at);
    let ext_width = name.get(name.len() - ext..).map_or(0, UnicodeWidthStr::width);
    let tail_width = room.div_ceil(2).max(ext_width.min(room.saturating_sub(1)));
    let tail = take_end(name, tail_width);
    let head = take_start(name, room - tail.width());
    format!("{head}\u{2026}{tail}")
}

/// A folder row's path (`packages/review-editor/components/`) in at most `width` columns:
/// as many folders from the end as fit, after the first folder when that fits too
/// (`packages/…/components/`), else after `…/`; a last folder too long alone is cut in the
/// middle.
pub(crate) fn folder(label: &str, width: usize) -> String {
    if label.width() <= width {
        return label.to_owned();
    }
    let segments: Vec<&str> = label.trim_end_matches('/').split('/').collect();
    let count = segments.len();
    for keep in (1..count).rev() {
        let tail = format!("{}/", segments.get(count - keep..).unwrap_or_default().join("/"));
        if let Some(first) = segments.first().filter(|_| keep + 1 < count) {
            let both = format!("{first}/\u{2026}/{tail}");
            if both.width() <= width {
                return both;
            }
        }
        let cut = format!("\u{2026}/{tail}");
        if cut.width() <= width {
            return cut;
        }
    }
    middle(label, width)
}

/// The longest start of `text` that fits in `width` columns.
fn take_start(text: &str, width: usize) -> &str {
    let mut used = 0;
    for (at, ch) in text.char_indices() {
        used += ch.width().unwrap_or(0);
        if used > width {
            return text.get(..at).unwrap_or_default();
        }
    }
    text
}

/// The longest end of `text` that fits in `width` columns.
fn take_end(text: &str, width: usize) -> &str {
    let mut used = 0;
    for (at, ch) in text.char_indices().rev() {
        used += ch.width().unwrap_or(0);
        if used > width {
            return text.get(at + ch.len_utf8()..).unwrap_or_default();
        }
    }
    text
}

/// What a file row shows right of its name, once the name has had its room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) status: bool,
    pub(crate) counts: bool,
}

/// Which columns every file row shows, decided once for the whole list so they line up.
/// `names` is what each file row needs for its name, indent included; `status` and
/// `counts` are the columns each takes, its gap included. The counts go first, then the
/// status letter, but a column goes only when that shows more names whole; on a tie the
/// list keeps it.
pub(crate) fn columns(names: &[usize], room: usize, status: usize, counts: usize) -> Shown {
    let options = [
        Shown { status: true, counts: true },
        Shown { status: true, counts: false },
        Shown { status: false, counts: false },
    ];
    let cut = |shown: &Shown| {
        let right = if shown.status { status } else { 0 } + if shown.counts { counts } else { 0 };
        names.iter().filter(|&&name| name + right > room).count()
    };
    let mut best = Shown { status: true, counts: true };
    let mut fewest = cut(&best);
    for option in options.into_iter().skip(1) {
        let n = cut(&option);
        if n < fewest {
            (best, fewest) = (option, n);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_name_keeps_its_start_its_end_and_its_extension() {
        let name = "ReviewAgentJobDetailPanel.tsx";
        assert_eq!(middle(name, 40), name);
        let cut = middle(name, 20);
        assert_eq!(cut.width(), 20);
        assert_eq!(cut, "ReviewAge\u{2026}lPanel.tsx");
        // Narrow: the extension still shows.
        let cut = middle(name, 8);
        assert!(cut.starts_with('R') && cut.chars().rev().take(4).eq("xst.".chars()), "{cut}");
        assert_eq!(middle(name, 1), "\u{2026}");
        assert_eq!(middle(name, 0), "");
    }

    #[test]
    fn wide_characters_are_cut_by_columns() {
        let cut = middle("設計ドキュメント一覧表.md", 12);
        assert!(cut.width() <= 12 && cut.chars().rev().take(3).eq("dm.".chars()), "{cut}");
    }

    #[test]
    fn a_folder_row_keeps_its_last_folders() {
        let label = "packages/review-editor/components/";
        assert_eq!(folder(label, 40), label);
        assert_eq!(folder(label, 30), "\u{2026}/review-editor/components/");
        assert_eq!(folder(label, 24), "packages/\u{2026}/components/");
        assert_eq!(folder(label, 15), "\u{2026}/components/");
        let cut = folder(label, 8);
        assert_eq!(cut.width(), 8);
        assert!(cut.ends_with('/'), "{cut}");
    }

    #[test]
    fn columns_go_for_every_row_and_only_when_that_shows_more_names_whole() {
        // Every name fits with everything.
        assert_eq!(columns(&[10, 18], 30, 2, 9), Shown { status: true, counts: true });
        // One name fits only without the counts: no row shows them.
        assert_eq!(columns(&[10, 20], 25, 2, 9), Shown { status: true, counts: false });
        // A name that fits only with nothing beside it: neither column for any row.
        assert_eq!(columns(&[10, 24], 25, 2, 9), Shown { status: false, counts: false });
        // A name too long for any choice keeps the columns it cannot gain from.
        assert_eq!(columns(&[10, 60], 25, 2, 9), Shown { status: true, counts: true });
    }

    #[test]
    fn the_list_fits_its_rows_within_two_fifths_of_the_room() {
        // A small change: as narrow as its rows, but never under the minimum.
        assert_eq!(width(14, 160, None, false), Some(MIN_WIDTH));
        assert_eq!(width(30, 160, None, false), Some(30));
        // A long name: capped at two fifths of the room.
        assert_eq!(width(90, 160, None, false), Some(64));
        // `<` and `>` may pass the share, never the diff's minimum.
        assert_eq!(width(30, 160, Some(100), false), Some(100));
        assert_eq!(width(30, 160, Some(200), false), Some(160 - 1 - DIFF_MIN_WIDTH));
        assert_eq!(width(30, 160, Some(4), false), Some(MIN_WIDTH));
    }

    #[test]
    fn without_room_the_list_hides_unless_it_has_the_focus() {
        // 80 columns beside a notes rail leave 51 for the list and the diff.
        assert_eq!(width(30, 51, None, false), None);
        assert_eq!(width(30, 51, None, true), None);
        assert_eq!(width(30, 64, None, true), Some(23));
        // 100 columns beside a notes rail leave 69: the narrowest list.
        assert_eq!(width(40, 69, None, false), Some(MIN_WIDTH));
        // 80 columns and no rail: room for both.
        assert_eq!(width(40, 80, None, false), Some(31));
    }
}
