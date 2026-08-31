# Spec: Full-mode Windows support for Plannotator TUI + Herdr Annotate

Status: implementation contract, 2026-08-30. This document extends
[`spec-herdr-integration.md`](spec-herdr-integration.md) and
[`spec-last-message.md`](spec-last-message.md). It specifies native Windows support for the
Full document-review path. It does not change Annotate Lite.

`UNVERIFIED` is a result, not a placeholder. It means the source establishes the intended
plumbing but no native Windows + Herdr run has established the user-visible behavior.

## Scope and acceptance

Full mode is supported on native Windows when all of the following are true:

- Herdr installs the pinned, checksummed `plannotator-tui.exe` without Bash, `sh`, `uname`,
  `chmod`, or another Unix compatibility layer.
- `Ctrl+B O`, `Ctrl+B Shift+O`, and Ctrl-click on a Markdown `file://` link reach the same
  direct executable entry points as macOS and Linux.
- Overlay, split, and popup placements launch the executable through Herdr's native ConPTY
  pane runtime. No manifest command depends on shell parsing.
- `last` selects the session Herdr reported. An exact session id or path wins over pid, cwd,
  mtime, or “newest session” inference.
- Feedback is delivered as one argv value to `herdr agent prompt`; fallback behavior remains
  the contract in `spec-herdr-integration.md`.
- The complete Rust workspace test suite and Windows-specific subprocess checks pass on
  `windows-latest` before merge.
- Native Windows x64 is required. Native Windows ARM64 is also shipped before the plugin
  advertises ARM64 Full support; cross-build evidence is not a substitute for the live ARM64
  check in this spec.

