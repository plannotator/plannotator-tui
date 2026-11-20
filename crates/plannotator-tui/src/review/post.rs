//! Posting the notes to the pull request on GitHub (`ctrl+g`, or the Review menu): a
//! summary first, always, then one review in one `gh api` request
//! ([`plannotator_tui_review::post`]). Only a pull request review offers it.
//!
//! The summary names the PR, how many notes go inline and how many in the review body (and
//! why), what is left out, and the verdict: Comment unless the person picks Approve or
//! Request changes, as the browser offers them. `Enter` posts and `Esc` cancels; nothing
//! leaves the machine before `Enter`. Comments are anchored to the commit the diff was
//! captured at, the one the person reviewed; when the PR has moved on since, the summary
//! says so. A post that fails posted nothing (GitHub takes the review whole or not at all),
//! and the summary stays open with the reason. A post that succeeds marks its notes posted,
//! a state of its own beside sent, saved with the notes; a later post carries only notes that
//! are new or edited since.

use std::fmt::Write as _;

use plannotator_tui_review::post::{Event, InBody, Plan, PostError, Posting, Skip, Target};
use plannotator_tui_review::{CodeAnnotation, PrSpec, Scope};
use ratatui::crossterm::event::KeyEvent;

use super::{Mode, Screen};
use crate::keys::{self, Action};

/// The summary on screen before a post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Confirm {
    pub(super) plan: Plan,
    pub(super) event: Event,
    /// What `gh pr view` says about the PR's head now.
    pub(super) head: Head,
    /// Why the last try posted nothing.
    pub(super) failed: Option<String>,
}

/// The PR's head on GitHub, against the commit the person reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Head {
    Reviewed,
    /// The PR has newer commits than the reviewed one.
    Moved {
        now: String,
    },
    /// `gh pr view` failed; the post will say what is wrong.
    Unknown(String),
}

impl Screen {
    /// `post-to-github`: open the summary, or say why there is nothing to post.
    pub(super) fn open_post(&mut self) {
        let (Some(pull), Some(gh)) = (&self.capture.pull, self.repo.gh()) else {
            self.status = Some(format!(
                "Post to GitHub needs a pull request review (plannotator-tui review <PR>); this one is {}",
                self.capture.scope.title()
            ));
            return;
        };
        let plan = Plan::build(
            self.store.annotations(),
            &self.capture.files,
            Target { repo: &pull.view.repo, number: pull.view.number, reviewed_head: &pull.head },
            |note| self.posting(note),
        );
        if plan.is_empty() {
            let posted = plan.count_skipped(Skip::AlreadyPosted);
            self.status = Some(match (self.store.annotations().is_empty(), posted) {
                (true, _) => "no notes to post to GitHub".to_owned(),
                (false, 0) => "nothing to post to GitHub: the notes are empty".to_owned(),
                (false, n) => format!("nothing new to post to GitHub ({n} already posted)"),
            });
            return;
        }
        // Read-only: where the PR's head is now.
        let spec = match &self.capture.scope {
            Scope::PullRequest { url, number } => PrSpec::parse(url).unwrap_or(PrSpec::Number(*number)),
            _ => PrSpec::Number(pull.view.number),
        };
        let head = match gh.view(&spec, self.repo.workdir()) {
            Ok(view) if view.head_sha == pull.head => Head::Reviewed,
            Ok(view) => Head::Moved { now: view.head_sha },
            Err(err) => Head::Unknown(err.to_string()),
        };
        self.mode = Mode::Post(Confirm { plan, event: Event::default(), head, failed: None });
    }

    fn posting(&self, note: &CodeAnnotation) -> Posting {
        if self.store.is_posted(note) {
            Posting::Posted
        } else if self.store.was_posted(note) {
            Posting::Edited
        } else {
            Posting::New
        }
    }

    pub(super) fn post_key(&mut self, key: KeyEvent, mut confirm: Confirm) {
        let step = |event: Event, by: isize| {
            let at = Event::ALL.iter().position(|e| *e == event).unwrap_or(0);
            let next = at.saturating_add_signed(by).min(Event::ALL.len() - 1);
            Event::ALL.get(next).copied().unwrap_or_default()
        };
        match self.keys.action(&key, keys::POST_REVIEW) {
            Some(Action::Left) => confirm.event = step(confirm.event, -1),
            Some(Action::Right) => confirm.event = step(confirm.event, 1),
            Some(Action::Confirm) => return self.post(confirm),
            Some(Action::Back | Action::Interrupt) => {
                self.mode = Mode::Browse;
                self.status = Some("nothing posted to GitHub".into());
                return;
            }
            _ => return,
        }
        self.mode = Mode::Post(confirm);
    }

