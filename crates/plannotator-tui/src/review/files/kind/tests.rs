use super::{Kind, classify};

/// Every path in `paths` is of `kind`, or the failures are listed.
fn all(kind: Kind, paths: &[&str]) {
    let wrong: Vec<(&str, Kind)> =
        paths.iter().map(|p| (*p, classify(p))).filter(|(_, got)| *got != kind).collect();
    assert_eq!(wrong, Vec::<(&str, Kind)>::new(), "expected {kind:?}");
}

/// The browser's own test cases (`packages/core/file-kind.test.ts`), so both apps agree.
#[test]
fn tests_are_the_browsers_tests() {
    all(
        Kind::Test,
        &[
            "src/retry.test.ts",
            "src/Button.spec.tsx",
            "src/sync/__tests__/client.ts",
            "src/__mocks__/fs.ts",
            "src/components/__snapshots__/Button.test.tsx.snap",
            "src/Button.stories.tsx",
            "src/__fixtures__/user.json",
            "tests/e2e/sync.spec.ts",
            "e2e/login.ts",
            "test/helpers.js",
            "pkg/server/handler_test.go",
            "internal/testdata/golden.txt",
            "app/tests/test_models.py",
            "test_utils.py",
            "utils_test.py",
            "crates/core/tests/integration.rs",
            "src/test/java/com/acme/AppTest.java",
            "spec/models/user_spec.rb",
            "Tests/AppTests/AppTests.swift",
            "packages/core/fixtures/guide.json",
            "app/conftest.py",
            "polls/tests.py",
            "cypress/login.cy.ts",
            "src/app.e2e.ts",
            "src/types.test-d.ts",
            "app/src/androidTest/kotlin/MainTest.kt",
            "shared/src/commonTest/kotlin/Sync.kt",
            "service/src/integrationTest/java/ApiIT.java",
            "lib/src/testFixtures/java/Builders.java",
            "Acme.Tests/OrderTests.cs",
            "src/Acme.UnitTests/Fakes.cs",
            "src/__snapshots__/view.snap",
        ],
    );
}

#[test]
fn names_that_only_look_like_tests_are_not_tests() {
    let not_tests = [
        "src/contest.ts",
        "src/latest.ts",
        "testing-library.config.js",
        "jest.config.js",
        "vitest.setup.ts",
        "src/test.ts",
        "src/testing/render.ts",
        "src/attestation.ts",
        "src/specs.md",
        "docs/spec.md",
        "src/protest_test",
        "scripts/testify.sh",
        "src/test_helpers.ts",
        "README.md",
        ".github/workflows/test.yml",
        "src/contest/score.ts",
        "src/latest_test",
        ".test.ts",
        "openapi.spec.yaml",
        "contracts/payment.spec.json",
        "scripts/load_test.md",
        "docs/api.test.html",
        "src/main/kotlin/LatestTest/Foo.kt",
        "app/main/androidTest/Foo.kt",
        "Tests.Shared/Helpers.cs",
        "src/AcmeTests/Helpers.cs",
        "src/settings.py",
    ];
    let tests: Vec<&str> = not_tests.into_iter().filter(|p| classify(p) == Kind::Test).collect();
    assert_eq!(tests, Vec::<&str>::new());
}

#[test]
fn generated_files_are_the_browsers_list_then_lockfiles_vendored_folders_and_codegen() {
    all(
        Kind::Generated,
        &[
            "bun.lock",
            "Cargo.lock",
            "web/dist/app.min.js",
            "styles/site.min.css",
            "static/app.js.map",
            "go.sum",
            "packages/web/package-lock.json",
            "pnpm-lock.yaml",
            // Generated wins over tests, as in the browser.
            "tests/fixtures/data.min.js",
            "ios/Podfile.lock",
            "deno.lock",
            "vendor/github.com/pkg/errors/errors.go",
            "web/node_modules/left-pad/index.js",
            "third_party/zlib/zlib.h",
            "api/v1/user.pb.go",
            "proto/user_pb2.py",
            "lib/models/user.g.dart",
            "src/__generated__/graphql.ts",
        ],
    );
    assert_ne!(classify(".min.js"), Kind::Generated, "a bare suffix is not a name");
    assert_ne!(classify("src/vendors.ts"), Kind::Generated);
}

#[test]
fn docs_are_prose_by_extension_or_name_and_what_sits_in_a_docs_folder() {
    all(
        Kind::Docs,
        &[
            "README.md",
            "packages/review-editor/README.md",
            "CHANGELOG",
            "LICENSE",
            "LICENSE-MIT",
            "docs/guide.mdx",
            "docs/spec.rst",
            "docs/images/flow.svg",
            "notes.txt",
            "CONTRIBUTING.md",
        ],
    );
    // Code in a docs folder is still code.
    assert_eq!(classify("docs/site/build.ts"), Kind::Code);
}

#[test]
fn config_is_data_files_dotfiles_build_files_and_tool_configs() {
    all(
        Kind::Config,
        &[
            "package.json",
            "tsconfig.base.json",
            "Cargo.toml",
            "rustfmt.toml",
            ".github/workflows/ci.yml",
            "docker-compose.yml",
            "Dockerfile",
            "services/api/Dockerfile.dev",
            "Makefile",
            ".gitignore",
            ".editorconfig",
            ".eslintrc.cjs",
            ".prettierrc",
            "vite.config.ts",
            "packages/web/tailwind.config.js",
            "requirements.txt",
            "requirements-dev.txt",
            "go.mod",
            "CMakeLists.txt",
            ".env.example",
        ],
    );
}

#[test]
fn code_is_source_by_extension_and_the_rest_is_other() {
    all(
        Kind::Code,
        &[
            "packages/review-editor/components/ReviewAgentJobDetailPanel.tsx",
            "crates/plannotator-tui/src/review/draw.rs",
            "src/config.ts",
            "scripts/release.sh",
            "web/styles/app.scss",
            "db/migrations/001_init.sql",
            "cmd/server/main.go",
            "src/testing/render.ts",
        ],
    );
    all(Kind::Other, &["assets/logo.png", "fonts/Inter.woff2", "bin/tool", "data/blob.bin"]);
}

#[test]
fn data_and_config_extensions_win_over_documentation_names() {
    all(
        Kind::Config,
        &[
            ".github/workflows/security.yml",
            "install.yml",
            "notice.yaml",
            "data/history.json",
            "CHANGELOG.json",
            "docs/_config.yml",
        ],
    );
    // Without an extension only a capitalised name is documentation.
    assert_eq!(classify("scripts/install"), Kind::Other);
    assert_eq!(classify("INSTALL"), Kind::Docs);
    assert_eq!(classify("License"), Kind::Docs);
}

#[test]
fn a_dist_folder_inside_src_is_source_and_one_outside_is_built() {
    assert_eq!(classify("src/dist/mod.rs"), Kind::Code);
    assert_eq!(classify("crates/core/src/dist/mod.rs"), Kind::Code);
    assert_eq!(classify("dist/index.js"), Kind::Generated);
    assert_eq!(classify("packages/app/dist/index.js"), Kind::Generated);
}
