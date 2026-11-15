//! Notes on decorated lists still anchor to their exact source and carry the text a reader
//! sees, without bullets or boxes, through the real binary and a private data dir.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::Path;
use std::process::Command;

/// `--annotate` opens documents 100 columns wide, so this item wraps with a hanging indent.
const DOC: &str = "# Plan\n\n- ship the login page\n- [ ] write the migration guide\n  - nested \
detail that runs long enough to wrap past one hundred columns of text, so its second row hangs \
under the item text here\n";

/// Annotate `quote` and return the stored anchor.
fn anchor(root: &Path, quote: &str) -> serde_json::Value {
    let doc = root.join("doc.md");
    let data = root.join("data");
    let out = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .env("PLANNOTATOR_DATA_DIR", &data)
        .args(["--annotate", &doc.display().to_string(), quote, "note"])
        .output()
        .expect("annotate runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let file = walk(&data).into_iter().find(|p| p.ends_with("annotations.json")).expect("annotations saved");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).expect("read")).expect("json");
    let annotations = saved["annotations"].as_array().expect("list");
    annotations.last().expect("one annotation")["anchor"].clone()
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    entries.flat_map(|e| if e.path().is_dir() { walk(&e.path()) } else { vec![e.path()] }).collect()
}

#[test]
fn quotes_across_bullets_wraps_and_task_boxes_anchor_to_their_source() {
    let root = std::env::temp_dir().join(format!("plannotator-tui-decorated-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("dirs");
    std::fs::write(root.join("doc.md"), DOC).expect("doc");

    for (quote, rendered) in [
        ("login page\n- [ ] write", "login page write"),
        ("wrap past one hundred columns of text, so its second row hangs under", ""),
        ("write the migration guide", ""),
    ] {
        let anchor = anchor(&root, quote);
        let expected = if rendered.is_empty() { quote } else { rendered };
        assert_eq!(anchor["originalText"], expected, "{quote:?}");
        let source = &anchor["plannotator_tui"]["source"];
        let (start, end) = (source["start"].as_u64().expect("start"), source["end"].as_u64().expect("end"));
        let range = usize::try_from(start).expect("fits")..usize::try_from(end).expect("fits");
        assert_eq!(&DOC[range], quote);
    }
    std::fs::remove_dir_all(&root).expect("cleanup");
}
