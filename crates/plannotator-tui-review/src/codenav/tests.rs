//! Pure tests. The `classify`, `rank`, `is_path_allowed` and path-exclusion cases are ported
//! one for one from the browser's `packages/shared/code-nav.test.ts` (test names kept in the
//! comments); the rest cover what this port adds.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "tests assert by panicking")]

use super::*;

const TS: Option<Language> = Some(Language::TypeScript);
const PY: Option<Language> = Some(Language::Python);
const GO: Option<Language> = Some(Language::Go);
const RS: Option<Language> = Some(Language::Rust);

fn kind(line: &str, symbol: &str, language: Option<Language>) -> Kind {
    classify(line, symbol, language).0
}

// --- classifyMatch: TypeScript/JavaScript --------------------------------------------------

#[test]
fn classify_typescript_definitions() {
    for (line, symbol) in [
        ("function startServer(", "startServer"), // function declaration
        ("export async function startServer(", "startServer"), // async function declaration
        ("export function handleRequest(", "handleRequest"), // export function
        ("const startServer = async () => {", "startServer"), // const assignment
        ("let counter = 0;", "counter"),          // let assignment
        ("export class ReviewServer {", "ReviewServer"), // class declaration
        ("export interface CodeNavRequest {", "CodeNavRequest"), // interface declaration
        ("type DiffType = 'unified' | 'split';", "DiffType"), // type declaration
        ("enum Status {", "Status"),              // enum declaration
        ("  async handleRequest(", "handleRequest"), // method in class/object
        ("const runtime: CodeNavRuntime = {", "runtime"), // const with type annotation
    ] {
        assert_eq!(kind(line, symbol, TS), Kind::Definition, "{line}");
    }
}

