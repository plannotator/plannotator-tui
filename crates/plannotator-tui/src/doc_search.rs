//! Finding text in the open document (`/`, then `n` and `N`).
//!
//! Matching runs over the rendered text, the characters the reader sees, in the lines the
//! renderer produced before wrapping. A soft wrap therefore never enters the searched text,
//! so a match can run across one, and the index is built once per document rather than once
//! per width. Every rendered character carries its source byte (`srcmap`), so a match comes
//! back as a source range: the same coordinates the screen's cells carry, which is how the
//! highlight finds its cells after any wrap, in a table cell or in a list item.

use std::ops::Range;

use crate::layout::DocLayout;

/// Rendered characters that came from consecutive source bytes, so a byte of `text` inside
/// it converts to a source byte by offset alone.
#[derive(Debug, Clone, Copy)]
struct Run {
    text: usize,
    source: usize,
    len: usize,
}

/// The document's rendered text, ready to search.
#[derive(Debug)]
pub(crate) struct Index {
    /// Every rendered line, one per `\n`. A query is one line, so no match crosses a line
    /// break: a hard break, a list item, a table row or a block boundary.
    text: String,
    /// `text` with each character folded to lower case, byte for byte the same length.
    folded: String,
    /// The mapped stretches of `text`, in text order. Decoration (bullets, borders) and
    /// the space a soft break became map to nothing and fall between runs.
    runs: Vec<Run>,
}

impl Index {
    pub(crate) fn of(layout: &DocLayout) -> Self {
        let mut index = Self { text: String::new(), folded: String::new(), runs: Vec::new() };
        for (line, offsets) in layout.rendered_lines() {
            let chars = line.spans.iter().flat_map(|span| span.content.chars());
            for (ch, offset) in chars.zip(offsets.iter().copied().chain(std::iter::repeat(None))) {
                index.push(ch, offset);
            }
            index.push('\n', None);
        }
        index
    }

    fn push(&mut self, ch: char, offset: Option<usize>) {
        let at = self.text.len();
        self.text.push(ch);
        self.folded.push(fold(ch));
        let Some(source) = offset else { return };
        match self.runs.last_mut() {
            Some(run) if run.text + run.len == at && run.source + run.len == source => {
                run.len += ch.len_utf8();
            }
            _ => self.runs.push(Run { text: at, source, len: ch.len_utf8() }),
        }
    }

    /// The source range of every match of `query`, in document order. Plain text, not a
    /// pattern; case is ignored unless the query has a capital letter (smartcase). A match
    /// made only of decoration has no source to show and is left out.
    pub(crate) fn find(&self, query: &str) -> Vec<Range<usize>> {
        if query.is_empty() {
            return Vec::new();
        }
        let (haystack, needle) = if query.chars().any(char::is_uppercase) {
            (&self.text, query.to_owned())
        } else {
            (&self.folded, query.chars().map(fold).collect())
        };
        // Matches arrive in text order, so the run each one starts in is found by walking
        // forward from the last one's, not by a search per match.
        let mut run = 0;
        let mut found: Vec<Range<usize>> = haystack
            .match_indices(needle.as_str())
            .filter_map(|(start, text)| {
                while self.runs.get(run).is_some_and(|r| r.text + r.len <= start) {
                    run += 1;
                }
                self.source_of(run, start..start + text.len())
            })
            .collect();
        // Rendered order is source order everywhere but a reflowed table; sorting an
        // already sorted list is one pass.
        found.sort_by_key(|range| range.start);
        found
    }

    /// The source bytes under `text[span]`, from its first mapped byte to just past its
    /// last. `run` is the first run that ends after the span starts.
    fn source_of(&self, run: usize, span: Range<usize>) -> Option<Range<usize>> {
        let first = self.runs.get(run)?;
        if first.text >= span.end {
            return None;
        }
        let mut last_index = run;
        while self.runs.get(last_index + 1).is_some_and(|r| r.text < span.end) {
            last_index += 1;
        }
        let last = self.runs.get(last_index)?;
        let start = first.source + span.start.saturating_sub(first.text);
        let end = last.source + (span.end.min(last.text + last.len) - last.text);
        Some(start..end)
    }
}

/// Lower case where it keeps the character's UTF-8 length, so the folded text lines up with
/// the original byte for byte. The few characters whose lower case is longer or shorter
/// (`İ`, the Kelvin sign) match only as typed.
fn fold(ch: char) -> char {
    let mut lower = ch.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(folded), None) if folded.len_utf8() == ch.len_utf8() => folded,
        _ => ch,
    }
}

/// The match covering source byte `offset`.
pub(crate) fn covering(matches: &[Range<usize>], offset: usize) -> Option<usize> {
    let index = matches.partition_point(|range| range.end <= offset);
    matches.get(index).is_some_and(|range| range.start <= offset).then_some(index)
}

/// The first match starting at or after `offset`, else the first: where typing lands.
pub(crate) fn at_or_after(matches: &[Range<usize>], offset: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let index = matches.partition_point(|range| range.start < offset);
    Some(if index < matches.len() { index } else { 0 })
}

/// The match after `offset` and whether reaching it wrapped past the end.
pub(crate) fn next(matches: &[Range<usize>], offset: usize) -> Option<(usize, bool)> {
    if matches.is_empty() {
        return None;
    }
    let index = matches.partition_point(|range| range.start <= offset);
    Some(if index < matches.len() { (index, false) } else { (0, true) })
}

/// The match before `offset` and whether reaching it wrapped past the start.
pub(crate) fn previous(matches: &[Range<usize>], offset: usize) -> Option<(usize, bool)> {
    let last = matches.len().checked_sub(1)?;
    Some(match matches.partition_point(|range| range.start < offset).checked_sub(1) {
        Some(index) => (index, false),
        None => (last, true),
    })
}

/// One match on from `current` of `len`, forward or back, and whether that wrapped.
pub(crate) fn step(len: usize, current: usize, forward: bool) -> Option<(usize, bool)> {
    let last = len.checked_sub(1)?;
    Some(match (forward, current) {
        (true, c) if c >= last => (0, true),
        (true, c) => (c + 1, false),
        (false, 0) => (last, true),
        (false, c) => (c.min(len) - 1, false),
    })
}

/// Per column of a row, the match it shows. A column with no source byte (the space a soft
/// break became, a wide table's padding) between two columns of one match belongs to it, so
/// the highlight has no holes.
pub(crate) fn row_hits(cells: &[Option<usize>], matches: &[Range<usize>]) -> Vec<Option<usize>> {
    let mut hits: Vec<Option<usize>> =
        cells.iter().map(|cell| cell.and_then(|o| covering(matches, o))).collect();
    let mut previous: Option<(usize, Option<usize>)> = None;
    for (col, cell) in cells.iter().enumerate() {
        if cell.is_none() {
            continue;
        }
        let hit = hits.get(col).copied().flatten();
        if let (Some((from, Some(before))), Some(now)) = (previous, hit)
            && before == now
            && let Some(gap) = hits.get_mut(from + 1..col)
        {
            gap.fill(Some(now));
        }
        previous = Some((col, hit));
    }
    hits
}

#[cfg(test)]
mod tests;
