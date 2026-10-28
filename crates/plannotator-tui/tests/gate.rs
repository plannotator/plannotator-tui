//! Gate flags through the real binary: a strict gate's own failures exit 2, never 1, and
//! leave an existing result file alone. Every case fails before the review would draw, so
//! none needs a terminal; stdout is a pipe here, which a gated review refuses.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};
use std::process::Output;

fn run(root: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_plannotator-tui"))
        .current_dir(root)
        .env("PLANNOTATOR_DATA_DIR", root.join("data"))
        .env_remove("HERDR_ENV")
        .args(args)
        .output()
        .expect("runs")
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("plannotator-tui-gate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("dirs");
    std::fs::write(root.join("plan.md"), "# Plan\n\nShip it.\n").expect("doc");
    root
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn strict_failures_exit_2_and_ordinary_ones_exit_1() {
    let root = scratch("codes");
    let cases: [(&[&str], i32, &str); 8] = [
        (&["plan.md", "--require-approval"], 2, "require --gate --json"),
        (&["plan.md", "--gate", "--result-file", "r.json"], 2, "require --gate --json"),
        (&["plan.md", "--gat", "--require-approval"], 2, "unknown flag --gat"),
        (&["missing.md", "--gate", "--json", "--require-approval"], 2, "no such file or folder"),
        (&["missing.md", "--gate", "--json"], 1, "no such file or folder"),
        (&["plan.md", "--gate", "--json", "--require-approval"], 2, "stdout is not a terminal"),
        (&["plan.md", "--gate"], 1, "stdout is not a terminal"),
        (&["plan.md", "--gate", "--json", "--result-file", "nowhere/r.json"], 2, "folder does not exist"),
    ];
    for (args, code, message) in cases {
        let out = run(&root, args);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains(message), "{args:?}: {}", stderr(&out));
        assert!(out.stdout.is_empty(), "{args:?}: no decision was made");
    }
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn an_existing_result_file_is_refused_and_left_as_it_was() {
    let root = scratch("existing");
    std::fs::write(root.join("r.json"), "keep me\n").expect("existing");
    let out = run(&root, &["plan.md", "--gate", "--json", "--result-file", "r.json"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));
    assert_eq!(std::fs::read_to_string(root.join("r.json")).expect("read"), "keep me\n");
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn herdr_gate_flags_belong_to_open() {
    let root = scratch("herdr");
    let out = run(&root, &["herdr", "open", "plan.md", "--result-file", "r.json"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("--result-file requires --gate"), "{}", stderr(&out));
    let out = run(&root, &["herdr", "last", "--gate"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("only for `herdr open`"), "{}", stderr(&out));
    std::fs::write(root.join("r.json"), "").expect("existing");
    let out = run(&root, &["herdr", "open", "plan.md", "--gate", "--result-file", "r.json"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn headless_commands_ignore_what_looks_like_a_gate_flag() {
    // `--json` as an annotation body is text, not a flag: tools keep their own arguments.
    let root = scratch("headless");
    let out = run(&root, &["--annotate", "plan.md", "Ship it.", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let out = run(&root, &["--export", "plan.md"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("> --json"), "{}", stderr(&out));
    std::fs::remove_dir_all(&root).expect("cleanup");
}
