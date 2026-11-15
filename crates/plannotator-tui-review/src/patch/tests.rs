//! The fixture is `git diff --cached` of a real repository (an addition, a pure rename, a
//! binary, two hunks with function context, a deletion, a path git quotes, and a path with
//! a space that git ends with a tab). Expected files and anchors are what Plannotator's
//! `parseDiffToFiles` and `captureAnchor` return for the same patch.
#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use super::*;

const MIXED: &str = concat!(
    "diff --git a/added.txt b/added.txt\n",
    "new file mode 100644\n",
    "index 0000000..5786b13\n",
    "--- /dev/null\n",
    "+++ b/added.txt\n",
    "@@ -0,0 +1,2 @@\n",
    "+brand\n",
    "+new\n",
    "diff --git a/before.txt b/after.txt\n",
    "similarity index 100%\n",
    "rename from before.txt\n",
    "rename to after.txt\n",
    "diff --git a/bin.dat b/bin.dat\n",
    "index 8352675..a903574 100644\n",
    "Binary files a/bin.dat and b/bin.dat differ\n",
    "diff --git a/code.rs b/code.rs\n",
    "index 1297e05..f0ec433 100644\n",
    "--- a/code.rs\n",
    "+++ b/code.rs\n",
    "@@ -1,6 +1,6 @@\n",
    " fn alpha() {\n",
    "     one();\n",
    "-    two();\n",
    "+    TWO();\n",
    "     three();\n",
    " }\n",
    " \n",
    "@@ -12,6 +12,6 @@ fn beta() {\n",
    "     eight();\n",
    "     nine();\n",
    "     ten();\n",
    "-    eleven();\n",
    "+    ELEVEN();\n",
    "     twelve();\n",
    " }\n",
    "diff --git a/old.txt b/old.txt\n",
    "deleted file mode 100644\n",
    "index d1f10a2..0000000\n",
    "--- a/old.txt\n",
    "+++ /dev/null\n",
    "@@ -1,2 +0,0 @@\n",
    "-gone\n",
    "-file\n",
    "diff --git \"a/q\\\"t\\\\\\\\b.txt\" \"b/q\\\"t\\\\\\\\b.txt\"\n",
    "new file mode 100644\n",
    "index 0000000..7898192\n",
    "--- /dev/null\n",
    "+++ \"b/q\\\"t\\\\\\\\b.txt\"\n",
    "@@ -0,0 +1 @@\n",
    "+a\n",
    "diff --git a/sp ace é.txt b/sp ace é.txt\n",
    "index 587be6b..975fbec 100644\n",
    "--- a/sp ace é.txt\t\n",
    "+++ b/sp ace é.txt\t\n",
    "@@ -1 +1 @@\n",
    "-x\n",
    "+y\n",
);

fn files() -> Vec<FilePatch> {
    parse(MIXED)
}

fn file<'a>(files: &'a [FilePatch], path: &str) -> &'a FilePatch {
    files.iter().find(|f| f.path == path).expect("file in the patch")
}

fn context(before: [Option<&str>; 2], after: [Option<&str>; 2], hunk: Option<&str>) -> AnchorContext {
    let own = |lines: [Option<&str>; 2]| lines.iter().map(|l| l.map(str::to_owned)).collect();
    AnchorContext { before: own(before), after: own(after), hunk: hunk.map(str::to_owned) }
}

