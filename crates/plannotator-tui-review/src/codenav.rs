//! Definition search for the review screen: given a word on a line, the places in the
//! repository that define or use it, best first.
//!
//! A port of the browser app's search-based code navigation
//! (`packages/shared/code-nav.ts`): the same whole-word search, the same definition patterns
//! per language with the same generic fallback, the same path exclusions and the same ranking
//! and caps. One difference: the browser runs `rg` over the working tree, and this runs
//! `git grep -n -w -F` against the side of the diff the person is looking at:
//!
//! | Side ([`SideSource`]) | Searched |
//! |---|---|
//! | `Rev(rev)` | the commit or tree `rev` (`git grep … <rev>`) |
//! | `Index` | the staged content (`git grep --cached`) |
//! | `Worktree` | the files on disk, untracked non-ignored files included (`git grep --untracked`), what `rg` sees |
//!
//! So it finds definitions in an old version, and in a pull request fetched into a bare
//! cache repository with no working tree at all.
//!
//! Where `git grep` cannot do what `rg` does, this mirrors it after the fact or says so:
//! - `rg --max-count 50` (per file) is applied while reading git's output, so it works with
//!   any git version.
//! - `rg` skips hidden files and directories; so does this, except the directories the
//!   origin file is in (the same per-instance rule the browser uses for its excluded
//!   directories), so a file under `.github/` still finds its siblings.
//! - `rg --max-filesize 1M` has no `git grep` counterpart: large files are searched, and the
//!   per-file cap, [`PARSE_CAP`] and the time limit bound the cost instead.
//! - On a `Rev` or `Index` side, tracked files that `.gitignore` also matches are searched;
//!   `rg` would skip them on disk. The `Worktree` side respects `.gitignore` as `rg` does.
//! - `git grep -w` counts only ASCII letters, digits and `_` as word characters; `rg -w` is
//!   Unicode-aware. Non-ASCII identifiers can match where `rg` would not.
//! - Snippets are cut at 200 characters, not 200 UTF-16 units.
//! - [`PARSE_CAP`] counts matches kept after the path filters; the browser counts before.
//!
//! The caller is thin: [`identifier_at`] (or [`identifiers_on_line`]) picks the word under
//! the cursor, [`search`] returns ranked [`Candidate`]s, and [`Found::best`] is the jump
//! target. Nothing here draws or binds keys.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead as _, BufReader, Read as _};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use regex::Regex;

use crate::capture::SideSource;
use crate::error::{Error, Result};
use crate::git;
use crate::patch::FilePatch;

/// Candidates kept after ranking (`rankLocations`' `cap = 50`).
pub const RESULT_CAP: usize = 50;
/// Matches read from the search before it is stopped (`PARSE_CAP`).
pub const PARSE_CAP: usize = 500;
/// Matches kept per file (`rg --max-count 50`).
pub const PER_FILE_CAP: usize = 50;
/// How long a jump search may run (`resolveCodeNav`'s default 5 s).
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// How long a hover search may run (`HOVER_RG_TIMEOUT_MS`).
pub const HOVER_TIMEOUT: Duration = Duration::from_secs(3);
/// A candidate's text is cut to this many characters, then `…` (`parseRgJsonOutput`).
pub const SNIPPET_CHARS: usize = 200;
/// Past this many template literals inside one another, the rest of the line is read as
/// text: each level is a recursion, and a line can come from anyone's diff.
const MAX_TEMPLATE_NESTING: usize = 32;

/// Directory names that can only be tool output: excluded at any depth
/// (`CODE_NAV_ALWAYS_IGNORED_DIRS`).
const ALWAYS_IGNORED_DIRS: [&str; 8] =
    ["node_modules", ".git", ".next", "__pycache__", ".turbo", ".cache", ".venv", ".pytest_cache"];

/// Ambiguous directory names: excluded only at the repository root
/// (`CODE_NAV_ROOT_ONLY_IGNORED_DIRS`).
const ROOT_ONLY_IGNORED_DIRS: [&str; 5] = ["vendor", "target", "build", "dist", "coverage"];

/// The languages that change how a search runs: the browser's `detectLanguage` names for
/// the files `rg --type` knows (`RG_TYPE_MAP`). Any other file searches every file and uses
/// only the generic definition patterns, as the browser does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    TypeScript,
    JavaScript,
    Python,
    Go,
    Rust,
    Java,
    Ruby,
    Cpp,
    C,
}

