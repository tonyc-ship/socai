#!/usr/bin/env python3
"""Update the desktop app version across release metadata files."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+$")


def update_json_version(path: Path, version: str) -> None:
    data = json.loads(path.read_text())
    data["version"] = version
    path.write_text(json.dumps(data, indent=2) + "\n")


def update_app_cargo_toml(path: Path, version: str) -> None:
    text = path.read_text()
    text, count = re.subn(
        r'(?m)^version = "[^"]+"',
        f'version = "{version}"',
        text,
        count=1,
    )
    if count != 1:
        raise SystemExit(f"expected exactly one package version in {path}")
    path.write_text(text)


def update_cargo_lock(path: Path, version: str) -> None:
    if not path.exists():
        return

    text = path.read_text()
    text, count = re.subn(
        r'(name = "socai_app"\nversion = ")[^"]+("\n)',
        rf'\g<1>{version}\2',
        text,
        count=1,
    )
    if count != 1:
        raise SystemExit(f"expected exactly one socai_app package entry in {path}")
    path.write_text(text)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: set-app-version.py VERSION")

    version = sys.argv[1]
    if not SEMVER_RE.fullmatch(version):
        raise SystemExit(f"version must be strict MAJOR.MINOR.PATCH semver, got: {version}")

    update_json_version(Path("app/package.json"), version)
    update_json_version(Path("app/src-tauri/tauri.conf.json"), version)
    update_app_cargo_toml(Path("app/src-tauri/Cargo.toml"), version)
    update_cargo_lock(Path("Cargo.lock"), version)


if __name__ == "__main__":
    main()
