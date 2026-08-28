//! Folder mode covers every annotated file, through the real binary and a private data dir.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
}

#[test]
fn folder_export_includes_every_annotated_file() {
    let root = std::env::temp_dir().join(format!("plannotator-tui-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).expect("dirs");
    std::fs::write(docs.join("a.md"), "# A\n\nalpha text\n").expect("a");
    std::fs::write(docs.join("b.md"), "# B\n\nbeta text\n").expect("b");
    let data: PathBuf = root.join("data");

    for (file, quote) in [("a.md", "alpha"), ("b.md", "beta")] {
        let status = bin()
            .env("PLANNOTATOR_DATA_DIR", &data)
            .args(["--annotate", &docs.join(file).display().to_string(), quote, "note", "comment"])
            .status()
            .expect("annotate runs");
        assert!(status.success(), "annotating {file}");
    }
    let out = bin()
        .env("PLANNOTATOR_DATA_DIR", &data)
        .args(["--export", &docs.display().to_string()])
        .output()
        .expect("export runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("## File: a.md"), "{text}");
    assert!(text.contains("## File: b.md"), "{text}");
    assert!(text.contains("\"alpha\"") && text.contains("\"beta\""), "{text}");
    std::fs::remove_dir_all(&root).expect("cleanup");
}