impl Language {
    /// The language of `path` by its extension, as the browser's `detectLanguage` maps it
    /// (`.h` is C, only `.cpp` is C++). `None` for anything else.
    pub fn from_path(path: &str) -> Option<Self> {
        // `filePath.split('.').pop()?.toLowerCase()`: a name with no dot is its own extension.
        let ext = path.rsplit('.').next()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "ts" | "tsx" => Self::TypeScript,
            "js" | "jsx" => Self::JavaScript,
            "py" => Self::Python,
            "rb" => Self::Ruby,
            "rs" => Self::Rust,
            "go" => Self::Go,
            "java" => Self::Java,
            "cpp" => Self::Cpp,
            "c" | "h" => Self::C,
            _ => return None,
        })
    }

    /// The files a search for this language looks in: ripgrep 15.1's `--type` globs for the
    /// browser's `RG_TYPE_MAP` entry, as git glob pathspecs.
    fn globs(self) -> &'static [&'static str] {
        match self {
            Self::TypeScript => &["*.cts", "*.mts", "*.ts", "*.tsx"],
            Self::JavaScript => &["*.cjs", "*.js", "*.jsx", "*.mjs", "*.vue"],
            Self::Python => &["*.py", "*.pyi"],
            Self::Go => &["*.go"],
            Self::Rust => &["*.rs"],
            Self::Java => &["*.java", "*.jsp", "*.jspx", "*.properties"],
            Self::Ruby => {
                &["*.gemspec", "*.rake", "*.rb", "*.rbw", ".irbrc", "Gemfile", "Rakefile", "config.ru"]
            }
            Self::Cpp => &[
                "*.[ChH]",
                "*.[ChH].in",
                "*.[ch]pp",
                "*.[ch]pp.in",
                "*.[ch]xx",
                "*.[ch]xx.in",
                "*.cc",
                "*.cc.in",
                "*.hh",
                "*.hh.in",
                "*.inl",
            ],
            Self::C => &["*.[chH]", "*.[chH].in", "*.cats"],
        }
    }

    fn definition_patterns(self) -> &'static [(&'static str, SymbolKind)] {
        match self {
            Self::TypeScript | Self::JavaScript => &TS_JS_PATTERNS,
            Self::Python => &PYTHON_PATTERNS,
            Self::Go => &GO_PATTERNS,
            Self::Rust => &RUST_PATTERNS,
            Self::Java | Self::Ruby | Self::Cpp | Self::C => &[],
        }
    }
}

/// Whether a match defines the word or uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Definition,
    Reference,
}

/// What a definition pattern says was defined (the browser's `SymbolKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Type,
    Enum,
    Const,
    Variable,
    Struct,
    Trait,
    Module,
}

/// One place the word appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Repository-relative path, `/`-separated.
    pub path: String,
    /// 1-based line number.
    pub line: u32,
    /// 0-based byte offset of the first whole-word match on the line.
    pub column: u32,
    /// The line, trailing whitespace removed, cut to [`SNIPPET_CHARS`] characters plus `…`.
    pub text: String,
    pub kind: Kind,
    /// For a definition, what the matching pattern says was defined.
    pub symbol: Option<SymbolKind>,
}

/// What the person is on, and the word to look for.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    /// The repository-relative path of the file on screen, on the side being searched.
    pub file: &'a str,
    /// The 1-based line the person is on; [`Found::best`] skips a match on this very line.
    pub line: u32,
    /// The identifier, matched as a whole word.
    pub word: &'a str,
    /// The review's changed files (see [`changed_paths`]), ranked above the rest.
    pub changed: &'a [String],
}

/// A search's ranked answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// Definitions, best first.
    pub definitions: Vec<Candidate>,
    /// References, best first.
    pub references: Vec<Candidate>,
    /// More than [`RESULT_CAP`] matches were ranked; the rest were dropped.
    pub capped: bool,
    /// False when the search was stopped early, by [`PARSE_CAP`] or the time limit.
    pub complete: bool,
    pub elapsed: Duration,
}

impl Found {
    /// Where a "go to definition" jumps: the best definition that is not the line the person
    /// is on, else the best such reference.
    pub fn best(&self, query: &Query<'_>) -> Option<&Candidate> {
        let elsewhere = |c: &&Candidate| !(c.path == query.file && c.line == query.line);
        self.definitions.iter().find(elsewhere).or_else(|| self.references.iter().find(elsewhere))
    }

    /// Every candidate, definitions first, each list in rank order.
    pub fn candidates(&self) -> impl Iterator<Item = &Candidate> {
        self.definitions.iter().chain(&self.references)
    }
}

