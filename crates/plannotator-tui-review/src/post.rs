//! A pull request review on GitHub, built from a review's notes the way the browser app
//! builds one (`packages/review-editor/components/ReviewSubmissionDialog.tsx`), and posted in
//! one `gh api` request ([`crate::Gh::post_review`]), so GitHub takes all of it or none.
//!
//! - **Inline comments**: line notes GitHub can anchor at the reviewed commit. `side` is
//!   `LEFT` for old-side lines and `RIGHT` for new ones; `line` is the last line, and
//!   `start_line`/`start_side` (the same side) are added only for a range. The body is the
//!   note's text (after a browser note's conventional-comment prefix) and, for a suggestion,
//!   a `` ```suggestion `` block.
//! - **The review body**, in note order: whole-change notes as plain paragraphs, file notes
//!   as `**path:** text`, and line notes GitHub cannot take inline (outdated, or their lines
//!   are no longer in the diff) as `**path (L5-L9, new):**` with the code they were made on.
//!   Notes on lines outside the diff (`outsideDiff`) are never inline: they close the body
//!   under **Comments outside the diff**, each with its code quoted and a link to the lines
//!   at the reviewed commit. A body that would be empty beside inline comments is
//!   `See inline comments.`, as GitHub requires one for a comment review.
//!
//! One difference from the browser: on GitHub it posts file notes as file-level threads
//! (a pending review, a GraphQL call per file, then a submit), which is not atomic. Here
//! they go in the body in the browser's own fallback shape (`**path:** text`), so a post is
//! always one request.
//!
//! Notes a post already carried are left out unless edited since; an edited one is posted
//! again as a new comment ([`Posting::Edited`]).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::Value;

use crate::annotation::{CodeAnnotation, NoteScope, Side, Status};
use crate::export::OUTDATED_ANNOTATION_LABEL;
use crate::github::PrRepo;
use crate::patch::FilePatch;

mod submit;

pub use submit::{PostError, PostedReview};

/// The browser's `OUTSIDE_DIFF_BODY_HEADING`.
pub const OUTSIDE_DIFF_BODY_HEADING: &str = "Comments outside the diff";

/// The browser's `OUTSIDE_DIFF_QUOTE_MAX_LINES`: code quoted per note outside the diff.
pub const OUTSIDE_DIFF_QUOTE_MAX_LINES: usize = 40;

/// The browser's `COMMENT_BODY_PLACEHOLDER`.
pub const BODY_PLACEHOLDER: &str = "See inline comments.";

/// GitHub refuses a review body longer than this many characters.
pub const MAX_BODY_CHARS: usize = 65_536;

/// The review's verdict. The browser defaults to a comment and offers the other two as
/// explicit choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Event {
    #[default]
    Comment,
    Approve,
    RequestChanges,
}

impl Event {
    pub const ALL: [Self; 3] = [Self::Comment, Self::Approve, Self::RequestChanges];

    pub fn label(self) -> &'static str {
        match self {
            Self::Comment => "Comment",
            Self::Approve => "Approve",
            Self::RequestChanges => "Request changes",
        }
    }
}

/// Where a note stands with GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posting {
    /// No post carried it.
    New,
    /// A post carried it, and it has not changed since.
    Posted,
    /// A post carried it, and it was edited after.
    Edited,
}

/// One inline comment, as the create-review endpoint takes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InlineComment {
    pub path: String,
    pub line: u32,
    pub side: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<&'static str>,
    pub body: String,
}

/// The JSON `POST /repos/{owner}/{repo}/pulls/{number}/reviews` receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewRequest {
    pub commit_id: String,
    pub body: String,
    pub event: Event,
    pub comments: Vec<InlineComment>,
}

/// Why a note goes in the review body rather than inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InBody {
    /// A note on the whole change.
    Change,
    /// A note on a whole file.
    File,
    /// A line note on lines outside the diff, made in the whole file.
    OutsideDiff,
    /// A line note whose code changed, or whose lines the diff no longer shows.
    OffDiff,
}

/// Why a note is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// A post already carried it, unchanged since.
    AlreadyPosted,
    /// It has no text and no suggested code.
    Empty,
}

/// The pull request a post goes to, and the commit its comments are anchored to.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub repo: &'a PrRepo,
    pub number: u64,
    /// The head commit the person reviewed: the one the captured diff ends at.
    pub reviewed_head: &'a str,
}

/// What a post would carry, before the person confirms it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub commit_id: String,
    /// Notes in the body's order, without the placeholder.
    pub body: String,
    pub comments: Vec<InlineComment>,
    /// Ids of the notes posted inline, parallel to `comments`.
    pub inline: Vec<String>,
    /// Ids of the notes in the body, and why each is there.
    pub in_body: Vec<(String, InBody)>,
    pub skipped: Vec<(String, Skip)>,
    /// Ids of included notes a post carried before they were edited: posted again, new.
    pub reposted: Vec<String>,
}

