//! Reading a unified git patch: its files, and which lines of each side it shows.
//!
//! Ported from Plannotator so both tools read a patch the same way: file splitting, paths
//! and status from `packages/core/diff-files.ts` and `diff-paths.ts`; the per-side line map
//! and the anchor capture from `packages/review-editor/utils/codeAnnotationAnchor.ts`.
//! Pure over the patch text; no git, no I/O.

use std::collections::BTreeMap;

use crate::annotation::{AnchorContext, Side};

/// Lines of context recorded on each side of an anchor (`ANCHOR_CONTEXT_LINES`).
pub const ANCHOR_CONTEXT_LINES: u32 = 2;
/// Anchor text longer than this (in UTF-16 units, as the browser counts) is not recorded
/// (`MAX_ANCHOR_TEXT_CHARS`).
pub const MAX_ANCHOR_TEXT_CHARS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

/// One file's section of a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// The new path; for a deleted file, its old path (the browser's `DiffFile.path`).
    pub path: String,
    /// The old path, when it differs from `path`.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    /// Git printed `Binary files … differ` (or a binary patch) and no hunks.
    pub binary: bool,
    old: BTreeMap<u32, SideLine>,
    new: BTreeMap<u32, SideLine>,
    /// Function context of each hunk header, in hunk order.
    hunks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SideLine {
    text: String,
    hunk: usize,
}

impl FilePatch {
    /// The text of line `n` of `side`, when the patch shows that line.
    pub fn line(&self, side: Side, n: u32) -> Option<&str> {
        self.side(side).get(&n).map(|l| l.text.as_str())
    }

    /// Every line `side` shows, in line order, with its number.
    pub fn lines(&self, side: Side) -> impl Iterator<Item = (u32, &str)> {
        self.side(side).iter().map(|(n, line)| (*n, line.text.as_str()))
    }

    /// True when every line of `first..=last` on `side` is shown in this file's hunks.
    pub fn shows(&self, side: Side, first: u32, last: u32) -> bool {
        first >= 1 && first <= last && (first..=last).all(|n| self.side(side).contains_key(&n))
    }

    /// The function context of the hunk showing line `n` of `side` (`""` when git printed
    /// none), or `None` when no hunk shows it.
    pub fn hunk_context(&self, side: Side, n: u32) -> Option<&str> {
        let line = self.side(side).get(&n)?;
        self.hunks.get(line.hunk).map(String::as_str)
    }

    /// The anchor the browser records for lines `first..=last` of `side` (`captureAnchor`):
    /// their text joined with `\n`, two lines of context each way (`None` where the patch
    /// does not show the line), and the hunk's function context. `None` when a line is not
    /// in the patch or the text is over [`MAX_ANCHOR_TEXT_CHARS`].
    pub fn anchor(&self, side: Side, first: u32, last: u32) -> Option<(String, AnchorContext)> {
        if !self.shows(side, first, last) {
            return None;
        }
        let text = (first..=last).filter_map(|n| self.line(side, n)).collect::<Vec<_>>().join("\n");
        if text.encode_utf16().count() > MAX_ANCHOR_TEXT_CHARS {
            return None;
        }
        let read = |n: u32| (n >= 1).then(|| self.line(side, n).map(str::to_owned)).flatten();
        let before =
            (0..ANCHOR_CONTEXT_LINES).rev().map(|k| first.checked_sub(k + 1).and_then(read)).collect();
        let after = (1..=ANCHOR_CONTEXT_LINES).map(|k| read(last + k)).collect();
        let hunk = self.hunk_context(side, first).filter(|h| !h.is_empty()).map(str::to_owned);
        Some((text, AnchorContext { before, after, hunk }))
    }

    fn side(&self, side: Side) -> &BTreeMap<u32, SideLine> {
        match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        }
    }
}

/// Every file in `patch`, in patch order. Sections whose paths cannot be read are skipped,
/// as the browser skips them.
pub fn parse(patch: &str) -> Vec<FilePatch> {
    split_files(patch).filter_map(parse_file).collect()
}

