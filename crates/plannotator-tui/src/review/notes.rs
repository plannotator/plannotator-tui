//! The notes rail's order and wording, as pure functions over the review's notes.
//!
//! Notes on the whole change come first, under "Whole change", as the feedback lists the
//! browser's global comments first. Then notes are grouped by file, files by path, as the
//! feedback text orders them: file comments first, then by line. Line notes the diff does
//! not show (made in the whole file, or whose lines are unchanged now) follow under "Not in
//! the diff", and notes whose code is gone sit last, under "Outdated".

use plannotator_tui_review::{CodeAnnotation, NoteScope, NoteType, Status};

/// One row of the rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row<'a> {
    /// The heading over notes on the whole change.
    Change,
    /// A file's heading.
    File(&'a str),
    /// The heading over line notes the diff does not show.
    NotInDiff,
    /// The heading over notes whose code is gone.
    Outdated,
    /// A note, by its index in the record.
    Note(usize),
}

pub(super) fn is_outdated(note: &CodeAnnotation) -> bool {
    note.outdated || note.status() == Some(Status::Outdated)
}

/// A line note whose code is there but that the diff does not show: made outside the diff
/// in the whole file, or on lines that are unchanged now. Enter shows it in the whole file.
pub(super) fn is_not_in_diff(note: &CodeAnnotation) -> bool {
    note.scope == NoteScope::Line && !is_outdated(note) && note.status() == Some(Status::NotInDiff)
}

/// The rail's groups, in order: in the diff, not in the diff, outdated.
fn group(note: &CodeAnnotation) -> u8 {
    if is_outdated(note) { 2 } else { u8::from(is_not_in_diff(note)) }
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
            (group(n), !change, n.file_path.clone(), n.scope != NoteScope::File, n.line_start, i)
        })
    });
    indices
}

/// The rail: a heading before the notes on the whole change, before each file's notes,
/// before the notes not in the diff, and before the outdated ones.
pub(super) fn rows(notes: &[CodeAnnotation]) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    let mut last: Option<(u8, Option<&str>)> = None;
    for index in order(notes) {
        let Some(note) = notes.get(index) else { continue };
        let path = (note.scope != NoteScope::General).then_some(note.file_path.as_str());
        let at = (group(note), path);
        if last != Some(at) {
            match at {
                (kind, _) if kind > 0 && last.is_some_and(|(previous, _)| previous == kind) => {}
                (2, _) => rows.push(Row::Outdated),
                (1, _) => rows.push(Row::NotInDiff),
                (_, None) => rows.push(Row::Change),
                (_, Some(path)) => rows.push(Row::File(path)),
            }
            last = Some(at);
        }
        rows.push(Row::Note(index));
    }
    rows
}

/// Where a note is: `file`, `12 new`, `12-14 old`. Notes not in the diff and outdated ones
/// also name their file, since their group does not.
pub(super) fn place(note: &CodeAnnotation) -> String {
    let at = match note.scope {
        NoteScope::File => "file".to_owned(),
        NoteScope::General => "review note".to_owned(),
        NoteScope::Line if note.line_start == note.line_end => {
            format!("{} {}", note.line_start, note.side.as_str())
        }
        NoteScope::Line => format!("{}-{} {}", note.line_start, note.line_end, note.side.as_str()),
    };
    if group(note) > 0 && note.scope != NoteScope::General {
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
    fn notes_the_diff_does_not_show_have_their_own_group_before_outdated() {
        let mut outside = note("src/b.rs", "line", 40, false);
        outside.plannotator_tui.resolution = Some(Status::NotInDiff);
        let mut unchanged = note("src/a.rs", "line", 2, false);
        unchanged.plannotator_tui.resolution = Some(Status::NotInDiff);
        let notes =
            vec![outside, note("src/a.rs", "line", 9, true), unchanged, note("src/a.rs", "line", 5, false)];
        assert_eq!(
            rows(&notes),
            [
                Row::File("src/a.rs"),
                Row::Note(3),
                Row::NotInDiff,
                Row::Note(2),
                Row::Note(0),
                Row::Outdated,
                Row::Note(1)
            ]
        );
        assert_eq!(notes.first().map(place).as_deref(), Some("b.rs 40 new"));
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