    /// The person confirmed: post, then mark what was posted.
    fn post(&mut self, mut confirm: Confirm) {
        let (Some(pull), Some(gh)) = (&self.capture.pull, self.repo.gh()) else {
            self.mode = Mode::Browse;
            return;
        };
        if confirm.plan.body_too_long() {
            confirm.failed = Some(format!(
                "the review body is over GitHub's {} characters; post fewer notes",
                plannotator_tui_review::post::MAX_BODY_CHARS
            ));
            self.mode = Mode::Post(confirm);
            return;
        }
        let request = confirm.plan.request(confirm.event);
        match gh.post_review(&pull.view.repo, pull.view.number, &request) {
            Ok(review) => {
                let ids = confirm.plan.ids();
                let saved = self.store.record_post(&review.url, &confirm.plan.commit_id, &ids);
                let mut status = if review.confirmed {
                    let (inline, body) = (confirm.plan.comments.len(), confirm.plan.in_body.len());
                    format!("posted to GitHub: {inline} inline, {body} in the body \u{b7} {}", review.url)
                } else {
                    format!(
                        "posted to GitHub, but gh's reply could not be read; the notes are marked posted so \
                         they are not posted twice \u{b7} check {}",
                        review.url
                    )
                };
                if let Err(err) = saved {
                    let _ = write!(
                        status,
                        " \u{b7} could not save the posted state ({err}); a later post may repeat these notes"
                    );
                }
                self.status = Some(status);
                self.mode = Mode::Browse;
                self.after_note_change();
            }
            Err(err) => {
                let mut why = format!("Nothing was posted: {err}.");
                if matches!(err, PostError::Positions { .. }) {
                    let _ = write!(
                        why,
                        " The inline comments, each checked against the diff you reviewed, were on {}.",
                        comment_places(&confirm.plan)
                    );
                    if let Head::Moved { now } = &confirm.head {
                        let reload = self.keys.labelled(Action::Reload, "reloads it").unwrap_or_default();
                        let _ =
                            write!(why, " The PR is at {} now; {reload} and the notes follow.", short(now));
                    }
                }
                self.status = Some(format!("nothing posted to GitHub: {}", short_reason(&err)));
                confirm.failed = Some(why);
                self.mode = Mode::Post(confirm);
            }
        }
    }

    /// The Review menu row's words.
    pub(super) fn post_menu_label(&self) -> String {
        if self.capture.pull.is_none() {
            return "Post to GitHub\u{2026} \u{b7} pull requests only".to_owned();
        }
        let new = self.store.annotations().iter().filter(|n| !self.store.is_posted(n)).count();
        format!("Post to GitHub\u{2026} \u{b7} {new} new")
    }
}

/// `app.py:9 new, app.py:3-5 old`.
fn comment_places(plan: &Plan) -> String {
    let places: Vec<String> = plan
        .comments
        .iter()
        .map(|c| {
            let side = if c.side == "LEFT" { "old" } else { "new" };
            match c.start_line {
                Some(start) => format!("{}:{start}-{} {side}", c.path, c.line),
                None => format!("{}:{} {side}", c.path, c.line),
            }
        })
        .collect();
    places.join(", ")
}

/// A few words for the footer; the summary has the rest.
fn short_reason(err: &PostError) -> &'static str {
    match err {
        PostError::GhMissing => "gh is not installed",
        PostError::NotLoggedIn { .. } => "gh is not logged in",
        PostError::NoPermission { .. } => "no permission",
        PostError::RateLimited { .. } => "rate limited",
        PostError::Offline { .. } => "GitHub unreachable",
        PostError::HeadGone { .. } => "the reviewed commit is gone from the PR",
        PostError::Positions { .. } => "GitHub refused a comment's place",
        PostError::OwnPullRequest { .. } => "your own pull request",
        PostError::Other { .. } => "gh api failed",
    }
}

pub(super) fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The summary's lines about what goes where, for the overlay.
pub(super) fn summary(plan: &Plan) -> Vec<String> {
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut lines = vec![plural(plan.comments.len(), "inline comment", "inline comments")];
    let body: Vec<String> = [
        (InBody::Change, "whole-change note", "whole-change notes"),
        (InBody::File, "file note", "file notes"),
        (
            InBody::OutsideDiff,
            "note outside the diff, quoted with a link",
            "notes outside the diff, quoted with a link",
        ),
        (
            InBody::OffDiff,
            "line note outdated or no longer in the diff, quoted",
            "line notes outdated or no longer in the diff, quoted",
        ),
    ]
    .into_iter()
    .filter_map(|(kind, one, many)| {
        let n = plan.count_in_body(kind);
        (n > 0).then(|| format!("  {}", plural(n, one, many)))
    })
    .collect();
    if body.is_empty() {
        lines.push("nothing else in the review body".to_owned());
    } else {
        lines.push(format!("{} in the review body:", plan.in_body.len()));
        lines.extend(body);
    }
    if !plan.reposted.is_empty() {
        lines.push(format!(
            "{} edited since posted: posted again as new comments (the earlier ones stay on GitHub)",
            plan.reposted.len()
        ));
    }
    let skipped: Vec<String> = [(Skip::AlreadyPosted, "already posted"), (Skip::Empty, "empty")]
        .into_iter()
        .filter_map(|(why, label)| {
            let n = plan.count_skipped(why);
            (n > 0).then(|| format!("{n} {label}"))
        })
        .collect();
    if !skipped.is_empty() {
        lines.push(format!("skipped: {}", skipped.join(", ")));
    }
    lines
}
