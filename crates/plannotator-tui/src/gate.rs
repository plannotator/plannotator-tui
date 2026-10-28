//! Gate mode: an explicit Approve beside Send and quit, and the machine-readable decision a
//! script reads when the review ends (decision 17).
//!
//! The contract is Plannotator's `annotate --gate` (`apps/hook/server/strict-annotate-result.ts`,
//! `annotate-output.ts`, `packages/shared/prompts.ts`): knowledge copied, not code. One review,
//! one decision: `approved` (with or without notes), `annotated`, or `dismissed`.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

#[cfg(test)]
mod tests;

/// What ending the review means. `Off` is the ordinary review: sending leaves it open and
/// quitting decides nothing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    #[default]
    Off,
    /// `--json` alone: Send or quit ends the review with a decision; there is no Approve.
    Decision,
    /// `--gate`: Approve as well.
    Approve,
}

/// How a gated review ended. `feedback` is the numbered feedback the notes render to, the
/// same text Send delivers; `count` is how many notes it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Approved { feedback: String, count: usize },
    Annotated { feedback: String, count: usize },
    Dismissed,
}

/// Plannotator's plaintext approval, byte for byte.
pub(crate) const APPROVED: &str = "The user approved.";

/// Plannotator's `approvedWithNotes` annotate prompt; `{context}` is a `File:`/`Folder:` line
/// followed by a blank line.
const APPROVED_WITH_NOTES: &str = "# Approved with Notes\n\nThe artifact is approved. The notes below are \
non-blocking guidance, not a request for another revision.\n\n{context}{feedback}\n\nDo not revise or \
reopen the artifact solely because of these notes unless the user explicitly requests it. Carry the \
notes into subsequent work where applicable.";

/// The exit code for a failure of the gate itself under a strict flag: bad flags, a document
/// that cannot be opened, a result file that cannot be published. Never a reviewer outcome.
pub(crate) const GATE_ERROR_EXIT: u8 = 2;

/// An error the strict gate reports with [`GATE_ERROR_EXIT`].
#[derive(Debug)]
pub(crate) struct GateError(pub(crate) anyhow::Error);

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for GateError {}

/// One decision record. Field order is the browser's.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Record<'a> {
    decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    feedback: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotation_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<&'a str>,
}

impl Record<'_> {
    fn line(&self) -> String {
        // A struct of strings and numbers always serializes.
        serde_json::to_string(self).unwrap_or_default()
    }
}

impl Decision {
    pub(crate) fn is_approved(&self) -> bool {
        matches!(self, Self::Approved { .. })
    }

    /// The record `--require-approval` and `--result-file` publish: the decision and its
    /// feedback, nothing else. A bare approval carries no `feedback`; a dismissal never does.
    pub(crate) fn strict_json(&self) -> String {
        self.record().line()
    }

    /// `--json` without a strict flag: the strict record plus `annotationCount` and the
    /// reviewed `target` (an absolute path), which the browser adds there too.
    pub(crate) fn json(&self, target: &Path) -> String {
        let target = target.display().to_string();
        let annotation_count = match self {
            Self::Approved { count, .. } | Self::Annotated { count, .. } => Some(*count),
            Self::Dismissed => None,
        };
        Record { annotation_count, target: Some(&target), ..self.record() }.line()
    }

    fn record(&self) -> Record<'_> {
        let (decision, feedback) = match self {
            Self::Approved { feedback, .. } => ("approved", non_empty(feedback)),
            Self::Annotated { feedback, .. } => ("annotated", Some(feedback.as_str())),
            Self::Dismissed => ("dismissed", None),
        };
        Record { decision, feedback, annotation_count: None, target: None }
    }

    /// Plaintext stdout under `--gate`: the approval message, the feedback, or nothing for a
    /// dismissal or an empty send.
    pub(crate) fn plain(&self, context: &str) -> Option<String> {
        match self {
            Self::Approved { feedback, .. } => Some(approval_message(feedback, context)),
            Self::Annotated { feedback, .. } => non_empty(feedback.trim_end()).map(str::to_owned),
            Self::Dismissed => None,
        }
    }

    /// The feedback archive's `decision` for this outcome (Plannotator's `FeedbackDecision`).
    pub(crate) fn archive_decision(&self) -> &'static str {
        match self {
            Self::Approved { feedback, .. } if feedback.trim().is_empty() => "approved",
            Self::Approved { .. } => "approved-with-notes",
            Self::Annotated { .. } => "feedback",
            Self::Dismissed => "dismissed",
        }
    }

    /// `--require-approval` exits 0 only on approval; otherwise every decision exits 0.
    pub(crate) fn exit_code(&self, require_approval: bool) -> u8 {
        u8::from(require_approval && !self.is_approved())
    }
}

