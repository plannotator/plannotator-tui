//! Pull requests on GitHub: reading a PR link or number, asking `gh` (the person's own login)
//! for the PR's branches and commits, and finding the clone a PR belongs to.
//!
//! Nothing here writes to GitHub: the only `gh` call is `gh pr view`. Posting a review is
//! [`crate::post`]'s, and only after the person confirms it.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::error::{Error, Result};

/// Where `gh` is: the program name, looked up on `PATH` unless it is a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gh {
    program: OsString,
}

impl Default for Gh {
    fn default() -> Self {
        Self { program: "gh".into() }
    }
}

/// A pull request as the person wrote it: a link, or a number in the current repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrSpec {
    /// `https://<host>/<owner>/<repo>/pull/<number>`, with anything after the number ignored.
    Url(PrRepo, u64),
    /// `123` or `#123`, in the repository of the folder the review runs in.
    Number(u64),
}

/// The repository a pull request is in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrRepo {
    pub host: String,
    pub owner: String,
    pub name: String,
}

/// What `gh pr view` says about a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrView {
    pub repo: PrRepo,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_sha: String,
    pub head_sha: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewJson {
    number: u64,
    url: String,
    #[serde(default)]
    title: String,
    base_ref_name: String,
    #[serde(default)]
    head_ref_name: String,
    base_ref_oid: String,
    head_ref_oid: String,
}

const VIEW_FIELDS: &str = "number,url,title,baseRefName,headRefName,baseRefOid,headRefOid";

impl PrSpec {
    /// A PR link, `#123` or `123`; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(number) = number(text.strip_prefix('#').unwrap_or(text)) {
            return Some(Self::Number(number));
        }
        let (repo, number) = parse_url(text)?;
        Some(Self::Url(repo, number))
    }

    /// The text handed to `gh pr view`.
    fn arg(&self) -> String {
        match self {
            Self::Url(repo, number) => repo.pull_url(*number),
            Self::Number(number) => number.to_string(),
        }
    }

    pub fn number(&self) -> u64 {
        match self {
            Self::Url(_, number) | Self::Number(number) => *number,
        }
    }
}

fn number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

/// `https://<host>/<owner>/<repo>/pull/<number>[/...][?...][#...]`.
fn parse_url(text: &str) -> Option<(PrRepo, u64)> {
    let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://"))?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = rest.split('/');
    let host = parts.next().filter(|h| !h.is_empty())?;
    let owner = parts.next().filter(|o| !o.is_empty())?;
    let name = parts.next().filter(|n| !n.is_empty())?;
    if parts.next()? != "pull" {
        return None;
    }
    let number = number(parts.next()?)?;
    Some((PrRepo { host: host.to_lowercase(), owner: owner.to_owned(), name: name.to_owned() }, number))
}

impl PrRepo {
    /// `owner/name`.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// `https://<host>/<owner>/<name>`: the record key every clone of the repository shares.
    pub fn web_url(&self) -> String {
        format!("https://{}/{}/{}", self.host, self.owner, self.name)
    }

    /// The clone URL for a repository with no local clone.
    pub fn clone_url(&self) -> String {
        format!("{}.git", self.web_url())
    }

    fn pull_url(&self, number: u64) -> String {
        format!("{}/pull/{number}", self.web_url())
    }

    /// Whether a git remote URL names this repository: `https://host/o/r(.git)`,
    /// `git@host:o/r(.git)`, `ssh://git@host[:port]/o/r(.git)`, compared without case.
    pub fn matches_remote(&self, url: &str) -> bool {
        let Some((host, path)) = remote_host_and_path(url) else { return false };
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        host.eq_ignore_ascii_case(&self.host) && path.eq_ignore_ascii_case(&self.slug())
    }
}

/// The host (no user, no port) and path of a remote URL.
fn remote_host_and_path(url: &str) -> Option<(&str, &str)> {
    if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host);
        return Some((host, path));
    }
    // scp-like: [user@]host:path
    let (authority, path) = url.split_once(':')?;
    if authority.contains('/') {
        return None;
    }
    Some((authority.rsplit('@').next().unwrap_or(authority), path.trim_start_matches('/')))
}

impl Gh {
    /// `gh` at `program` instead of the one on `PATH`.
    pub fn at(program: impl Into<OsString>) -> Self {
        Self { program: program.into() }
    }

    pub fn program(&self) -> &OsString {
        &self.program
    }

