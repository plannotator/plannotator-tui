//! What kind of file a changed path is: code, tests, docs, config, generated or other. The
//! file list's kind filter (`filter-kinds`) shows or hides each kind.
//!
//! Tests and generated files follow the browser app's rules exactly, so both apps agree on
//! them: Plannotator's `packages/core/file-kind.ts` (`isTestPath`, `TEST_FOLDERS`,
//! `TEST_NAME`, `DEFAULT_GENERATED_PATTERNS`). The browser calls everything else "code";
//! here that splits into code, docs, config and other, by GitHub linguist's documentation
//! and vendored conventions and common file names. A few more generated rules have no
//! counterpart in the browser yet: more lockfiles, vendored folders, protobuf output.
//!
//! Pure: it reads the path only, never the file.

/// The kinds, in the order the picker lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Kind {
    Code,
    Test,
    Docs,
    Config,
    Generated,
    Other,
}

pub(crate) const KINDS: [Kind; 6] =
    [Kind::Code, Kind::Test, Kind::Docs, Kind::Config, Kind::Generated, Kind::Other];

impl Kind {
    /// The picker's name for the kind; "Code", "Tests" and "Generated" are the browser's.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Code => "Code",
            Self::Test => "Tests",
            Self::Docs => "Docs",
            Self::Config => "Config",
            Self::Generated => "Generated",
            Self::Other => "Other",
        }
    }

    /// Where the kind is a position in [`KINDS`].
    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

