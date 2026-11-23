//! Posting a [`ReviewRequest`] with the person's own `gh` login: one
//! `gh api --method POST repos/{owner}/{repo}/pulls/{number}/reviews --input -`, the JSON on
//! stdin. No token passes through this app. GitHub creates the review with every comment, or
//! refuses it whole, so a failure posts nothing.

use std::fmt;
use std::io::Write as _;
use std::process::{Command, Stdio};

use serde::Deserialize;

use super::ReviewRequest;
use crate::github::{Gh, PrRepo};

/// The review GitHub created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedReview {
    /// The review's page; the PR's link when gh's reply could not be read.
    pub url: String,
    /// False when gh succeeded but printed something this app could not read: the review
    /// was posted, its link is a guess.
    pub confirmed: bool,
}

/// Why a post failed. Every case posted nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostError {
    /// `gh` is not installed.
    GhMissing,
    /// `gh` is not logged in to the PR's host.
    NotLoggedIn { detail: String },
    /// The login cannot write reviews to this repository.
    NoPermission { detail: String },
    /// GitHub's API rate limit.
    RateLimited { detail: String },
    /// GitHub could not be reached.
    Offline { detail: String },
    /// The reviewed commit is no longer part of the PR (a force-push).
    HeadGone { detail: String },
    /// GitHub refused where one or more inline comments point.
    Positions { detail: String },
    /// An approval or a change request on one's own pull request.
    OwnPullRequest { detail: String },
    /// Anything else, with gh's and GitHub's words.
    Other { detail: String },
}

impl fmt::Display for PostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GhMissing => write!(
                f,
                "posting needs the GitHub CLI (gh), and it is not installed; install it from \
                 https://cli.github.com and run `gh auth login`"
            ),
            Self::NotLoggedIn { detail } => {
                write!(f, "gh is not logged in to GitHub; run `gh auth login` ({detail})")
            }
            Self::NoPermission { detail } => write!(
                f,
                "your GitHub login cannot review this pull request (no access to the repository, \
                 or gh's token lacks the repo scope; `gh auth refresh -s repo`) ({detail})"
            ),
            Self::RateLimited { detail } => {
                write!(f, "GitHub's rate limit was reached; wait a few minutes and post again ({detail})")
            }
            Self::Offline { detail } => {
                write!(f, "could not reach GitHub; check the connection and post again ({detail})")
            }
            Self::HeadGone { detail } => write!(
                f,
                "the commit you reviewed is no longer part of the pull request (it was force-pushed \
                 since); reload with r, check the notes, and post again ({detail})"
            ),
            Self::Positions { detail } => {
                write!(f, "GitHub refused the place of an inline comment ({detail})")
            }
            Self::OwnPullRequest { detail } => write!(
                f,
                "GitHub does not let you approve or request changes on your own pull request; post \
                 it as a Comment ({detail})"
            ),
            Self::Other { detail } => write!(f, "gh api failed: {detail}"),
        }
    }
}

impl std::error::Error for PostError {}

#[derive(Deserialize)]
struct ReviewJson {
    html_url: String,
}

