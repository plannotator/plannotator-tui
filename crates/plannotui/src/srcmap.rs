//! Source map by alignment: which source byte does each rendered character come from?
//!
//! The renderer discards positions, but for one block we hold both its source text and
//! the plain text it rendered to. They differ only by markup the renderer concealed
//! (`**`, `#`, backticks) and decoration it added (bullets, table borders). A character
//! diff between the two lines them up; every rendered character that survived unchanged
//! maps to its source byte, and decoration maps to nothing. No markdown knowledge needed.

use similar::{Algorithm, DiffOp, capture_diff_slices};

/// Per rendered line, per rendered char: the absolute source byte offset, if any.
pub type LineOffsets = Vec<Option<usize>>;

/// Align `rendered_lines` (plain text, one entry per line) against `source`, whose first
/// byte sits at absolute offset `base`.
pub fn align(rendered_lines: &[String], source: &str, base: usize) -> Vec<LineOffsets> {
    let rendered: Vec<char> = rendered_lines.join("\n").chars().collect();
    let (src_chars, src_bytes): (Vec<char>, Vec<usize>) =
        source.char_indices().map(|(i, c)| (c, base + i)).unzip();

    let mut flat: Vec<Option<usize>> = vec![None; rendered.len()];
    for op in capture_diff_slices(Algorithm::Myers, &rendered, &src_chars) {
        if let DiffOp::Equal { old_index, new_index, len } = op {
            for k in 0..len {
                flat[old_index + k] = Some(src_bytes[new_index + k]);
            }
        }
    }

    // Split the flat map back into lines (the joining '\n' entries are dropped).
    let mut out = Vec::with_capacity(rendered_lines.len());
    let mut cursor = 0;
    for line in rendered_lines {
        let n = line.chars().count();
        out.push(flat[cursor..cursor + n].to_vec());
        cursor += n + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_through_concealed_markup() {
        let source = "Ship the **login page** by Friday.";
        let rendered = vec!["Ship the login page by Friday.".to_string()];
        let map = align(&rendered, source, 100);
        // 'l' of "login" is rendered char 9; in source it is byte 11 (after "**").
        assert_eq!(map[0][9], Some(111));
        assert_eq!(map[0][0], Some(100));
        assert_eq!(map[0].last().copied().flatten(), Some(100 + source.len() - 1));
    }

    #[test]
    fn decoration_maps_to_nothing() {
        let source = "- alpha\n- beta";
        let rendered = vec!["• alpha".to_string(), "• beta".to_string()];
        let map = align(&rendered, source, 0);
        assert_eq!(map[0][0], None);
        assert_eq!(map[0][2], Some(2)); // 'a' of alpha
        assert_eq!(map[1][2], Some(10)); // 'b' of beta
    }

    #[test]
    fn heading_without_marker() {
        let map = align(&["Trust and security".to_string()], "## Trust and security", 50);
        assert_eq!(map[0][0], Some(53));
        assert_eq!(map[0].iter().filter(|o| o.is_none()).count(), 0);
    }
}
