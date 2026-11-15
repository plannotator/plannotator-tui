//! A delivery command: any program that takes the feedback on stdin, for agents and
//! multiplexers without a built-in target. `PLANNOTATOR_TUI_DELIVER_CMD`, else
//! `[deliver] command`, names it.
//!
//! The value is one command line, run by the shell (`sh -c` on unix, `cmd /S /C` on Windows),
//! so it can be a pipeline or use variables. The feedback arrives on stdin and the exit status
//! is the outcome: 0 delivered (the first line on stdout, if any, joins the status), 75 the
//! agent cannot take a message yet (sysexits' `EX_TEMPFAIL`), 69 there is no agent to send to
//! (`EX_UNAVAILABLE`); the review copies on both. Anything else failed, with the command's
//! stderr. The review waits for the command, so one that hangs is stopped after [`TIMEOUT`].

use std::cell::RefCell;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{Delivery, DeliveryError};

/// `PLANNOTATOR_TUI_DELIVER_CMD`: the command line; overrides `[deliver] command`.
pub(crate) const COMMAND_VAR: &str = "PLANNOTATOR_TUI_DELIVER_CMD";
/// `PLANNOTATOR_TUI_REVIEW_PATH`: set for the command to the file or folder under review.
pub(crate) const PATH_VAR: &str = "PLANNOTATOR_TUI_REVIEW_PATH";
/// The agent is at a dialog or busy: retry later. sysexits' `EX_TEMPFAIL`.
pub(crate) const EXIT_BLOCKED: i32 = 75;
/// There is no agent to send to. sysexits' `EX_UNAVAILABLE`.
pub(crate) const EXIT_UNAVAILABLE: i32 = 69;
/// The longest the review waits for the command before stopping it.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);
/// How long output is still read after the command exits, for a background child that keeps
/// stdout open.
const DRAIN: Duration = Duration::from_millis(200);

#[derive(Debug)]
pub(crate) struct DeliverCommand {
    line: String,
    /// The file or folder under review, when the review opened one.
    path: Option<PathBuf>,
    timeout: Duration,
    /// The first line the last successful run printed.
    notice: RefCell<Option<String>>,
}

impl DeliverCommand {
    /// The command [`COMMAND_VAR`] names, else `configured`; `None` when neither is set or both
    /// are blank.
    pub(crate) fn from_lookup(
        env: impl Fn(&str) -> Option<String>,
        configured: Option<&str>,
        path: Option<&Path>,
    ) -> Option<Self> {
        let blank = |line: &String| line.trim().is_empty();
        let line = env(COMMAND_VAR)
            .filter(|line| !blank(line))
            .or_else(|| configured.map(str::to_owned).filter(|line| !blank(line)))?;
        Some(Self { line, path: path.map(Path::to_path_buf), timeout: TIMEOUT, notice: RefCell::default() })
    }

    /// The command line, as written.
    pub(crate) fn line(&self) -> &str {
        &self.line
    }

    /// The program the line starts with, by file name: what the footer calls the target.
    fn program(&self) -> &str {
        let word = self.line.split_whitespace().find(|word| !word.contains('=')).unwrap_or("command");
        let word = word.trim_matches(|c| c == '"' || c == '\'');
        word.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()).unwrap_or(word)
    }

    fn run(&self, feedback: &str) -> Result<(ExitStatus, String, String), DeliveryError> {
        let mut command = shell(&self.line);
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(path) = &self.path {
            command.env(PATH_VAR, path);
        }
        let mut child = command.spawn().map_err(|err| {
            DeliveryError::Unavailable(format!("cannot run the delivery command {}: {err}", self.program()))
        })?;
        // Write and read on threads: a command that prints before it reads, or never reads,
        // must not leave both sides waiting on a full pipe.
        if let Some(mut stdin) = child.stdin.take() {
            let text = feedback.to_owned();
            std::thread::spawn(move || stdin.write_all(text.as_bytes()));
        }
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let status = wait(&mut child, self.timeout).ok_or_else(|| {
            DeliveryError::Failed(anyhow::anyhow!(
                "{} did not finish within {} s and was stopped",
                self.program(),
                self.timeout.as_secs_f32()
            ))
        })?;
        Ok((status, collect(&stdout), collect(&stderr)))
    }
}

/// The shell that runs `line`.
#[cfg(not(windows))]
fn shell(line: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(line);
    command
}

/// The shell that runs `line`. `/S` makes cmd strip exactly the quotes added here, so the
/// line reaches it as written.
#[cfg(windows)]
fn shell(line: &str) -> Command {
    use std::os::windows::process::CommandExt as _;
    let mut command = Command::new("cmd");
    command.args(["/S", "/C"]).raw_arg(format!("\"{line}\""));
    command
}

/// What `pipe` yields, in chunks on a channel as they arrive, read on its own thread.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        std::thread::spawn(move || {
            let mut chunk = [0; 8192];
            while let Ok(n @ 1..) = pipe.read(&mut chunk) {
                if tx.send(chunk.get(..n).unwrap_or_default().to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    rx
}

/// The output on `rx` up to its end, or up to a [`DRAIN`] without more, since a child the
/// command left running can hold the pipe open long after the command exits.
fn collect(rx: &mpsc::Receiver<Vec<u8>>) -> String {
    let mut bytes = Vec::new();
    while let Ok(chunk) = rx.recv_timeout(DRAIN) {
        bytes.extend(chunk);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The child's exit status, or `None` once `timeout` passes, after stopping it.
fn wait(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The first non-blank line of `text`, trimmed.
fn first_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|line| !line.is_empty()).map(str::to_owned)
}

/// What one run of `program` means, from its exit code (`None`: killed by a signal) and output.
/// Success yields the line to show beside the status.
fn outcome(
    program: &str,
    code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<Option<String>, DeliveryError> {
    let said = || first_line(stderr).or_else(|| first_line(stdout));
    match code {
        Some(0) => Ok(first_line(stdout)),
        Some(EXIT_BLOCKED) => {
            Err(DeliveryError::Blocked(said().unwrap_or_else(|| format!("{program} says the agent is busy"))))
        }
        Some(EXIT_UNAVAILABLE) => {
            Err(DeliveryError::Unavailable(said().unwrap_or_else(|| format!("{program} found no agent"))))
        }
        Some(code) => Err(DeliveryError::Failed(match first_line(stderr) {
            Some(line) => anyhow::anyhow!("{program} exited with {code}: {line}"),
            None => anyhow::anyhow!("{program} exited with {code}"),
        })),
        None => Err(DeliveryError::Failed(anyhow::anyhow!("{program} was stopped by a signal"))),
    }
}

impl Delivery for DeliverCommand {
    fn describe(&self) -> String {
        format!("cmd · {}", self.program())
    }

    fn is_agent(&self) -> bool {
        true
    }

    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError> {
        self.notice.take();
        let (status, stdout, stderr) = self.run(feedback)?;
        let notice = outcome(self.program(), status.code(), &stdout, &stderr)?;
        *self.notice.borrow_mut() = notice;
        Ok(())
    }

    fn notice(&self) -> Option<String> {
        self.notice.take()
    }
}

#[cfg(test)]
mod tests;
