//! The Quick Open scan: a cancellable walk of the whole folder on its own thread, with the
//! tree's rules (`tree::is_walkable_dir`): dependency/build directories and symlinked
//! directories are never entered, so a symlink loop cannot trap it. A bounded channel
//! limits how many unread batches pile up.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use anyhow::{Context, Result};

use crate::tree::{is_hidden, is_markdown, is_walkable_dir};

pub(crate) enum Update {
    Files(Vec<PathBuf>),
    Error(String),
    Done,
}

pub(crate) struct Scan {
    pub(crate) updates: Receiver<Update>,
    cancelled: Arc<AtomicBool>,
}

impl Scan {
    pub(crate) fn start(root: PathBuf, show_hidden: bool) -> Result<Self> {
        let (sender, updates) = sync_channel(64);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::Builder::new()
            .name("file-search".into())
            .spawn(move || walk(root, show_hidden, &stop, &sender))
            .context("starting filename search")?;
        Ok(Self { updates, cancelled })
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        // Dropping the receiver also releases a worker blocked on a full channel.
    }
}

fn walk(root: PathBuf, show_hidden: bool, stop: &AtomicBool, sender: &SyncSender<Update>) {
    let mut directories = vec![root];
    let mut batch = Vec::new();
    while let Some(dir) = directories.pop() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                if sender.send(Update::Error(format!("reading {}: {error}", dir.display()))).is_err() {
                    return;
                }
                continue;
            }
        };
        for entry in entries {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    if sender.send(Update::Error(format!("reading {}: {error}", dir.display()))).is_err() {
                        return;
                    }
                    continue;
                }
            };
            if !show_hidden && is_hidden(&path) {
                continue;
            }
            if is_walkable_dir(&path) {
                directories.push(path);
            } else if path.is_file() && is_markdown(&path) {
                batch.push(path);
                if batch.len() == 64 && sender.send(Update::Files(std::mem::take(&mut batch))).is_err() {
                    return;
                }
            }
        }
        if !batch.is_empty() && sender.send(Update::Files(std::mem::take(&mut batch))).is_err() {
            return;
        }
    }
    let _ = sender.send(Update::Done);
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn discovery_descends_but_respects_hidden_skipped_and_symlink_directories() {
        let root = std::env::temp_dir().join(format!("plannotator-search-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["docs/deep", ".agents", "node_modules", ".git"] {
            std::fs::create_dir_all(root.join(dir)).expect("mkdir");
            std::fs::write(root.join(dir).join("plan.MDX"), "").expect("write");
        }
        std::fs::write(root.join("notes.txt"), "").expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("loop")).expect("symlink");
        for hidden in [false, true] {
            let scan = Scan::start(root.clone(), hidden).expect("scan");
            let mut paths = Vec::new();
            loop {
                match scan.updates.recv_timeout(Duration::from_secs(5)).expect("update") {
                    Update::Files(batch) => paths.extend(batch),
                    Update::Error(error) => panic!("{error}"),
                    Update::Done => break,
                }
            }
            paths.sort();
            let mut expected = vec![root.join("docs/deep/plan.MDX")];
            if hidden {
                expected.push(root.join(".agents/plan.MDX"));
            }
            expected.sort();
            assert_eq!(paths, expected);
        }
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