/// The paths a review changes, both sides of a rename (the browser's `extractChangedFiles`).
pub fn changed_paths(files: &[FilePatch]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for file in files {
        for path in file.old_path.iter().chain(std::iter::once(&file.path)) {
            if seen.insert(path.as_str()) {
                out.push(path.clone());
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Searching
// ---------------------------------------------------------------------------------------

/// Search `side` of the repository at `repo` (a work tree, or a bare repository for `Rev`
/// sides) for `query.word`, with the jump time limit [`TIMEOUT`].
pub fn search(repo: &Path, side: &SideSource, query: &Query<'_>) -> Result<Found> {
    search_with(repo, side, query, TIMEOUT)
}

/// [`search`] with a time limit of its own (for a hover, [`HOVER_TIMEOUT`]). A search that
/// runs out of time answers with what it found so far, `complete: false`.
pub fn search_with(repo: &Path, side: &SideSource, query: &Query<'_>, timeout: Duration) -> Result<Found> {
    let started = Instant::now();
    let word = query.word;
    if word.trim().is_empty() || word.contains(['\n', '\r', '\0']) {
        return Ok(Found { complete: true, ..Found::default() });
    }
    let language = Language::from_path(query.file);
    let mut cmd = git::command(repo);
    // The pathspecs below use glob magic; these would change what they mean.
    for var in ["GIT_LITERAL_PATHSPECS", "GIT_GLOB_PATHSPECS", "GIT_NOGLOB_PATHSPECS", "GIT_ICASE_PATHSPECS"]
    {
        cmd.env_remove(var);
    }
    cmd.args([
        "grep",
        "--no-color",
        "--line-number",
        "--column",
        "--null",
        "-I",
        "--word-regexp",
        "--fixed-strings",
        "--full-name",
        "--no-recurse-submodules",
        "-e",
        word,
    ]);
    let prefix = match side {
        SideSource::Worktree => {
            // `git grep --untracked` in a bare repository quietly finds nothing; say why instead.
            let mut check = git::command(repo);
            check.args(["rev-parse", "--is-inside-work-tree"]);
            if git::run_text(check, "rev-parse --is-inside-work-tree").ok().as_deref() != Some("true") {
                return Err(Error::Git {
                    command: "grep".into(),
                    detail: format!("{} has no working tree to search", repo.display()),
                });
            }
            cmd.arg("--untracked");
            None
        }
        SideSource::Index => {
            cmd.arg("--cached");
            None
        }
        SideSource::Rev(rev) => {
            if rev.starts_with('-') || rev.is_empty() {
                return Err(Error::Git {
                    command: "grep".into(),
                    detail: format!("{rev} is not a revision"),
                });
            }
            cmd.arg(rev);
            Some(format!("{rev}:"))
        }
    };
    cmd.arg("--").args(pathspecs(language, query.file));

    let classifier = Classifier::new(word, language);
    let mut per_file: HashMap<String, usize> = HashMap::new();
    let mut matches = Vec::new();
    let outcome = stream_lines(cmd, timeout, |line| {
        let Some(m) = parse_grep_line(line, prefix.as_deref()) else { return true };
        if !is_path_allowed(&m.path, Some(query.file)) || is_hidden(&m.path, query.file) {
            return true;
        }
        let count = per_file.entry(m.path.clone()).or_default();
        if *count >= PER_FILE_CAP {
            return true;
        }
        *count += 1;
        let (kind, symbol) = classifier.classify(&m.text);
        matches.push(Candidate {
            path: m.path,
            line: m.line,
            column: m.column,
            text: snippet(&m.text),
            kind,
            symbol,
        });
        matches.len() < PARSE_CAP
    })?;

    let changed: HashSet<&str> = query.changed.iter().map(String::as_str).collect();
    let (definitions, references, capped) = rank(matches, query.file, &changed, RESULT_CAP);
    Ok(Found {
        definitions,
        references,
        capped,
        complete: outcome == Outcome::Finished,
        elapsed: started.elapsed(),
    })
}

/// The pathspecs for a search from `origin`: the language's files, minus the excluded
/// directories, lifted for a directory the origin itself is in (the browser's `buildRgArgs`).
fn pathspecs(language: Option<Language>, origin: &str) -> Vec<String> {
    let mut specs: Vec<String> =
        language.map(|l| l.globs().iter().map(|g| format!(":(glob)**/{g}")).collect()).unwrap_or_default();
    let segments = split_path(origin);
    for dir in ALWAYS_IGNORED_DIRS {
        if !segments.contains(&dir) {
            specs.push(format!(":(exclude,glob)**/{dir}/**"));
        }
    }
    let root = segments.first().copied().unwrap_or_default();
    for dir in ROOT_ONLY_IGNORED_DIRS {
        if root != dir {
            specs.push(format!(":(exclude,glob){dir}/**"));
        }
    }
    specs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// git finished on its own.
    Finished,
    /// The callback asked to stop, or the time ran out.
    Stopped,
}

/// Run `cmd`, handing each stdout line (newline removed) to `on_line` until it returns false,
/// git finishes, or `timeout` passes. git is stopped as soon as it is not needed.
fn stream_lines(
    mut cmd: std::process::Command,
    timeout: Duration,
    mut on_line: impl FnMut(&[u8]) -> bool,
) -> Result<Outcome> {
    let deadline = Instant::now() + timeout;
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(Error::GitMissing)?;
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = pipe.read_to_end(&mut text);
            text
        })
    });
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(256);
    let reader = child.stdout.take().map(|pipe| {
        std::thread::spawn(move || {
            let mut pipe = BufReader::new(pipe);
            let mut line = Vec::new();
            while pipe.read_until(b'\n', &mut line).unwrap_or(0) > 0 {
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if tx.send(std::mem::take(&mut line)).is_err() {
                    break;
                }
            }
        })
    });
    let mut outcome = Outcome::Finished;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => {
                if !on_line(&line) {
                    outcome = Outcome::Stopped;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                outcome = Outcome::Stopped;
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if outcome == Outcome::Stopped {
        let _ = child.kill();
    }
    drop(rx);
    let status = child.wait().map_err(Error::GitMissing)?;
    if let Some(reader) = reader {
        let _ = reader.join();
    }
    let stderr = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
    // 1 is "no match"; anything else is a real failure, unless we stopped git ourselves.
    if outcome == Outcome::Finished && !status.success() && status.code() != Some(1) {
        let detail = String::from_utf8_lossy(&stderr).trim().to_owned();
        return Err(Error::Git { command: "grep".into(), detail });
    }
    Ok(outcome)
}

#[derive(Debug, PartialEq, Eq)]
struct GrepLine {
    path: String,
    line: u32,
    column: u32,
    text: String,
}

/// One `git grep --null -n --column` line: `[<rev>:]<path>\0<line>\0<column>\0<text>`, the
/// column 1-based.
fn parse_grep_line(raw: &[u8], prefix: Option<&str>) -> Option<GrepLine> {
    let mut parts = raw.splitn(4, |b| *b == 0);
    let name = String::from_utf8_lossy(parts.next()?).into_owned();
    let line = std::str::from_utf8(parts.next()?).ok()?.trim().parse().ok()?;
    let column: u32 = std::str::from_utf8(parts.next()?).ok()?.trim().parse().ok()?;
    let text = String::from_utf8_lossy(parts.next()?).into_owned();
    let path = match prefix {
        Some(prefix) => name.strip_prefix(prefix)?.to_owned(),
        None => name,
    };
    Some(GrepLine { path, line, column: column.saturating_sub(1), text })
}

/// The line as a candidate shows it: `trimEnd`, then cut to [`SNIPPET_CHARS`] plus `…`.
fn snippet(text: &str) -> String {
    let trimmed = text.trim_end();
    match trimmed.char_indices().nth(SNIPPET_CHARS) {
        Some((cut, _)) => format!("{}…", trimmed.get(..cut).unwrap_or(trimmed)),
        None => trimmed.to_owned(),
    }
}

/// `rg` skips hidden files and directories (a name starting with `.`); so does this, except
/// for the directories the origin file is itself in, and the origin file.
fn is_hidden(path: &str, origin: &str) -> bool {
    if path == origin {
        return false;
    }
    let segments = split_path(path);
    let origin = split_path(origin);
    let Some((file, dirs)) = segments.split_last() else { return false };
    if file.starts_with('.') {
        return true;
    }
    dirs.iter()
        .enumerate()
        .any(|(i, name)| name.starts_with('.') && !is_ancestor_of_origin(&segments, &origin, i))
}

// ---------------------------------------------------------------------------------------
// Path exclusions (isCodeNavPathAllowed)
// ---------------------------------------------------------------------------------------

/// Ordered path segments, `/` and `\` alike, with `.` and empties dropped (`splitPath`).
fn split_path(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect()
}

/// Whether a match at `path` is kept for a search from `origin` (the browser's
/// `isCodeNavPathAllowed`): a match under an excluded directory is kept only when that same
/// directory is an ancestor of the origin file. Root-only names count only at the root.
pub fn is_path_allowed(path: &str, origin: Option<&str>) -> bool {
    let segments = split_path(path);
    let origin = origin.map(split_path).unwrap_or_default();
    // Only directory segments can be excluded; the last segment is the file.
    let dirs = segments.len().saturating_sub(1);
    segments.iter().take(dirs).enumerate().all(|(i, name)| {
        let excluded =
            ALWAYS_IGNORED_DIRS.contains(name) || (i == 0 && ROOT_ONLY_IGNORED_DIRS.contains(name));
        !excluded || is_ancestor_of_origin(&segments, &origin, i)
    })
}

/// Do `segments[0..=i]` name the same directory the origin sits under (`isAncestorOfOrigin`)?
fn is_ancestor_of_origin(segments: &[&str], origin: &[&str], i: usize) -> bool {
    origin.len() > i + 1 && segments.iter().zip(origin).take(i + 1).all(|(a, b)| a == b)
}

// ---------------------------------------------------------------------------------------
// Classification (classifyMatchDetailed)
// ---------------------------------------------------------------------------------------

/// `DEFINITION_PATTERNS` for TypeScript and JavaScript, in order.
const TS_JS_PATTERNS: [(&str, SymbolKind); 8] = [
    (r"(?:export\s+)?(?:async\s+)?function\s+SYMBOL\b", SymbolKind::Function),
    (r"(?:export\s+)?const\s+SYMBOL\s*[=:]", SymbolKind::Const),
    (r"(?:export\s+)?(?:let|var)\s+SYMBOL\s*[=:]", SymbolKind::Variable),
    (r"(?:export\s+)?class\s+SYMBOL\b", SymbolKind::Class),
    (r"(?:export\s+)?interface\s+SYMBOL\b", SymbolKind::Interface),
    (r"(?:export\s+)?type\s+SYMBOL\b", SymbolKind::Type),
    (r"(?:export\s+)?enum\s+SYMBOL\b", SymbolKind::Enum),
    (
        r"^\s+(?:(?:async|static|readonly|get|set|private|protected|public)\s+)+SYMBOL\s*[(<:]",
        SymbolKind::Method,
    ),
];

const PYTHON_PATTERNS: [(&str, SymbolKind); 3] = [
    (r"(?:^|\s)def\s+SYMBOL\s*\(", SymbolKind::Function),
    (r"(?:^|\s)class\s+SYMBOL\b", SymbolKind::Class),
    (r"^SYMBOL\s*=", SymbolKind::Variable),
];

const GO_PATTERNS: [(&str, SymbolKind); 4] = [
    (r"func\s+\([^)]+\)\s+SYMBOL\s*\(", SymbolKind::Method),
    (r"func\s+SYMBOL\s*\(", SymbolKind::Function),
    (r"type\s+SYMBOL\s", SymbolKind::Type),
    (r"var\s+SYMBOL\s", SymbolKind::Variable),
];

const RUST_PATTERNS: [(&str, SymbolKind); 6] = [
    (r"(?:pub(?:\([^)]*\))?\s+)?fn\s+SYMBOL\b", SymbolKind::Function),
    (r"(?:pub(?:\([^)]*\))?\s+)?struct\s+SYMBOL\b", SymbolKind::Struct),
    (r"(?:pub(?:\([^)]*\))?\s+)?enum\s+SYMBOL\b", SymbolKind::Enum),
    (r"(?:pub(?:\([^)]*\))?\s+)?trait\s+SYMBOL\b", SymbolKind::Trait),
    (r"(?:pub(?:\([^)]*\))?\s+)?type\s+SYMBOL\b", SymbolKind::Type),
    (r"(?:pub(?:\([^)]*\))?\s+)?mod\s+SYMBOL\b", SymbolKind::Module),
];

/// `GENERIC_DEFINITION_PATTERNS`, tried after the language's own.
const GENERIC_PATTERNS: [(&str, SymbolKind); 9] = [
    (r"(?:function|def|func|fn)\s+SYMBOL\b", SymbolKind::Function),
    (r"class\s+SYMBOL\b", SymbolKind::Class),
    (r"struct\s+SYMBOL\b", SymbolKind::Struct),
    (r"enum\s+SYMBOL\b", SymbolKind::Enum),
    (r"trait\s+SYMBOL\b", SymbolKind::Trait),
    (r"interface\s+SYMBOL\b", SymbolKind::Interface),
    (r"type\s+SYMBOL\b", SymbolKind::Type),
    (r"const\s+SYMBOL\s*[=:]", SymbolKind::Const),
    (r"(?:let|var|val)\s+SYMBOL\s*[=:]", SymbolKind::Variable),
];

/// The definition patterns for one word and language, compiled once per search.
#[derive(Debug)]
pub struct Classifier {
    patterns: Vec<(Regex, SymbolKind)>,
}

impl Classifier {
    pub fn new(symbol: &str, language: Option<Language>) -> Self {
        let escaped = regex::escape(symbol);
        let own = language.map(Language::definition_patterns).unwrap_or_default();
        let patterns = own
            .iter()
            .chain(&GENERIC_PATTERNS)
            .filter_map(|(pattern, kind)| {
                // JavaScript's `\b` is ASCII-only; the regex crate's is Unicode unless told.
                let pattern = pattern.replace(r"\b", r"(?-u:\b)").replacen("SYMBOL", &escaped, 1);
                Regex::new(&pattern).ok().map(|re| (re, *kind))
            })
            .collect();
        Self { patterns }
    }

    /// Whether `line` defines the word, and what kind of thing it defines.
    pub fn classify(&self, line: &str) -> (Kind, Option<SymbolKind>) {
        self.patterns
            .iter()
            .find(|(re, _)| re.is_match(line))
            .map_or((Kind::Reference, None), |(_, kind)| (Kind::Definition, Some(*kind)))
    }
}

/// Classify one line (the browser's `classifyMatchDetailed`).
pub fn classify(line: &str, symbol: &str, language: Option<Language>) -> (Kind, Option<SymbolKind>) {
    Classifier::new(symbol, language).classify(line)
}

// ---------------------------------------------------------------------------------------
// Ranking (rankLocations)
// ---------------------------------------------------------------------------------------

/// `isTestFile`: `/(?:test|spec|__tests__|_test\.|\.test\.|\.spec\.)/i`, which comes down to
/// "the path contains `test` or `spec`, any case".
pub fn is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("test") || lower.contains("spec")
}

/// The browser's `sameDirectory`, quirk included: a path with no `/` is compared by
/// `slice(0, -1)`, everything but its last character.
fn same_directory(a: &str, b: &str) -> bool {
    match (a.rfind('/'), b.rfind('/')) {
        (None, None) => true,
        (da, db) => js_dir(a, da) == js_dir(b, db),
    }
}

fn js_dir(path: &str, slash: Option<usize>) -> &str {
    match slash {
        Some(i) => path.get(..i).unwrap_or(path),
        None => path.char_indices().last().and_then(|(i, _)| path.get(..i)).unwrap_or(""),
    }
}

/// Score and order matches the way the browser does: same file +1000, else a changed file
/// +500, else the same folder +200; a test file −300 unless the origin is one; a definition
/// +100 and, being "likely", +50 more. Ties keep search order. Keeps `cap`; the bool says
/// whether any were dropped.
pub fn rank<S: std::hash::BuildHasher>(
    matches: Vec<Candidate>,
    origin: &str,
    changed: &HashSet<&str, S>,
    cap: usize,
) -> (Vec<Candidate>, Vec<Candidate>, bool) {
    let capped = matches.len() > cap;
    let origin_is_test = is_test_file(origin);
    let score = |c: &Candidate| {
        let mut s: i32 = 0;
        if c.path == origin {
            s += 1000;
        } else if changed.contains(c.path.as_str()) {
            s += 500;
        } else if same_directory(&c.path, origin) {
            s += 200;
        }
        if is_test_file(&c.path) && !origin_is_test {
            s -= 300;
        }
        if c.kind == Kind::Definition {
            s += 100 + 50;
        }
        s
    };
    let mut scored: Vec<(i32, Candidate)> = matches.into_iter().map(|c| (score(&c), c)).collect();
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    let (definitions, references) =
        scored.into_iter().take(cap).map(|(_, c)| c).partition(|c| c.kind == Kind::Definition);
    (definitions, references, capped)
}

// ---------------------------------------------------------------------------------------
// Identifiers on a line
// ---------------------------------------------------------------------------------------

/// One identifier on a line: its byte range and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ident<'a> {
    pub start: usize,
    pub end: usize,
    pub text: &'a str,
}

