//! Source map by alignment: which source byte does each rendered character come from?
//!
//! The renderer discards positions, but for one block we hold both its source text and
//! the plain text it rendered to. They differ only by markup the renderer concealed
//! (`**`, `#`, backticks) and decoration it added (bullets, table borders). A diff
//! between the two lines them up; every rendered character that survived unchanged
//! maps to its source byte, and decoration maps to nothing. No markdown knowledge needed.

use std::ops::Range;

use similar::{Algorithm, DiffOp, capture_diff_slices};

/// Per rendered line, per rendered char: the absolute source byte offset, if any.
pub(crate) type LineOffsets = Vec<Option<usize>>;

/// Align `rendered_lines` (plain text, one entry per line) against `source`, whose first
/// byte sits at absolute offset `base`.
pub(crate) fn align(rendered_lines: &[String], source: &str, base: usize) -> Vec<LineOffsets> {
    let rendered: Vec<char> = rendered_lines.join("\n").chars().collect();
    let (src_chars, src_bytes): (Vec<char>, Vec<usize>) =
        source.char_indices().map(|(i, c)| (c, base + i)).unzip();

    // A character diff of the whole block gives up once the edit distance grows, and a
    // table's borders and padding make it large: most cells came back unmapped and could
    // not be selected. Diffing runs (words, whitespace, single symbols) keeps the distance
    // small; only the runs that differ (markup, padding, borders) are then lined up
    // character by character.
    let rendered_runs = runs(&rendered);
    let source_runs = runs(&src_chars);
    let rendered_tokens: Vec<&[char]> =
        rendered_runs.iter().filter_map(|r| rendered.get(r.clone())).collect();
    let source_tokens: Vec<&[char]> = source_runs.iter().filter_map(|r| src_chars.get(r.clone())).collect();
    let tokens = capture_diff_slices(Algorithm::Myers, &rendered_tokens, &source_tokens);
    let span = |runs: &[Range<usize>], index: usize, len: usize| {
        let start = runs.get(index).map_or(0, |r| r.start);
        let end = if len == 0 { start } else { runs.get(index + len - 1).map_or(start, |r| r.end) };
        start..end
    };
    let mut flat: Vec<Option<usize>> = vec![None; rendered.len()];
    let mut map = |old: Range<usize>, new: Range<usize>| {
        for (target, byte) in
            flat.iter_mut().skip(old.start).take(old.len()).zip(src_bytes.iter().skip(new.start))
        {
            *target = Some(*byte);
        }
    };
    for op in tokens {
        match op {
            DiffOp::Equal { old_index, new_index, len } => {
                map(span(&rendered_runs, old_index, len), span(&source_runs, new_index, len));
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                let old = span(&rendered_runs, old_index, old_len);
                let new = span(&source_runs, new_index, new_len);
                let (Some(old_chars), Some(new_chars)) =
                    (rendered.get(old.clone()), src_chars.get(new.clone()))
                else {
                    continue;
                };
                for op in capture_diff_slices(Algorithm::Myers, old_chars, new_chars) {
                    if let DiffOp::Equal { old_index, new_index, len } = op {
                        let at = old.start + old_index;
                        let from = new.start + new_index;
                        map(at..at + len, from..from + len);
                    }
                }
            }
            DiffOp::Delete { .. } | DiffOp::Insert { .. } => {}
        }
    }

    // Split the flat map back into lines (the joining '\n' entries are dropped).
    let mut cursor = 0;
    rendered_lines
        .iter()
        .map(|line| {
            let n = line.chars().count();
            let offsets = flat.get(cursor..cursor + n).map(<[_]>::to_vec).unwrap_or_default();
            cursor += n + 1;
            offsets
        })
        .collect()
}

/// Maximal runs of word characters or of whitespace; any other character is a run alone.
fn runs(chars: &[char]) -> Vec<Range<usize>> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut previous = Class::Other;
    for (i, &c) in chars.iter().enumerate() {
        let current = class(c);
        match runs.last_mut() {
            Some(run) if current != Class::Other && current == previous => run.end = i + 1,
            _ => runs.push(i..i + 1),
        }
        previous = current;
    }
    runs
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn maps_through_concealed_markup() {
        let source = "Ship the **login page** by Friday.";
        let map = align(&["Ship the login page by Friday.".to_owned()], source, 100);
        let line = map.first().expect("one line");
        // 'l' of "login" is rendered char 9; in source it is byte 11 (after "**").
        assert_eq!(line.get(9).copied().flatten(), Some(111));
        assert_eq!(line.first().copied().flatten(), Some(100));
        assert_eq!(line.last().copied().flatten(), Some(100 + source.len() - 1));
    }

    #[test]
    fn decoration_maps_to_nothing() {
        let map = align(&["• alpha".to_owned(), "• beta".to_owned()], "- alpha\n- beta", 0);
        assert_eq!(map.first().and_then(|l| l.first().copied()), Some(None));
        assert_eq!(map.first().and_then(|l| l.get(2).copied()), Some(Some(2)));
        assert_eq!(map.get(1).and_then(|l| l.get(2).copied()), Some(Some(10)));
    }
}