WSL is not this scope. A Linux Herdr running inside WSL already follows the Linux plugin path.
Remote Windows hosts are also out of scope because Herdr does not support Windows as a
`herdr --remote` target
([`windows-beta.mdx`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/docs/next/website/src/content/docs/windows-beta.mdx#L116-L134)).

## Evidence baseline

The survey used these revisions. Line citations to another repository are permalinks to the
revision in this table.

| Source | Revision | Purpose |
|---|---:|---|
| `plannotator/plannotator-tui` | `d5c3a6e18c78` | current TUI, host readers, CI, release |
| `plannotator/herdr-annotate` | `ba4903b28fbb` | current Full manifest and Bash scripts |
| `plannotator/herdr-annotate` PR [#27](https://github.com/plannotator/herdr-annotate/pull/27) | `0c5231f8ca6f` | Lite Rust PowerShell installer and relative-exe manifest prior art |
| `ogulcancelik/herdr` | `7b675f42af35` | Windows plugin execution, ConPTY panes, env, integration hooks |
| `backnotprop/cc-open` | `48337af04bb2` | extracted Claude Code storage behavior |
| `openai/codex` | `0fb559f0f6e2` | Codex storage and native Windows support |
| `badlogic/pi-mono` | `fa07e7bd92c9` | Pi storage and native Windows support |
| `can1357/oh-my-pi` | `72000acfeb90` | OMP storage and native Windows support |
| `NousResearch/hermes-agent` | `2c232761e4c1` | Hermes storage and native Windows support |
| `anomalyco/opencode` | `ad905f8e6c8c` | OpenCode storage and native Windows support |
| `github/copilot-cli` | `be82101e70f0` | official Windows support statement; implementation is distributed |
| `@github/copilot` | `1.0.68` | installed official distribution inspected for platform packages and home resolution |
| `Factory-AI/factory` | `1fd9026d72f8` | official Droid Windows installation statement |
| `Factory-AI/droid-sdk-typescript` | `87b4c4c4e4fc` | official local-session listing contract |

No private transcript content is evidence in this document.

## Current state

### The TUI already builds and tests on Windows

Release v0.5.0 publishes
[`plannotator-tui-x86_64-pc-windows-msvc.exe`](https://github.com/plannotator/plannotator-tui/releases/tag/v0.5.0)
and `SHA256SUMS`. The release matrix and filename are declared in
[`release.yml`](../.github/workflows/release.yml#L22-L68). The post-merge matrix runs Clippy,
the workspace tests, and a release build on `windows-latest`
([`post-merge.yml`](../.github/workflows/post-merge.yml#L12-L28)); the baseline revision's
[Windows run passed](https://github.com/plannotator/plannotator-tui/actions/runs/33332168649).
Pull requests currently run only Windows Clippy
([`ci.yml`](../.github/workflows/ci.yml#L31-L40)), so a failing Windows test can still merge.

The application already contains the Windows-specific path behavior required by Full mode:

- Config resolves to `%APPDATA%\plannotator-tui\config.toml`, after the explicit
  `PLANNOTATOR_TUI_CONFIG` override
  ([`config.rs`](../crates/plannotator-tui/src/config.rs#L107-L118)).
- `file:///C:/...` removes the URL-only leading slash before constructing a `PathBuf`
  ([`launch.rs`](../crates/plannotator-tui/src/herdr/launch.rs#L130-L149)).
- Hermes defaults to `%LOCALAPPDATA%\hermes\state.db`, with `HERMES_HOME` taking precedence
  ([`locate.rs`](../crates/plannotator-tui/src/last/locate.rs#L71-L78),
  [`locate.rs`](../crates/plannotator-tui/src/last/locate.rs#L180-L193)).
- OpenCode path comparisons convert backslashes to slashes and compare case-insensitively on
  Windows ([`opencode.rs`](../crates/plannotator-tui-hosts/src/opencode.rs#L291-L299)). Copilot
  and Droid directory matching also tolerate Windows separators and case
  ([`copilot.rs`](../crates/plannotator-tui-hosts/src/copilot.rs#L160-L163),
  [`droid.rs`](../crates/plannotator-tui-hosts/src/droid.rs#L28-L37)).
- Annotation storage remains Plannotator-compatible: it uses `PLANNOTATOR_DATA_DIR` or
  `~/.plannotator`, not `%APPDATA%`. That is the existing cross-client data contract, not a
  Windows omission ([`datadir.rs`](../crates/plannotator-tui-schema/src/datadir.rs#L12-L40)).

The non-test `#[cfg]` branches affecting this flow are limited to config location, Hermes
home, parent-pid availability, and `file://` drive normalization. The other Windows branches
found under `crates/plannotator-tui` are test fixtures. Popup, overlay, rendering, mouse, and
delivery do not have a separate Windows implementation in this repository.

### Herdr's action execution is sufficient; pane resolution blocks Windows Full mode

Herdr 0.8.0 and later has the action, build, and ConPTY facilities Full mode needs, but not the
required pane-program resolution. The relative-command resolver used by actions is present in the
[`v0.8.0` source](https://github.com/ogulcancelik/herdr/blob/v0.8.0/src/plugin_command.rs#L7-L49):

- Plugin build commands and actions are argv arrays. Herdr resolves an explicit relative
  program such as `./bin/plannotator-tui.exe` against the plugin root, then uses
  `std::process::Command`; only `.cmd` and `.bat` programs receive an explicit `cmd.exe /d /c`
  wrapper ([`plugin_command.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/plugin_command.rs#L7-L49)).
- A build runs with the plugin root as cwd, null stdin, and captured stdout/stderr. A nonzero
  build exits the plugin installation
  ([`cli/plugin.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/cli/plugin.rs#L1266-L1365)).
  Herdr runtime variables are removed from build processes
  ([`cli/plugin.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/cli/plugin.rs#L1502-L1519));
  an installer must derive its root from `$PSScriptRoot` or the cwd.
- An action runs without a PTY, with the plugin root as cwd, and receives
  `HERDR_PLUGIN_ROOT`, config/state directories, `HERDR_ENV`, `HERDR_BIN_PATH`, the invocation
  context JSON, and focused pane identifiers
  ([`runtime.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/api/plugins/runtime.rs#L15-L80),
  [`runtime.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/api/plugins/runtime.rs#L103-L179)).
- A plugin pane passes the program and every argument separately to `portable_pty`, which uses
  native ConPTY on Windows. Unlike an action, the pane command does not pass through Herdr's
  plugin-relative program resolver; there is no shell between the manifest and the process
  ([`pane.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/pane.rs#L1814-L1857),
  [`pty/backend.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/pty/backend.rs#L7-L39)).
- Popup, overlay, and split all use that same argv pane runtime
  ([`popup.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/popup.rs#L84-L113),
  [`navigate.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/input/navigate.rs#L1081-L1165),
  [`panes.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/api/plugins/panes.rs#L82-L171)).
- The pane cwd is the explicit `--cwd`, else the plugin root. A relative pane program therefore
  resolves from the folder under review when `--cwd` is present, not from the plugin root. The
  launch env protects Herdr's own keys, then injects the plugin paths, context, and binary path
  ([`panes.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/api/plugins/panes.rs#L236-L269),
  [`panes.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/api/plugins/panes.rs#L329-L358)).
- Item-level `platforms` overrides the plugin-level list. An unsupported build is skipped; an
  unsupported action or pane returns `platform_unsupported`. An unsupported link handler is
  not selected. There is no fallback to another shell or command.

Herdr's documentation calls Windows plugins “preview” and “best-effort” while naming GitHub
install, local link, build, actions, events, and panes as present
([`windows-beta.mdx`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/docs/next/website/src/content/docs/windows-beta.mdx#L28-L56)).
In code, those words are a support qualification, not different invocation semantics. The
source proves argv, cwd, env, platform filtering, and ConPTY construction. It also proves that
the current one-manifest design cannot start a relative Windows pane when review supplies
`--cwd`: the Unix wrapper can anchor on `HERDR_PLUGIN_ROOT`, but Windows has no corresponding
shell. Full mode remains gated on Windows until Herdr resolves pane executables against the
plugin root without changing the requested pane cwd.

### Full mode is currently disabled by packaging

The distributed Annotate manifest advertises Windows for Lite at the plugin level, then gates
every Full entry to macOS/Linux. There are **six**, not five, Full gates: build, `doc` pane,
`open`, `open-link`, `last`, and `markdown-file`
([`herdr-plugin.toml`](https://github.com/plannotator/herdr-annotate/blob/ba4903b28fbb/herdr-plugin.toml#L45-L96)).
Removing only the five executable-entry gates leaves Ctrl-click disabled on Windows.

The build invokes Bash
([`herdr-plugin.toml`](https://github.com/plannotator/herdr-annotate/blob/ba4903b28fbb/herdr-plugin.toml#L52-L54)).
Every Full action and pane invokes `sh -c`, then a Bash launcher. The fetcher relies on Bash,
`uname`, executable bits, `curl`/`wget`, `mktemp`, `trap`, `sha256sum`/`shasum`, and Unix
target names
([`fetch-plannotator-tui.sh`](https://github.com/plannotator/herdr-annotate/blob/ba4903b28fbb/scripts/fetch-plannotator-tui.sh#L1-L79)).
The launcher relies on Bash, executable bits, and `$HERDR_PLUGIN_ROOT`
([`plannotator-tui.sh`](https://github.com/plannotator/herdr-annotate/blob/ba4903b28fbb/scripts/plannotator-tui.sh#L1-L15)).
The smoke suite is also Bash-only
([`smoke.sh`](https://github.com/plannotator/herdr-annotate/blob/ba4903b28fbb/scripts/smoke.sh#L1-L31)).

The development manifest stages the Cargo output under a common `.exe` filename on all three
platforms and keeps Windows action entries. Its `doc` pane remains macOS/Linux-only and uses
`HERDR_PLUGIN_ROOT` to reach that staged binary from the review cwd
([`herdr/herdr-plugin.toml`](../herdr/herdr-plugin.toml#L1-L56)).

## Gap inventory

### G1. Pane programs resolve from the review cwd, not the plugin root

**Evidence.** Actions pass through Herdr's plugin-relative command resolver and run with the
plugin root as cwd. Panes instead pass the first argv item directly to `portable_pty` and set
the process cwd to the explicit `--cwd` when present. `./bin/plannotator-tui.exe` in a pane
therefore names `<review-cwd>/bin/plannotator-tui.exe`, not the staged plugin binary. Herdr
also rejects duplicate pane IDs before platform filtering, so separate Unix and Windows `doc`
entries are not a manifest-level workaround
([`manifest.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/api/plugins/manifest.rs#L193-L193),
[`manifest.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/app/api/plugins/manifest.rs#L337-L346)).

**Affected files.** `herdr-annotate/herdr-plugin.toml`,
`herdr-annotate/scripts/plannotator-tui.sh`, `herdr/herdr-plugin.toml`, and upstream Herdr's
`src/app/api/plugins/panes.rs` and pane-spec validation.

**Required change.** Keep all six distributed Full gates (`build`, `doc`, `open`, `open-link`,
`last`, and `markdown-file`) on macOS/Linux. Keep the Unix pane on its shell wrapper so
`HERDR_PLUGIN_ROOT` supplies an absolute launcher path. Actions may use direct relative argv
because the action runtime resolves their program against the plugin root:

```toml
command = ["sh", "-c", "exec bash \"$HERDR_PLUGIN_ROOT/scripts/plannotator-tui.sh\" herdr pane"]
command = ["./bin/plannotator-tui.exe", "herdr", "open"]
command = ["./bin/plannotator-tui.exe", "herdr", "last"]
```

`open-link` intentionally uses the same `herdr open` argv as `open`; the invocation context
contains the clicked URL. The development manifest keeps Windows staging and action entries,
but limits `doc` to macOS/Linux and uses
`$HERDR_PLUGIN_ROOT/bin/plannotator-tui.exe` through `sh -c` there.

Upstream Herdr must add a pane-program resolution contract that anchors an explicit relative
program to the plugin root while preserving the requested pane cwd. The fix must also define
how one cross-platform `doc` entry works; duplicate platform-specific pane IDs are invalid.
Only after a Herdr release contains that behavior may the distributed Windows build and all six
Full gates be enabled, with `min_herdr_version` raised to the first fixed release.

**Risk.** The PowerShell fetcher and staged Windows binary have no distributed runtime consumer
until the upstream dependency lands. Keeping them tested is deliberate prerequisite work, not
evidence that Windows Full mode can open a pane.

### G2. There is no Windows installer

**Evidence.** The current fetcher maps only Darwin and Linux and manipulates Unix executable
bits. Release v0.5.0 already supplies one Windows asset, but the plugin never requests it.

**Affected files.** `herdr-annotate/herdr-plugin.toml`,
`herdr-annotate/scripts/fetch-plannotator-tui.sh`, new
`herdr-annotate/scripts/fetch-plannotator-tui.ps1`, and the plugin CI files specified below.

**Required change.** Implement and test the Windows PowerShell fetcher, but do not expose it as
a distributed `[[build]]` entry until G1's upstream dependency lands. The current manifest has
only the Unix build:

```toml
[[build]]
platforms = ["macos", "linux"]
command = ["bash", "scripts/fetch-plannotator-tui.sh"]
```

Both installers write `bin/plannotator-tui.exe`; `.exe` is a legal ordinary filename on Unix
and mandatory on Windows. The Unix installer therefore changes only its destination name and
stamp check; the downloaded Unix release asset retains its upstream name.

The PowerShell algorithm is frozen in “Manifest and installer design” below.

**Risk.** Windows locks running executables. Herdr installs GitHub plugins into a new plugin
root, so normal upgrades do not replace the executable in a running old root. A hand-run
fetcher against a linked plugin can still fail if that exact binary is running; the script
must report the path and leave the old file and stamp intact.

### G3. Windows ARM64 has no Plannotator TUI asset

**Evidence.** The TUI release matrix contains Windows x64 only
([`release.yml`](../.github/workflows/release.yml#L27-L33)). Herdr, Pi, Codex, OpenCode, Droid,
and Copilot publish or declare ARM64 Windows support, and Lite PR #27 maps both `X64` and
`Arm64` to native Rust targets.

**Affected files.** `.github/workflows/release.yml` in this repository and the PowerShell
fetcher in `herdr-annotate`.

**Required change.** Add `aarch64-pc-windows-msvc` to the release matrix and publish
`plannotator-tui-aarch64-pc-windows-msvc.exe` in `SHA256SUMS`. The PowerShell fetcher maps
`RuntimeInformation.OSArchitecture` exactly:

| Windows architecture | Rust target | Release asset |
|---|---|---|
| `X64` | `x86_64-pc-windows-msvc` | `plannotator-tui-x86_64-pc-windows-msvc.exe` |
| `Arm64` | `aarch64-pc-windows-msvc` | `plannotator-tui-aarch64-pc-windows-msvc.exe` |
| other | unsupported warning; Full remains unavailable | none |

Do not silently install x64 on ARM64. Emulation may work, but it is UNVERIFIED and would make
the installed architecture differ from the manifest's prior art.

**Risk.** `windows-latest` can prove the cross-build only if the bundled SQLite C dependency
finds the ARM64 MSVC toolchain. Running the result requires an ARM64 Windows host and remains
UNVERIFIED until the live matrix passes.

### G4. Development staging and action support exceed pane availability

**Evidence.** Unix Cargo writes `plannotator-tui`; Windows writes `plannotator-tui.exe`.
Platform-specific staging can normalize those names, and actions can use the staged relative
program on all three platforms. G1 prevents the same command from opening a Windows pane.

**Affected files.** `herdr/herdr-plugin.toml`; new `herdr/stage-plannotator-tui.sh` and
`herdr/stage-plannotator-tui.ps1`, or equivalently named staging scripts.

**Required change.** Make the development manifest top-level platforms all three. Give it
platform-specific build commands that run Cargo and stage the result as
`herdr/bin/plannotator-tui.exe`. Keep direct relative argv for actions. Limit the pane to
macOS/Linux and launch the staged binary through `sh -c` with an absolute
`HERDR_PLUGIN_ROOT` path. The staging scripts contain no download logic. They copy only the
just-built binary and fail on any error.

**Risk.** The development manifest can drift from the distributed manifest. Add structural
tests for the intentional split: both use a plugin-root wrapper for the Unix pane and matching
direct action tails; the distributed manifest keeps all six Full gates on macOS/Linux, while
the development manifest retains Windows staging and actions but no Windows pane.

### G5. Exact Herdr session IDs are discarded for file-backed hosts

**Evidence.** The launcher carries Herdr's `agent_session` into
`PLANNOTATOR_TUI_SESSION` or `PLANNOTATOR_TUI_SESSION_ID`
([`launch.rs`](../crates/plannotator-tui/src/herdr/launch.rs#L243-L256)). `locate` consumes the
id only for Hermes and OpenCode; Claude, Codex, Copilot, Droid, Pi, and OMP ignore it
([`locate.rs`](../crates/plannotator-tui/src/last/locate.rs#L33-L107)). Codex then chooses
`CODEX_THREAD_ID` from the new pane's environment or the globally newest rollout. Copilot
tries POSIX `ps`; Droid, Pi, and OMP choose by cwd/mtime.

Herdr's native Windows hooks report an id for Codex, Copilot, and Droid
([Codex hook](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/codex/herdr-agent-state.ps1#L20-L46),
[Copilot hook](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/copilot/herdr-agent-state.ps1#L35-L53),
[Droid hook](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/droid/herdr-agent-state.ps1#L9-L26)).
Using “newest” after receiving an exact id is a correctness bug on every OS and becomes more
visible on Windows because the POSIX process fallbacks are absent.

**Affected files.** `crates/plannotator-tui/src/{cli.rs,herdr/context.rs,herdr/launch.rs}`,
`crates/plannotator-tui/src/last/locate.rs`; host-specific locator modules and tests under
`crates/plannotator-tui-hosts`; Herdr launch tests in this repository.

**Required change.** Freeze this precedence for every host:

1. explicit transcript path (`--session` / `PLANNOTATOR_TUI_SESSION`);
2. explicit session id (`--session-id` / `PLANNOTATOR_TUI_SESSION_ID`);
3. host-specific pid metadata;
4. cwd and mtime fallback.

Session-id resolution is host-specific and rejects separators, `..`, NUL, and an empty value
before touching the filesystem:

| Host | ID lookup |
|---|---|
| Claude Code | `<config>/projects/*/<id>.jsonl`, preferring the current cwd slug |
| Codex | existing `codex::find_transcripts(codex_home, Some(id))` |
| Copilot CLI | `$COPILOT_HOME/session-state/<id>/events.jsonl` |
| Droid | `$FACTORY_CONFIG_DIR/sessions/*/<id>.jsonl`, preferring the current cwd slug |
| Pi / OMP | the current cwd bucket first, then `sessions/**/<timestamp>_<id>.jsonl` |
| Hermes / OpenCode | existing database lookup by id |

An exact id that is not found is an error naming the searched root. It does not fall through
to a different session. Herdr's screen fallback may still open, with the exact lookup error in
the status line.

An exact Herdr session must also remove the launcher's pid dependency. Refactor the launch
message from `(pid, host)` to `{ host, pid: Option<u32> }`. Call `herdr agent get` first. When
it returns a supported host and session reference, build the launch without calling
`herdr pane process-info`; emit `PLANNOTATOR_TUI_HOST`, `PLANNOTATOR_TUI_SESSION_ID` or
`PLANNOTATOR_TUI_SESSION`, and `PLANNOTATOR_TUI_CWD`, but no message pid. The `herdr pane`
entrypoint runs `last` when any session path, session id, or message pid is present. Call
process-info only when `agent get` has no usable session reference. This prevents a missing or
transient Windows process-tree snapshot from discarding authoritative session identity.

**Risk.** Copilot and Droid are distributed implementations. The id-to-directory/file mapping
is consistent with the current reader fixtures and Herdr hooks but is not source-verified in
their proprietary CLIs. Keep those two mappings UNVERIFIED until the live host check passes.

### G6. Host-specific custom roots do not reliably cross the pane boundary

**Evidence.** A plugin pane inherits Herdr's process environment, not arbitrary environment
changes made later inside the agent pane. Herdr stores one session reference. It prefers a
path only for Pi and OMP; all other official integrations retain the id even when the hook
also reported a path
([`agent_resume.rs`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/agent_resume.rs#L53-L70)).
The TUI also hardcodes `~/.claude` in its fallback instead of honoring
`CLAUDE_CONFIG_DIR` ([`locate.rs`](../crates/plannotator-tui/src/last/locate.rs#L235-L249)).

**Affected files.** `crates/plannotator-tui/src/last/locate.rs` and its tests. A complete
per-pane custom-root solution would also affect Herdr's session-reference API and integration
hooks; that repository is an upstream dependency, not an implementation target of this spec.

**Required change.** Honor all host root overrides when they are present in the
Plannotator/Herdr process environment: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `COPILOT_HOME`,
`FACTORY_CONFIG_DIR`, `PI_CODING_AGENT_SESSION_DIR`, `PI_CODING_AGENT_DIR`, `HERMES_HOME`,
`OPENCODE_DB`, and `XDG_DATA_HOME`. Full Windows v1 supports default roots and overrides
present when Herdr starts. An override set only inside one agent pane is explicitly out of
scope until Herdr transports the transcript path or root.

**Risk.** A user can run the correct host in a non-default store and receive the wrong-screen
fallback. Documentation must not claim arbitrary per-pane profiles. Named Hermes and OMP
profiles are included in this limitation.

### G7. Pi's Windows hook drops its exact transcript path

**Evidence.** The Pi integration accepts a session path only when it starts with `/`
([`pi/herdr-agent-state.ts`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/pi/herdr-agent-state.ts#L73-L105)).
The OMP integration correctly accepts both POSIX and Windows absolute paths
([`omp/herdr-agent-state.ts`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/omp/herdr-agent-state.ts#L90-L117)).
On Windows, Pi therefore reports an id. The current TUI ignores that id.

**Affected files.** The ID resolver from G5 provides the in-scope default-root workaround.
The source defect is `herdr/src/integration/assets/pi/herdr-agent-state.ts` in the read-only
upstream Herdr repository.

**Required change.** Implement G5 before enabling the manifest. Separately, Herdr should make
Pi's path check match OMP's `path.posix.isAbsolute(file) || path.win32.isAbsolute(file)` so
custom session directories work. Record the upstream dependency in release notes; do not file
an issue as part of this work.

**Risk.** Default-root Pi can pass through ID lookup. Custom Pi session directories remain
UNVERIFIED and unsupported until a Herdr release contains the hook correction.

### G8. POSIX process inspection remains in fallback discovery

**Evidence.** `parent_pid()` returns `0` off Unix; `process_table()` always invokes `ps`
([`locate.rs`](../crates/plannotator-tui/src/last/locate.rs#L338-L356)). Copilot additionally
uses `ps -o comm=` to validate lock owners
([`locate.rs`](../crates/plannotator-tui/src/last/locate.rs#L252-L277)). On native Windows
these paths either return an empty result or fail. The launcher also calls
`herdr pane process-info` before it can use `agent get`'s exact session
([`cli.rs`](../crates/plannotator-tui/src/cli.rs#L175-L180)). Herdr has Windows process-tree
selection, but exact session identity should not depend on a second, transient process probe.

**Affected files.** `crates/plannotator-tui/src/last/locate.rs`, subprocess tests.

**Required change.** Do not add `wmic`, PowerShell, or `tasklist` parsing for the primary Herdr
path. The launch refactor and exact session identity from G5 eliminate both process probes.
Keep cwd fallback for standalone use. Rename or cfg-gate the POSIX helpers so Windows tests
establish that neither `herdr pane process-info` nor `ps` is required when an id is present.

**Risk.** Standalone `plannotator-tui last --host copilot` without `--session-id` remains
heuristic on Windows. That is outside Full-mode acceptance and must have an error that suggests
`--session-id` rather than silently selecting a different session.

### G9. Keyboard enhancement is intentionally unavailable through Crossterm on Windows

**Evidence.** The TUI queries Crossterm before pushing Kitty keyboard flags
([`cli.rs`](../crates/plannotator-tui/src/cli.rs#L243-L269)). Crossterm 0.29's Windows backend
always returns `Ok(false)` for this query
([`windows.rs`](https://docs.rs/crossterm/0.29.0/src/crossterm/terminal/sys/windows.rs.html#71-77)).
The compose box already accepts Alt+Enter and
Ctrl+J, and advertises Shift+Enter only when the query succeeds
([`compose.rs`](../crates/plannotator-tui/src/app/compose.rs#L1-L49),
[`draw.rs`](../crates/plannotator-tui/src/app/draw.rs#L260-L269)). Herdr says its native input
path can preserve Shift+Enter through ConPTY when the outer terminal distinguishes it
([`windows-beta.mdx`](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/docs/next/website/src/content/docs/windows-beta.mdx#L104-L108)).

**Affected files.** No required production change. Windows live-test documentation and, if
desired, an explicit Windows assertion around the compose hint.

**Required change.** Keep the safe fallback: Enter saves; Alt+Enter and Ctrl+J insert a
newline; the UI does not advertise Shift+Enter. Do not emit a manual Kitty query from the TUI
in this phase.

**Risk.** Windows Terminal binds Alt+Enter to fullscreen in some configurations. Ctrl+J is the
portable fallback and must be exercised live. Native Shift+Enter support remains a future
Crossterm/upstream enhancement.

### G10. OSC 52 and ConPTY UI behavior have no native live proof

**Evidence.** Clipboard delivery is an OSC 52 sequence written to stdout
([`delivery.rs`](../crates/plannotator-tui/src/delivery.rs#L48-L63)). Herdr documents its own
drag-selection copy and ConPTY support, but that does not establish that OSC 52 emitted by a
child pane reaches the outer Windows terminal. Popup, overlay, and split share the same source
runtime, but none has been exercised with this TUI on native Windows.

**Affected files.** No production file until a live run identifies a defect. Validation
documentation only.

**Required change.** Do not claim native Windows clipboard fallback, mouse selection,
overlay restoration, popup modality, resize behavior, or Unicode rendering until the live
checklist passes. A failed OSC 52 check is a real product bug because blocked/unavailable agent
delivery relies on clipboard fallback; record it before release rather than weakening the
contract silently.

**Risk.** This is the largest remaining user-visible uncertainty. CI cannot emulate an outer
Windows terminal, Herdr's ConPTY renderer, and clipboard policy end to end.

### G11. Windows tests are post-merge and the plugin has no Windows CI

**Evidence.** TUI pull requests run Windows Clippy only; full Windows tests are post-merge.
`herdr-annotate` has no checked-in GitHub Actions workflow. Its smoke suite requires Bash and a
live disposable Herdr session.

**Affected files.** `.github/workflows/ci.yml` and Windows test support in this repository;
new `.github/workflows/ci.yml` plus PowerShell/TOML test scripts in `herdr-annotate`.

**Required change.** Make the Windows checks in “Validation plan” pull-request gates in both
repositories. Keep the live Herdr session suite separate because it needs a real desktop host
and agent credentials.

**Risk.** A green CI result proves the binary, installer, manifest structure, path logic, and
argv boundaries. It does not prove ConPTY presentation or host hooks in their real CLIs.

## Per-host Windows verdict

“In scope” means the agent has a meaningful native Windows distribution and Herdr has a
Windows integration. “Current result” describes the baseline before G5–G8.

| Host | Native Windows and store | Herdr identity on Windows | Current Full result | Contract after implementation |
|---|---|---|---|---|
| Claude Code | **In scope.** Config home is `CLAUDE_CONFIG_DIR`, else `%USERPROFILE%\.claude`; transcripts are `projects/<sanitized-cwd>/<session-id>.jsonl`, and the sanitizer removes Windows-reserved punctuation ([env](https://github.com/backnotprop/cc-open/blob/48337af04bb2/utils/envUtils.ts#L5-L14), [transcript](https://github.com/backnotprop/cc-open/blob/48337af04bb2/utils/sessionStorage.ts#L198-L205), [sanitizer](https://github.com/backnotprop/cc-open/blob/48337af04bb2/utils/sessionStoragePortable.ts#L299-L319)). | PowerShell hook reports id and transcript path, but Herdr retains the id for Claude ([hook](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/claude/herdr-agent-state.ps1#L20-L48), [selection](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/agent_resume.rs#L53-L70)). | Default root usually resolves through the reported pid and Claude's `sessions/<pid>.json`. `CLAUDE_CONFIG_DIR` is ignored by the fallback. No live Windows proof. | **Supported at the default root** through ID-first lookup; support the override when inherited by Herdr. Per-pane custom root is out of scope. |
| Codex | **In scope.** `CODEX_HOME`, else `%USERPROFILE%\.codex`; rollouts are `sessions/YYYY/MM/DD/rollout-<colon-free-time>-<thread-id>.jsonl` ([home](https://github.com/openai/codex/blob/0fb559f0f6e2/codex-rs/utils/home-dir/src/lib.rs#L5-L60), [rollout](https://github.com/openai/codex/blob/0fb559f0f6e2/codex-rs/rollout/src/recorder.rs#L1525-L1548)). The official README provides a native PowerShell installer ([README](https://github.com/openai/codex/blob/0fb559f0f6e2/README.md#L14-L40)). | PowerShell hook reports the exact session id. | The TUI ignores it and can select the newest global non-subagent rollout because `CODEX_THREAD_ID` is not normally present in the plugin pane. This is a confirmed logic bug. | **Supported** after passing the id to the existing Codex thread lookup. Custom `CODEX_HOME` must be inherited by Herdr. |
| GitHub Copilot CLI | **In scope.** GitHub lists native Windows with PowerShell 6+ and WinGet/npm installs ([official README](https://github.com/github/copilot-cli)). The official [`@github/copilot` 1.0.68 package](https://www.npmjs.com/package/@github/copilot/v/1.0.68) ships `win32-x64` and `win32-arm64` packages and resolves `COPILOT_HOME`, else `%USERPROFILE%\.copilot`. Session implementation source is not public. | PowerShell hook reports the exact session id. | The TUI ignores the id, runs POSIX `ps`, then falls back by cwd/mtime under `session-state`. Current lock-file continuity is UNVERIFIED against 1.0.68. | **Provisionally supported** by direct `session-state/<id>/events.jsonl` lookup; live current-version fixture is required. No Windows `ps` dependency. |
| Pi | **In scope.** Native Windows requires Bash for agent shell tools, but Pi publishes x64 and ARM64 Windows binaries. Config is `PI_CODING_AGENT_DIR`, else `%USERPROFILE%\.pi\agent`; sessions are under its `sessions` directory with separators and drive colon encoded ([config](https://github.com/badlogic/pi-mono/blob/fa07e7bd92c9/packages/coding-agent/src/config.ts#L488-L521), [sessions](https://github.com/badlogic/pi-mono/blob/fa07e7bd92c9/packages/coding-agent/src/core/session-manager.ts#L472-L488), [Windows](https://github.com/badlogic/pi-mono/blob/fa07e7bd92c9/packages/coding-agent/docs/windows.md#L1-L17)). | The current Herdr hook rejects a `C:\...` path and falls back to id. | The TUI ignores the id and chooses newest by cwd. | **Supported at the default root** after ID lookup. Custom session directories require the upstream Herdr hook correction and remain out of scope until then. |
| Oh My Pi (OMP) | **In scope on x64.** OMP states native Windows without WSL and currently lists `win32-x64` only ([README](https://github.com/can1357/oh-my-pi/blob/72000acfeb90/README.md#L82-L95), [native statement](https://github.com/can1357/oh-my-pi/blob/72000acfeb90/README.md#L197-L200), [platforms](https://github.com/can1357/oh-my-pi/blob/72000acfeb90/README.md#L453-L459)). Current sessions are `%USERPROFILE%\.omp\agent\sessions/<encoded-cwd>/...`; the encoding is no longer identical to Pi for home/temp paths, and profiles use `.omp/profiles/<name>/agent` ([session spec](https://github.com/can1357/oh-my-pi/blob/72000acfeb90/docs/session.md#L35-L63), [profiles](https://github.com/can1357/oh-my-pi/blob/72000acfeb90/docs/context-files.md#L29-L35)). | TypeScript hook accepts a Windows absolute transcript path and Herdr preserves Pi/OMP paths. | Exact-path Herdr flow works in source. Standalone fallback still assumes Pi's older encoding and the default profile. Live Windows is UNVERIFIED. | **Supported for Herdr-reported paths.** Update standalone fallback or explicitly error rather than guessing when no path/id exists. ARM64 host availability is out of scope until OMP publishes it. |
| Droid | **In scope.** Factory publishes native Windows x64 and ARM64 downloads and a PowerShell installer ([CLI reference](https://docs.factory.ai/droid-cli/cli-reference)). Windows settings are `%USERPROFILE%\.factory\settings.json` ([settings](https://docs.factory.ai/droid-cli/settings)); the official SDK reads local sessions from `~/.factory/sessions/` ([SDK](https://github.com/Factory-AI/droid-sdk-typescript#listing-sessions)). The proprietary CLI's JSONL schema remains source-unavailable. | PowerShell hook reports the exact session id. | The TUI ignores it and selects the newest file for the cwd slug. | **Provisionally supported** through exact ID lookup. The current official CLI must supply a sanitized live fixture before release; schema compatibility remains UNVERIFIED until then. |
| Hermes | **In scope.** Hermes states native Windows support and `%LOCALAPPDATA%\hermes` ([README](https://github.com/NousResearch/hermes-agent/blob/2c232761e4c1/README.md#L43-L59)). `HERMES_HOME` overrides the platform default and `state.db` is inside it ([constants](https://github.com/NousResearch/hermes-agent/blob/2c232761e4c1/hermes_constants.py#L53-L74), [database](https://github.com/NousResearch/hermes-agent/blob/2c232761e4c1/hermes_state.py#L392-L409)). | Python hook reports `sessions.id` and has a Windows no-console subprocess branch ([hook](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/hermes/__init__.py#L23-L58)). | The TUI already consumes the id and uses the correct Windows default root. | **Supported at the default root** subject to live SQLite/WAL proof. A named profile whose `HERMES_HOME` is not inherited by Herdr is out of scope. |
| OpenCode 1 | **In scope.** Scoop/Chocolatey installers and Windows binaries are published ([README](https://github.com/anomalyco/opencode/blob/ad905f8e6c8c/README.md#L46-L61)). Data is `xdgData/opencode`, DB is `OPENCODE_DB` or `opencode*.db`, and stored Windows paths use forward slashes ([global](https://github.com/anomalyco/opencode/blob/ad905f8e6c8c/packages/core/src/global.ts#L1-L29), [database](https://github.com/anomalyco/opencode/blob/ad905f8e6c8c/packages/core/src/database/database.ts#L39-L55), [paths](https://github.com/anomalyco/opencode/blob/ad905f8e6c8c/packages/core/src/database/path.ts#L5-L40)). | JS integration reports the selected session id over a Windows named pipe ([hook](https://github.com/ogulcancelik/herdr/blob/7b675f42af35/src/integration/assets/opencode/herdr-tui-session.js#L13-L49)). | The TUI already consumes the id and normalizes Windows path comparison. | **Supported** subject to live DB/WAL proof. `OPENCODE_DB` must be inherited by Herdr when custom. |
| OpenCode 2 | **In scope.** Same native distribution and root. Current source writes `session`, legacy `message`/`part`, and `session_message` together ([schema](https://github.com/anomalyco/opencode/blob/ad905f8e6c8c/packages/core/src/session/sql.ts#L22-L138)); the reader's compatibility path must be kept under fixture coverage. | Same selected-session id as OpenCode 1. | ID lookup and Windows normalization exist. Current source no longer names a `session_v2` table, so that branch of the reader is historical compatibility rather than the current schema. | **Supported** only after a fixture generated from the pinned current schema passes on Windows. Keep older `session_v2` compatibility; do not make it the only path. |

No named host is excluded for lack of native Windows support. OMP is x64-only at the surveyed
revision; every other named host meaningfully runs on native Windows. This verdict does not
extend to WSL-only installations, cloud sessions without a local transcript, or per-pane
custom storage roots that Herdr does not transport.

## Manifest and installer design

### Alignment with Lite PR #27

Full mode adopts these Lite decisions:

- one Unix build now and a platform-gated Windows build once G1's Herdr dependency lands;
- `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ...` on Windows;
- `RuntimeInformation.OSArchitecture`, not environment-string parsing;
- a common relative `./bin/<name>.exe` program on all three operating systems;
- direct argv for actions; panes retain a plugin-root wrapper until Herdr resolves pane
  programs independently from their cwd;
- `Invoke-WebRequest`, exact `SHA256SUMS` asset matching, `Get-FileHash`, a unique temporary
  directory, replacement only after verification, and `finally` cleanup.

The pattern is visible in PR #27's
[`herdr-plugin.toml`](https://github.com/plannotator/herdr-annotate/blob/0c5231f8ca6fa10c57ef2b91fe1224b5e5473690/lite-rs/herdr-plugin.toml#L9-L27)
and
[`fetch-herdr-annotate.ps1`](https://github.com/plannotator/herdr-annotate/blob/0c5231f8ca6fa10c57ef2b91fe1224b5e5473690/lite-rs/scripts/fetch-herdr-annotate.ps1#L1-L52).

Full mode diverges in one deliberate way: a remote download, checksum-list, unsupported
architecture, or checksum failure remains nonfatal to the plugin installation. Annotate Lite
must stay usable when the optional Full binary cannot be installed. A bad explicit
`PLANNOTATOR_TUI_BIN` remains fatal because the user requested that exact local file.

### PowerShell fetch contract

`scripts/fetch-plannotator-tui.ps1` performs these steps in order:

1. Set `$ErrorActionPreference = "Stop"`; change to `Join-Path $PSScriptRoot ".."`.
2. Read and trim `plannotator-tui.version`; an empty pin is a fatal repository error.
3. Set destination `bin/plannotator-tui.exe` and stamp
   `bin/plannotator-tui.version`; create `bin`.
4. If destination exists, stamp equals the pin, and `PLANNOTATOR_TUI_BIN` is absent, print the
   installed version and exit 0 without network access.
5. If `PLANNOTATOR_TUI_BIN` is set, require a leaf file. Copy it to a unique temporary file in
   `bin`, replace the destination, write the pin without a newline, and exit 0. Any failure in
   this branch exits nonzero.
6. Map architecture using G3. Unsupported architecture calls the nonfatal warning path.
7. Set asset `plannotator-tui-<target>.exe` and release base
   `https://github.com/plannotator/plannotator-tui/releases/download/v<version>`. A documented
   test-only `PLANNOTATOR_TUI_RELEASE_BASE` may override the base so CI can use a loopback
   fixture server rather than GitHub.
8. Create a GUID-named temporary directory. Download the asset and `SHA256SUMS` with
   `Invoke-WebRequest -UseBasicParsing`.
9. Select exactly one checksum line whose final whitespace-delimited field equals the asset.
   Reject no match or multiple matches. Compare lowercase SHA-256 values with `Get-FileHash`.
10. Copy the verified file to a unique `bin/*.tmp`, replace the destination, then write the
    stamp. The stamp is never written before a successful replacement.
11. Remove the temporary directory in `finally`.
12. Catch remote/architecture/checksum failures, write a warning that Full review is
    unavailable until plugin reinstall/update, preserve any prior destination and stamp, and
    exit 0. Do not catch the empty pin or invalid local override.

The Unix fetcher must preserve the same failure policy and change its installed filename to
`plannotator-tui.exe`. Its version stamp stays the pin alone; plugin roots are platform-local,
so recording the asset in the stamp adds no useful migration behavior.

### Version ordering

Publish in this order:

1. merge TUI session resolution and Windows pull-request tests;
2. publish a TUI release with both Windows assets and checksums;
3. update `herdr-annotate/plannotator-tui.version` to that release;
4. merge the PowerShell fetcher and the still-macOS/Linux manifest changes;
5. land and release G1's upstream Herdr pane-resolution dependency;
6. enable the Windows build and all six Full gates, then perform the live matrix from an
   installed GitHub plugin, not a developer checkout.

The plugin must never pin a release that lacks either advertised Windows asset.

## Validation plan

### CI-provable: `plannotator-tui`

Change the pull-request `check-windows` job to run on `windows-latest` with Rustfmt and Clippy,
then execute exactly:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --release -p plannotator-tui
& .\target\release\plannotator-tui.exe --version
```

The Rust suite must add Windows-path cases for every G5 ID resolver, including drive-letter
case, backslashes, a root containing spaces and non-ASCII text, malformed IDs, exact-ID miss,
and “exact miss does not choose newest.” Existing macOS/Linux fixtures remain unchanged.

Add a Windows subprocess probe, not only mocked command construction:

- Compile a small checked-in `tests/support/fake-herdr.rs` with `rustc` to
  `$RUNNER_TEMP\fake herdr\herdr.exe`.
- The helper records argv as JSON and returns fixture JSON for `agent get` and
  `pane process-info`. It never parses a command line string.
- Run the built TUI launcher with `HERDR_ENV=1`, `HERDR_BIN_PATH` set to that executable, a
  plugin root and cwd containing spaces, a `file:///C:/...` clicked URL, and an agent session
  id.
- Assert exact argv elements for `plugin pane open`, `--cwd`, each `--env`, and
  `agent prompt`; a feedback body containing newlines, quotes, `%`, `&`, `|`, and non-ASCII
  text must remain one argument.
- Assert that the ID-first Herdr launch never calls `pane process-info`, that `last --print`
  never attempts `ps`, and that the named transcript wins when a newer unrelated transcript
  exists.

The release workflow separately cross-builds `aarch64-pc-windows-msvc`. It cannot run that
binary on `windows-latest`; build success is the CI claim.

### CI-provable: `herdr-annotate`

Add a `windows-latest` pull-request job with these independent checks:

1. **Local override.** Put a synthetic source file under a path with spaces, set
   `PLANNOTATOR_TUI_BIN`, run the PowerShell fetcher, and assert destination bytes and stamp.
   Unset the variable, run again, and assert the idempotent path performs no replacement.
   A missing explicit override must exit nonzero.
2. **Download and checksum.** Serve a synthetic Windows asset and `SHA256SUMS` from a loopback
   Python HTTP server; set `PLANNOTATOR_TUI_RELEASE_BASE`; run the fetcher and assert bytes,
   stamp, and selected target. Repeat with a wrong checksum and assert exit 0, warning text,
   and no modification to the previously installed file or stamp.
3. **Manifest parse.** Use Python 3.11 `tomllib` to parse `herdr-plugin.toml`. Assert top-level
   Windows support for Lite; no effective Windows Full build; exact macOS/Linux gates for
   `build`, `doc`, `open`, `open-link`, `last`, and `markdown-file`; the plugin-root wrapper for
   `doc`; and exact direct relative `.exe` argv for the actions. Compare those commands with the
   development manifest while preserving its intentional Windows staging/action entries.
4. **Installed manifest.** Use a pinned Herdr 0.8.x Windows binary to link the plugin and list
   its actions/panes. Assert all Lite and Full entries parse, while every Full action plus `doc`
   retains macOS/Linux platforms and excludes Windows. This proves Herdr preserves the gates
   rather than only proving that local TOML parsing succeeds.
5. **Unix regression.** The existing Unix job installs the renamed `.exe` destination and
   executes `--version`; the action argv and pane wrapper must name that staged file.

The Bash `scripts/smoke.sh` remains the Unix live suite. Do not translate its global-plugin
mutation and disposable-session logic into CI without a real Herdr host.

### Needs a live native Windows + Herdr host

Run this matrix after CI is green. Record Herdr version, Windows version, architecture,
terminal, shell, plugin commit, TUI version, and host version for every result.

| Area | Required live checks |
|---|---|
| Install | Fresh GitHub install, upgrade from the last mac/Linux-only manifest, reinstall after deleting the binary, checksum failure behavior, local override, paths with spaces and non-ASCII text |
| Entry points | `Ctrl+B O`, `Ctrl+B Shift+O`, Ctrl-click Markdown `file://`; verify the clicked `C:\...` file, focused pane, cwd, and delivery target |
| Placement | Overlay fills pane area and restores prior zoom/focus on exit; split targets the agent pane; popup is modal/singleton and closes cleanly; repeated open/close leaves no orphan process |
| TUI | Markdown and Unicode render, resize/reflow, tree navigation, mouse drag/select, comment edit/delete, paste, `E`, quit confirmation |
| Keyboard | Enter saves; Ctrl+J inserts newline; Alt+Enter behavior is documented for the chosen terminal; Shift+Enter is not advertised unless actually detected |
| Delivery | Multi-paragraph feedback is one agent prompt; `ok`, `agent_blocked`, missing pane, and closed pane outcomes match the existing contract |
| Clipboard | Standalone copy and Herdr fallback emit OSC 52 that reaches the outer Windows clipboard; test Windows Terminal and one other ConPTY-capable terminal |
| Hosts | For every in-scope row, run two concurrent sessions in the same cwd, select the older one, and prove `last` opens the Herdr-reported session rather than newest; repeat after `/new` or equivalent |
| Storage | Default root for every host; inherited custom root for every advertised override; named Hermes/OMP and Pi custom-dir cases remain expected failures until the upstream path contract exists |
| ARM64 | Native ARM64 Herdr and TUI install, launch, render, and one exact-session round trip; x64 emulation does not satisfy this row |

Until this matrix is recorded, all placement, input, OSC 52, and end-to-end host claims are
UNVERIFIED. Source and CI evidence may not relabel them.

## Ordered implementation plan

Sizes are implementation effort, not elapsed release time.

1. **Exact-session resolution (M).** Implement G5, root overrides, safe ID validation, and
   “no fallback after exact miss” across `plannotator-tui` and `plannotator-tui-hosts`. Update
   the launcher so an exact Herdr session does not require process-info. Update OMP's stale
   fallback behavior or make the unsupported fallback explicit.
2. **Windows host/path tests (M).** Add pure fixtures for all hosts and run the full workspace
   suite on Windows pull requests. Include current OpenCode schema, current Copilot/Droid
   sanitized fixtures, and default/inherited-root cases.
3. **Windows subprocess proof (M).** Add the fake Herdr executable and PowerShell harness;
   prove argv and feedback boundaries through real `CreateProcess` calls.
4. **Windows ARM64 release (M, UNVERIFIED until probed).** Add the release target, solve any
   SQLite cross-compile issue, publish checksums, and run on a native ARM64 host.
5. **Development manifest parity (S).** Stage Cargo output under the common `.exe` name, use
   direct argv for actions, and retain a plugin-root shell wrapper for the macOS/Linux pane.
6. **Distributed PowerShell installer (M).** Implement the frozen fetch contract and CI tests;
   rename the Unix installed destination.
7. **Distributed manifest (S).** Keep the Unix build and all six Full gates on macOS/Linux,
   switch actions to direct argv, and retain the runtime wrapper for the pane.
8. **Plugin structural/install CI (M).** Add local-override, loopback-download, checksum,
   `tomllib`, pinned-Herdr, and Unix regression jobs.
9. **Upstream Herdr dependencies (S to specify, external to these repos).** Resolve explicit
   relative pane programs from the plugin root without changing `--cwd`, permit one portable
   `doc` entry, and correct Pi's Windows path hook from G7. Raise `min_herdr_version` before
   ungating Full mode.
10. **Native Windows x64 qualification (L, human scheduling).** Run the full live matrix on a
   dedicated Windows host and record failures without broadening scope.
11. **Native Windows ARM64 qualification (M, human scheduling).** Run the ARM64 subset only
    after a native asset exists.
12. **Upstream follow-up (S to specify, external to these repos).** Design
    transcript-path/custom-root transport for other hosts. Do not silently claim profile
    support while this is pending.

The manifest must not be ungated before steps 1–9 are green. A release may be tagged only
after step 10 passes for x64. Advertise ARM64 only after step 11 passes.

## Open questions for the maintainer

1. **Must Full mode support native Windows ARM64 at first release?**
   **Recommended: yes.** Herdr and Lite establish a two-architecture Windows precedent.
   Shipping x64-only is a defensible preview, but then the manifest/release notes must say
   “Windows x64” and the PowerShell installer must reject ARM64 rather than use emulation.

2. **Should a remote Full-binary download failure abort the whole Annotate install?**
   **Recommended: no.** Preserve today's Full fetch policy so Lite remains usable. Keep empty
   pins and invalid explicit local overrides fatal; preserve an older verified binary on remote
   failure.

3. **Should runtime commands retain a script solely for a friendlier missing-binary error?**
   **Recommended: retain it for Unix panes only until G1 lands.** Actions can use direct
   relative `.exe` argv because Herdr resolves them against the plugin root. Panes resolve from
   `--cwd`, so the wrapper is required to anchor the executable through `HERDR_PLUGIN_ROOT`.

4. **What storage overrides are part of Windows Full v1?**
   **Recommended: default roots plus overrides inherited by the Herdr process.** Per-pane
   profiles/custom roots require a richer Herdr session-reference contract. State that limit
   rather than scanning unrelated roots or selecting newest.

5. **Should an unresolved exact session id fall back to cwd/newest?**
   **Recommended: no.** Exact identity is authoritative. Show Herdr's screen fallback with a
   diagnostic; never review a different conversation without telling the user.

6. **Does Pi's Windows hook defect block release?**
   **Recommended: it blocks custom Pi session directories, not the default root.** Ship only
   after ID-first lookup passes the default-root live test, and list custom paths as unsupported
   until a Herdr release accepts Windows absolute transcript paths.

7. **May the release claim clipboard fallback based on Herdr's Windows copy support?**
   **Recommended: no.** Herdr drag-copy and child-emitted OSC 52 are different paths. Require
   the live OSC 52 check because delivery failure depends on it.

8. **Should `min_herdr_version` increase above 0.8.0?**
   **Recommended: keep 0.8.0 while Full remains gated on Windows, then raise it.** Windows Full
   requires the first Herdr release that resolves pane programs against the plugin root while
   preserving `--cwd`. Cite that release and commit when the six gates are removed.

9. **Should this work add a Windows process-table implementation?**
   **Recommended: no for Full mode.** Herdr already reports exact identity. Add native process
   inspection only as a separately scoped improvement to standalone `last` after exact-session
   support is correct.

10. **What does “Windows supported” mean before the live matrix exists?**
    **Recommended: CI-supported preview, not released Full support.** Keep every live-only row
    marked UNVERIFIED in the PR and release notes until a human records it on native Windows.
