#!/bin/sh
# Run against a musl release binary on the host or inside a minimal Linux container.
# No Rust toolchain, compiler, or interactive terminal is required.
set -eu
binary="${1:?usage: sh scripts/smoke-musl.sh <binary>}"
case "$binary" in
/*) ;;
*) binary="$PWD/$binary" ;;
esac
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
test "$("$binary" --version)" = "plannotator-tui $version"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export PLANNOTATOR_DATA_DIR="$tmp/data"
export PLANNOTATOR_FEEDBACK_HISTORY=0
document="$tmp/smoke.md"
printf '# Musl compatibility test\n\nThis binary runs without glibc.\n' >"$document"

test "$("$binary" --export "$document")" = "No annotations."
"$binary" --annotate "$document" 'without glibc' 'Musl smoke test feedback'
"$binary" --export "$document" >"$tmp/feedback.md"
grep -Fq '# Annotations on smoke.md' "$tmp/feedback.md"
grep -Fq 'Comment on: "without glibc"' "$tmp/feedback.md"
grep -Fq '> Musl smoke test feedback' "$tmp/feedback.md"
"$binary" --snapshot "$document" 80 24 0 >"$tmp/snapshot.txt"
grep -Fq 'Musl compatibility test' "$tmp/snapshot.txt"
echo "musl smoke test passed"
