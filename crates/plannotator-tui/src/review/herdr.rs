//! `plannotator-tui herdr review [--staged | --base REF | --commit SHA] [--placement P]
//! [--deliver-to PANE]`: a code review of the focused pane's repository in a Herdr pane.
//!
//! The launcher captures the diff before asking Herdr for a pane, so a folder that is no
//! repository, or has nothing to review, fails where the person can read it. The pane is
//! told the scope in `PLANNOTATOR_TUI_REVIEW` and the repository in `PLANNOTATOR_TUI_REPO`; the
//! pane side is [`super::scope::pane`].

use anyhow::{Context as _, Result};

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
    let refusal = "herdr review takes no folder: it reviews the focused pane's repository";
    let request = super::scope::request(&scope, refusal)?;
    let env = HerdrEnv::from_env();
    let config = Config::load()?;
    let cwd = std::env::current_dir().context("current directory")?;
    let (root, scope) = super::scope::changes(&review_folder(&env, &cwd), &request)?;
    let launch = plan_review(&env, &config, open, &root, scope)?;
    run(&env, &launch)
}