#[test]
fn files_paths_statuses_and_counts_match_the_browser() {
    let summary: Vec<(String, Option<String>, FileStatus, u32, u32)> =
        files().into_iter().map(|f| (f.path, f.old_path, f.status, f.additions, f.deletions)).collect();
    let row = |p: &str, old: Option<&str>, s, a, d| (p.to_owned(), old.map(str::to_owned), s, a, d);
    assert_eq!(
        summary,
        [
            row("added.txt", None, FileStatus::Added, 2, 0),
            row("after.txt", Some("before.txt"), FileStatus::Renamed, 0, 0),
            row("bin.dat", None, FileStatus::Modified, 0, 0),
            row("code.rs", None, FileStatus::Modified, 2, 2),
            row("old.txt", None, FileStatus::Deleted, 0, 2),
            row("q\"t\\\\b.txt", None, FileStatus::Added, 1, 0),
            row("sp ace é.txt", None, FileStatus::Modified, 1, 1),
        ]
    );
    assert!(file(&files(), "bin.dat").binary);
    assert!(!file(&files(), "code.rs").binary);
}

#[test]
fn anchors_match_the_browsers_capture() {
    let files = files();
    let code = file(&files, "code.rs");
    let anchor = |f: &FilePatch, side, a, b| f.anchor(side, a, b).expect("lines in the patch");
    assert_eq!(
        anchor(code, Side::New, 3, 3),
        (
            "    TWO();".into(),
            context([Some("fn alpha() {"), Some("    one();")], [Some("    three();"), Some("}")], None)
        )
    );
    assert_eq!(anchor(code, Side::Old, 3, 3).0, "    two();");
    assert_eq!(
        anchor(code, Side::New, 1, 2),
        (
            "fn alpha() {\n    one();".into(),
            context([None, None], [Some("    TWO();"), Some("    three();")], None)
        )
    );
    assert_eq!(
        anchor(code, Side::New, 15, 16),
        (
            "    ELEVEN();\n    twelve();".into(),
            context([Some("    nine();"), Some("    ten();")], [Some("}"), None], Some("fn beta() {"))
        )
    );
    assert_eq!(
        anchor(code, Side::New, 12, 12),
        (
            "    eight();".into(),
            context([None, None], [Some("    nine();"), Some("    ten();")], Some("fn beta() {"))
        )
    );
    assert_eq!(code.anchor(Side::New, 8, 8), None, "line 8 is between the hunks");
    assert_eq!(
        anchor(file(&files, "added.txt"), Side::New, 1, 2),
        ("brand\nnew".into(), context([None, None], [None, None], None))
    );
    assert_eq!(
        anchor(file(&files, "old.txt"), Side::Old, 2, 2),
        ("file".into(), context([None, Some("gone")], [None, None], None))
    );
}

#[test]
fn shows_requires_every_line_of_the_range() {
    let files = files();
    let code = file(&files, "code.rs");
    assert!(code.shows(Side::New, 1, 6));
    assert!(!code.shows(Side::New, 5, 13), "lines 7 to 11 are not in a hunk");
    assert!(!code.shows(Side::New, 0, 1));
    assert!(!file(&files, "old.txt").shows(Side::New, 1, 1), "a deleted file has no new side");
    assert_eq!(code.hunk_context(Side::Old, 14), Some("fn beta() {"));
    assert_eq!(code.hunk_context(Side::Old, 1), Some(""));
}

#[test]
fn quoted_paths_decode_octal_utf8_and_c_escapes() {
    assert_eq!(unquote("\"caf\\303\\251.txt\""), "café.txt");
    assert_eq!(unquote("\"a\\tb\\\"c\\\\d\""), "a\tb\"c\\d");
    assert_eq!(unquote("plain name"), "plain name");
    assert_eq!(unquote("\"odd\\qescape\""), "odd\\qescape", "an unknown escape keeps its backslash");
}

#[test]
fn header_paths_are_read_when_there_are_no_file_lines() {
    assert_eq!(git_header_paths("diff --git a/x b/y b/z"), (Some("x b/y".into()), Some("z".into())));
    assert_eq!(
        git_header_paths("diff --git \"a/sp ace\" \"b/sp ace\""),
        (Some("sp ace".into()), Some("sp ace".into()))
    );
    assert_eq!(git_header_paths("not a header"), (None, None));
}

#[test]
fn an_empty_patch_has_no_files() {
    assert_eq!(parse(""), []);
}