/// The identifier under byte `column` of `line`, else the first one after it.
pub fn identifier_at(line: &str, column: usize, language: Option<Language>) -> Option<Ident<'_>> {
    identifiers_on_line(line, language).into_iter().find(|id| column < id.end)
}

/// The identifiers on one source line, left to right, without keywords, literals, strings
/// or comments, as far as a single-line lexer can tell. A line inside a multi-line string or
/// block comment cannot be told apart from code, except a `*` continuation line in a C-style
/// block comment. A JavaScript template literal's text is skipped, but the code in its
/// `${…}` substitutions is read like any other, nested templates included up to
/// [`MAX_TEMPLATE_NESTING`] deep; past that, the rest of the line is text.
pub fn identifiers_on_line(line: &str, language: Option<Language>) -> Vec<Ident<'_>> {
    let lex = Lexer::new(line, language);
    lex.run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comments {
    /// `//` and `/* */`.
    Slash,
    /// `#`.
    Hash,
}

struct Lexer<'a> {
    line: &'a str,
    chars: Vec<(usize, char)>,
    language: Option<Language>,
    comments: Comments,
}

impl<'a> Lexer<'a> {
    fn new(line: &'a str, language: Option<Language>) -> Self {
        let comments = match language {
            Some(Language::Python | Language::Ruby) => Comments::Hash,
            _ => Comments::Slash,
        };
        Self { line, chars: line.char_indices().collect(), language, comments }
    }

    fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).map(|(_, c)| *c)
    }

    fn offset(&self, i: usize) -> usize {
        self.chars.get(i).map_or(self.line.len(), |(o, _)| *o)
    }

    fn starts(&self, i: usize, s: &str) -> bool {
        self.line.get(self.offset(i)..).is_some_and(|rest| rest.starts_with(s))
    }

    fn is_js(&self) -> bool {
        matches!(self.language, Some(Language::TypeScript | Language::JavaScript))
    }

    fn ident_start(&self, c: char) -> bool {
        c.is_alphabetic() || c == '_' || (c == '$' && self.is_js())
    }

    fn ident_continue(&self, c: char) -> bool {
        c.is_alphanumeric() || c == '_' || (c == '$' && self.is_js())
    }

    fn run(&self) -> Vec<Ident<'a>> {
        let mut out = Vec::new();
        let trimmed = self.line.trim_start();
        if self.comments == Comments::Slash
            && (trimmed.starts_with("* ")
                || trimmed == "*"
                || trimmed.starts_with("*/")
                || trimmed.starts_with("**"))
        {
            return out; // a block comment's continuation line
        }
        if matches!(self.language, Some(Language::C | Language::Cpp)) && trimmed.starts_with("#include") {
            return out;
        }
        self.scan(0, None, &mut out);
        out
    }

    /// Read code from char `i`, adding its identifiers to `out`, to the end of the line; or,
    /// inside a substitution (`within` is its template's depth), to the `}` that closes it. Returns the char index where it stopped (past that `}`).
    fn scan(&self, mut i: usize, within: Option<usize>, out: &mut Vec<Ident<'a>>) -> usize {
        let mut depth = 0usize;
        while let Some(c) = self.at(i) {
            if within.is_some() && c == '{' {
                depth += 1;
            }
            if within.is_some() && c == '}' {
                if depth == 0 {
                    return i + 1;
                }
                depth -= 1;
            }
            if self.comments == Comments::Slash && self.starts(i, "//") {
                break;
            }
            if self.comments == Comments::Hash && c == '#' {
                break;
            }
            if self.comments == Comments::Slash && self.starts(i, "/*") {
                i = self.find_from(i + 2, "*/").map_or(self.chars.len(), |end| end + 2);
                continue;
            }
            if c == '`' && self.is_js() {
                i = self.skip_template(i, within.map_or(1, |n| n + 1), out);
                continue;
            }
            if c == '"' || c == '`' || (c == '\'' && self.language != Some(Language::Rust)) {
                i = self.skip_string(i);
                continue;
            }
            if c == '\'' {
                i = self.skip_rust_quote(i);
                continue;
            }
            if c == '#' && matches!(self.language, Some(Language::C | Language::Cpp)) {
                // A preprocessor directive's name (`#define`), not an identifier.
                i += 1;
                while self.at(i).is_some_and(|c| self.ident_continue(c)) {
                    i += 1;
                }
                continue;
            }
            if c.is_ascii_digit() {
                // A number, with any suffix or exponent (`0x1f`, `1e5`, `10u8`, `1.5`).
                while self.at(i).is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                    i += 1;
                }
                continue;
            }
            if self.ident_start(c) {
                let start = i;
                while self.at(i).is_some_and(|c| self.ident_continue(c)) {
                    i += 1;
                }
                let (from, to) = (self.offset(start), self.offset(i));
                let text = self.line.get(from..to).unwrap_or_default();
                if let Some(end) = self.prefixed_string(text, i) {
                    i = end;
                    continue;
                }
                if self.language == Some(Language::Rust) && text == "r" && self.at(i) == Some('#') {
                    i += 1; // a raw identifier, `r#type`: the name after `#` is the identifier
                    continue;
                }
                if text != "_" && !is_keyword(text, self.language) {
                    out.push(Ident { start: from, end: to, text });
                }
                continue;
            }
            i += 1;
        }
        self.chars.len()
    }

    /// Past a JavaScript template literal starting at `i` (its backtick): its text is
    /// skipped, the code in each `${…}` is read for identifiers. `nesting` is its depth among
    /// enclosing templates (1 at top level). Unclosed, or nested deeper than
    /// [`MAX_TEMPLATE_NESTING`], it runs to the end of the line.
    fn skip_template(&self, i: usize, nesting: usize, out: &mut Vec<Ident<'a>>) -> usize {
        if nesting > MAX_TEMPLATE_NESTING {
            return self.chars.len();
        }
        let mut j = i + 1;
        while let Some(c) = self.at(j) {
            if c == '\\' {
                j += 2;
                continue;
            }
            if c == '`' {
                return j + 1;
            }
            if self.starts(j, "${") {
                j = self.scan(j + 2, Some(nesting), out);
                continue;
            }
            j += 1;
        }
        self.chars.len()
    }

    /// The char index where `needle` next starts at or after `from`.
    fn find_from(&self, from: usize, needle: &str) -> Option<usize> {
        (from..self.chars.len()).find(|&j| self.starts(j, needle))
    }

    /// Past a string starting at `i` (its quote): a triple-quoted Python string, a backtick
    /// string, or a quoted string with backslash escapes. Unclosed runs to the end.
    fn skip_string(&self, i: usize) -> usize {
        let Some(quote) = self.at(i) else { return i + 1 };
        if self.language == Some(Language::Python) && (self.starts(i, "\"\"\"") || self.starts(i, "'''")) {
            let close = if quote == '"' { "\"\"\"" } else { "'''" };
            return self.find_from(i + 3, close).map_or(self.chars.len(), |end| end + 3);
        }
        let mut j = i + 1;
        while let Some(c) = self.at(j) {
            if c == '\\' && (quote != '`' || self.is_js()) {
                j += 2;
                continue;
            }
            if c == quote {
                return j + 1;
            }
            j += 1;
        }
        self.chars.len()
    }

    /// Rust's `'`: a char literal (`'x'`, `'\n'`) or a lifetime / label (`'a`), skipped either way.
    fn skip_rust_quote(&self, i: usize) -> usize {
        match (self.at(i + 1), self.at(i + 2)) {
            (Some('\\'), _) => self.skip_string(i),
            (Some(_), Some('\'')) => i + 3,
            _ => {
                let mut j = i + 1;
                while self.at(j).is_some_and(|c| self.ident_continue(c)) {
                    j += 1;
                }
                j.max(i + 1)
            }
        }
    }

    /// When the identifier `text`, ending at char `i`, is a string prefix (`r"…"`, `b'…'`,
    /// `f"…"`, Rust's `r#"…"#`) and a string follows, the char index past that string.
    fn prefixed_string(&self, text: &str, i: usize) -> Option<usize> {
        let lower = text.to_ascii_lowercase();
        let next = self.at(i)?;
        match self.language {
            Some(Language::Python)
                if matches!(lower.as_str(), "r" | "b" | "u" | "f" | "rb" | "br" | "fr" | "rf")
                    && matches!(next, '"' | '\'') =>
            {
                Some(self.skip_string(i))
            }
            Some(Language::Rust) if matches!(text, "b" | "c") && matches!(next, '"' | '\'') => {
                Some(self.skip_string(i))
            }
            Some(Language::Rust) if matches!(text, "r" | "br" | "cr") && matches!(next, '"' | '#') => {
                // A raw string: `r"…"`, `r#"…"#`, no escapes.
                let mut j = i;
                while self.at(j) == Some('#') {
                    j += 1;
                }
                if self.at(j) != Some('"') {
                    return None;
                }
                let close = format!("\"{}", "#".repeat(j - i));
                Some(self.find_from(j + 1, &close).map_or(self.chars.len(), |end| end + 1 + (j - i)))
            }
            Some(Language::Cpp | Language::C)
                if matches!(text, "L" | "u" | "U" | "u8") && matches!(next, '"' | '\'') =>
            {
                Some(self.skip_string(i))
            }
            _ => None,
        }
    }
}

