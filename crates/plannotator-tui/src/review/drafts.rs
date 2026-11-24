//! Making and changing notes: what `c`, `s`, `C`, `M` and the rail's `e` and `x` do (and
//! Enter and Delete on a note's box in the diff), and saving what the comment box holds.

use anyhow::Result;
use plannotator_tui_review::{CodeAnnotation, NoteType, Side};

use super::diff::Lines;
use super::{Draft, Focus, Mode, Screen};
use crate::ui::compose::Compose;

impl Screen {
    /// The lines a note made now would cover: the selection, else the cursor's line.
    fn target_lines(&self) -> Result<Option<Lines>> {
        match self.diff.selection() {
            Some(lines) => Ok(Some(lines)),
            None => self.diff.cursor(),
        }
    }

    /// `c`: comment on the selection or the cursor line.
    pub(super) fn start_comment(&mut self) -> Result<()> {
        let Some(lines) = self.target_lines()? else {
            self.status = Some("move the cursor onto a line of code first".into());
            return Ok(());
        };
        self.compose = Compose::default();
        self.mode = Mode::Compose(Draft::Comment(lines));
        Ok(())
    }

    /// `s`: the comment box filled with the selected new-side lines; what the person makes
    /// of them is the suggested code.
    pub(super) fn start_suggestion(&mut self) -> Result<()> {
        let Some(lines) = self.target_lines()? else {
            self.status = Some("move the cursor onto a line of code first".into());
            return Ok(());
        };
        if lines.side == Side::Old {
            self.status = Some("a suggestion replaces new lines: pick lines on the new side".into());
            return Ok(());
        }
        let Some(file) = self.file(&lines.path) else { return Ok(()) };
        let code: Vec<&str> = (lines.first..=lines.last).filter_map(|n| file.line(Side::New, n)).collect();
        self.compose = Compose::with_text(&code.join("\n"));
        self.mode = Mode::Compose(Draft::Suggestion(lines));
        Ok(())
    }

    /// `C`: comment on the whole file under the cursor.
    pub(super) fn start_file_comment(&mut self) {
        let Some(path) = self.file_under_cursor() else {
            self.status = Some("move the cursor onto a line of the file first".into());
            return;
        };
        self.compose = Compose::default();
        self.mode = Mode::Compose(Draft::File(path));
    }

    /// `M`, or the Review menu's first row: comment on the whole change, on no file. It sits
    /// first in the rail and first in the feedback, as the browser's global comment does.
    pub(super) fn start_change_comment(&mut self) {
        self.compose = Compose::default();
        self.mode = Mode::Compose(Draft::Change);
    }

    /// `e` in the rail: the comment box with the note's comment, or its code for a
    /// suggestion.
    pub(super) fn start_edit(&mut self) {
        if let Some(id) = self.selected_note().map(|note| note.id.clone()) {
            self.edit_note(&id);
        }
    }

    /// The comment box with note `id`'s comment, or its code for a suggestion; the rail
    /// selects the note.
    pub(super) fn edit_note(&mut self, id: &str) {
        let Some(note) = self.store.annotations().iter().find(|note| note.id == id) else { return };
        let text = match note.kind {
            NoteType::Suggestion => note.suggested_code.clone(),
            NoteType::Comment | NoteType::Concern => note.text.clone(),
        };
        self.compose = Compose::with_text(&text.unwrap_or_default());
        self.mode = Mode::Compose(Draft::Edit(id.to_owned()));
        self.select_note(id);
    }

    /// Save what the comment box holds. An empty comment saves nothing.
    pub(super) fn save_draft(&mut self, draft: &Draft) -> Result<()> {
        self.mode = Mode::Browse;
        let value = self.compose.value();
        let text = value.trim().to_owned();
        // Code is kept as it is, carriage returns and trailing spaces included, so an unedited
        // suggestion is the original; only a new line typed after the last one goes.
        let code = value.strip_suffix('\n').unwrap_or(&value).to_owned();
        let key = self.capture.patch_key.clone();
        let note = match draft {
            _ if text.is_empty() => {
                self.status = Some("empty note not saved".into());
                return Ok(());
            }
            Draft::Comment(lines) => self.file(&lines.path).map(|file| {
                CodeAnnotation::line_comment(file, lines.side, lines.first, lines.last, text, &key)
            }),
            Draft::Suggestion(lines) => self
                .file(&lines.path)
                .map(|file| CodeAnnotation::suggestion(file, lines.first, lines.last, code, None, &key)),
            Draft::File(path) => self.file(path).map(|file| CodeAnnotation::file_comment(file, text, &key)),
            Draft::Change => Some(CodeAnnotation::general_comment(text, &key)),
            Draft::Outside(lines, code) => Some(CodeAnnotation::outside_diff_comment(
                &lines.path,
                lines.first,
                lines.last,
                code.clone(),
                text,
                &key,
            )),
            Draft::Edit(id) => {
                let changed = self.store.edit(id, |note| {
                    let field = match note.kind {
                        NoteType::Suggestion => &mut note.suggested_code,
                        NoteType::Comment | NoteType::Concern => &mut note.text,
                    };
                    let value = if note.kind == NoteType::Suggestion { code } else { text };
                    let changed = field.as_deref() != Some(value.as_str());
                    *field = Some(value);
                    changed
                })?;
                self.status = Some(if changed { "note updated" } else { "note unchanged" }.into());
                self.after_note_change();
                return Ok(());
            }
        };
        let Some(note) = note else {
            self.status = Some("that file is no longer in the diff".into());
            return Ok(());
        };
        self.add_note(note)?;
        self.status = Some("note saved".into());
        Ok(())
    }

    pub(super) fn add_note(&mut self, note: CodeAnnotation) -> Result<()> {
        let id = note.id.clone();
        self.store.add(note)?;
        // The selection became the note; the next one starts fresh.
        let _ = self.diff.clear_selection();
        self.after_note_change();
        self.select_note(&id);
        Ok(())
    }

    /// `x` in the rail.
    pub(super) fn remove_selected(&mut self) -> Result<()> {
        match self.selected_note().map(|n| n.id.clone()) {
            Some(id) => self.remove_note(&id),
            None => Ok(()),
        }
    }

    /// Remove note `id`, at once, as the rail's `x` does.
    pub(super) fn remove_note(&mut self, id: &str) -> Result<()> {
        if self.store.remove(id)? {
            self.status = Some("note removed".into());
        }
        self.after_note_change();
        if self.store.annotations().is_empty() {
            self.focus = Focus::Diff;
        }
        Ok(())
    }
}
