#!/usr/bin/env python3
"""Generate the Homebrew formula for one immutable Yashik release."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from urllib.parse import urlsplit


ASSETS = {
    "macos-arm": "yashik-macos-aarch64.tar.gz",
    "macos-intel": "yashik-macos-x86_64.tar.gz",
    "linux-arm": "yashik-linux-aarch64.tar.gz",
    "linux-intel": "yashik-linux-x86_64.tar.gz",
}
CHECKSUM_FILES = {*ASSETS.values(), "install.sh"}
VERSION_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
CHECKSUM_RE = re.compile(r"^([0-9a-fA-F]{64}) [ *]([^\s/]+)$")
PRODUCTION_BASE = "https://github.com/AlexGladkov/Yashik/releases/download"


def parse_checksums(path: Path) -> dict[str, str]:
    checksums: dict[str, str] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise ValueError(f"cannot read checksum manifest {path}: {exc}") from exc

    for number, raw_line in enumerate(lines, 1):
        line = raw_line.strip()
        if not line:
            continue
        match = CHECKSUM_RE.fullmatch(line)
        if not match:
            raise ValueError(f"malformed checksum manifest line {number}")
        digest, filename = match.groups()
        if filename in checksums:
            raise ValueError(f"duplicate checksum entry for {filename}")
        checksums[filename] = digest.lower()

    missing = sorted(CHECKSUM_FILES - checksums.keys())
    unexpected = sorted(checksums.keys() - CHECKSUM_FILES)
    if missing:
        raise ValueError(f"checksum manifest is missing: {', '.join(missing)}")
    if unexpected:
        raise ValueError(f"checksum manifest has unexpected entries: {', '.join(unexpected)}")
    return checksums


def validate_base_url(value: str) -> str:
    parts = urlsplit(value)
    if parts.scheme not in {"http", "https"} or not parts.netloc:
        raise ValueError("base URL must be an absolute HTTP(S) URL")
    if parts.username or parts.password or parts.query or parts.fragment:
        raise ValueError("base URL must not include credentials, a query, or a fragment")
    return value.rstrip("/")


def render(version: str, checksums: dict[str, str], base_url: str) -> str:
    formula_version = "" if base_url == f"{PRODUCTION_BASE}/v{version}" else f'  version "{version}"\n'

    def asset_url(key: str) -> str:
        return f"{base_url}/{ASSETS[key]}"

    return f'''class Yashik < Formula
  desc "Install and reconcile personal AI client environments"
  homepage "https://github.com/AlexGladkov/Yashik"
{formula_version.rstrip()}
  license "MIT"

  on_macos do
    on_arm do
      url "{asset_url("macos-arm")}"
      sha256 "{checksums[ASSETS["macos-arm"]]}"
    end
    on_intel do
      url "{asset_url("macos-intel")}"
      sha256 "{checksums[ASSETS["macos-intel"]]}"
    end
  end

  on_linux do
    on_arm do
      url "{asset_url("linux-arm")}"
      sha256 "{checksums[ASSETS["linux-arm"]]}"
    end
    on_intel do
      url "{asset_url("linux-intel")}"
      sha256 "{checksums[ASSETS["linux-intel"]]}"
    end
  end

  def install
    bin.install "yashik"
  end

  test do
    assert_equal "yashik #{{version}}", shell_output("#{{bin}}/yashik --version").strip
    assert_match "Usage:", shell_output("#{{bin}}/yashik --help")
  end
end
'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="stable Cargo/release version")
    parser.add_argument("--checksums", required=True, type=Path, help="SHA256SUMS manifest")
    parser.add_argument("--output", type=Path, help="write the formula to this path (default: stdout)")
    parser.add_argument(
        "--base-url",
        help="override the production release URL base for local CI fixtures",
    )
    args = parser.parse_args()

    try:
        if not VERSION_RE.fullmatch(args.version):
            raise ValueError("version must be a stable MAJOR.MINOR.PATCH value")
        checksums = parse_checksums(args.checksums)
        base_url = (
            validate_base_url(args.base_url)
            if args.base_url
            else f"{PRODUCTION_BASE}/v{args.version}"
        )
        formula = render(args.version, checksums, base_url)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(formula, encoding="utf-8")
        else:
            sys.stdout.write(formula)
    except (OSError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
