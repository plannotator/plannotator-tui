//! The names, extensions and folders the kinds are told by.

/// Plannotator's `DEFAULT_GENERATED_PATTERNS`, matched against the last path segment:
/// whole names, then `*.`-suffixes with at least one character before the suffix.
pub(super) const GENERATED_NAMES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "Cargo.lock",
    "Gemfile.lock",
    "composer.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "go.sum",
    "flake.lock",
    "packages.lock.json",
];
pub(super) const GENERATED_SUFFIXES: &[&str] = &[".min.js", ".min.css", ".map"];

/// Lockfiles and generated sources the browser does not list yet (linguist's
/// `generated.rb`): whole names, then suffixes.
pub(super) const MORE_GENERATED_NAMES: &[&str] = &[
    "deno.lock",
    "pdm.lock",
    "mix.lock",
    "pubspec.lock",
    "Podfile.lock",
    "Package.resolved",
    "gradle.lockfile",
    "go.work.sum",
];
pub(super) const MORE_GENERATED_SUFFIXES: &[&str] = &[
    ".pb.go",
    "_pb2.py",
    "_pb2_grpc.py",
    ".pb.h",
    ".pb.cc",
    ".g.dart",
    ".freezed.dart",
    ".designer.cs",
    ".generated.ts",
    ".generated.js",
];

/// Folders whose files are vendored or built (linguist's `vendor.yml`), compared with each
/// folder of the path; `dist` only outside `src`.
pub(super) const GENERATED_FOLDERS: &[&str] =
    &["vendor", "node_modules", "third_party", "third-party", "dist", "__generated__"];

/// Plannotator's `TEST_FOLDERS`, compared case-insensitively with each folder of the path.
pub(super) const TEST_FOLDERS: &[&str] = &[
    "__tests__",
    "__snapshots__",
    "__fixtures__",
    "__mocks__",
    "test",
    "tests",
    "spec",
    "e2e",
    "fixtures",
    "testdata",
    "testfixtures",
];

/// Plannotator's `CODE_EXT`: a test marker counts only before one of these, so
/// `openapi.spec.yaml` and `load_test.md` are not tests.
pub(super) const TEST_CODE_EXT: &[&str] = &[
    "js", "jsx", "ts", "tsx", "cjs", "cjsx", "cts", "ctsx", "mjs", "mjsx", "mts", "mtsx", "rb", "py", "go",
    "rs", "java", "kt", "kts", "swift", "cs", "fs", "ex", "exs", "php", "scala", "dart", "groovy", "clj",
    "cljc", "cljs", "lua", "erl", "hs", "ml", "jl", "r", "c", "cc", "cpp", "h", "hpp", "m", "mm", "vue",
    "svelte",
];

/// The markers of `foo.test.ts`, `foo.spec.rb`, `login.cy.ts`, `app.e2e.ts`, `t.test-d.ts`.
pub(super) const TEST_MARKERS: &[&str] = &["test", "spec", "cy", "e2e", "test-d"];

/// Extensions of source code, beyond the test markers' list.
pub(super) const CODE_EXT: &[&str] = &[
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "sql", "html", "htm", "css", "scss", "sass", "less",
    "astro", "zig", "nim", "pl", "pm", "gradle", "tf", "hcl", "nix", "proto", "graphql", "gql", "elm", "fsx",
    "vb", "asm", "s", "cxx", "hh", "hxx", "ino", "cu", "metal", "glsl", "wgsl", "hlsl", "sol", "v", "sv",
    "vhd", "f90", "cob", "d", "cr", "rkt", "scm", "lisp", "el", "vim", "tcl", "awk", "ipynb",
];

/// Documentation by extension (linguist's prose languages and `documentation.yml`).
pub(super) const DOCS_EXT: &[&str] =
    &["md", "mdx", "markdown", "rst", "adoc", "asciidoc", "txt", "org", "rdoc", "pod", "tex"];

/// Documentation by name, extension or none: `README`, `README.md`, `CHANGELOG.rst`.
pub(super) const DOCS_NAMES: &[&str] = &[
    "readme",
    "changelog",
    "changes",
    "history",
    "license",
    "licence",
    "copying",
    "notice",
    "authors",
    "contributors",
    "contributing",
    "code_of_conduct",
    "security",
    "install",
];

/// Folders of documentation (linguist's `documentation.yml`).
pub(super) const DOCS_FOLDERS: &[&str] = &["docs", "doc", "documentation"];

/// Configuration by extension.
pub(super) const CONFIG_EXT: &[&str] = &[
    "json",
    "jsonc",
    "json5",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "properties",
    "xml",
    "plist",
    "env",
    "cmake",
    "mk",
    "editorconfig",
];

/// Configuration and build files by whole name, compared case-insensitively.
pub(super) const CONFIG_NAMES: &[&str] = &[
    "dockerfile",
    "containerfile",
    "makefile",
    "gnumakefile",
    "justfile",
    "procfile",
    "gemfile",
    "rakefile",
    "pipfile",
    "brewfile",
    "vagrantfile",
    "jenkinsfile",
    "cmakelists.txt",
    "requirements.txt",
    "go.mod",
    "go.work",
    "codeowners",
    "build",
    "build.bazel",
    "workspace",
    "module.bazel",
];
