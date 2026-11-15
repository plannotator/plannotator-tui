//! `plannotator-tui herdr review [--staged | --base REF | --commit SHA] [--placement P]
//! [--deliver-to PANE]`: a code review of the focused pane's repository in a Herdr pane, and
//! the pane side that opens it.
//!
//! The launcher captures the diff before asking Herdr for a pane, so a folder that is no
//! repository, or has nothing to review, fails where the person can read it. The pane is
//! told the scope in `PLANNOTATOR_TUI_REVIEW` and the repository in `PLANNOTATOR_TUI_REPO`.

use std::path::Path;

use anyhow::{Context as _, Result};
use plannotator_tui_review::Request;

use super::cli::{self, ReviewArgs};
use crate::config::Config;
use crate::herdr::context::HerdrEnv;
use crate::herdr::launch::{OpenArgs, plan_review, review_folder, run};

/// The launcher: find the repository, check it has changes, open the pane.
pub(crate) fn launch(args: &[String]) -> Result<()> {
    let mut open = OpenArgs::default();
    let mut scope = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--placement" => {
                let value = rest.next().context("--placement needs a value")?;
                open.placement = Some(value.parse()?);
            }
            "--deliver-to" => {
                open.deliver_to = Some(rest.next().context("--deliver-to needs a value")?.clone());
            }
            _ => scope.push(arg.clone()),
        }
    }
    let ReviewArgs { request, dir } = ReviewArgs::parse(&scope)?;
    anyhow::ensure!(dir.is_none(), "herdr review takes no folder: it reviews the focused pane's repository");
    let env = HerdrEnv::from_env();
    let config = Config::load()?;
    let cwd = std::env::current_dir().context("current directory")?;
    let (repo, _) = super::capture(&review_folder(&env, &cwd), &request)?;
    let launch = plan_review(&env, &config, open, repo.root(), scope_value(&request))?;
    run(&env, &launch)
}

/// The pane: open the code review the launcher named.
pub(crate) fn pane(scope: &str, repo: Option<&Path>) -> Result<()> {
    cli::open(ReviewArgs { request: parse_scope(scope)?, dir: repo.map(Path::to_path_buf) })
}

/// A scope as `PLANNOTATOR_TUI_REVIEW` carries it.
pub(super) fn scope_value(request: &Request) -> String {
    match request {
        Request::Uncommitted { .. } => "uncommitted".to_owned(),
        Request::Staged => "staged".to_owned(),
        Request::Branch { base: Some(base) } => format!("base:{base}"),
        Request::Branch { base: None } => "base".to_owned(),
        Request::Commit { rev } => format!("commit:{rev}"),
    }
}

/// `PLANNOTATOR_TUI_REVIEW` back into a scope.
pub(super) fn parse_scope(value: &str) -> Result<Request> {
    Ok(match value.split_once(':') {
        None if value == "uncommitted" => Request::Uncommitted { base: None },
        None if value == "staged" => Request::Staged,
        None if value == "base" => Request::Branch { base: None },
        Some(("base", base)) if !base.is_empty() => Request::Branch { base: Some(base.to_owned()) },
        Some(("commit", rev)) if !rev.is_empty() => Request::Commit { rev: rev.to_owned() },
        _ => anyhow::bail!("PLANNOTATOR_TUI_REVIEW: {value:?} is not a scope"),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn every_scope_survives_the_trip_through_the_environment() {
        for request in [
            Request::Uncommitted { base: None },
            Request::Staged,
            Request::Branch { base: Some("origin/main".into()) },
            Request::Branch { base: None },
            Request::Commit { rev: "HEAD~1".into() },
        ] {
            assert_eq!(parse_scope(&scope_value(&request)).expect("parses"), request);
        }
        assert_eq!(scope_value(&Request::Commit { rev: "1a2b:c".into() }), "commit:1a2b:c");
        assert_eq!(parse_scope("commit:1a2b:c").expect("parses"), Request::Commit { rev: "1a2b:c".into() });
        assert!(parse_scope("commit:").is_err());
        assert!(parse_scope("everything").is_err());
    }
}
