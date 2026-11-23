//! The rows the viewer lays out for each file, worked out from the patch it was given with
//! its own layout code (`diffs_rs::projection`), so the app can say which file and which
//! hunk a row belongs to without moving the viewer's cursor. Files are read when first
//! needed, in order, so a large review reads only the ones the cursor visits.

use std::collections::HashMap;
use std::ops::Range;

use diffs_rs::model::{Document, RowKind};
use diffs_rs::projection::Projection;

/// One file's rows.
struct File {
    projection: Projection,
    /// Each line's id by its numbers, as the viewer reports them for the cursor.
    lines: HashMap<(Option<u32>, Option<u32>), usize>,
    /// Each hunk's first and last line on the old side and on the new one.
    hunks: Vec<[Option<(u32, u32)>; 2]>,
}

/// The patch the viewer reads and the files read from it so far.
#[derive(Default)]
pub(super) struct Rows {
    patch: Vec<u8>,
    /// Each file patch's bytes in `patch`, as the viewer splits them.
    units: Vec<Range<usize>>,
    /// The next unit to read.
    next: usize,
    files: Vec<File>,
}

/// What a row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    Heading,
    Hunk(usize),
    Gap,
    Note,
    Code,
    EndMark,
}

impl Rows {
    pub(super) fn new(patch: Vec<u8>) -> Self {
        let mut units = Vec::new();
        let _ = diffs_rs::ingest::visit_files(std::io::Cursor::new(&patch), |base, unit| {
            let start = usize::try_from(base).unwrap_or(usize::MAX);
            units.push(start..start.saturating_add(unit.len()));
            Ok(())
        });
        Self { patch, units, next: 0, files: Vec::new() }
    }

    /// File `index`, read now if it was not yet. `None` past the end, or when the viewer's
    /// own parser fails on it (the viewer then shows an error).
    fn file(&mut self, index: usize) -> Option<&File> {
        while self.files.len() <= index {
            let unit = self.units.get(self.next)?.clone();
            self.next += 1;
            let bytes = self.patch.get(unit)?;
            let Ok(parsed) = Document::parse_file(bytes) else { return None };
            for meta in &parsed.files {
                let projection = Projection::new(&parsed, meta);
                let base = meta.hunks.first().map_or(0, |h| h.lines.start);
                let mut lines = HashMap::new();
                let mut hunks = Vec::new();
                for hunk in &meta.hunks {
                    let mut spans: [Option<(u32, u32)>; 2] = [None, None];
                    for id in hunk.lines.clone() {
                        let Some(line) = parsed.lines.get(id) else { continue };
                        lines.insert((line.old, line.new), id - base);
                        for (span, number) in spans.iter_mut().zip([line.old, line.new]) {
                            if let Some(n) = number {
                                let span = span.get_or_insert((n, n));
                                *span = (span.0.min(n), span.1.max(n));
                            }
                        }
                    }
                    hunks.push(spans);
                }
                self.files.push(File { projection, lines, hunks });
            }
        }
        self.files.get(index)
    }

    /// How many rows file `index` takes.
    pub(super) fn len(&mut self, index: usize, split: bool) -> Option<usize> {
        Some(self.file(index)?.projection.rows(split, 0, 0).len())
    }

    /// What row `row` of file `index` shows.
    pub(super) fn kind(&mut self, index: usize, split: bool, row: usize) -> Option<Row> {
        let row = self.file(index)?.projection.rows(split, 0, 0).get(row)?;
        Some(match row.kind {
            RowKind::Header => Row::Heading,
            RowKind::Hunk(h) => Row::Hunk(h),
            RowKind::Gap => Row::Gap,
            RowKind::Metadata => Row::Note,
            RowKind::Code { .. } => Row::Code,
            RowKind::NoNewline { .. } => Row::EndMark,
        })
    }

    /// The row of the line numbered `old` and `new` (one may be `None`) in file `index`.
    pub(super) fn row_of(
        &mut self,
        index: usize,
        split: bool,
        old: Option<u32>,
        new: Option<u32>,
    ) -> Option<usize> {
        let file = self.file(index)?;
        let id = *file.lines.get(&(old, new))?;
        Some(file.projection.row_for_source(id, split))
    }

    /// The row of line `line` on the old side (`old`) or the new one in file `index`.
    pub(super) fn row_on(&mut self, index: usize, split: bool, old: bool, line: u32) -> Option<usize> {
        let file = self.file(index)?;
        let id = file
            .lines
            .iter()
            .find(|((o, n), _)| if old { *o == Some(line) } else { *n == Some(line) })
            .map(|(_, id)| *id)?;
        Some(file.projection.row_for_source(id, split))
    }

    /// The first and last line on one side of the hunk that holds `line` on that side.
    pub(super) fn hunk_span(&mut self, index: usize, old: bool, line: u32) -> Option<(u32, u32)> {
        let file = self.file(index)?;
        file.hunks
            .iter()
            .filter_map(|&[on_old, on_new]| if old { on_old } else { on_new })
            .find(|&(first, last)| (first..=last).contains(&line))
    }

    /// The row of hunk `hunk`'s heading in file `index`.
    pub(super) fn hunk_row(&mut self, index: usize, split: bool, hunk: usize) -> Option<usize> {
        let rows = self.file(index)?.projection.rows(split, 0, 0);
        let mut row = rows.first_boundary(true, false)?;
        for _ in 0..hunk {
            row = rows.boundary(row, true, false)?;
        }
        Some(row)
    }

    /// The row the next (`forward`) or previous hunk heading is on, from row `row` of file
    /// `index`, in it or in the files after or before it, as the viewer's `]` and `[` go.
    pub(super) fn hunk_from(
        &mut self,
        index: usize,
        split: bool,
        row: usize,
        forward: bool,
        files: usize,
    ) -> Option<(usize, usize)> {
        if let Some(target) = self.file(index)?.projection.rows(split, 0, 0).boundary(row, forward, false) {
            return Some((index, target));
        }
        let others: Box<dyn Iterator<Item = usize>> =
            if forward { Box::new(index + 1..files) } else { Box::new((0..index).rev()) };
        for other in others {
            if let Some(target) =
                self.file(other)?.projection.rows(split, 0, 0).first_boundary(forward, false)
            {
                return Some((other, target));
            }
        }
        None
    }
}
