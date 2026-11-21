#!/usr/bin/env python3
"""Increment the Native package version and its lockfile entry.

The release workflow calls this script so that Cargo.toml and Cargo.lock stay
in sync before the version commit and native-vX.Y.Z tag are created.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path


PACKAGE_NAME = "liteterm-native"
VERSION_RE = re.compile(r'(?m)^version = "(\d+)\.(\d+)\.(\d+)"\s*$')
LOCK_PACKAGE_RE = re.compile(
    rf'(?ms)(\[\[package\]\]\nname = "{re.escape(PACKAGE_NAME)}"\nversion = ")([^"\n]+)(")'
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "bump",
        choices=("patch", "minor", "major"),
        help="SemVer component to increment",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "Cargo.toml",
    )
    parser.add_argument(
        "--lock",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "Cargo.lock",
    )
    return parser.parse_args()


def next_version(current: tuple[int, int, int], bump: str) -> tuple[int, int, int]:
    major, minor, patch = current
    if bump == "major":
        return major + 1, 0, 0
    if bump == "minor":
        return major, minor + 1, 0
    return major, minor, patch + 1


def read_current_version(manifest: str) -> tuple[int, int, int]:
    match = VERSION_RE.search(manifest)
    if not match:
        raise SystemExit("无法从 Cargo.toml 读取严格的 MAJOR.MINOR.PATCH 版本")
    return tuple(int(part) for part in match.groups())


def update_manifest(path: Path, old: tuple[int, int, int], new: str) -> None:
    text = path.read_text(encoding="utf-8")
    old_version = ".".join(str(part) for part in old)
    replacement = f'version = "{new}"'
    updated, count = re.subn(
        rf'(?m)^version = "{re.escape(old_version)}"\s*$', replacement, text, count=1
    )
    if count != 1:
        raise SystemExit(f"{path} 中未找到 Native 包版本 {old_version}")
    path.write_text(updated, encoding="utf-8")


def update_lock(path: Path, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    updated, count = LOCK_PACKAGE_RE.subn(rf'\g<1>{new}\g<3>', text, count=1)
    if count != 1:
        raise SystemExit(f"{path} 中未找到 {PACKAGE_NAME} 的锁定版本")
    path.write_text(updated, encoding="utf-8")


def main() -> None:
    args = parse_args()
    manifest_text = args.manifest.read_text(encoding="utf-8")
    current = read_current_version(manifest_text)
    bumped = next_version(current, args.bump)
    version = ".".join(str(part) for part in bumped)
    update_manifest(args.manifest, current, version)
    update_lock(args.lock, version)
    print(version)


if __name__ == "__main__":
    main()
