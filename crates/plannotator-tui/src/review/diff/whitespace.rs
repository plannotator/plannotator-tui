//! Which lines a hidden-whitespace diff does not show: the lines of a hunk whose every change
//! differs from a line it pairs with only in whitespace, which the viewer draws as its
//! heading alone. The viewer says nothing about them, so the app can tell before it uses a
//! selection or a note's lines whether they are still on screen.
//!
//! The rule is the viewer's (`diffs_rs::engine::Whitespace`): in each run of removed and
//! added lines, a removed line pairs with an added one equal once whitespace is set aside,
//! in order on both sides; a hunk is hidden when every line pairs, so when the run's removed
//! lines and its added lines read the same. Whitespace here is any Unicode whitespace, more
//! than Git's six bytes, so a hunk the viewer hides is never taken for one it shows.

use plannotator_tui_review::{Capture, Side};

use super::place::spans;
use super::{Diff, Lines};
use crate::config::review::Whitespace;

impl Diff {
    /// Whether the diff, with `whitespace`, hides `lines` of `capture` as part of a hunk of
    /// whitespace changes only.
    pub(in crate::review) fn hides(&self, capture: &Capture, lines: &Lines, whitespace: Whitespace) -> bool {
        if whitespace == Whitespace::Show {
            return false;
        }
        let Some((_, sections)) = &self.sections else { return false };
        sections
            .ranges_of(&lines.path)
            .filter_map(|range| capture.patch.get(range))
            .any(|section| hunk_hidden(section, lines.side, lines.first, whitespace))
    }
}

/// Whether the hunk of the file's patch `section` that holds `line` on `side` has only
/// whitespace changes under `whitespace`.
fn hunk_hidden(section: &[u8], side: Side, line: u32, whitespace: Whitespace) -> bool {
    let text = String::from_utf8_lossy(section);
    let mut lines = text.lines().peekable();
    while let Some(row) = lines.next() {
        let Some(span) = spans(row).and_then(|s| s.get(usize::from(side == Side::New)).copied()) else {
            continue;
        };
        let body = std::iter::from_fn(|| lines.next_if(|l| !l.starts_with("@@ ") && !l.starts_with("diff ")));
        if span.is_some_and(|(first, last)| (first..=last).contains(&line)) {
            return only_whitespace(body, whitespace);
        }
        body.for_each(drop);
    }
    false
}

/// Whether a hunk's `body` lines change only whitespace: in each run of removed and added
/// lines, the removed ones read as the added ones.
fn only_whitespace<'a>(body: impl Iterator<Item = &'a str>, whitespace: Whitespace) -> bool {
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let mut changed = false;
    let mut runs_match = true;
    let mut end_run = |removed: &mut Vec<String>, added: &mut Vec<String>| {
        runs_match &= removed == added;
        removed.clear();
        added.clear();
    };
    for line in body {
        match line.as_bytes().first() {
            Some(b'-') => removed.push(normalized(line.get(1..).unwrap_or_default(), whitespace)),
            Some(b'+') => added.push(normalized(line.get(1..).unwrap_or_default(), whitespace)),
            // "\ No newline at end of file" belongs to the line before it.
            Some(b'\\') => continue,
            _ => {
                end_run(&mut removed, &mut added);
                continue;
            }
        }
        changed = true;
    }
    end_run(&mut removed, &mut added);
    changed && runs_match
}

/// `line` as `whitespace` compares it: without its whitespace (`-w`), or with each run of it
/// one space and none at the end (`-b`).
fn normalized(line: &str, whitespace: Whitespace) -> String {
    match whitespace {
        Whitespace::IgnoreAll => line.chars().filter(|c| !c.is_whitespace()).collect(),
        Whitespace::IgnoreChange => {
            let mut out = String::with_capacity(line.len());
            let mut run = false;
            for c in line.chars() {
                if c.is_whitespace() {
                    run = true;
                } else {
                    if std::mem::take(&mut run) {
                        out.push(' ');
                    }
                    out.push(c);
                }
            }
            out
        }
        Whitespace::Show => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTION: &str = "diff --git a/m.py b/m.py\n--- a/m.py\n+++ b/m.py\n\
        @@ -1,3 +1,3 @@\n def a():\n-  x = 1\n+    x = 1\n y\n\
        @@ -10,3 +10,3 @@\n z\n-  w = 2\n+    w = 3\n q\n\
        @@ -20,2 +20,2 @@\n-a  b\n+a b\n-c\n+c   \n\\ No newline at end of file\n";

    fn hidden(side: Side, line: u32, whitespace: Whitespace) -> bool {
        hunk_hidden(SECTION.as_bytes(), side, line, whitespace)
    }

    #[test]
    fn a_hunk_of_whitespace_changes_only_is_hidden_and_one_with_a_real_change_is_not() {
        for side in [Side::Old, Side::New] {
            assert!(hidden(side, 2, Whitespace::IgnoreAll), "re-indented only");
            assert!(!hidden(side, 11, Whitespace::IgnoreAll), "re-indented with a real change");
            assert!(hidden(side, 20, Whitespace::IgnoreAll) && hidden(side, 21, Whitespace::IgnoreChange));
            assert!(!hidden(side, 2, Whitespace::Show));
            assert!(!hidden(side, 50, Whitespace::IgnoreAll), "no hunk holds it");
        }
    }

    #[test]
    fn ignoring_changes_in_amount_still_counts_whitespace_added_where_there_was_none() {
        // `-b`: indentation and runs compare as one space; a space where there was none counts.
        assert!(hidden(Side::New, 2, Whitespace::IgnoreChange));
        let section = "@@ -1 +1 @@\n-a+b\n+a + b\n";
        assert!(hunk_hidden(section.as_bytes(), Side::New, 1, Whitespace::IgnoreAll));
        assert!(!hunk_hidden(section.as_bytes(), Side::New, 1, Whitespace::IgnoreChange));
        // An added line with no partner is a change.
        let section = "@@ -1 +1,2 @@\n-a\n+ a\n+b\n";
        assert!(!hunk_hidden(section.as_bytes(), Side::New, 1, Whitespace::IgnoreAll));
    }
}