/// Keywords, literals and builtin type names: words a definition search has nothing to find for.
fn is_keyword(word: &str, language: Option<Language>) -> bool {
    let list = match language {
        Some(Language::TypeScript | Language::JavaScript) => TS_JS_KEYWORDS,
        Some(Language::Python) => PYTHON_KEYWORDS,
        Some(Language::Go) => GO_KEYWORDS,
        Some(Language::Rust) => RUST_KEYWORDS,
        Some(Language::Java) => JAVA_KEYWORDS,
        Some(Language::Ruby) => RUBY_KEYWORDS,
        Some(Language::C | Language::Cpp) => C_KEYWORDS,
        None => GENERIC_KEYWORDS,
    };
    list.contains(&word)
}

const TS_JS_KEYWORDS: &[&str] = &[
    "abstract",
    "any",
    "as",
    "async",
    "await",
    "bigint",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "never",
    "new",
    "null",
    "number",
    "object",
    "of",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "static",
    "string",
    "super",
    "switch",
    "symbol",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unknown",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "cls", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "self", "try", "while", "with", "yield",
];

const GO_KEYWORDS: &[&str] = &[
    "any",
    "append",
    "bool",
    "break",
    "byte",
    "cap",
    "case",
    "chan",
    "complex128",
    "complex64",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "error",
    "fallthrough",
    "false",
    "float32",
    "float64",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "int",
    "int16",
    "int32",
    "int64",
    "int8",
    "interface",
    "iota",
    "len",
    "make",
    "map",
    "new",
    "nil",
    "package",
    "panic",
    "range",
    "recover",
    "return",
    "rune",
    "select",
    "string",
    "struct",
    "switch",
    "true",
    "type",
    "uint",
    "uint16",
    "uint32",
    "uint64",
    "uint8",
    "uintptr",
    "var",
];

