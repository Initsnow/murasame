#!/usr/bin/env python3
"""
Automatically computes SRI sha256 hashes for release artifacts
and updates them in flake.nix.
"""

import argparse
import base64
import hashlib
import re
import sys
from pathlib import Path


def compute_sri_hash(file_path: Path) -> str:
    h = hashlib.sha256()
    with open(file_path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    b64 = base64.b64encode(h.digest()).decode("ascii")
    return f"sha256-{b64}"


def update_flake(version: str, appimage_path: Path, tar_path: Path, flake_path: Path):
    if not appimage_path.exists():
        sys.exit(f"Error: AppImage file not found: {appimage_path}")
    if not tar_path.exists():
        sys.exit(f"Error: Tarball file not found: {tar_path}")
    if not flake_path.exists():
        sys.exit(f"Error: flake.nix file not found: {flake_path}")

    appimage_hash = compute_sri_hash(appimage_path)
    tar_hash = compute_sri_hash(tar_path)

    print(f"Updating {flake_path} for version {version}:")
    print(f"  AppImage ({appimage_path.name}): {appimage_hash}")
    print(f"  Tarball  ({tar_path.name}):  {tar_hash}")

    with open(flake_path, "r", encoding="utf-8") as f:
        content = f.read()

    content = re.sub(
        r'murasameVersion\s*=\s*"[^"]+";',
        f'murasameVersion = "{version}";',
        content,
        count=1,
    )
    content = re.sub(
        r'(murasame-appimage-src\s*=\s*pkgs\.fetchurl\s*\{[\s\S]*?hash\s*=\s*")[^"]+(";)',
        rf'\g<1>{appimage_hash}\2',
        content,
        count=1,
    )
    content = re.sub(
        r'(murasame-tar\s*=\s*pkgs\.stdenv\.mkDerivation[\s\S]*?src\s*=\s*pkgs\.fetchurl\s*\{[\s\S]*?hash\s*=\s*")[^"]+(";)',
        rf'\g<1>{tar_hash}\2',
        content,
        count=1,
    )

    with open(flake_path, "w", encoding="utf-8") as f:
        f.write(content)
    print(f"Successfully updated {flake_path}.")


def main():
    parser = argparse.ArgumentParser(description="Update flake.nix hashes for release.")
    parser.add_argument("--version", required=True, help="Release version (without 'v' prefix, e.g. 0.2.2)")
    parser.add_argument("--appimage", required=True, type=Path, help="Path to AppImage file")
    parser.add_argument("--tar", required=True, type=Path, help="Path to linux tarball file")
    parser.add_argument("--flake", default=Path("flake.nix"), type=Path, help="Path to flake.nix")
    args = parser.parse_args()

    clean_version = args.version.lstrip("v")
    update_flake(clean_version, args.appimage, args.tar, args.flake)


if __name__ == "__main__":
    main()
