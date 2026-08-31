#!/usr/bin/env bash
set -euo pipefail

plugin_root="$(cd "$(dirname "$0")" && pwd)"
repository_root="$(cd "$plugin_root/.." && pwd)"

mkdir -p "$plugin_root/bin"
# Remove first: cp over an existing signed binary reuses the inode and invalidates the
# macOS code signature, and the next exec is killed.
rm -f "$plugin_root/bin/plannotator-tui.exe"
cp "$repository_root/target/release/plannotator-tui" \
  "$plugin_root/bin/plannotator-tui.exe"
