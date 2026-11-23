//! `.gitattributes` `linguist-generated`, read as Plannotator reads it
//! (`packages/shared/generated-files.ts`): one `git check-attr --stdin -z` for every path,
//! whose answer wins both ways. `linguist-generated` or `=true` marks a file generated,
//! `-linguist-generated` or `=false` unmarks one the name rules would mark, and anything
//! else leaves the name rules. It runs on its own thread, so the list shows at once by name
//! and the kinds follow when git answers; any failure leaves the name rules standing.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

const ATTRIBUTE: &str = "linguist-generated";
/// Git gets this long, as in Plannotator; after it, the name rules stand.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Ask git about `paths` in `root` on another thread; the answer comes on the receiver,
/// then `wake` wakes the event loop.
pub(super) fn start(
    root: &Path,
    paths: Vec<String>,
    wake: SyncSender<()>,
) -> Receiver<HashMap<String, bool>> {
    let (answer, receiver) = mpsc::channel();
    let root = root.to_path_buf();
    let spawned = std::thread::Builder::new().name("check-attr".into()).spawn(move || {
        let states = check_attr(&root, &paths).unwrap_or_default();
        if answer.send(states).is_ok() {
            let _ = wake.try_send(());
        }
    });
    drop(spawned);
    receiver
}

/// Each path git has an opinion on: `true` generated, `false` not.
fn check_attr(root: &PathBuf, paths: &[String]) -> Option<HashMap<String, bool>> {
    if paths.is_empty() {
        return None;
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-attr", "--stdin", "-z", ATTRIBUTE])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut input = paths.join("\0");
    input.push('\0');
    let mut stdin = child.stdin.take()?;
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).map(|_| output)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let _ = writer.join();
    let output = reader.join().ok()?.ok()?;
    status.success().then(|| parse(&String::from_utf8_lossy(&output)))
}

/// `git check-attr -z` output: path, attribute, value, each ended by a NUL.
fn parse(output: &str) -> HashMap<String, bool> {
    let fields: Vec<&str> = output.split('\0').collect();
    fields
        .chunks_exact(3)
        .filter_map(|triple| match triple {
            [path, attribute, value] if *attribute == ATTRIBUTE => match *value {
                "set" | "true" => Some(((*path).to_owned(), true)),
                "unset" | "false" => Some(((*path).to_owned(), false)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_true_mark_unset_and_false_unmark_and_the_rest_says_nothing() {
        let output = "a.js\0linguist-generated\0set\0b.js\0linguist-generated\0true\0\
                      c.lock\0linguist-generated\0unset\0d.lock\0linguist-generated\0false\0\
                      e.js\0linguist-generated\0unspecified\0";
        let states = parse(output);
        let expected: HashMap<String, bool> =
            [("a.js", true), ("b.js", true), ("c.lock", false), ("d.lock", false)]
                .into_iter()
                .map(|(p, s)| (p.to_owned(), s))
                .collect();
        assert_eq!(states, expected);
    }
}
