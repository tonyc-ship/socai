#!/usr/bin/env python3
"""Package the standalone socai CLI release archive."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import tarfile
import tempfile
from datetime import datetime, timezone
from pathlib import Path

SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+$")
ARCHIVE_NAME = "socai-cli-macos-universal.tar.gz"
CHECKSUM_NAME = f"{ARCHIVE_NAME}.sha256"


def utc_timestamp() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def add_file(tar: tarfile.TarFile, path: Path, arcname: str, mode: int) -> None:
    info = tar.gettarinfo(str(path), arcname=arcname)
    info.mode = mode
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    with path.open("rb") as source:
        tar.addfile(info, source)


def write_checksum(archive_path: Path, checksum_path: Path) -> None:
    digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path.write_text(f"{digest}  {archive_path.name}\n")


def existing_file(value: str) -> Path:
    path = Path(value)
    if not path.is_file():
        raise argparse.ArgumentTypeError(f"not a file: {path}")
    return path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="release version without leading v")
    parser.add_argument("--target", default="macos-universal", help="release target label")
    parser.add_argument("--git-sha", required=True, help="git commit SHA used to build the binary")
    parser.add_argument("--binary", required=True, type=existing_file, help="path to the universal socai binary")
    parser.add_argument("--skill", default="SKILL.md", type=existing_file, help="path to SKILL.md")
    parser.add_argument("--install", default="install.md", type=existing_file, help="path to install.md")
    parser.add_argument("--out-dir", default="dist-artifacts", type=Path, help="directory for release assets")
    parser.add_argument("--created-at", default=None, help=argparse.SUPPRESS)
    args = parser.parse_args()

    if not SEMVER_RE.fullmatch(args.version):
        raise SystemExit(f"version must be strict MAJOR.MINOR.PATCH semver, got: {args.version}")
    if not args.git_sha.strip():
        raise SystemExit("git SHA must not be empty")

    created_at = args.created_at or utc_timestamp()
    manifest = {
        "version": args.version,
        "target": args.target,
        "git_sha": args.git_sha,
        "created_at": created_at,
    }

    args.out_dir.mkdir(parents=True, exist_ok=True)
    archive_path = args.out_dir / ARCHIVE_NAME
    checksum_path = args.out_dir / CHECKSUM_NAME
    archive_path.unlink(missing_ok=True)
    checksum_path.unlink(missing_ok=True)

    with tempfile.TemporaryDirectory() as temp_dir:
        manifest_path = Path(temp_dir) / "manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")

        with tarfile.open(archive_path, "w:gz") as tar:
            add_file(tar, args.binary, "socai", 0o755)
            add_file(tar, args.skill, "SKILL.md", 0o644)
            add_file(tar, args.install, "install.md", 0o644)
            add_file(tar, manifest_path, "manifest.json", 0o644)

    write_checksum(archive_path, checksum_path)
    print(f"created {archive_path}")
    print(f"created {checksum_path}")


if __name__ == "__main__":
    main()