const RUST_KEYWORDS: &[&str] = &[
    "Self",
    "as",
    "async",
    "await",
    "bool",
    "break",
    "char",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "f32",
    "f64",
    "false",
    "fn",
    "for",
    "i128",
    "i16",
    "i32",
    "i64",
    "i8",
    "if",
    "impl",
    "in",
    "isize",
    "let",
    "loop",
    "macro_rules",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "str",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "u128",
    "u16",
    "u32",
    "u64",
    "u8",
    "unsafe",
    "use",
    "usize",
    "where",
    "while",
];

const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];

const RUBY_KEYWORDS: &[&str] = &[
    "BEGIN", "END", "alias", "and", "begin", "break", "case", "class", "def", "defined", "do", "else",
    "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or", "redo",
    "rescue", "retry", "return", "self", "super", "then", "true", "undef", "unless", "until", "when",
    "while", "yield",
];

const C_KEYWORDS: &[&str] = &[
    "NULL",
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "const_cast",
    "constexpr",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "extern",
    "false",
    "final",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "operator",
    "override",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

const GENERIC_KEYWORDS: &[&str] = &[
    "False",
    "None",
    "True",
    "and",
    "as",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "def",
    "default",
    "do",
    "else",
    "enum",
    "export",
    "false",
    "finally",
    "fn",
    "for",
    "from",
    "func",
    "function",
    "if",
    "import",
    "in",
    "interface",
    "let",
    "new",
    "nil",
    "not",
    "null",
    "of",
    "or",
    "package",
    "return",
    "self",
    "struct",
    "switch",
    "this",
    "throw",
    "trait",
    "true",
    "try",
    "type",
    "use",
    "var",
    "while",
];

#[cfg(all(test, unix))]
mod git_tests;
#[cfg(test)]
mod tests;
