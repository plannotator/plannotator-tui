//! Source map by alignment: which source byte does each rendered character come from?
//!
//! The renderer discards positions, but for one block we hold both its source text and
//! the plain text it rendered to. They differ only by markup the renderer concealed
//! (`**`, `#`, backticks) and decoration it added (bullets, table borders). A character
//! diff between the two lines them up; every rendered character that survived unchanged
//! maps to its source byte, and decoration maps to nothing. No markdown knowledge needed.

use similar::{Algorithm, DiffOp, capture_diff_slices};

/// Per rendered line, per rendered char: the absolute source byte offset, if any.
pub(crate) type LineOffsets = Vec<Option<usize>>;

/// Align `rendered_lines` (plain text, one entry per line) against `source`, whose first
/// byte sits at absolute offset `base`.
pub(crate) fn align(rendered_lines: &[String], source: &str, base: usize) -> Vec<LineOffsets> {
    let rendered: Vec<char> = rendered_lines.join("\n").chars().collect();
    let (src_chars, src_bytes): (Vec<char>, Vec<usize>) =
        source.char_indices().map(|(i, c)| (c, base + i)).unzip();

    let mut flat: Vec<Option<usize>> = vec![None; rendered.len()];
    for op in capture_diff_slices(Algorithm::Myers, &rendered, &src_chars) {
        if let DiffOp::Equal { old_index, new_index, len } = op {
            let targets = flat.iter_mut().skip(old_index).take(len);
            let sources = src_bytes.iter().skip(new_index);
            for (target, byte) in targets.zip(sources) {
                *target = Some(*byte);
            }
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
