//! Key hints: the footer's rows and a picker's bottom line.

use unicode_width::UnicodeWidthStr as _;

/// The most rows the footer's hints take.
pub(crate) const FOOTER_MAX_ROWS: usize = 3;

/// Hints in the fewest rows that hold them all, up to [`FOOTER_MAX_ROWS`]; past that, the
/// last ones are dropped. Rows are `width` wide, except the last, which has `last_width`.
pub(crate) fn flow(hints: &[String], width: usize, last_width: usize) -> Vec<String> {
    let fill = |rows: usize| {
        let mut left = hints.iter().peekable();
        let filled: Vec<String> = (1..=rows)
            .map(|row| {
                let limit = if row == rows { last_width } else { width };
                let mut line = String::new();
                while let Some(hint) = left.peek() {
                    let joined =
                        if line.is_empty() { (*hint).clone() } else { format!("{line} \u{b7} {hint}") };
                    if joined.width() > limit {
                        break;
                    }
                    line = joined;
                    left.next();
                }
                line
            })
            .collect();
        (filled, left.next().is_none())
    };
    (1..FOOTER_MAX_ROWS)
        .map(fill)
        .find_map(|(filled, complete)| complete.then_some(filled))
        .unwrap_or_else(|| fill(FOOTER_MAX_ROWS).0)
}

/// A picker's bottom help: the arrows that move its cursor, then the bound keys.
pub(crate) fn picker(moves: &str, keys: impl IntoIterator<Item = Option<String>>) -> String {
    let parts: Vec<String> =
        std::iter::once(format!("\u{2191}\u{2193} {moves}")).chain(keys.into_iter().flatten()).collect();
    format!(" {} ", parts.join(" \u{b7} "))
}

/// The footer's hints in at most `rows` rows, dropping hints from the end of the list
/// until they fit; the last `keep` (send, help, quit) always stay. Rows are `width` wide,
/// except the last, which has `last_width`. Two rows split where they come out most even,
/// so no hint is left alone on a row. There is always a row, empty when no hint fits.
pub(crate) fn fit(
    hints: &[String],
    keep: usize,
    width: usize,
    last_width: usize,
    rows: usize,
) -> Vec<String> {
    let kept = hints.len().saturating_sub(keep);
    let (droppable, tail) = hints.split_at(kept);
    for shown in (0..=droppable.len()).rev() {
        let candidate: Vec<&String> = droppable.iter().take(shown).chain(tail).collect();
        if let Some(laid) = lay_out(&candidate, width, last_width, rows) {
            return laid;
        }
    }
    // Not even the kept ones fit: as many of them as one row holds, maybe none; the row
    // stays for the text beside it.
    let tail: Vec<&String> = tail.iter().collect();
    (0..=tail.len())
        .rev()
        .find_map(|n| lay_out(tail.get(..n).unwrap_or_default(), width, last_width, 1))
        .unwrap_or_else(|| vec![String::new()])
}

fn join(hints: &[&String]) -> String {
    hints.iter().map(|h| h.as_str()).collect::<Vec<_>>().join(" \u{b7} ")
}

/// `hints` in one row, or in two split as evenly as they go, each row with two hints or
/// more; `None` when they do not fit in `rows`.
fn lay_out(hints: &[&String], width: usize, last_width: usize, rows: usize) -> Option<Vec<String>> {
    let one = join(hints);
    if one.width() <= last_width {
        return Some(vec![one]);
    }
    if rows < 2 || hints.len() < 4 {
        return None;
    }
    (2..=hints.len() - 2)
        .filter_map(|split| {
            let (first, second) = hints.split_at(split);
            let (first, second) = (join(first), join(second));
            (first.width() <= width && second.width() <= last_width).then_some((first, second))
        })
        .min_by_key(|(first, second)| first.width().abs_diff(second.width()))
        .map(|(first, second)| vec![first, second])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|l| (*l).to_owned()).collect()
    }

    #[test]
    fn hints_that_do_not_fit_one_row_drop_from_the_end_and_keep_help_and_quit() {
        let all = hints(&["v select", "c comment", "r reload", "? help", "q quit"]);
        assert_eq!(
            fit(&all, 2, 80, 80, 1),
            ["v select \u{b7} c comment \u{b7} r reload \u{b7} ? help \u{b7} q quit"]
        );
        assert_eq!(fit(&all, 2, 40, 40, 1), ["v select \u{b7} c comment \u{b7} ? help \u{b7} q quit"]);
        assert_eq!(fit(&all, 2, 20, 20, 1), ["? help \u{b7} q quit"]);
    }

    #[test]
    fn two_rows_split_evenly_and_never_leave_a_hint_alone() {
        let all = hints(&["v select", "c comment", "s suggest", "r reload", "? help", "q quit"]);
        let rows = fit(&all, 2, 40, 40, 2);
        assert_eq!(
            rows,
            ["v select \u{b7} c comment \u{b7} s suggest", "r reload \u{b7} ? help \u{b7} q quit"]
        );
        // Three hints do not split: one row, else fewer hints.
        let few = hints(&["v select", "? help", "q quit"]);
        assert_eq!(fit(&few, 2, 20, 20, 2), ["? help \u{b7} q quit"]);
    }
}