impl Plan {
    /// Build the post of `notes` (in record order) against the reviewed diff's `files`.
    /// `posting` says where each note stands with GitHub.
    pub fn build(
        notes: &[CodeAnnotation],
        files: &[FilePatch],
        target: Target<'_>,
        posting: impl Fn(&CodeAnnotation) -> Posting,
    ) -> Self {
        let mut plan = Self {
            commit_id: target.reviewed_head.to_owned(),
            body: String::new(),
            comments: Vec::new(),
            inline: Vec::new(),
            in_body: Vec::new(),
            skipped: Vec::new(),
            reposted: Vec::new(),
        };
        let mut parts: Vec<String> = Vec::new();
        let mut outside: Vec<String> = Vec::new();
        for note in notes {
            let state = posting(note);
            if state == Posting::Posted {
                plan.skipped.push((note.id.clone(), Skip::AlreadyPosted));
                continue;
            }
            let place = placement(note, files);
            let placed = match place {
                Placement::Inline => inline_comment(note).map(|comment| plan.comments.push(comment)),
                Placement::Body(InBody::OutsideDiff) => {
                    outside_entry(note, target).map(|entry| outside.push(entry))
                }
                Placement::Body(kind) => body_entry(note, kind).map(|entry| parts.push(entry)),
            };
            if placed.is_none() {
                plan.skipped.push((note.id.clone(), Skip::Empty));
                continue;
            }
            match place {
                Placement::Inline => plan.inline.push(note.id.clone()),
                Placement::Body(kind) => plan.in_body.push((note.id.clone(), kind)),
            }
            if state == Posting::Edited {
                plan.reposted.push(note.id.clone());
            }
        }
        if !outside.is_empty() {
            parts.push(format!("**{OUTSIDE_DIFF_BODY_HEADING}**\n\n{}", outside.join("\n\n")));
        }
        plan.body = parts.join("\n\n");
        plan
    }

    /// Whether there is anything to post.
    pub fn is_empty(&self) -> bool {
        self.comments.is_empty() && self.body.trim().is_empty()
    }

    /// The ids of every note the post carries.
    pub fn ids(&self) -> Vec<String> {
        self.inline.iter().chain(self.in_body.iter().map(|(id, _)| id)).cloned().collect()
    }

    /// How many notes go in the body for `kind`.
    pub fn count_in_body(&self, kind: InBody) -> usize {
        self.in_body.iter().filter(|(_, k)| *k == kind).count()
    }

    pub fn count_skipped(&self, why: Skip) -> usize {
        self.skipped.iter().filter(|(_, s)| *s == why).count()
    }

    /// The request for `event`: an empty body beside inline comments becomes the
    /// placeholder, except on an approval, which may have none.
    pub fn request(&self, event: Event) -> ReviewRequest {
        let body = if self.body.trim().is_empty() && event != Event::Approve && !self.comments.is_empty() {
            BODY_PLACEHOLDER.to_owned()
        } else {
            self.body.clone()
        };
        ReviewRequest { commit_id: self.commit_id.clone(), body, event, comments: self.comments.clone() }
    }

    /// Whether the body is over GitHub's limit.
    pub fn body_too_long(&self) -> bool {
        self.body.chars().count() > MAX_BODY_CHARS
    }
}

#[derive(Clone, Copy)]
enum Placement {
    Inline,
    Body(InBody),
}

/// Inline when GitHub can anchor it: a line note, not outdated, not outside the diff, whose
/// lines the reviewed diff shows on its side (all of them, so one hunk).
fn placement(note: &CodeAnnotation, files: &[FilePatch]) -> Placement {
    match note.scope {
        NoteScope::General => Placement::Body(InBody::Change),
        NoteScope::File => Placement::Body(InBody::File),
        NoteScope::Line if note.outside_diff => Placement::Body(InBody::OutsideDiff),
        NoteScope::Line => {
            let outdated = note.outdated || note.status() == Some(Status::Outdated);
            let off = note.status() == Some(Status::NotInDiff);
            let shown = files
                .iter()
                .find(|f| f.path == note.file_path)
                .is_some_and(|f| f.shows(note.side, note.line_start, note.line_end));
            if outdated || off || !shown { Placement::Body(InBody::OffDiff) } else { Placement::Inline }
        }
    }
}

fn github_side(side: Side) -> &'static str {
    match side {
        Side::Old => "LEFT",
        Side::New => "RIGHT",
    }
}

/// The browser's `buildAnnotationFileComments` for one note; `None` when its body is empty.
fn inline_comment(note: &CodeAnnotation) -> Option<InlineComment> {
    let mut body = format!("{}{}", conventional_prefix(&note.other), text(note));
    if let Some(code) = suggested(note) {
        let _ = write!(body, "\n\n```suggestion\n{code}\n```");
    }
    let body = body.trim().to_owned();
    if body.is_empty() {
        return None;
    }
    let side = github_side(note.side);
    let range = note.line_start != note.line_end;
    Some(InlineComment {
        path: note.file_path.clone(),
        line: note.line_end,
        side,
        start_line: range.then_some(note.line_start),
        start_side: range.then_some(side),
        body,
    })
}