fn non_empty(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

/// What an approval tells the agent: Plannotator's bare marker, or its approved-with-notes
/// message naming the reviewed file or folder when notes ride along.
pub(crate) fn approval_message(feedback: &str, context: &str) -> String {
    let feedback = feedback.trim_end();
    if feedback.trim().is_empty() {
        return APPROVED.to_owned();
    }
    let context = if context.is_empty() { String::new() } else { format!("{context}\n\n") };
    APPROVED_WITH_NOTES.replace("{context}", &context).replace("{feedback}", feedback)
}

/// The `File: <path>` or `Folder: <path>` line an approval with notes names its target by.
pub(crate) fn context_line(path: &Path, folder: bool) -> String {
    format!("{}: {}", if folder { "Folder" } else { "File" }, path.display())
}

/// `plannotator-tui <file | folder> [--gate] [--json] [--require-approval] [--result-file PATH]`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReviewArgs {
    pub(crate) path: PathBuf,
    pub(crate) gate: bool,
    pub(crate) json: bool,
    pub(crate) require_approval: bool,
    /// As given; the caller resolves it from the invocation's directory.
    pub(crate) result_file: Option<PathBuf>,
}

/// The flags that turn a review into a decision.
pub(crate) const FLAGS: [&str; 4] = ["--gate", "--json", "--require-approval", "--result-file"];

/// Whether the invocation carries a strict flag, which owns the exit-code contract: every
/// failure, even a mistyped flag, exits [`GATE_ERROR_EXIT`] so it never reads as a rejection.
pub(crate) fn is_strict(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--require-approval" || arg == "--result-file")
}

impl ReviewArgs {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let mut out = Self::default();
        let mut path = None;
        let mut rest = args.iter();
        let once = |flag: &mut bool, name: &str| {
            anyhow::ensure!(!*flag, "{name} may only be specified once");
            *flag = true;
            Ok(())
        };
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--gate" => once(&mut out.gate, "--gate")?,
                "--json" => once(&mut out.json, "--json")?,
                "--require-approval" => once(&mut out.require_approval, "--require-approval")?,
                "--result-file" => {
                    anyhow::ensure!(out.result_file.is_none(), "--result-file may only be specified once");
                    let value = rest.next().filter(|v| !v.starts_with("--"));
                    out.result_file = Some(PathBuf::from(value.context("missing value for --result-file")?));
                }
                flag if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}"),
                other if path.is_none() => path = Some(PathBuf::from(other)),
                other => anyhow::bail!("unexpected argument {other:?}"),
            }
        }
        out.path = path.context("missing the file or folder to review")?;
        if out.strict() {
            anyhow::ensure!(
                out.gate && out.json,
                "--require-approval and --result-file require --gate --json"
            );
        }
        Ok(out)
    }

    pub(crate) fn strict(&self) -> bool {
        self.require_approval || self.result_file.is_some()
    }

    pub(crate) fn mode(&self) -> Gate {
        if self.gate {
            Gate::Approve
        } else if self.json {
            Gate::Decision
        } else {
            Gate::Off
        }
    }
}

/// Fail before the review opens when the result could never be published: the file must not
/// exist (not even as a dangling link) and its folder must.
pub(crate) fn check_result_file(path: &Path) -> Result<()> {
    match path.symlink_metadata() {
        Ok(_) => anyhow::bail!("result file already exists: {}", path.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("checking result file {}", path.display())),
    }
    let parent = path.parent().unwrap_or(Path::new(""));
    anyhow::ensure!(parent.is_dir(), "result file folder does not exist: {}", parent.display());
    Ok(())
}

/// Publish `record` and a newline to `path` atomically, never over an existing file: a
/// private temporary file beside it, flushed and closed, then hard-linked into place. A
/// link fails when the name is taken, so a reader sees no file or the whole record.
pub(crate) fn publish(path: &Path, record: &str) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new(""));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temporary = parent.join(format!(".{name}.{}.{nanos}.tmp", std::process::id()));
    let result = write_new(&temporary, record).and_then(|()| std::fs::hard_link(&temporary, path));
    let _ = std::fs::remove_file(&temporary);
    result.with_context(|| format!("publishing result file {}", path.display()))
}

fn write_new(path: &Path, record: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(format!("{record}\n").as_bytes())?;
    file.sync_all()
}
