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
