#!/usr/bin/env bash
set -euo pipefail

plugin_root="$(cd "$(dirname "$0")" && pwd)"
repository_root="$(cd "$plugin_root/.." && pwd)"

mkdir -p "$plugin_root/bin"
cp "$repository_root/target/release/plannotator-tui" \
  "$plugin_root/bin/plannotator-tui.exe"