impl Gh {
    /// Create one review on PR `number` of `repo` with `request`. Writes to GitHub: call it
    /// only once the person confirmed what it posts.
    pub fn post_review(
        &self,
        repo: &PrRepo,
        number: u64,
        request: &ReviewRequest,
    ) -> Result<PostedReview, PostError> {
        let json = serde_json::to_vec(request).map_err(|e| PostError::Other { detail: e.to_string() })?;
        let mut cmd = Command::new(self.program());
        cmd.arg("api");
        if !repo.host.eq_ignore_ascii_case("github.com") {
            cmd.args(["--hostname", &repo.host]);
        }
        let endpoint = format!("repos/{}/{}/pulls/{number}/reviews", repo.owner, repo.name);
        cmd.args(["--method", "POST", &endpoint, "--input", "-"])
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                PostError::GhMissing
            } else {
                PostError::Other { detail: format!("could not run gh: {source}") }
            }
        })?;
        let written = child.stdin.take().map(|mut stdin| stdin.write_all(&json));
        let output = child
            .wait_with_output()
            .map_err(|e| PostError::Other { detail: format!("waiting for gh: {e}") })?;
        if let Some(Err(e)) = written
            && output.status.success()
        {
            return Err(PostError::Other { detail: format!("could not hand the review to gh: {e}") });
        }
        if !output.status.success() {
            return Err(classify(
                &String::from_utf8_lossy(&output.stdout),
                &String::from_utf8_lossy(&output.stderr),
            ));
        }
        Ok(match serde_json::from_slice::<ReviewJson>(&output.stdout) {
            Ok(review) => PostedReview { url: review.html_url, confirmed: true },
            Err(_) => PostedReview { url: format!("{}/pull/{number}", repo.web_url()), confirmed: false },
        })
    }
}

/// gh's failure, from its stderr and the response body it prints on stdout, named for the
/// person.
pub(crate) fn classify(stdout: &str, stderr: &str) -> PostError {
    let detail = detail(stdout, stderr);
    let all = format!("{stdout}\n{stderr}").to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| all.contains(n));
    if has(&["gh auth login", "not logged in", "bad credentials", "http 401", "requires authentication"]) {
        PostError::NotLoggedIn { detail }
    } else if has(&["rate limit", "http 429"]) {
        PostError::RateLimited { detail }
    } else if has(&[
        "error connecting to",
        "could not resolve host",
        "no such host",
        "network is unreachable",
        "connection refused",
        "i/o timeout",
        "tls handshake timeout",
        "dial tcp",
    ]) {
        PostError::Offline { detail }
    } else if has(&["your own pull request"]) {
        PostError::OwnPullRequest { detail }
    } else if has(&["commit_id", "not part of the pull request", "no commit found for sha"]) {
        PostError::HeadGone { detail }
    } else if has(&["http 403", "http 404", "resource not accessible", "must have push access"]) {
        PostError::NoPermission { detail }
    } else if has(&["http 422"]) && has(&["line", "position", "path", "diff hunk", "start_line", "side"]) {
        PostError::Positions { detail }
    } else {
        PostError::Other { detail }
    }
}

/// The most specific words available, capped: GitHub's `errors[]` (strings or `{message}`)
/// and `message` from the JSON gh prints, else gh's stderr.
fn detail(stdout: &str, stderr: &str) -> String {
    const MAX: usize = 300;
    let from_json = serde_json::from_str::<serde_json::Value>(stdout.trim()).ok().and_then(|json| {
        let errors: Vec<String> = json
            .get("errors")
            .and_then(serde_json::Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|e| {
                        e.as_str().map(str::to_owned).or_else(|| {
                            e.get("message").and_then(serde_json::Value::as_str).map(str::to_owned)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let message = json.get("message").and_then(serde_json::Value::as_str).unwrap_or("");
        match (errors.is_empty(), message.is_empty()) {
            (false, true) => Some(errors.join("; ")),
            (false, false) => Some(format!("{message}: {}", errors.join("; "))),
            (true, false) => Some(message.to_owned()),
            (true, true) => None,
        }
    });
    let stderr = stderr.trim().trim_start_matches("gh: ").to_owned();
    // gh's own line repeats GitHub's message with the status (`Unprocessable Entity (HTTP
    // 422)`); GitHub's words are the specific ones.
    let text = match from_json {
        Some(json) => json,
        None if stderr.is_empty() => "gh api failed with no message".to_owned(),
        None => stderr,
    };
    let text = text.replace('\n', " ");
    if text.chars().count() > MAX {
        let cut: String = text.chars().take(MAX).collect();
        format!("{cut}\u{2026}")
    } else {
        text
    }
}
