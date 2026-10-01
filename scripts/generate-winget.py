#!/usr/bin/env python3
"""Generate the three-file WinGet manifest for a pinned Yashik release."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


PACKAGE_ID = "AlexGladkov.Yashik"
MANIFEST_VERSION = "1.12.0"
ARCHIVES = {
    "x64": "yashik-windows-x86_64.zip",
    "arm64": "yashik-windows-aarch64.zip",
}
VERSION_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
CHECKSUM_RE = re.compile(r"^([0-9a-fA-F]{64})  ([^/\\\s]+)$")
RELEASE_BASE = "https://github.com/AlexGladkov/Yashik/releases/download"


def parse_checksums(path: Path) -> dict[str, str]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise ValueError(f"cannot read checksum manifest {path}: {exc}") from exc

    checksums: dict[str, str] = {}
    for number, line in enumerate(lines, 1):
        if not line:
            continue
        match = CHECKSUM_RE.fullmatch(line)
        if not match:
            raise ValueError(f"malformed checksum manifest line {number}")
        digest, filename = match.groups()
        if filename in checksums:
            raise ValueError(f"duplicate checksum entry for {filename}")
        checksums[filename] = digest.lower()

    missing = sorted(set(ARCHIVES.values()) - checksums.keys())
    if missing:
        raise ValueError(f"checksum manifest is missing: {', '.join(missing)}")
    return checksums


def render(version: str, checksums: dict[str, str]) -> dict[str, str]:
    base = f"{RELEASE_BASE}/v{version}"
    version_manifest = f'''# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.{MANIFEST_VERSION}.schema.json
PackageIdentifier: {PACKAGE_ID}
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: {MANIFEST_VERSION}
'''
    installer_manifest = f'''# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.{MANIFEST_VERSION}.schema.json
PackageIdentifier: {PACKAGE_ID}
PackageVersion: {version}
Platform:
- Windows.Desktop
Installers:
- Architecture: x64
  InstallerType: zip
  NestedInstallerType: portable
  NestedInstallerFiles:
  - RelativeFilePath: yashik.exe
    PortableCommandAlias: yashik
  InstallerUrl: "{base}/{ARCHIVES['x64']}"
  InstallerSha256: "{checksums[ARCHIVES['x64']].upper()}"
  Scope: user
- Architecture: arm64
  InstallerType: zip
  NestedInstallerType: portable
  NestedInstallerFiles:
  - RelativeFilePath: yashik.exe
    PortableCommandAlias: yashik
  InstallerUrl: "{base}/{ARCHIVES['arm64']}"
  InstallerSha256: "{checksums[ARCHIVES['arm64']].upper()}"
  Scope: user
ManifestType: installer
ManifestVersion: {MANIFEST_VERSION}
'''
    locale_manifest = f'''# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.{MANIFEST_VERSION}.schema.json
PackageIdentifier: {PACKAGE_ID}
PackageVersion: {version}
PackageLocale: en-US
Publisher: Alex Gladkov
PublisherUrl: https://github.com/AlexGladkov
PublisherSupportUrl: https://github.com/AlexGladkov/Yashik/issues
Author: Alex Gladkov
PackageName: Yashik
PackageUrl: https://github.com/AlexGladkov/Yashik
License: MIT
LicenseUrl: "https://github.com/AlexGladkov/Yashik/blob/v{version}/LICENSE"
ShortDescription: Manage AI client environments through an existing WSL distribution
Description: Windows command-line bridge for Yashik running in an already installed and initialized WSL distribution. It does not install WSL or change Windows features.
Tags:
- AI
- WSL
- command-line
ManifestType: defaultLocale
ManifestVersion: {MANIFEST_VERSION}
'''
    return {
        f"{PACKAGE_ID}.yaml": version_manifest,
        f"{PACKAGE_ID}.installer.yaml": installer_manifest,
        f"{PACKAGE_ID}.locale.en-US.yaml": locale_manifest,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="stable release version")
    parser.add_argument("--checksums", required=True, type=Path, help="release SHA256SUMS")
    parser.add_argument("--output-dir", required=True, type=Path, help="manifest version directory")
    args = parser.parse_args()

    try:
        if not VERSION_RE.fullmatch(args.version):
            raise ValueError("version must be a stable MAJOR.MINOR.PATCH value")
        checksums = parse_checksums(args.checksums)
        manifests = render(args.version, checksums)
        args.output_dir.mkdir(parents=True, exist_ok=True)
        for filename, contents in manifests.items():
            (args.output_dir / filename).write_text(contents, encoding="utf-8", newline="\n")
    except (OSError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
