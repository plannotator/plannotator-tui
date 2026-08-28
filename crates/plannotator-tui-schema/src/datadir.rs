//! Where annotations live: the Plannotator data directory, keyed the way Plannotator keys
//! files, so one file maps to one directory in both tools.
//!
//! Ported from Plannotator (`packages/shared/data-dir.ts`, `packages/core/project.ts`,
//! `packages/shared/annotate-history.ts`); the tests carry Plannotator's own vectors.
//! Pure over its inputs: the environment and the filesystem are passed in.

use std::path::{Path, PathBuf};

use sha2::Digest;

/// Plannotator's data directory.
///
/// Order: `PLANNOTATOR_DATA_DIR` (a leading `~` expands to `home`) → an existing
/// `home/.plannotator` → `XDG_DATA_HOME/plannotator` when that is set and absolute →
/// `home/.plannotator`. The XDG spec's implicit `~/.local/share` is deliberately not applied.
pub fn data_dir(
    env: impl Fn(&str) -> Option<String>,
    home: &Path,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(dir) = env("PLANNOTATOR_DATA_DIR").map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
        return match dir.strip_prefix("~/").or_else(|| dir.strip_prefix("~\\")) {
            Some(rest) => home.join(rest),
            None if dir == "~" => home.to_path_buf(),
            None => PathBuf::from(dir),
        };
    }
    let legacy = home.join(".plannotator");
    if exists(&legacy) {
        return legacy;
    }
    if let Some(xdg) = env("XDG_DATA_HOME").map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return xdg.join("plannotator");
        }
    }
    legacy
}

/// Plannotator's `sanitizeTag`: lowercase, spaces and underscores to hyphens, strip
/// anything outside `[a-z0-9-]`, collapse hyphens, trim edge hyphens, cap at 30, and
/// `None` under 2 characters.
pub fn sanitize_tag(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut pending_hyphen = false;
    for ch in name.to_lowercase().trim().chars() {
        let ch = if ch.is_whitespace() || ch == '_' { '-' } else { ch };
        match ch {
            '-' => pending_hyphen = true,
            'a'..='z' | '0'..='9' => {
                if pending_hyphen && !out.is_empty() {
                    out.push('-');
                }
                pending_hyphen = false;
                out.push(ch);
            }
            _ => {}
        }
    }
    let out: String = out.chars().take(30).collect();
    let out = out.trim_matches('-').to_owned();
    (out.chars().count() >= 2).then_some(out)
}

/// Plannotator's project name: the git toplevel's basename, else the cwd's basename,
/// else `_unknown` — each candidate through [`sanitize_tag`], falling through on `None`.
pub fn project_name(git_toplevel: Option<&Path>, cwd: &Path) -> String {
    git_toplevel
        .and_then(basename)
        .and_then(|n| sanitize_tag(&n))
        .or_else(|| basename(cwd).and_then(|n| sanitize_tag(&n)))
        .unwrap_or_else(|| "_unknown".to_owned())
}

fn basename(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Plannotator's `deriveAnnotateHistorySlug`: `annotate-<base>-<8 hex>` where `base` is the
/// basename lowercased with `[^a-z0-9]+` runs as `-`, edge hyphens trimmed, capped at 60
/// (fallback `document`), and the hex is the first 8 of sha256 over the resolved path
/// **exactly as given** — not realpath'd, not lowercased.
pub fn history_slug(resolved_path: &str) -> String {
    let file = resolved_path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("document");
    let mut base = String::new();
    let mut pending = false;
    for ch in file.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            if pending && !base.is_empty() {
                base.push('-');
            }
            pending = false;
            base.push(ch);
        } else {
            pending = true;
        }
    }
    let base: String = base.chars().take(60).collect();
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "document" } else { base };
    let digest = sha2_hex(resolved_path.as_bytes());
    format!("annotate-{base}-{}", &digest[..8])
}

fn sha2_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Directory holding plannotator-tui's annotations for one file.
pub fn annotations_dir(data_dir: &Path, project: &str, resolved_path: &str) -> PathBuf {
    data_dir
        .join("clients")
        .join("plannotator-tui")
        .join("annotations")
        .join(project)
        .join(history_slug(resolved_path))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn data_dir_follows_plannotators_order() {
        let home = Path::new("/home/u");
        let legacy_exists = |p: &Path| p == Path::new("/home/u/.plannotator");
        let nothing = |_: &Path| false;
        // PLANNOTATOR_DATA_DIR wins and expands a leading ~
        assert_eq!(
            data_dir(
                env_of(&[("PLANNOTATOR_DATA_DIR", "~/relocated"), ("XDG_DATA_HOME", "/x")]),
                home,
                legacy_exists
            ),
            Path::new("/home/u/relocated")
        );
        assert_eq!(
            data_dir(env_of(&[("PLANNOTATOR_DATA_DIR", "/custom")]), home, nothing),
            Path::new("/custom")
        );
        // an existing ~/.plannotator wins over XDG
        assert_eq!(
            data_dir(env_of(&[("XDG_DATA_HOME", "/xdg")]), home, legacy_exists),
            Path::new("/home/u/.plannotator")
        );
        // XDG applies only when absolute and the legacy dir is absent
        assert_eq!(
            data_dir(env_of(&[("XDG_DATA_HOME", "/xdg")]), home, nothing),
            Path::new("/xdg/plannotator")
        );
        assert_eq!(
            data_dir(env_of(&[("XDG_DATA_HOME", "relative")]), home, nothing),
            Path::new("/home/u/.plannotator")
        );
        assert_eq!(
            data_dir(env_of(&[("XDG_DATA_HOME", "  ")]), home, nothing),
            Path::new("/home/u/.plannotator")
        );
        assert_eq!(data_dir(env_of(&[]), home, nothing), Path::new("/home/u/.plannotator"));
    }

    #[test]
    fn sanitize_tag_matches_plannotator() {
        assert_eq!(sanitize_tag("My Repo_Name!"), Some("my-repo-name".into()));
        assert_eq!(sanitize_tag("--a--b--"), Some("a-b".into()));
        assert_eq!(sanitize_tag("x"), None);
        assert_eq!(
            sanitize_tag("a-very-long-repository-name-that-goes-on"),
            Some("a-very-long-repository-name-th".into())
        );
    }

    #[test]
    fn project_name_falls_through_on_short_candidates() {
        assert_eq!(project_name(Some(Path::new("/work/herdr")), Path::new("/work/herdr/docs")), "herdr");
        assert_eq!(project_name(Some(Path::new("/work/x")), Path::new("/work/x/notes")), "notes");
        assert_eq!(project_name(None, Path::new("/")), "_unknown");
    }

    #[test]
    fn history_slug_matches_plannotator() {
        let path = "/Users/ramos/notes/Plan Draft.md";
        let slug = history_slug(path);
        assert!(slug.starts_with("annotate-plan-draft-md-"), "{slug}");
        assert_eq!(slug.len(), "annotate-plan-draft-md-".len() + 8);
        assert_eq!(&slug[slug.len() - 8..], &sha2_hex(path.as_bytes())[..8]);
        // hash input is the path as given: case and symlinks are not normalized
        assert_ne!(history_slug("/a/B.md"), history_slug("/a/b.md"));
        // a trailing separator leaves no basename: JS `split().pop()` gives "" → "document"
        assert_eq!(history_slug("/x/"), format!("annotate-document-{}", &sha2_hex(b"/x/")[..8]));
    }
}
