//! The notes rail's order and wording, as pure functions over the review's notes.
//!
//! Notes on the whole change come first, under "Whole change", as the feedback lists the
//! browser's global comments first. Then notes are grouped by file, files by path, as the
//! feedback text orders them: file comments first, then by line. Notes whose code is gone
//! sit last, under "Outdated".

use plannotator_tui_review::{CodeAnnotation, NoteScope, NoteType, Status};

/// One row of the rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row<'a> {
    /// The heading over notes on the whole change.
    Change,
    /// A file's heading.
    File(&'a str),
    /// The heading over notes whose code is gone.
    Outdated,
    /// A note, by its index in the record.
    Note(usize),
}

pub(super) fn is_outdated(note: &CodeAnnotation) -> bool {
    note.outdated || note.status() == Some(Status::Outdated)
}

/// Whether the diff shows the note's lines, so it gets a gutter marker.
pub(super) fn in_diff(note: &CodeAnnotation) -> bool {
    note.scope == NoteScope::Line && matches!(note.status(), None | Some(Status::StillThere | Status::Moved))
}

/// The notes' indices in rail order.
pub(super) fn order(notes: &[CodeAnnotation]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..notes.len()).collect();
    indices.sort_by_key(|&i| {
        let note = notes.get(i);
        note.map(|n| {
            let change = n.scope == NoteScope::General;
            (is_outdated(n), !change, n.file_path.clone(), n.scope != NoteScope::File, n.line_start, i)
        })
    });
    indices
}

/// The rail: a heading before the notes on the whole change, before each file's notes, and
/// before the outdated ones.
pub(super) fn rows(notes: &[CodeAnnotation]) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    let mut last: Option<(bool, Option<&str>)> = None;
    for index in order(notes) {
        let Some(note) = notes.get(index) else { continue };
        let path = (note.scope != NoteScope::General).then_some(note.file_path.as_str());
        let group = (is_outdated(note), path);
        if last != Some(group) {
            match group {
                (true, _) if last.is_some_and(|(outdated, _)| outdated) => {}
                (true, _) => rows.push(Row::Outdated),
                (false, None) => rows.push(Row::Change),
                (false, Some(path)) => rows.push(Row::File(path)),
            }
            last = Some(group);
        }
        rows.push(Row::Note(index));
    }
    rows
}

/// Where a note is: `file`, `12 new`, `12-14 old`. Outdated notes also name their file,
/// since their group does not.
pub(super) fn place(note: &CodeAnnotation) -> String {
    let at = match note.scope {
        NoteScope::File => "file".to_owned(),
        NoteScope::General => "review note".to_owned(),
        NoteScope::Line if note.line_start == note.line_end => {
            format!("{} {}", note.line_start, note.side.as_str())
        }
        NoteScope::Line => format!("{}-{} {}", note.line_start, note.line_end, note.side.as_str()),
    };
    if is_outdated(note) && note.scope != NoteScope::General {
        format!("{} {at}", file_name(&note.file_path))
    } else {
        at
    }
}

/// The note's glyph: a comment or a suggested change.
pub(super) fn glyph(note: &CodeAnnotation) -> &'static str {
    match note.kind {
        NoteType::Suggestion => "\u{270e}",
        NoteType::Comment | NoteType::Concern => "\u{1f4ac}",
    }
}

/// What the rail shows of a note: its comment, else its suggested code.
pub(super) fn body(note: &CodeAnnotation) -> String {
    let text = note.text.as_deref().filter(|t| !t.is_empty());
    match (text, note.suggested_code.as_deref()) {
        (Some(text), Some(code)) => format!("{text}\n{code}"),
        (Some(text), None) => text.to_owned(),
        (None, Some(code)) => code.to_owned(),
        (None, None) => String::new(),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn note(path: &str, scope: &str, line: u32, outdated: bool) -> CodeAnnotation {
        let json = format!(
            r#"{{"id":"{path}{line}","type":"comment","scope":"{scope}","filePath":"{path}","lineStart":{line},
               "lineEnd":{line},"side":"new","createdAt":1,"outdated":{outdated}}}"#
        );
        serde_json::from_str(&json).expect("note")
    }

    #[test]
    fn files_sort_by_path_file_notes_first_and_outdated_last() {
        let notes = vec![
            note("src/b.rs", "line", 9, false),
            note("src/a.rs", "line", 3, true),
            note("src/a.rs", "line", 7, false),
            note("src/b.rs", "file", 1, false),
            note("src/c.rs", "line", 1, true),
        ];
        assert_eq!(
            rows(&notes),
            [
                Row::File("src/a.rs"),
                Row::Note(2),
                Row::File("src/b.rs"),
                Row::Note(3),
                Row::Note(0),
                Row::Outdated,
                Row::Note(1),
                Row::Note(4),
            ]
        );
    }

    #[test]
    fn notes_on_the_whole_change_come_first_under_their_own_heading() {
        let mut change = note("", "general", 0, false);
        change.line_end = 0;
        assert_eq!(place(&change), "review note");
        let notes = vec![note("src/a.rs", "line", 3, false), change, note("src/a.rs", "file", 1, false)];
        assert_eq!(
            rows(&notes),
            [Row::Change, Row::Note(1), Row::File("src/a.rs"), Row::Note(2), Row::Note(0)]
        );
    }

    #[test]
    fn places_name_lines_and_sides_and_outdated_ones_their_file() {
        let mut range = note("src/a.rs", "line", 3, false);
        range.line_end = 5;
        range.side = plannotator_tui_review::Side::Old;
        assert_eq!(place(&range), "3-5 old");
        assert_eq!(place(&note("src/a.rs", "file", 1, false)), "file");
        assert_eq!(place(&note("src/a.rs", "line", 3, true)), "a.rs 3 new");
    }
}
