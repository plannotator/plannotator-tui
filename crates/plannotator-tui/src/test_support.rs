//! Fixtures shared by tests across the crate.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `script` to `path` and make it executable, for a test to run as a fake command.
///
/// A child process writes the file. Had this process written it, a fork on another test
/// thread in that moment would inherit the open file, and running the script would fail
/// with "Text file busy" until that child execs.
#[allow(clippy::expect_used, reason = "test fixtures assert by panicking")]
pub(crate) fn write_script(path: &Path, script: &str) {
    let mut sh = Command::new("/bin/sh")
        .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("run sh");
    sh.stdin.take().expect("sh stdin").write_all(script.as_bytes()).expect("write script");
    assert!(sh.wait().expect("wait for sh").success(), "cannot write {}", path.display());
}
