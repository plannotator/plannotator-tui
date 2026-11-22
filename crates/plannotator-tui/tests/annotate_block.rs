//! `--annotate-block` takes the same optional type as `--annotate`, and both refuse a type
//! they do not know, through the real binary and a private data dir.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
}

fn annotate_block(data: &Path, doc: &Path, block: &str, text: &str, extra: &[&str]) -> Output {
    let mut args =
        vec!["--annotate-block".to_owned(), doc.display().to_string(), block.to_owned(), text.to_owned()];
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    bin().env("PLANNOTATOR_DATA_DIR", data).args(&args).output().expect("annotate-block runs")
}

fn export(data: &Path, doc: &Path) -> String {
    let out = bin()
        .env("PLANNOTATOR_DATA_DIR", data)
        .args(["--export", &doc.display().to_string()])
        .output()
        .expect("export runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn scratch(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("plannotator-tui-block-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("dirs");
    let doc = root.join("doc.md");
    std::fs::write(&doc, "# Draft\n\nKeep this part.\n\nShip this part.\n\nDrop this part.\n").expect("doc");
    (root.clone(), doc, root.join("data"))
}

#[test]
fn the_type_names_the_kind_of_note_on_the_block() {
    let (root, doc, data) = scratch("types");
    // As with `--annotate`, the text is the note's body and may be empty for any type.
    for (block, text, kind) in
        [("1", "why keep it?", "comment"), ("2", "", "looks_good"), ("3", "", "delete")]
    {
        let out = annotate_block(&data, &doc, block, text, &[kind]);
        assert!(out.status.success(), "{kind}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let text = export(&data, &doc);
    assert!(
        text.contains("## Annotation 1 (line 3)\nComment on: \"Keep this part.\"\n> why keep it?"),
        "{text}"
    );
    assert!(text.contains("## Annotation 2 (line 5)\nLooks good: \"Ship this part.\"\n\n"), "{text}");
    assert!(text.contains("## Annotation 3 (line 7)\nRemove this:\n"), "{text}");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn without_a_type_the_block_gets_a_comment() {
    let (root, doc, data) = scratch("default");
    let out = annotate_block(&data, &doc, "1", "note", &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = export(&data, &doc);
    assert!(text.contains("## Annotation 1 (line 3)\nComment on: \"Keep this part.\"\n> note"), "{text}");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn what_annotate_refuses_after_the_type_is_refused_here_and_nothing_is_saved() {
    let (root, doc, data) = scratch("refused");
    for (extra, wording) in [
        (&["--delete"][..], "unknown flag --delete"),
        (&["delete", "comment"][..], "unexpected argument \"comment\""),
        // A block is one place: there is no occurrence to pick.
        (&["--occurrence", "2"][..], "unknown flag --occurrence"),
    ] {
        let out = annotate_block(&data, &doc, "1", "note", extra);
        assert!(!out.status.success(), "{extra:?} was accepted");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(wording) && err.contains("usage:"), "{extra:?}: {err}");
    }
    assert_eq!(export(&data, &doc), "No annotations.");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

/// `--annotate` on block 1's text and `--annotate-block` on block 1, with the same trailing words.
fn both_commands(data: &Path, doc: &Path, extra: &[&str]) -> [Output; 2] {
    let quote = bin()
        .env("PLANNOTATOR_DATA_DIR", data)
        .args(["--annotate", &doc.display().to_string(), "Keep this part.", "note"])
        .args(extra)
        .output()
        .expect("annotate runs");
    [quote, annotate_block(data, doc, "1", "note", extra)]
}

#[test]
fn an_unknown_type_fails_names_the_accepted_words_and_saves_nothing() {
    let (root, doc, data) = scratch("unknown");
    for word in ["bogus", "looks-good", "Comment", ""] {
        for out in both_commands(&data, &doc, &[word]) {
            assert!(!out.status.success(), "{word:?} was accepted");
            let err = String::from_utf8_lossy(&out.stderr);
            let wording = format!("unknown type {word:?}: use comment, looks_good or delete");
            assert!(err.contains(&wording) && err.contains("usage:"), "{word:?}: {err}");
        }
    }
    assert_eq!(export(&data, &doc), "No annotations.");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn approve_still_means_looks_good() {
    let (root, doc, data) = scratch("approve");
    for out in both_commands(&data, &doc, &["approve"]) {
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    let text = export(&data, &doc);
    assert_eq!(text.matches("Looks good: \"Keep this part.\"").count(), 2, "{text}");
    std::fs::remove_dir_all(&root).expect("cleanup");
}