/// Every file in `patch` by path, with the new-side object id its `index <old>..<new>` line
/// names, abbreviated as git printed it; `None` for a file with no index line (a mode-only
/// change, a pure rename). Viewed marks check a working-tree hash against it
/// ([`crate::progress`]), as the browser checks against its own patch.
pub(crate) fn new_side_ids(patch: &str) -> BTreeMap<String, Option<String>> {
    split_files(patch)
        .filter_map(|chunk| {
            let lines: Vec<&str> = chunk.split('\n').collect();
            let (file_old, file_new) = file_line_paths(&lines);
            let (_, meta_new) = metadata_paths(&lines);
            let (_, head_new) = git_header_paths(lines.first().copied().unwrap_or(""));
            let file_path = file_new.or(file_old).or(meta_new).or(head_new)?;
            let id = header_lines(&lines)
                .find_map(|l| l.strip_prefix("index "))
                .and_then(|range| range.split_once(".."))
                .map(|(_, new)| new.split(' ').next().unwrap_or_default().trim_end_matches('\r').to_owned());
            Some((file_path, id))
        })
        .collect()
}

/// Sections starting at each `diff --git ` line.
fn split_files(patch: &str) -> impl Iterator<Item = &str> {
    let mut starts: Vec<usize> = Vec::new();
    let mut offset = 0;
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let ends: Vec<usize> = starts.iter().skip(1).copied().chain([patch.len()]).collect();
    starts.into_iter().zip(ends).filter_map(|(start, end)| patch.get(start..end))
}

fn parse_file(chunk: &str) -> Option<FilePatch> {
    let lines: Vec<&str> = chunk.split('\n').collect();
    let (file_old, file_new) = file_line_paths(&lines);
    let (meta_old, meta_new) = metadata_paths(&lines);
    let (head_old, head_new) = git_header_paths(lines.first().copied().unwrap_or(""));
    let old_path = file_old.clone().or_else(|| file_new.clone()).or(meta_old).or(head_old)?;
    let path = file_new.or(file_old).or(meta_new).or(head_new)?;
    let status = status(&lines, &old_path, &path);
    let binary = header_lines(&lines).any(|l| l.starts_with("Binary files ") || l == "GIT binary patch");
    let mut file = FilePatch {
        old_path: (old_path != path).then_some(old_path),
        path,
        status,
        additions: 0,
        deletions: 0,
        binary,
        old: BTreeMap::new(),
        new: BTreeMap::new(),
        hunks: Vec::new(),
    };
    read_hunks(&mut file, &lines);
    Some(file)
}

