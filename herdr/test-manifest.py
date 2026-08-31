#!/usr/bin/env python3
"""Check the development manifest's staged Full-mode contract."""

from __future__ import annotations

import tomllib
from pathlib import Path


PROGRAM = "./bin/plannotator-tui.exe"
PANE_COMMAND = [
    "sh",
    "-c",
    'exec "$HERDR_PLUGIN_ROOT/bin/plannotator-tui.exe" herdr pane',
]
ACTION_COMMANDS = {
    ("actions", "open"): [PROGRAM, "herdr", "open"],
    ("actions", "open-link"): [PROGRAM, "herdr", "open"],
    ("actions", "last"): [PROGRAM, "herdr", "last"],
}
LINK_HANDLERS = {"markdown-file": "open-link"}
DEVELOPMENT_BUILDS = [
    ["cargo", "build", "--release", "--manifest-path", "../Cargo.toml"],
    ["bash", "stage-plannotator-tui.sh"],
    [
        "powershell.exe",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        "stage-plannotator-tui.ps1",
    ],
]


def fail(path: Path, message: str) -> None:
    raise AssertionError(f"{path}: {message}")


def platforms(manifest: dict[str, object], entry: dict[str, object]) -> set[str]:
    platforms = entry.get("platforms", manifest.get("platforms", []))
    if not isinstance(platforms, list) or not all(
        isinstance(platform, str) for platform in platforms
    ):
        raise AssertionError(f"invalid platforms: {platforms!r}")
    return set(platforms)


def entry(
    path: Path,
    manifest: dict[str, object],
    table: str,
    entry_id: str,
) -> dict[str, object]:
    entries = manifest.get(table, [])
    if not isinstance(entries, list):
        fail(path, f"[[{table}]] is not an array")
    matches = [
        item
        for item in entries
        if isinstance(item, dict) and item.get("id") == entry_id
    ]
    if len(matches) != 1:
        fail(path, f"expected one {table}.{entry_id}, found {len(matches)}")
    return matches[0]


def check_entries(path: Path) -> None:
    with path.open("rb") as handle:
        manifest = tomllib.load(handle)

    if set(manifest.get("platforms", [])) != {"macos", "linux", "windows"}:
        fail(path, "top-level platforms must be macos, linux, and windows")

    pane = entry(path, manifest, "panes", "doc")
    if platforms(manifest, pane) != {"macos", "linux"}:
        fail(path, "panes.doc must be limited to macOS and Linux")
    if pane.get("command") != PANE_COMMAND:
        fail(
            path,
            f"panes.doc command is {pane.get('command')!r}, expected {PANE_COMMAND!r}",
        )

    for (table, entry_id), expected in ACTION_COMMANDS.items():
        item = entry(path, manifest, table, entry_id)
        if "windows" not in platforms(manifest, item):
            fail(path, f"{table}.{entry_id} must retain Windows support")
        command = item.get("command")
        if command != expected:
            fail(path, f"{table}.{entry_id} command is {command!r}, expected {expected!r}")

    for entry_id, expected_action in LINK_HANDLERS.items():
        handler = entry(path, manifest, "link_handlers", entry_id)
        if "windows" not in platforms(manifest, handler):
            fail(path, f"link_handlers.{entry_id} must retain Windows support")
        action = handler.get("action")
        if action != expected_action:
            fail(path, f"link_handlers.{entry_id} action is {action!r}, expected {expected_action!r}")


def check_development_builds(path: Path) -> None:
    with path.open("rb") as handle:
        manifest = tomllib.load(handle)
    builds = manifest.get("build", [])
    if not isinstance(builds, list):
        fail(path, "[[build]] is not an array")
    commands = [item.get("command") for item in builds if isinstance(item, dict)]
    if commands != DEVELOPMENT_BUILDS:
        fail(path, f"development build commands are {commands!r}")
    if "windows" not in platforms(manifest, builds[0]):
        fail(path, "Cargo build does not run on Windows")
    if set(builds[1].get("platforms", [])) != {"macos", "linux"}:
        fail(path, "Unix staging command must run on macOS and Linux only")
    if set(builds[2].get("platforms", [])) != {"windows"}:
        fail(path, "PowerShell staging command must run on Windows only")


def main() -> None:
    path = Path(__file__).with_name("herdr-plugin.toml")
    check_entries(path)
    check_development_builds(path)


if __name__ == "__main__":
    main()
