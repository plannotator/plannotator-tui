#!/usr/bin/env python3
"""Check the Windows Full entrypoint contract across one or more manifests."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path


PROGRAM = "./bin/plannotator-tui.exe"
COMMANDS = {
    ("panes", "doc"): [PROGRAM, "herdr", "pane"],
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


def supports_windows(manifest: dict[str, object], entry: dict[str, object]) -> bool:
    platforms = entry.get("platforms", manifest.get("platforms", []))
    return isinstance(platforms, list) and "windows" in platforms


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
        if isinstance(item, dict)
        and item.get("id") == entry_id
        and supports_windows(manifest, item)
    ]
    if len(matches) != 1:
        fail(path, f"expected one Windows {table}.{entry_id}, found {len(matches)}")
    return matches[0]


def shape(path: Path) -> dict[str, tuple[str, ...]]:
    with path.open("rb") as handle:
        manifest = tomllib.load(handle)

    if set(manifest.get("platforms", [])) != {"macos", "linux", "windows"}:
        fail(path, "top-level platforms must be macos, linux, and windows")

    result: dict[str, tuple[str, ...]] = {}
    for (table, entry_id), expected in COMMANDS.items():
        command = entry(path, manifest, table, entry_id).get("command")
        if command != expected:
            fail(path, f"{table}.{entry_id} command is {command!r}, expected {expected!r}")
        result[entry_id] = tuple(expected[1:])

    for entry_id, expected_action in LINK_HANDLERS.items():
        action = entry(path, manifest, "link_handlers", entry_id).get("action")
        if action != expected_action:
            fail(path, f"link_handlers.{entry_id} action is {action!r}, expected {expected_action!r}")
        result[entry_id] = (expected_action,)

    return result


def check_development_builds(path: Path) -> None:
    with path.open("rb") as handle:
        manifest = tomllib.load(handle)
    builds = manifest.get("build", [])
    if not isinstance(builds, list):
        fail(path, "[[build]] is not an array")
    commands = [item.get("command") for item in builds if isinstance(item, dict)]
    if commands != DEVELOPMENT_BUILDS:
        fail(path, f"development build commands are {commands!r}")
    if not supports_windows(manifest, builds[0]):
        fail(path, "Cargo build does not run on Windows")
    if set(builds[1].get("platforms", [])) != {"macos", "linux"}:
        fail(path, "Unix staging command must run on macOS and Linux only")
    if set(builds[2].get("platforms", [])) != {"windows"}:
        fail(path, "PowerShell staging command must run on Windows only")


def main() -> None:
    paths = [Path(value) for value in sys.argv[1:]] or [
        Path(__file__).with_name("herdr-plugin.toml")
    ]
    reference = shape(paths[0])
    check_development_builds(paths[0])
    for path in paths[1:]:
        candidate = shape(path)
        if candidate != reference:
            fail(path, f"Full entrypoint tails differ: {candidate!r} != {reference!r}")


if __name__ == "__main__":
    main()
