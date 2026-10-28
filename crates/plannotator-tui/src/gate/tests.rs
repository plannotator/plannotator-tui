#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use super::*;

fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn approved(feedback: &str, count: usize) -> Decision {
    Decision::Approved { feedback: feedback.to_owned(), count }
}

fn annotated(feedback: &str, count: usize) -> Decision {
    Decision::Annotated { feedback: feedback.to_owned(), count }
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("plannotator-gate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

#[test]
fn the_strict_record_is_the_browsers_byte_for_byte() {
    assert_eq!(approved("", 0).strict_json(), r#"{"decision":"approved"}"#);
    assert_eq!(
        approved("# Annotations on plan.md\n", 1).strict_json(),
        r##"{"decision":"approved","feedback":"# Annotations on plan.md\n"}"##
    );
    assert_eq!(annotated("notes", 2).strict_json(), r#"{"decision":"annotated","feedback":"notes"}"#);
    assert_eq!(annotated("", 0).strict_json(), r#"{"decision":"annotated","feedback":""}"#);
    assert_eq!(Decision::Dismissed.strict_json(), r#"{"decision":"dismissed"}"#);
}

#[test]
fn plain_json_adds_the_count_and_target_the_browser_adds() {
    let target = Path::new("/work/plan.md");
    assert_eq!(
        approved("", 0).json(target),
        r#"{"decision":"approved","annotationCount":0,"target":"/work/plan.md"}"#
    );
    assert_eq!(
        approved("n", 1).json(target),
        r#"{"decision":"approved","feedback":"n","annotationCount":1,"target":"/work/plan.md"}"#
    );
    assert_eq!(
        annotated("n", 3).json(target),
        r#"{"decision":"annotated","feedback":"n","annotationCount":3,"target":"/work/plan.md"}"#
    );
    assert_eq!(Decision::Dismissed.json(target), r#"{"decision":"dismissed","target":"/work/plan.md"}"#);
}

#[test]
fn plaintext_is_the_approval_marker_the_feedback_or_nothing() {
    let context = "File: /work/plan.md";
    assert_eq!(approved("", 0).plain(context).as_deref(), Some("The user approved."));
    assert_eq!(annotated("# Annotations\n\n", 1).plain(context).as_deref(), Some("# Annotations"));
    assert_eq!(annotated("", 0).plain(context), None);
    assert_eq!(Decision::Dismissed.plain(context), None);
}

#[test]
fn an_approval_with_notes_uses_the_browsers_framing_and_names_the_target() {
    let message = approval_message("# Annotations on plan.md\n\n## Annotation 1\n\n", "File: /work/plan.md");
    assert_eq!(
        message,
        "# Approved with Notes\n\nThe artifact is approved. The notes below are non-blocking guidance, \
         not a request for another revision.\n\nFile: /work/plan.md\n\n# Annotations on plan.md\n\n\
         ## Annotation 1\n\nDo not revise or reopen the artifact solely because of these notes unless \
         the user explicitly requests it. Carry the notes into subsequent work where applicable."
    );
    assert!(approval_message("n", "").contains("revision.\n\nn\n\nDo not"), "no context, no blank block");
    assert_eq!(approval_message("  \n", "File: x"), APPROVED);
    assert_eq!(context_line(Path::new("/work/docs"), true), "Folder: /work/docs");
}

#[test]
fn only_an_approval_succeeds_under_require_approval() {
    assert_eq!(approved("", 0).exit_code(true), 0);
    assert_eq!(approved("n", 1).exit_code(true), 0);
    assert_eq!(annotated("n", 1).exit_code(true), 1);
    assert_eq!(Decision::Dismissed.exit_code(true), 1);
    for decision in [approved("", 0), annotated("n", 1), Decision::Dismissed] {
        assert_eq!(decision.exit_code(false), 0, "{decision:?}");
    }
}

#[test]
fn the_archive_records_the_browsers_decision_words() {
    assert_eq!(approved("", 0).archive_decision(), "approved");
    assert_eq!(approved("n", 1).archive_decision(), "approved-with-notes");
    assert_eq!(annotated("n", 1).archive_decision(), "feedback");
    assert_eq!(Decision::Dismissed.archive_decision(), "dismissed");
}

#[test]
fn flags_parse_in_any_order_around_the_path() {
    let parsed = ReviewArgs::parse(&args("--gate plan.md --json --require-approval --result-file out.json"))
        .expect("parses");
    assert_eq!(
        parsed,
        ReviewArgs {
            path: PathBuf::from("plan.md"),
            gate: true,
            json: true,
            require_approval: true,
            result_file: Some(PathBuf::from("out.json")),
        }
    );
    assert!(parsed.strict());
    assert_eq!(parsed.mode(), Gate::Approve);
    assert_eq!(ReviewArgs::parse(&args("plan.md --json")).expect("json alone").mode(), Gate::Decision);
}

#[test]
fn strict_flags_need_gate_and_json() {
    for bad in ["p.md --require-approval", "p.md --gate --require-approval", "p.md --json --result-file r"] {
        let err = ReviewArgs::parse(&args(bad)).expect_err(bad);
        assert!(err.to_string().contains("require --gate --json"), "{bad}: {err}");
    }
}

#[test]
fn malformed_invocations_are_rejected_with_a_reason() {
    let cases = [
        ("p.md --gate --gate", "only be specified once"),
        ("p.md --gate --json --result-file", "missing value"),
        ("p.md --gate --json --result-file --require-approval", "missing value"),
        ("p.md --gate --json --result-file a --result-file b", "only be specified once"),
        ("p.md --gat", "unknown flag --gat"),
        ("p.md q.md --gate", "unexpected argument"),
        ("--gate --json", "missing the file or folder"),
    ];
    for (input, expected) in cases {
        let err = ReviewArgs::parse(&args(input)).expect_err(input);
        assert!(err.to_string().contains(expected), "{input}: {err}");
    }
}

#[test]
fn a_strict_invocation_is_recognised_before_parsing() {
    assert!(is_strict(&args("p.md --gat --require-approval")));
    assert!(is_strict(&args("--result-file r p.md")));
    assert!(!is_strict(&args("p.md --gate --json")));
}

#[test]
fn a_result_file_is_published_once_and_never_overwritten() {
    let d = dir("publish");
    let path = d.join("decision.json");
    check_result_file(&path).expect("free name");
    publish(&path, r#"{"decision":"approved"}"#).expect("published");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "{\"decision\":\"approved\"}\n");

    let err = check_result_file(&path).expect_err("taken");
    assert!(err.to_string().contains("already exists"), "{err}");
    publish(&path, r#"{"decision":"dismissed"}"#).expect_err("no clobber");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "{\"decision\":\"approved\"}\n");

    let names: Vec<_> = std::fs::read_dir(&d).expect("list").map(|e| e.expect("entry").file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("decision.json")], "no temporary file left behind");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_result_file_needs_an_existing_folder() {
    let d = dir("parent");
    let err = check_result_file(&d.join("missing/decision.json")).expect_err("no folder");
    assert!(err.to_string().contains("folder does not exist"), "{err}");
    publish(&d.join("missing/decision.json"), "{}").expect_err("cannot publish");
    let _ = std::fs::remove_dir_all(d);
}

#[cfg(unix)]
#[test]
fn a_dangling_link_counts_as_taken() {
    let d = dir("dangling");
    let path = d.join("decision.json");
    std::os::unix::fs::symlink(d.join("nowhere"), &path).expect("symlink");
    check_result_file(&path).expect_err("a link occupies the name");
    let _ = std::fs::remove_dir_all(d);
}
