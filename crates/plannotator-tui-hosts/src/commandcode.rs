//! Command Code (`cmd`): pi's session format, one directory per project under
//! `~/.commandcode/projects/<slug>/<uuid>.jsonl`.
//!
//! The first line of a session is a `session` header carrying the `cwd`; the rest are pi
//! entries (`message`, `session_info`, `model_change`, ...) chained by `id`/`parentId`, so
//! pi's reader applies unchanged. The directory name is `@sindresorhus/slugify(cwd)`, or
//! `root` when that is empty (`getProjectDirName` in `command-code` 1.88.0). A slug is
//! lossy (case, punctuation and runs of separators collapse), so a session counts for a cwd
//! only when its header names that cwd, and a cwd whose slug directory holds no such session
//! is looked for in every project directory. The same directory also holds sidecars
//! (`<uuid>.checkpoints.jsonl`, `<uuid>.meta.json`), which are never sessions.

use std::io::BufRead as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Message, pi};

/// The projects directory relative to `$HOME`.
pub const DEFAULT_PROJECTS_DIR: &str = ".commandcode/projects";

/// The exact session `<id>.jsonl`: the cwd's slug directory first, then every other project.
pub fn find_transcript_by_id(
    projects_dir: &Path,
    cwd: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, crate::HostError> {
    let id = crate::validate_session_id(session_id)?;
    let preferred = projects_dir.join(project_slug(cwd));
    let file = |dir: &Path| dir.join(format!("{id}.jsonl"));
    if file(&preferred).is_file() {
        return Ok(Some(file(&preferred)));
    }
    let mut dirs = subdirs(projects_dir);
    dirs.sort();
    Ok(dirs.into_iter().filter(|dir| *dir != preferred).map(|dir| file(&dir)).find(|path| path.is_file()))
}

/// The newest session whose header names `cwd` and that holds at least one message, by
/// last write as pi ranks them. Nothing when no session was started in `cwd`: a session
/// from another folder is never offered as this one.
pub fn find_transcript(projects_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    let preferred = projects_dir.join(project_slug(cwd));
    let started_here = |files: Vec<PathBuf>| {
        files.into_iter().filter(|path| header_cwd(path).is_some_and(|c| same_dir(&c, cwd)))
    };
    if let Some(found) = pi::newest_with_messages(started_here(session_files(&preferred)).collect()) {
        return Some(found);
    }
    let elsewhere: Vec<PathBuf> = subdirs(projects_dir)
        .into_iter()
        .filter(|dir| *dir != preferred)
        .flat_map(|dir| session_files(&dir))
        .collect();
    pi::newest_with_messages(started_here(elsewhere).collect())
}

/// Command Code writes pi's entries; the same reader applies.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    pi::parse_messages(jsonl, n)
}

/// The project directory name for `cwd`: `@sindresorhus/slugify` 2.x with its defaults,
/// else `root`. `&` reads as ` and `; camel case is split into words; an apostrophe before a
/// final `t` or `s` is dropped; the result is lowercased and every run of anything but
/// ASCII letters and digits becomes one `-`, trimmed at both ends. Non-ASCII letters are
/// transliterated by the library and treated as separators here; the header scan in
/// [`find_transcript`] finds those sessions anyway.
pub fn project_slug(cwd: &Path) -> String {
    let text = cwd.to_string_lossy().replace('&', " and ");
    let lower: Vec<char> = decamelize(text.chars().collect()).iter().map(char::to_ascii_lowercase).collect();
    let mut slug = String::new();
    let mut separated = false;
    for (i, c) in lower.iter().enumerate() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if separated && !slug.is_empty() {
                slug.push('-');
            }
            separated = false;
            slug.push(*c);
        } else if !(*c == '\'' && is_contraction(&lower, i)) {
            separated = true;
        }
    }
    if slug.is_empty() { "root".to_owned() } else { slug }
}

/// `word't` or `word's` followed by whitespace or the end: the library drops the apostrophe.
fn is_contraction(chars: &[char], at: usize) -> bool {
    back(chars, at, 1).is_some_and(char::is_ascii_alphanumeric)
        && chars.get(at + 1).is_some_and(|c| matches!(c, 't' | 's'))
        && chars.get(at + 2).is_none_or(|c| c.is_whitespace())
}

/// The library's four decamelize passes, each a space inserted at every position where its
/// pattern's two groups meet: `ABC1` → `ABC 1`, `aBC` → `a BC`, `aB` → `a B`, and
/// `ABCd` → `AB Cd` (but not before an `s`, so `APIs` stays one word).
fn decamelize(chars: Vec<char>) -> Vec<char> {
    (0..4).fold(chars, |chars, pass| {
        let mut out = Vec::with_capacity(chars.len() + 4);
        for (i, c) in chars.iter().enumerate() {
            if splits_before(pass, &chars, i) {
                out.push(' ');
            }
            out.push(*c);
        }
        out
    })
}

/// Does decamelize pass `pass` put a space before `s[i]`?
fn splits_before(pass: u8, s: &[char], i: usize) -> bool {
    let upper = |c: Option<&char>| c.is_some_and(char::is_ascii_uppercase);
    let lower_or_digit = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    match pass {
        0 => s.get(i).is_some_and(char::is_ascii_digit) && upper(back(s, i, 1)) && upper(back(s, i, 2)),
        1 => lower_or_digit(back(s, i, 1)) && upper(s.get(i)) && upper(s.get(i + 1)),
        2 => lower_or_digit(back(s, i, 1)) && upper(s.get(i)),
        _ => {
            upper(back(s, i, 1))
                && upper(s.get(i))
                && s.get(i + 1).is_some_and(|c| c.is_ascii_digit() || (c.is_ascii_lowercase() && *c != 's'))
        }
    }
}

fn back(chars: &[char], at: usize, by: usize) -> Option<&char> {
    chars.get(at.checked_sub(by)?)
}

/// `<uuid>.jsonl` files directly in `dir`; sidecars such as `<uuid>.checkpoints.jsonl` are
/// not sessions.
fn session_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|x| x == "jsonl")
                && path.file_stem().and_then(|stem| stem.to_str()).is_some_and(crate::is_uuid)
                && path.is_file()
        })
        .collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect()
}

/// The `cwd` of the `session` header on the first line, reading only that line.
fn header_cwd(path: &Path) -> Option<PathBuf> {
    let mut line = String::new();
    std::io::BufReader::new(std::fs::File::open(path).ok()?).read_line(&mut line).ok()?;
    let header: Value = serde_json::from_str(line.trim()).ok()?;
    (header.get("type").and_then(Value::as_str) == Some("session"))
        .then(|| header.get("cwd").and_then(Value::as_str).map(PathBuf::from))
        .flatten()
}

/// Equal paths up to trailing separators; on Windows also up to case and slash direction.
fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let text = p.to_string_lossy();
        let text = text.trim_end_matches(['/', '\\']);
        if cfg!(windows) { text.replace('\\', "/").to_lowercase() } else { text.to_owned() }
    };
    norm(a) == norm(b)
}
