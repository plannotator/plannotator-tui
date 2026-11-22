# plannotator-tui — engineering notes

A small open-source terminal application. Optimize for a reader who has never seen the
code: clear module boundaries, boring Rust, no cleverness that needs a comment to defend it.

## Shape

```
crates/plannotator-tui-schema   annotation + anchor types and resolvers. Wire-exact with
                          Plannotator Workspaces. No I/O, no UI, no async. Pure functions.
crates/plannotator-tui          the app: parse → layout → draw → input. Talks to the schema crate,
                          never the other way round. Network clients live in their own
                          modules behind a trait so the app has one seam per external system.
herdr/                    the Herdr plugin manifest. The launcher it runs is
                          `plannotator-tui herdr open` (src/herdr/); no shell logic here.
                          Rex needs no manifest: its launchers and delivery
                          (`plannotator-tui rex …`, src/rex/) drive the `rex` CLI, and
                          Rex Annotate is src/rex/annotate.lua, embedded and written
                          by `plannotator-tui rex install`.
                          herdr-core needs no manifest: its launchers and delivery
                          (`plannotator-tui hc …`, src/hc/) drive the `hc` CLI.
```

Markdown parsing is `pulldown-cmark`; rendering to styled text is `tui-markdown`. We never
interpret markdown ourselves. Anything that needs to know "what is a heading" is a bug.

## Rules that are enforced (see `Cargo.toml` workspace lints)

- `unsafe` is forbidden. `unwrap`/`expect`/`panic!`/`todo!` warn — production code returns
  errors or handles the `None`. Indexing warns: use `.get()`, iterators, or slice patterns.
- `println!`/`eprintln!` warn outside the CLI entry points; those opt out locally with a
  one-line `#[allow]` and a reason.
- `cargo fmt --all` and `cargo clippy --workspace --all-targets` clean before commit.

## Rules that are judgement

- **Files stay small.** A module over ~300 lines is a signal to split by responsibility, not
  to add a second `impl` block. Tests live next to the code they test; when a test module
  outgrows the code, move it to `tests/` for that crate.
- **Tests are for behaviour that can regress.** One test per invariant, named for the
  invariant. No tests that restate the implementation, no mocks of our own types, no test
  helpers that need their own tests. Prefer a real temp dir over a fake filesystem, a real
  `TestBackend` over a fake terminal, real fixtures over builders.
- **Performance is structural, not micro.** Render once per block and cache; wrap on
  resize; touch only visible rows per frame; never reparse a whole document on an edit.
  Measure with `plannotator-tui --bench` before and after anything that changes those paths.
- **Errors carry context, not stack traces.** `anyhow` at the app boundary, typed errors in
  the schema crate. A user-facing failure names the file and what was being done.
- **No abstraction without a second caller.** No traits for one impl, no generics for one
  type, no builders for structs with four fields. A trait is justified only at a seam to an
  external system (Workspaces, Herdr, the clipboard) where a test needs a stand-in.
- **Ownership over cleverness.** Owned `String`s in long-lived structs; borrow in function
  signatures. Lifetimes in public types need a reason.
- **Commits are small and describe intent**, lowercase conventional style:
  `feat(schema): anchor resolution by rendered quote`.

## Tests share one process

`cargo test` runs a test binary's tests at once, as threads of one process. They share
ports, temp paths and file descriptors, and a child any test spawns holds a copy of every
open descriptor until it execs, close-on-exec ones included. A test that passes alone can
then fail once in a hundred runs or so.

- **Write a fake command with `test_support::write_script`** (`tests/support` for
  integration tests), never `fs::write`:
  a fork on another thread while the file is open keeps a write handle until it execs, and
  running the script meanwhile fails with "Text file busy".
- **A close doesn't reach the other end at once.** A pipe hangs up, and a pty's terminal
  sees its controller go, only once every copy is closed. Shut a socket down instead where
  you can; otherwise poll with a deadline. Open raw descriptors (`rustix`) close-on-exec
  atomically, never with a later `fcntl`: a child forked in between keeps its copy past
  exec, for as long as it runs.
- **Don't bind and release a port** to get a free or a closed one: a test running alongside
  can be handed it. Keep the listener, or use port 1 for one nothing listens on.
- **Paths are unique per test, not just per process:** the pid, a tag naming the test, and
  a counter if a helper can run twice with one tag.
- **A flake is a bug until shown otherwise**, sometimes in production code. Find the race;
  no retries, sleeps or `#[ignore]` as a fix.
- **Loop a new test that spawns processes or uses ports, pipes or ptys:** 100 runs of its
  whole test binary, unfiltered, so the tests that fork beside it run too.

## Data contract

Annotations are the Workspaces wire shape. The anchor object is opaque to the server; the
web client reads `originalText` (rendered text). Our fields ride alongside. See
`crates/plannotator-tui-schema/src/lib.rs` for the source of truth; do not redefine these types
elsewhere.