/// Plannotator's `DEFAULT_GENERATED_PATTERNS`, matched against the last path segment:
/// whole names, then `*.`-suffixes with at least one character before the suffix.
const GENERATED_NAMES: &[&str] = &[
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
const GENERATED_SUFFIXES: &[&str] = &[".min.js", ".min.css", ".map"];

/// Lockfiles and generated sources the browser does not list yet (linguist's
/// `generated.rb`): whole names, then suffixes.
const MORE_GENERATED_NAMES: &[&str] = &[
    "deno.lock",
    "pdm.lock",
    "mix.lock",
    "pubspec.lock",
    "Podfile.lock",
    "Package.resolved",
    "gradle.lockfile",
    "go.work.sum",
];
const MORE_GENERATED_SUFFIXES: &[&str] = &[
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
/// folder of the path.
const GENERATED_FOLDERS: &[&str] =
    &["vendor", "node_modules", "third_party", "third-party", "dist", "__generated__"];

/// Plannotator's `TEST_FOLDERS`, compared case-insensitively with each folder of the path.
const TEST_FOLDERS: &[&str] = &[
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
const TEST_CODE_EXT: &[&str] = &[
    "js", "jsx", "ts", "tsx", "cjs", "cjsx", "cts", "ctsx", "mjs", "mjsx", "mts", "mtsx", "rb", "py", "go",
    "rs", "java", "kt", "kts", "swift", "cs", "fs", "ex", "exs", "php", "scala", "dart", "groovy", "clj",
    "cljc", "cljs", "lua", "erl", "hs", "ml", "jl", "r", "c", "cc", "cpp", "h", "hpp", "m", "mm", "vue",
    "svelte",
];

/// The markers of `foo.test.ts`, `foo.spec.rb`, `login.cy.ts`, `app.e2e.ts`, `t.test-d.ts`.
const TEST_MARKERS: &[&str] = &["test", "spec", "cy", "e2e", "test-d"];

/// Extensions of source code, beyond the test markers' list.
const CODE_EXT: &[&str] = &[
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "sql", "html", "htm", "css", "scss", "sass", "less",
    "astro", "zig", "nim", "pl", "pm", "gradle", "tf", "hcl", "nix", "proto", "graphql", "gql", "elm", "fsx",
    "vb", "asm", "s", "cxx", "hh", "hxx", "ino", "cu", "metal", "glsl", "wgsl", "hlsl", "sol", "v", "sv",
    "vhd", "f90", "cob", "d", "cr", "rkt", "scm", "lisp", "el", "vim", "tcl", "awk", "ipynb",
];

/// Documentation by extension (linguist's prose languages and `documentation.yml`).
const DOCS_EXT: &[&str] =
    &["md", "mdx", "markdown", "rst", "adoc", "asciidoc", "txt", "org", "rdoc", "pod", "tex"];

/// Documentation by name, extension or none: `README`, `README.md`, `CHANGELOG.rst`.
const DOCS_NAMES: &[&str] = &[
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
const DOCS_FOLDERS: &[&str] = &["docs", "doc", "documentation"];

/// Configuration by extension.
const CONFIG_EXT: &[&str] = &[
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
const CONFIG_NAMES: &[&str] = &[
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

/// The kind of the file at `path`, a repository path with `/` between folders.
pub(crate) fn classify(path: &str) -> Kind {
    let (folders, name) = match path.rsplit_once('/') {
        Some((folders, name)) => (folders.split('/').collect::<Vec<_>>(), name),
        None => (Vec::new(), path),
    };
    if is_generated(&folders, name) {
        Kind::Generated
    } else if is_test(&folders, name) {
        Kind::Test
    } else {
        by_name(&folders, name)
    }
}

/// Plannotator's `isDefaultGeneratedPath`, then the rules it does not have.
fn is_generated(folders: &[&str], name: &str) -> bool {
    let suffix = |suffixes: &[&str]| suffixes.iter().any(|s| name.len() > s.len() && name.ends_with(s));
    GENERATED_NAMES.contains(&name)
        || suffix(GENERATED_SUFFIXES)
        || MORE_GENERATED_NAMES.contains(&name)
        || suffix(MORE_GENERATED_SUFFIXES)
        || folders.iter().any(|f| GENERATED_FOLDERS.contains(f))
}

/// Plannotator's `isTestPath`: a test file name, or a test folder above it.
fn is_test(folders: &[&str], name: &str) -> bool {
    is_test_name(name)
        || folders.iter().enumerate().any(|(i, folder)| {
            TEST_FOLDERS.contains(&folder.to_lowercase().as_str())
                || is_dotnet_test_project(folder)
                || (i > 0 && folders.get(i - 1) == Some(&"src") && is_source_set_test(folder))
        })
}

/// Plannotator's `TEST_NAME`, case-insensitive: `foo.test.ts`, `foo_test.go`,
/// `Button.stories.tsx`, `test_foo.py`, `conftest.py`, `tests.py`, `foo.snap`.
fn is_test_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    let dotfile = lower.starts_with('.');
    if let Some((stem, ext)) = lower.rsplit_once('.')
        && TEST_CODE_EXT.contains(&ext)
    {
        let marked = TEST_MARKERS.iter().any(|marker| {
            stem.strip_suffix(marker)
                .and_then(|s| s.strip_suffix('.'))
                .is_some_and(|before| !before.is_empty())
        });
        if marked && !dotfile {
            return true;
        }
        if stem.strip_suffix("_test").is_some_and(|before| !before.is_empty()) {
            return true;
        }
    }
    let stories = lower.find(".stories.").is_some_and(|at| at > 0 && lower.len() > at + ".stories.".len());
    (stories && !dotfile)
        || lower.strip_prefix("test_").and_then(|s| s.strip_suffix(".py")).is_some_and(|s| !s.is_empty())
        || lower == "conftest.py"
        || lower == "tests.py"
        || lower.strip_suffix(".snap").is_some_and(|s| !s.is_empty())
}

/// .NET test projects, `Acme.Tests` and `Acme.UnitTests` (case-sensitive, as in the browser).
fn is_dotnet_test_project(folder: &str) -> bool {
    [".Tests", ".UnitTests"].iter().any(|end| folder.strip_suffix(end).is_some_and(|s| !s.is_empty()))
}

/// Gradle and Kotlin source sets right under `src`: `androidTest`, `integrationTest`.
fn is_source_set_test(folder: &str) -> bool {
    let mut chars = folder.chars();
    folder.len() > "Test".len()
        && folder.ends_with("Test")
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// Not generated and not a test: config, docs, code or other, by name and extension. A
/// documentation name (`README`, `LICENSE-MIT`) counts only without a code extension, so
/// `security.ts` stays code; code in a docs folder stays code too.
fn by_name(folders: &[&str], name: &str) -> Kind {
    let lower = name.to_lowercase();
    let dotfile = lower.starts_with('.');
    let ext =
        lower.rsplit_once('.').map(|(_, ext)| ext).filter(|_| !dotfile || lower.matches('.').count() > 1);
    let stem = lower.split('.').next().unwrap_or_default();
    let requirements = lower.starts_with("requirements") && ext == Some("txt");
    if CONFIG_NAMES.contains(&lower.as_str())
        || stem == "dockerfile"
        || lower.starts_with("docker-compose")
        || requirements
        || is_tool_config(&lower)
    {
        return Kind::Config;
    }
    if ext.is_some_and(|e| DOCS_EXT.contains(&e)) {
        return Kind::Docs;
    }
    if ext.is_some_and(|e| TEST_CODE_EXT.contains(&e) || CODE_EXT.contains(&e)) {
        return Kind::Code;
    }
    let doc_name = DOCS_NAMES.iter().any(|doc| {
        stem.strip_prefix(doc).is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '_']))
    });
    if doc_name || folders.iter().any(|f| DOCS_FOLDERS.contains(&f.to_lowercase().as_str())) {
        return Kind::Docs;
    }
    if dotfile || ext.is_some_and(|e| CONFIG_EXT.contains(&e)) {
        return Kind::Config;
    }
    Kind::Other
}

/// A tool's own config file in a code language: `vite.config.ts`, `jest.config.js`,
/// `.eslintrc.cjs`, `babel.config.json`.
fn is_tool_config(lower: &str) -> bool {
    let Some((stem, _)) = lower.rsplit_once('.') else { return false };
    stem.ends_with(".config") || (stem.starts_with('.') && stem.ends_with("rc"))
}

#[cfg(test)]
mod tests;