/// The extended header: everything before the first hunk.
fn header_lines<'a>(lines: &'a [&'a str]) -> impl Iterator<Item = &'a str> {
    lines.iter().copied().take_while(|l| !l.starts_with("@@ "))
}

/// `patchSideLines`, for both sides at once.
fn read_hunks(file: &mut FilePatch, lines: &[&str]) {
    let (mut old_line, mut new_line) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in lines {
        if let Some((old_start, new_start, context)) = hunk_header(raw) {
            (old_line, new_line) = (old_start, new_start);
            file.hunks.push(context.to_owned());
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        let hunk = file.hunks.len().saturating_sub(1);
        let mut chars = raw.chars();
        let marker = chars.next();
        let text = chars.as_str().to_owned();
        match marker {
            Some(' ') => {
                file.old.insert(old_line, SideLine { text: text.clone(), hunk });
                file.new.insert(new_line, SideLine { text, hunk });
                old_line += 1;
                new_line += 1;
            }
            Some('-') => {
                file.old.insert(old_line, SideLine { text, hunk });
                file.deletions += 1;
                old_line += 1;
            }
            Some('+') => {
                file.new.insert(new_line, SideLine { text, hunk });
                file.additions += 1;
                new_line += 1;
            }
            Some('\\') => {} // "\ No newline at end of file" is not a line
            _ => in_hunk = false,
        }
    }
}

/// `@@ -A[,n] +B[,m] @@ context` → (A, B, context).
fn hunk_header(line: &str) -> Option<(u32, u32, &str)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = range_start(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new, rest) = range_start(rest)?;
    let rest = rest.strip_prefix(" @@")?;
    let context = rest.strip_prefix(' ').unwrap_or(rest);
    Some((old, new, context.strip_suffix('\r').unwrap_or(context)))
}

/// `12` or `12,4` at the start of `s`: the 12, and what follows the range.
fn range_start(s: &str) -> Option<(u32, &str)> {
    let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let start = s.get(..digits)?.parse().ok()?;
    let rest = s.get(digits..)?;
    let rest = match rest.strip_prefix(',') {
        Some(count) => count.trim_start_matches(|c: char| c.is_ascii_digit()),
        None => rest,
    };
    Some((start, rest))
}

fn status(lines: &[&str], old_path: &str, path: &str) -> FileStatus {
    for line in lines {
        if line.starts_with("@@") || line.starts_with("--- ") || line.starts_with("+++ ") {
            break;
        }
        if line.starts_with("new file mode") {
            return FileStatus::Added;
        }
        if line.starts_with("deleted file mode") {
            return FileStatus::Deleted;
        }
        if line.starts_with("rename from ") || line.starts_with("copy from ") {
            return FileStatus::Renamed;
        }
    }
    if old_path == path { FileStatus::Modified } else { FileStatus::Renamed }
}

/// `--- a/…` / `+++ b/…` (`parseDiffFilePathLines`); `/dev/null` reads as absent.
fn file_line_paths(lines: &[&str]) -> (Option<String>, Option<String>) {
    let (mut old, mut new) = (None, None);
    for line in lines {
        if line.starts_with("@@ ") || *line == "GIT binary patch" {
            break;
        }
        if let Some(token) = line.strip_prefix("--- ") {
            old = path_token(token, "a/").or(old);
        } else if let Some(token) = line.strip_prefix("+++ ") {
            new = path_token(token, "b/").or(new);
        }
    }
    (old, new)
}

/// `rename from/to`, `copy from/to` (`parseDiffMetadataPathLines`).
fn metadata_paths(lines: &[&str]) -> (Option<String>, Option<String>) {
    let (mut old, mut new) = (None, None);
    for line in lines {
        if let Some(p) = line.strip_prefix("rename from ").or_else(|| line.strip_prefix("copy from ")) {
            old = Some(unquote(p)).filter(|p| p != "/dev/null");
        } else if let Some(p) = line.strip_prefix("rename to ").or_else(|| line.strip_prefix("copy to ")) {
            new = Some(unquote(p)).filter(|p| p != "/dev/null");
        }
    }
    (old, new)
}

/// `diff --git a/X b/Y` (`parseDiffGitHeader`), quoted or not.
fn git_header_paths(header: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = header.strip_prefix("diff --git ") else { return (None, None) };
    if rest.trim_start().starts_with('"')
        && let Some((first, after)) = header_token(rest)
        && let Some((second, _)) = header_token(after)
    {
        return (path_token(first, "a/"), path_token(second, "b/"));
    }
    let Some(rest) = rest.strip_prefix("a/") else { return (None, None) };
    match rest.rfind(" b/") {
        Some(at) if at > 0 && at + 3 < rest.len() => {
            (rest.get(..at).map(str::to_owned), rest.get(at + 3..).map(str::to_owned))
        }
        _ => (None, None),
    }
}

/// One space-separated or double-quoted token, and the rest after it.
fn header_token(input: &str) -> Option<(&str, &str)> {
    let s = input.trim_start();
    if s.starts_with('"') {
        let mut escaped = false;
        for (i, ch) in s.char_indices().skip(1) {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => return Some((s.get(..=i)?, s.get(i + 1..)?)),
                _ => {}
            }
        }
        return None;
    }
    Some(s.split_once(' ').unwrap_or((s, "")))
}

/// A `---`/`+++` token: drop git's trailing tab metadata, unquote, strip the `a/`/`b/`
/// prefix. `/dev/null` and tokens without the prefix read as absent.
fn path_token(token: &str, prefix: &str) -> Option<String> {
    let token = if token.starts_with('"') { token } else { token.split('\t').next().unwrap_or(token) };
    if token == "/dev/null" {
        return None;
    }
    unquote(token).strip_prefix(prefix).map(str::to_owned)
}

/// Undo git's C-style quoting (`unquoteGitPath`): octal escapes are raw UTF-8 bytes.
fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };
    let mut out: Vec<u8> = Vec::with_capacity(inner.len());
    let bytes = inner.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        i += 1;
        if b != b'\\' {
            out.push(b);
            continue;
        }
        let next = bytes.get(i).copied();
        if matches!(next, Some(b'0'..=b'7')) {
            let mut value: u32 = 0;
            let mut digits = 0;
            while let Some(&d @ b'0'..=b'7') = bytes.get(i) {
                if digits == 3 {
                    break;
                }
                value = value * 8 + u32::from(d - b'0');
                digits += 1;
                i += 1;
            }
            out.push((value & 0xff) as u8);
        } else if let Some(byte) = next.and_then(c_escape) {
            out.push(byte);
            i += 1;
        } else {
            out.push(b'\\');
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn c_escape(c: u8) -> Option<u8> {
    Some(match c {
        b'"' => b'"',
        b'\\' => b'\\',
        b'a' => 0x07,
        b'b' => 0x08,
        b'f' => 0x0c,
        b'n' => b'\n',
        b'r' => b'\r',
        b't' => b'\t',
        b'v' => 0x0b,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