    /// `gh pr view <spec> --json …`, run in `dir` so a bare number resolves against that
    /// folder's repository. Read-only.
    pub fn view(&self, spec: &PrSpec, dir: &Path) -> Result<PrView> {
        let output = Command::new(&self.program)
            .args(["pr", "view", &spec.arg(), "--json", VIEW_FIELDS])
            .current_dir(dir)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    Error::GhMissing
                } else {
                    Error::Gh { detail: format!("could not run gh: {source}") }
                }
            })?;
        if !output.status.success() {
            return Err(gh_failure(spec, &String::from_utf8_lossy(&output.stderr)));
        }
        let json: ViewJson = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Gh { detail: format!("gh pr view printed something unexpected: {e}") })?;
        let (repo, _) = parse_url(&json.url)
            .ok_or_else(|| Error::Gh { detail: format!("gh pr view gave no PR link ({})", json.url) })?;
        Ok(PrView {
            repo,
            number: json.number,
            url: json.url,
            title: json.title,
            base_ref: json.base_ref_name,
            head_ref: json.head_ref_name,
            base_sha: json.base_ref_oid,
            head_sha: json.head_ref_oid,
        })
    }
}

/// `gh`'s failure, named for the person: not logged in, no such PR, no GitHub remote.
fn gh_failure(spec: &PrSpec, stderr: &str) -> Error {
    let detail = stderr.trim().to_owned();
    let lower = detail.to_lowercase();
    let what = match spec {
        PrSpec::Url(repo, number) => format!("PR #{number} in {}", repo.slug()),
        PrSpec::Number(number) => format!("PR #{number}"),
    };
    if lower.contains("gh auth login") || lower.contains("not logged in") || lower.contains("authentication")
    {
        Error::GhNotLoggedIn { detail }
    } else if lower.contains("could not resolve to a pullrequest")
        || lower.contains("no pull requests found")
        || lower.contains("could not resolve to a repository")
        || lower.contains("not found")
    {
        Error::PullNotFound { what, detail }
    } else if lower.contains("none of the git remotes") || lower.contains("no git remotes") {
        Error::NoGitHubRemote { detail }
    } else if lower.contains("not a git repository") {
        Error::NoGitHubRemote { detail: "this folder is not inside a git repository".into() }
    } else {
        Error::Gh {
            detail: if detail.is_empty() { "gh pr view failed with no message".into() } else { detail },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(host: &str, owner: &str, name: &str) -> PrRepo {
        PrRepo { host: host.into(), owner: owner.into(), name: name.into() }
    }

    #[test]
    fn specs_are_links_or_numbers() {
        assert_eq!(PrSpec::parse("158"), Some(PrSpec::Number(158)));
        assert_eq!(PrSpec::parse("#158"), Some(PrSpec::Number(158)));
        let link = PrSpec::Url(repo("github.com", "plannotator", "plannotator-tui"), 158);
        assert_eq!(
            PrSpec::parse("https://github.com/plannotator/plannotator-tui/pull/158"),
            Some(link.clone())
        );
        assert_eq!(
            PrSpec::parse("https://GitHub.com/plannotator/plannotator-tui/pull/158/files#diff"),
            Some(link)
        );
        assert_eq!(
            PrSpec::parse("https://ghe.example.com/o/r/pull/7?w=1"),
            Some(PrSpec::Url(repo("ghe.example.com", "o", "r"), 7))
        );
        for bad in ["", "#", "12a", "src", "https://github.com/o/r", "https://github.com/o/r/issues/3", "-1"]
        {
            assert_eq!(PrSpec::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn remotes_match_in_every_spelling() {
        let r = repo("github.com", "Plannotator", "plannotator-tui");
        for url in [
            "https://github.com/plannotator/plannotator-tui.git",
            "https://github.com/plannotator/plannotator-tui",
            "https://token@github.com/plannotator/plannotator-tui/",
            "git@github.com:plannotator/plannotator-tui.git",
            "ssh://git@github.com:22/plannotator/plannotator-tui.git",
        ] {
            assert!(r.matches_remote(url), "{url}");
        }
        for url in [
            "https://github.com/plannotator/plannotator",
            "git@gitlab.com:plannotator/plannotator-tui.git",
            "/tmp/plannotator-tui",
        ] {
            assert!(!r.matches_remote(url), "{url}");
        }
    }

    #[test]
    fn gh_failures_say_what_to_do() {
        let spec = PrSpec::Number(9);
        let auth = gh_failure(&spec, "To get started with GitHub CLI, please run:  gh auth login\n");
        assert!(matches!(auth, Error::GhNotLoggedIn { .. }), "{auth}");
        let missing = gh_failure(&spec, "GraphQL: Could not resolve to a PullRequest with the number of 9.");
        assert_eq!(
            missing.to_string(),
            "PR #9 not found (GraphQL: Could not resolve to a PullRequest with the number of 9.)"
        );
        let remote = gh_failure(
            &spec,
            "none of the git remotes configured for this repository point to a known GitHub host",
        );
        assert!(matches!(remote, Error::NoGitHubRemote { .. }), "{remote}");
        assert!(matches!(gh_failure(&spec, "boom"), Error::Gh { .. }));
    }
}