#[test]
fn classify_typescript_references() {
    for (line, symbol) in [
        ("  const result = startServer(config);", "startServer"), // plain reference (function call)
        ("  startServer(config);", "startServer"),                // bare indented call is not a definition
        ("    return startServer(config);", "startServer"),       // indented call in if/return
        (r#"import { startServer } from "./server";"#, "startServer"), // plain reference (import)
    ] {
        assert_eq!(kind(line, symbol, TS), Kind::Reference, "{line}");
    }
}

// --- classifyMatch: Python, Go, Rust ----------------------------------------------------

#[test]
fn classify_python() {
    assert_eq!(kind("def handle_request(self, req):", "handle_request", PY), Kind::Definition);
    assert_eq!(kind("class ReviewServer:", "ReviewServer", PY), Kind::Definition);
    assert_eq!(kind("DEFAULT_PORT = 8080", "DEFAULT_PORT", PY), Kind::Definition);
    assert_eq!(kind("  server = ReviewServer()", "ReviewServer", PY), Kind::Reference);
    // Added: a method is a def too; an indented assignment is not a definition.
    assert_eq!(kind("    def run(self):", "run", PY), Kind::Definition);
    assert_eq!(kind("    PORT = 1", "PORT", PY), Kind::Reference);
}

#[test]
fn classify_go() {
    assert_eq!(kind("func StartServer(config Config) error {", "StartServer", GO), Kind::Definition);
    assert_eq!(kind("func (s *Server) StartServer() error {", "StartServer", GO), Kind::Definition);
    assert_eq!(kind("type Config struct {", "Config", GO), Kind::Definition);
    assert_eq!(kind("  err := StartServer(cfg)", "StartServer", GO), Kind::Reference);
    // Added: a package-level var.
    assert_eq!(
        classify("var DefaultClient = &Client{}", "DefaultClient", GO),
        (Kind::Definition, Some(SymbolKind::Variable))
    );
}

#[test]
fn classify_rust() {
    assert_eq!(kind("fn start_server() -> Result<()> {", "start_server", RS), Kind::Definition);
    assert_eq!(kind("pub fn start_server(config: Config) {", "start_server", RS), Kind::Definition);
    assert_eq!(kind("pub struct Config {", "Config", RS), Kind::Definition);
    assert_eq!(kind("pub enum Status {", "Status", RS), Kind::Definition);
    assert_eq!(kind("pub trait Handler {", "Handler", RS), Kind::Definition);
    assert_eq!(kind("  let server = start_server(config);", "start_server", RS), Kind::Reference);
    // Added: pub(crate), and a `fn` with the word only as a prefix is not it.
    assert_eq!(kind("pub(crate) fn helper() {}", "helper", RS), Kind::Definition);
    assert_eq!(kind("fn helper_two() {}", "helper", RS), Kind::Reference);
}

// --- classifyMatch: generic fallback and edge cases ------------------------------------

#[test]
fn classify_generic_fallback() {
    assert_eq!(kind("function startServer(", "startServer", None), Kind::Definition);
    assert_eq!(kind("class MyClass {", "MyClass", None), Kind::Definition);
    assert_eq!(kind("const PORT = 8080;", "PORT", None), Kind::Definition);
    assert_eq!(kind("  startServer(config);", "startServer", None), Kind::Reference);
    // Languages with no table of their own use the generic patterns (Java, Ruby, C).
    assert_eq!(kind("public class Gateway {", "Gateway", Some(Language::Java)), Kind::Definition);
    assert_eq!(kind("def charge(amount)", "charge", Some(Language::Ruby)), Kind::Definition);
    assert_eq!(kind("struct point {", "point", Some(Language::C)), Kind::Definition);
}

#[test]
fn classify_edge_cases() {
    // regex metacharacter in symbol ($)
    assert_eq!(kind("const $el = document.querySelector('div');", "$el", TS), Kind::Definition);
    // regex metacharacter in symbol (.)
    assert_eq!(kind("  obj.method();", "obj.method", None), Kind::Reference);
    // Added: the browser's patterns carry no `\b` before the keyword, so this matches there too.
    assert_eq!(kind("myfunction foo()", "foo", None), Kind::Definition);
    // Added: `\b` is ASCII as in JavaScript, so a non-ASCII letter after the word is a boundary.
    assert_eq!(kind("function fooé()", "foo", None), Kind::Definition);
}

// --- classifyMatchDetailed table -------------------------------------------------------

#[test]
fn classify_detailed_table() {
    let cases: [(&str, &str, Option<Language>, Option<SymbolKind>); 20] = [
        ("export function charge(amount) {", "charge", TS, Some(SymbolKind::Function)),
        ("const MAX_ATTEMPTS = 3;", "MAX_ATTEMPTS", TS, Some(SymbolKind::Const)),
        ("let attempts = 0;", "attempts", TS, Some(SymbolKind::Variable)),
        ("export class Gateway {", "Gateway", TS, Some(SymbolKind::Class)),
        ("export interface Payment {", "Payment", TS, Some(SymbolKind::Interface)),
        ("export type Money = number;", "Money", TS, Some(SymbolKind::Type)),
        ("enum Status {", "Status", TS, Some(SymbolKind::Enum)),
        ("  private charge(amount: number) {", "charge", TS, Some(SymbolKind::Method)),
        ("def charge(amount):", "charge", PY, Some(SymbolKind::Function)),
        ("class Gateway:", "Gateway", PY, Some(SymbolKind::Class)),
        ("MAX_ATTEMPTS = 3", "MAX_ATTEMPTS", PY, Some(SymbolKind::Variable)),
        ("func Charge(amount int) error {", "Charge", GO, Some(SymbolKind::Function)),
        ("func (g *Gateway) Charge(amount int) error {", "Charge", GO, Some(SymbolKind::Method)),
        ("type Gateway struct {", "Gateway", GO, Some(SymbolKind::Type)),
        ("pub fn charge(amount: u64) {", "charge", RS, Some(SymbolKind::Function)),
        ("pub struct Gateway {", "Gateway", RS, Some(SymbolKind::Struct)),
        ("pub trait Payable {", "Payable", RS, Some(SymbolKind::Trait)),
        ("pub mod retry;", "retry", RS, Some(SymbolKind::Module)),
        ("fn charge(amount) {", "charge", None, Some(SymbolKind::Function)),
        ("charge(42);", "charge", TS, None),
    ];
    for (line, symbol, language, symbol_kind) in cases {
        let expected = if symbol_kind.is_some() { Kind::Definition } else { Kind::Reference };
        assert_eq!(classify(line, symbol, language), (expected, symbol_kind), "{line}");
    }
}

// --- rankLocations ---------------------------------------------------------------------

fn loc(path: &str, line: u32, kind: Kind) -> Candidate {
    Candidate { path: path.into(), line, column: 0, text: "some code".into(), kind, symbol: None }
}

fn ranked(
    matches: Vec<Candidate>,
    origin: &str,
    changed: &[&str],
    cap: usize,
) -> (Vec<Candidate>, Vec<Candidate>, bool) {
    let changed: HashSet<&str> = changed.iter().copied().collect();
    rank(matches, origin, &changed, cap)
}

#[test]
fn rank_same_file_first() {
    let (_, refs, _) = ranked(
        vec![loc("src/other.ts", 10, Kind::Reference), loc("src/main.ts", 5, Kind::Reference)],
        "src/main.ts",
        &[],
        50,
    );
    assert_eq!(refs[0].path, "src/main.ts");
}

#[test]
fn rank_changed_files_above_others() {
    let (_, refs, _) = ranked(
        vec![loc("lib/utils.ts", 1, Kind::Reference), loc("src/changed.ts", 1, Kind::Reference)],
        "src/main.ts",
        &["src/changed.ts"],
        50,
    );
    assert_eq!(refs[0].path, "src/changed.ts");
}

#[test]
fn rank_splits_definitions_and_references() {
    let (defs, refs, _) = ranked(
        vec![loc("src/a.ts", 1, Kind::Reference), loc("src/a.ts", 1, Kind::Definition)],
        "src/main.ts",
        &[],
        50,
    );
    assert_eq!((defs.len(), refs.len()), (1, 1));
}

#[test]
fn rank_demotes_tests_unless_origin_is_one() {
    let matches = || {
        vec![loc("src/__tests__/main.test.ts", 1, Kind::Reference), loc("src/utils.ts", 1, Kind::Reference)]
    };
    let (_, refs, _) = ranked(matches(), "src/main.ts", &[], 50);
    assert_eq!(refs[0].path, "src/utils.ts");
    let (_, refs, _) = ranked(matches(), "src/__tests__/other.test.ts", &[], 50);
    assert_eq!(refs[0].path, "src/__tests__/main.test.ts");
}

#[test]
fn rank_caps_results() {
    let many: Vec<Candidate> =
        (0..100).map(|i| loc(&format!("src/file{i}.ts"), i, Kind::Reference)).collect();
    let (_, refs, capped) = ranked(many, "src/main.ts", &[], 10);
    assert_eq!((refs.len(), capped), (10, true));
    let (_, _, capped) =
        ranked(vec![loc("a", 1, Kind::Reference), loc("b", 1, Kind::Reference)], "src/main.ts", &[], 50);
    assert!(!capped);
}

#[test]
fn rank_weights_add_up_as_in_the_browser() {
    // Added: the exact weights. A definition in a far folder (+150) loses to a reference in
    // the same folder (+200); a changed-file definition (+650) loses to a same-file
    // reference (+1000); a test file in the same folder (200 − 300) sinks below a plain far
    // reference (0); ties keep search order.
    let (defs, refs, _) = ranked(
        vec![
            loc("far/a.ts", 1, Kind::Definition),
            loc("src/near.ts", 2, Kind::Reference),
            loc("src/changed.ts", 3, Kind::Definition),
            loc("src/main.ts", 4, Kind::Reference),
            loc("src/main.test.ts", 5, Kind::Reference),
            loc("far/b.ts", 6, Kind::Reference),
            loc("far/c.ts", 7, Kind::Reference),
        ],
        "src/main.ts",
        &["src/changed.ts"],
        50,
    );
    let order = |v: &[Candidate]| v.iter().map(|c| c.line).collect::<Vec<_>>();
    assert_eq!(order(&defs), [3, 1]);
    assert_eq!(order(&refs), [4, 2, 6, 7, 5]);
}

#[test]
fn same_directory_keeps_the_browsers_slice_quirk() {
    assert!(same_directory("a.ts", "b.ts"));
    assert!(same_directory("src/a.ts", "src/b.ts"));
    assert!(!same_directory("src/a.ts", "lib/b.ts"));
    assert!(!same_directory("a.ts", "src/b.ts"));
    // `"srcX".slice(0, -1)` is "src": the browser counts these as one folder.
    assert!(same_directory("srcX", "src/b.ts"));
}

#[test]
fn test_files_are_paths_with_test_or_spec() {
    assert!(is_test_file("src/__tests__/a.ts"));
    assert!(is_test_file("pkg/a_test.go"));
    assert!(is_test_file("a.Spec.ts"));
    assert!(is_test_file("tests/integration.rs"));
    assert!(!is_test_file("src/main.rs"));
}

// --- buildRgArgs exclusions, as pathspecs ------------------------------------------------

fn specs(language: Option<Language>, origin: &str) -> Vec<String> {
    pathspecs(language, origin)
}

#[test]
fn always_ignored_names_are_excluded_at_any_depth() {
    let s = specs(None, "src/a.ts");
    assert!(s.contains(&":(exclude,glob)**/node_modules/**".to_owned()));
    assert!(!s.contains(&":(exclude,glob)node_modules/**".to_owned()));
}

#[test]
fn ambiguous_names_are_excluded_only_at_the_root() {
    let s = specs(None, "src/a.ts");
    for dir in ["vendor", "target", "build", "dist", "coverage"] {
        assert!(s.contains(&format!(":(exclude,glob){dir}/**")), "{dir}");
        assert!(!s.contains(&format!(":(exclude,glob)**/{dir}/**")), "{dir}");
    }
}

#[test]
fn a_deep_same_named_package_does_not_lift_the_root_only_exclusion() {
    let s = specs(Some(Language::Java), "src/main/java/com/example/vendor/app/ExampleService.java");
    assert!(s.contains(&":(exclude,glob)vendor/**".to_owned()));
    assert!(s.contains(&":(exclude,glob)**/node_modules/**".to_owned()));
    assert!(s.contains(&":(exclude,glob)target/**".to_owned()));
}

#[test]
fn an_origin_under_the_root_directory_lifts_it() {
    let s = specs(Some(Language::Java), "vendor/third_party/Vendored.java");
    assert!(!s.contains(&":(exclude,glob)vendor/**".to_owned()));
    assert!(s.contains(&":(exclude,glob)target/**".to_owned()));
    assert!(s.contains(&":(exclude,glob)**/node_modules/**".to_owned()));
}

#[test]
fn the_origin_also_lifts_an_always_ignored_segment() {
    let s = specs(None, "packages/node_modules/dep/index.js");
    assert!(!s.iter().any(|p| p.contains("node_modules")));
}

#[test]
fn a_language_limits_the_files_searched() {
    let s = specs(TS, "src/a.ts");
    assert_eq!(s[..4], [":(glob)**/*.cts", ":(glob)**/*.mts", ":(glob)**/*.ts", ":(glob)**/*.tsx"]);
    assert!(!specs(None, "Makefile").iter().any(|p| p.starts_with(":(glob)")));
}

// --- isCodeNavPathAllowed --------------------------------------------------------------

#[test]
fn root_excluded_tree_stays_excluded_for_a_deep_same_named_package() {
    let origin = Some("src/main/java/com/example/vendor/app/Widget.java");
    assert!(!is_path_allowed("vendor/RootVendorFile.java", origin));
    assert!(is_path_allowed("src/main/java/com/example/vendor/app/Other.java", origin));
    assert!(is_path_allowed("src/other/Other.java", origin));
}

#[test]
fn a_file_under_the_root_directory_keeps_its_siblings() {
    let origin = Some("vendor/RootVendorFile.java");
    assert!(is_path_allowed("vendor/Sibling.java", origin));
    assert!(is_path_allowed("vendor/nested/Deep.java", origin));
    assert!(!is_path_allowed("dist/Built.java", origin));
}

#[test]
fn always_ignored_names_are_per_instance_too() {
    let origin = Some("packages/app/node_modules/dep/index.js");
    assert!(is_path_allowed("packages/app/node_modules/dep/other.js", origin));
    assert!(!is_path_allowed("packages/lib/node_modules/dep/index.js", origin));
    assert!(!is_path_allowed("node_modules/dep/index.js", origin));
}

#[test]
fn with_no_origin_the_plain_rules_apply() {
    assert!(!is_path_allowed("vendor/x.java", None));
    assert!(is_path_allowed("src/x.java", None));
    assert!(is_path_allowed("src/vendor/x.java", None));
    assert!(is_path_allowed("src/vendor", None));
    assert!(!is_path_allowed("./vendor/x.java", None));
    assert!(!is_path_allowed("vendor\\x.java", None));
}

#[test]
fn hidden_paths_are_skipped_except_around_the_origin() {
    assert!(is_hidden(".github/workflows/ci.js", "src/a.js"));
    assert!(is_hidden("src/.eslintrc.js", "src/a.js"));
    assert!(!is_hidden(".github/scripts/b.js", ".github/scripts/a.js"));
    assert!(!is_hidden(".config.js", ".config.js"));
    assert!(!is_hidden("src/a.js", "src/b.js"));
}

// --- extractChangedFiles ----------------------------------------------------------------

#[test]
fn changed_paths_name_both_sides_of_a_rename_once() {
    let patch = "diff --git a/src/old.ts b/src/new.ts\nsimilarity index 90%\nrename from src/old.ts\nrename to src/new.ts\n\
                 diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1 @@\n-x\n+y\n";
    let files = crate::patch::parse(patch);
    assert_eq!(changed_paths(&files), ["src/old.ts", "src/new.ts", "src/a.ts"]);
    assert_eq!(changed_paths(&[]), Vec::<String>::new());
}

// --- detectLanguage ---------------------------------------------------------------------

#[test]
fn languages_follow_the_browsers_extension_map() {
    assert_eq!(Language::from_path("src/App.TSX"), TS);
    assert_eq!(Language::from_path("a.jsx"), Some(Language::JavaScript));
    assert_eq!(Language::from_path("a.h"), Some(Language::C));
    assert_eq!(Language::from_path("a.cpp"), Some(Language::Cpp));
    // The browser maps neither `.hpp` nor `.cc`, nor `.mjs`.
    assert_eq!(Language::from_path("a.hpp"), None);
    assert_eq!(Language::from_path("a.mjs"), None);
    assert_eq!(Language::from_path("Makefile"), None);
}

// --- parsing and snippets ---------------------------------------------------------------

#[test]
fn grep_lines_parse_with_and_without_a_revision() {
    let line = b"HEAD:src/a.rs\x001\x004\x00fn foo() {}";
    assert_eq!(
        parse_grep_line(line, Some("HEAD:")),
        Some(GrepLine { path: "src/a.rs".into(), line: 1, column: 3, text: "fn foo() {}".into() })
    );
    let line = b"src/a.rs\x002\x009\x00let x = foo(); // a\x00b";
    let parsed = parse_grep_line(line, None).expect("parses");
    assert_eq!((parsed.line, parsed.column, parsed.text.as_str()), (2, 8, "let x = foo(); // a\0b"));
    assert_eq!(parse_grep_line(b"Binary file x matches", None), None);
}

#[test]
fn snippets_trim_the_end_and_cut_at_200_characters() {
    assert_eq!(snippet("  let a = 1;  \r"), "  let a = 1;");
    let long = "é".repeat(250);
    let cut = snippet(&long);
    assert_eq!(cut.chars().count(), 201);
    assert!(cut.ends_with('…'));
}

// --- identifiers on a line ---------------------------------------------------------------

fn idents(line: &str, language: Option<Language>) -> Vec<&str> {
    identifiers_on_line(line, language).into_iter().map(|i| i.text).collect()
}

#[test]
fn typescript_identifiers() {
    assert_eq!(
        idents(r#"export const total = sum(items, "items") + $el.width; // count(all)"#, TS),
        ["total", "sum", "items", "$el", "width"]
    );
    assert_eq!(idents("const s = `hi ${name}`; /* note */ run(s)", TS), ["s", "run", "s"]);
    assert_eq!(idents("if (x instanceof Foo) return this.bar(0x1f, 1e5);", TS), ["x", "Foo", "bar"]);
    assert_eq!(idents(" * @param foo the thing", TS), Vec::<&str>::new());
}

#[test]
fn python_identifiers() {
    assert_eq!(idents("def load(self, path: str) -> None:  # read(path)", PY), ["load", "path", "str"]);
    assert_eq!(idents(r#"msg = f"{name}" + r'\d' + """doc""" + other"#, PY), ["msg", "other"]);
    assert_eq!(idents("@dataclass", PY), ["dataclass"]);
}

#[test]
fn go_identifiers() {
    assert_eq!(
        idents("func (s *Server) Start(ctx context.Context) error { return nil } // x", GO),
        ["s", "Server", "Start", "ctx", "context", "Context"]
    );
    assert_eq!(
        idents("re := regexp.MustCompile(`\\d+`) ; r := 'a'", GO),
        ["re", "regexp", "MustCompile", "r"]
    );
}

#[test]
fn rust_identifiers() {
    assert_eq!(
        idents("pub fn parse<'a>(text: &'a str, c: char) -> Option<Ident<'a>> { let q = '\\''; x }", RS),
        ["parse", "text", "c", "Option", "Ident", "q", "x"]
    );
    assert_eq!(
        idents(r##"let s = r#"raw "quoted" str"#; let b = b"bytes"; call(s)"##, RS),
        ["s", "b", "call", "s"]
    );
    assert_eq!(idents("let _ = r#type + 10u8;", RS), Vec::<&str>::new());
}

#[test]
fn c_identifiers() {
    assert_eq!(idents("#include <stdio.h>", Some(Language::C)), Vec::<&str>::new());
    assert_eq!(idents("#define MAX_LEN 10", Some(Language::C)), ["MAX_LEN"]);
    assert_eq!(
        idents("static int count = L'x' + strlen(name);", Some(Language::C)),
        ["count", "strlen", "name"]
    );
}

#[test]
fn identifier_at_finds_the_word_under_or_after_the_column() {
    let line = "let total = sum(items);";
    let at = |col| identifier_at(line, col, RS).map(|i| i.text);
    assert_eq!(at(0), Some("total")); // `let` is a keyword: the next identifier
    assert_eq!(at(5), Some("total"));
    assert_eq!(at(8), Some("total"));
    assert_eq!(at(9), Some("sum")); // past the end of `total`: the next one
    assert_eq!(at(10), Some("sum"));
    assert_eq!(at(17), Some("items"));
    assert_eq!(at(22), None);
    let id = identifier_at(line, 13, RS).expect("sum");
    assert_eq!((id.start, id.end), (12, 15));
}

// --- Found::best ------------------------------------------------------------------------

#[test]
fn best_skips_the_line_the_person_is_on() {
    let here = loc("src/a.rs", 3, Kind::Definition);
    let there = loc("src/b.rs", 9, Kind::Definition);
    let found = Found { definitions: vec![here.clone(), there.clone()], ..Found::default() };
    let query = Query { file: "src/a.rs", line: 3, word: "x", changed: &[] };
    assert_eq!(found.best(&query), Some(&there));
    let only_refs = Found { references: vec![here, loc("src/c.rs", 1, Kind::Reference)], ..Found::default() };
    assert_eq!(only_refs.best(&query).map(|c| c.path.as_str()), Some("src/c.rs"));
    assert_eq!(Found::default().best(&query), None);
}
