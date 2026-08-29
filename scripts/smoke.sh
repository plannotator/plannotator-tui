#!/usr/bin/env bash
# Smoke-test the standalone install channels for a released version:
# the release asset (with checksum), cargo install, and Homebrew (trust + install + upgrade).
#
#   bash scripts/smoke.sh [version]     default: the version in Cargo.toml
set -euo pipefail
version="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
base="https://github.com/plannotator/plannotator-tui/releases/download/v$version"
case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) target=aarch64-apple-darwin ;; Darwin/x86_64) target=x86_64-apple-darwin ;;
  Linux/x86_64) target=x86_64-unknown-linux-gnu ;; Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
  *) echo "unsupported platform" >&2; exit 2 ;;
esac
fail=0; ok() { echo "  ok   $*"; }; bad() { echo "  FAIL $*" >&2; fail=$((fail+1)); }

echo "== release asset v$version ($target)"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/plannotator-tui-$target" "$base/plannotator-tui-$target"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
(cd "$tmp" && grep " plannotator-tui-$target\$" SHA256SUMS | (sha256sum -c - 2>/dev/null || shasum -a 256 -c -) >/dev/null) && ok "checksum" || bad "checksum"
chmod +x "$tmp/plannotator-tui-$target"
[ "$("$tmp/plannotator-tui-$target" --version)" = "plannotator-tui $version" ] && ok "runs, reports $version" || bad "version mismatch"
"$tmp/plannotator-tui-$target" --snapshot samples/plugins.md 60 3 | grep -q "Herdr plugins" && ok "renders a document" || bad "render"

echo "== cargo install"
cargo install plannotator-tui --version "$version" --locked --root "$tmp/cargo" -q 2>/dev/null \
  && [ "$("$tmp/cargo/bin/plannotator-tui" --version)" = "plannotator-tui $version" ] && ok "cargo install $version" || bad "cargo install"

if command -v brew >/dev/null 2>&1; then
  echo "== homebrew (trust, install, upgrade)"
  had=0; brew list plannotator-tui >/dev/null 2>&1 && had=1
  brew trust plannotator/tap >/dev/null 2>&1 || true
  brew install -q plannotator/tap/plannotator-tui >/dev/null 2>&1 || brew upgrade -q plannotator-tui >/dev/null 2>&1 || true
  [ "$(plannotator-tui --version 2>/dev/null)" = "plannotator-tui $version" ] && ok "brew has $version" || bad "brew version: $(plannotator-tui --version 2>&1)"
  [ "$had" = 1 ] || brew uninstall -q plannotator-tui >/dev/null 2>&1 || true
fi
echo "== result: $fail failure(s)"; [ "$fail" -eq 0 ]
