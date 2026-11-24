//! What the code-review launchers (`herdr review`, `rex review`, `hc review`) share: the scope
//! flags, the check that a folder has something to review before any pane opens (or, for
//! `--pick`, only that it is in a repository, so the pane opens at once), and the scope as the
//! pane is told it.
//!
//! A Herdr pane is told the scope in `PLANNOTATOR_TUI_REVIEW`; a Rex or hc pane gets it as an
//! argument to `pane review`. Both carry the same text.

use std::path::{Path, PathBuf};

use anyhow::Result;
use plannotator_tui_review::{Repo, Request};

use super::cli::{self, ReviewArgs, Target};

/// The pane's scope value for the change picker.
const PICK: &str = "pick";

/// The scope `args` asks for (`--staged`, `--unstaged`, `--base REF`, `--commit SHA`, a pull
/// request as `--pr PR` or a bare link, `N` or `#N`, `--pick`, else uncommitted). A launcher
/// reviews its pane's repository, so a folder is refused, with `refusal` as the reason.
pub(crate) fn request(args: &[String], refusal: &str) -> Result<Target> {
    let ReviewArgs { target, dir } = ReviewArgs::parse(args)?;
    anyhow::ensure!(dir.is_none(), "{refusal}");
    Ok(target)
}

/// The root of the repository `dir` is in and `target` as the pane is told it, once there is
/// something to review: a folder outside a repository, or with no changes, fails here. A pull
/// request is told as its link, so the pane finds it from any folder; its folder is the clone
/// it was fetched into, else `dir`. The picker only needs a repository, so it opens at once;
/// a bare review with nothing to show opens the picker instead of failing, once the
/// repository has a commit to pick.
pub(crate) fn changes(dir: &Path, target: &Target) -> Result<(PathBuf, String)> {
    let request = match target {
        Target::Pick => return Ok((Repo::discover(dir)?.workdir().to_path_buf(), PICK.to_owned())),
        Target::Review(request) => request,
    };
    let (repo, capture) = super::capture_any(dir, request)?;
    let scope = match (&capture.pull, super::nothing_to_review(&repo, &capture)) {
        (_, Some(_)) if target.picks_when_empty() && super::picks_instead(&repo) => PICK.to_owned(),
        (_, Some(nothing)) => anyhow::bail!(nothing),
        (Some(pull), None) => {
            scope_value(&Target::Review(Request::PullRequest { pr: pull.view.url.clone() }))
        }
        (None, None) => scope_value(target),
    };
    Ok((repo.workdir().to_path_buf(), scope))
}

/// The pane: open the code review the launcher named.
pub(crate) fn pane(scope: &str, repo: Option<&Path>) -> Result<()> {
    cli::open(ReviewArgs { target: parse_scope(scope)?, dir: repo.map(Path::to_path_buf) })
}

/// A scope as the pane is told it.
pub(crate) fn scope_value(target: &Target) -> String {
    let Target::Review(request) = target else { return PICK.to_owned() };
    match request {
        Request::Uncommitted { .. } => "uncommitted".to_owned(),
        Request::Staged => "staged".to_owned(),
        Request::Unstaged => "unstaged".to_owned(),
        Request::Branch { base: Some(base) } => format!("base:{base}"),
        Request::Branch { base: None } => "base".to_owned(),
        Request::Commit { rev } => format!("commit:{rev}"),
        Request::PullRequest { pr } => format!("pr:{pr}"),
    }
}

/// [`scope_value`] back into a scope.
pub(crate) fn parse_scope(value: &str) -> Result<Target> {
    if value == PICK {
        return Ok(Target::Pick);
    }
    Ok(Target::Review(match value.split_once(':') {
        None if value == "uncommitted" => Request::Uncommitted { base: None },
        None if value == "staged" => Request::Staged,
        None if value == "unstaged" => Request::Unstaged,
        None if value == "base" => Request::Branch { base: None },
        Some(("base", base)) if !base.is_empty() => Request::Branch { base: Some(base.to_owned()) },
        Some(("commit", rev)) if !rev.is_empty() => Request::Commit { rev: rev.to_owned() },
        Some(("pr", pr)) if !pr.is_empty() => Request::PullRequest { pr: pr.to_owned() },
        _ => anyhow::bail!("PLANNOTATOR_TUI_REVIEW: {value:?} is not a scope"),
    }))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn every_scope_survives_the_trip_to_the_pane() {
        for target in [
            Request::Uncommitted { base: None },
            Request::Staged,
            Request::Unstaged,
            Request::Branch { base: Some("origin/main".into()) },
            Request::Branch { base: None },
            Request::Commit { rev: "HEAD~1".into() },
            Request::PullRequest { pr: "https://github.com/o/r/pull/12".into() },
        ]
        .map(Target::Review)
        .into_iter()
        .chain([Target::Pick])
        {
            assert_eq!(parse_scope(&scope_value(&target)).expect("parses"), target);
        }
        assert_eq!(
            (scope_value(&Target::Pick), scope_value(&Target::Review(Request::Unstaged))),
            ("pick".to_owned(), "unstaged".to_owned())
        );
        let commit = Target::Review(Request::Commit { rev: "1a2b:c".into() });
        assert_eq!(scope_value(&commit), "commit:1a2b:c");
        assert_eq!(parse_scope("commit:1a2b:c").expect("parses"), commit);
        assert!(parse_scope("commit:").is_err());
        assert!(parse_scope("everything").is_err());
    }

    #[test]
    fn a_launcher_takes_a_scope_but_no_folder() {
        let args = |list: &[&str]| list.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            request(&args(&["--staged"]), "no folder").expect("staged"),
            Target::Review(Request::Staged)
        );
        assert_eq!(request(&args(&["--pick"]), "no folder").expect("pick"), Target::Pick);
        assert!(request(&args(&["--pick", "--staged"]), "no folder").is_err(), "--pick takes no scope");
        let err = request(&args(&["../other"]), "rex review takes no folder").expect_err("refused");
        assert_eq!(err.to_string(), "rex review takes no folder");
    }
}