/// One paragraph of the browser's `buildFileScopedBody` (with no file-level threads).
fn body_entry(note: &CodeAnnotation, kind: InBody) -> Option<String> {
    let text = text(note);
    let entry = match kind {
        InBody::Change if !text.is_empty() => text.to_owned(),
        InBody::File if !text.is_empty() => format!("**{}:** {text}", note.file_path),
        InBody::OffDiff if !text.is_empty() || suggested(note).is_some() => {
            let outdated = note.outdated || note.status() == Some(Status::Outdated);
            let label = if outdated { format!("{OUTDATED_ANNOTATION_LABEL} ") } else { String::new() };
            let commented_on = note
                .anchor_text
                .as_deref()
                .map(|code| format!("\n\nCommented on:\n```\n{code}\n```"))
                .unwrap_or_default();
            let suggestion = suggested(note)
                .map(|code| format!("\n\nSuggested code:\n```\n{code}\n```"))
                .unwrap_or_default();
            format!(
                "**{} ({}, {}):** {label}{text}{commented_on}{suggestion}",
                note.file_path,
                lines(note),
                note.side.as_str()
            )
        }
        _ => return None,
    };
    Some(entry.trim().to_owned())
}

/// One entry of the browser's `buildOutsideDiffSection`.
fn outside_entry(note: &CodeAnnotation, target: Target<'_>) -> Option<String> {
    let text = text(note);
    if text.is_empty() && suggested(note).is_none() {
        return None;
    }
    let lines = lines(note);
    let link = permalink(target, &note.file_path, note.line_start, note.line_end);
    let quoted = note
        .anchor_text
        .as_deref()
        .or(note.original_code.as_deref())
        .map(|code| format!("\n\n{}", code_fence(&cap_quoted(code))))
        .unwrap_or_default();
    let suggestion =
        suggested(note).map(|code| format!("\n\nSuggested code:\n{}", code_fence(code))).unwrap_or_default();
    let prefix = conventional_prefix(&note.other);
    let entry = format!("**[{} ({lines})]({link}):** {prefix}{text}{quoted}{suggestion}", note.file_path);
    Some(entry.trim().to_owned())
}

fn text(note: &CodeAnnotation) -> &str {
    note.text.as_deref().unwrap_or("")
}

fn suggested(note: &CodeAnnotation) -> Option<&str> {
    note.suggested_code.as_deref().filter(|code| !code.is_empty())
}

/// `L5` or `L5-L9`.
fn lines(note: &CodeAnnotation) -> String {
    if note.line_start == note.line_end {
        format!("L{}", note.line_start)
    } else {
        format!("L{}-L{}", note.line_start, note.line_end)
    }
}

/// The browser's `formatConventionalPrefix`, for a note the browser wrote with a
/// conventional-comment label: `**issue (blocking):** `. Notes made here have none.
fn conventional_prefix(other: &BTreeMap<String, Value>) -> String {
    let Some(label) = other.get("conventionalLabel").and_then(Value::as_str).filter(|l| !l.is_empty()) else {
        return String::new();
    };
    let decorations: Vec<&str> = other
        .get("decorations")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let decorations =
        if decorations.is_empty() { String::new() } else { format!(" ({})", decorations.join(", ")) };
    format!("**{label}{decorations}:** ")
}

/// The browser's `prBlobPermalink` on GitHub: the lines at the reviewed commit.
fn permalink(target: Target<'_>, path: &str, first: u32, last: u32) -> String {
    let path: Vec<String> = path.split('/').map(encode_component).collect();
    let lines = if first == last { format!("L{first}") } else { format!("L{first}-L{last}") };
    format!(
        "{}/blob/{}/{}#{lines}",
        target.repo.web_url(),
        encode_component(target.reviewed_head),
        path.join("/")
    )
}

/// JavaScript's `encodeURIComponent`.
fn encode_component(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    for byte in part.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The browser's `capQuotedCode`: the first [`OUTSIDE_DIFF_QUOTE_MAX_LINES`] lines, then `…`.
fn cap_quoted(code: &str) -> String {
    let lines: Vec<&str> = code.split('\n').collect();
    if lines.len() <= OUTSIDE_DIFF_QUOTE_MAX_LINES {
        return code.to_owned();
    }
    let kept: Vec<&str> = lines.into_iter().take(OUTSIDE_DIFF_QUOTE_MAX_LINES).collect();
    format!("{}\n\u{2026}", kept.join("\n"))
}

/// The browser's `codeFence`: one backtick longer than the longest run inside, at least 3.
fn code_fence(code: &str) -> String {
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0).max(2);
    let fence = "`".repeat(longest + 1);
    format!("{fence}\n{code}\n{fence}")
}

#[cfg(test)]
mod tests;
