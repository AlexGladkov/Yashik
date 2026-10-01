#!/usr/bin/env python3
"""Validate and package one Windows Yashik bridge executable as a ZIP."""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import stat
import struct
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path


MACHINES = {"x86_64": 0x8664, "aarch64": 0xAA64}
VERSION_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")


def pe_machine(path: Path) -> int:
    try:
        data = path.read_bytes()
    except OSError as exc:
        raise ValueError(f"cannot read Windows executable {path}: {exc}") from exc

    if len(data) < 0x40 or data[:2] != b"MZ":
        raise ValueError("binary is not a valid PE executable (missing MZ header)")
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if pe_offset > len(data) - 26 or data[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise ValueError("binary is not a valid PE executable (missing PE signature)")

    machine = struct.unpack_from("<H", data, pe_offset + 4)[0]
    optional_size = struct.unpack_from("<H", data, pe_offset + 20)[0]
    optional_offset = pe_offset + 24
    if optional_size < 2 or optional_offset + optional_size > len(data):
        raise ValueError("binary has a truncated PE optional header")
    optional_magic = struct.unpack_from("<H", data, optional_offset)[0]
    if optional_magic != 0x20B:
        raise ValueError("binary must use the 64-bit PE optional header")
    return machine


def run_probe(binary: Path, version: str, flag: str) -> str:
    try:
        result = subprocess.run(
            [str(binary), flag],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ValueError(f"binary probe {flag} failed to run: {exc}") from exc
    if result.returncode != 0:
        raise ValueError(f"binary probe {flag} returned {result.returncode}: {result.stderr.strip()}")
    if flag == "--version" and result.stdout.strip() != f"yashik {version}":
        raise ValueError(f"binary reports unexpected version: {result.stdout.strip()!r}")
    if flag == "--help" and "WSL" not in result.stdout:
        raise ValueError("binary help does not describe the WSL bridge")
    return result.stdout


def write_archive(binary: Path, output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(prefix=".yashik-windows-", suffix=".zip", dir=output.parent, delete=False) as temp:
        temp_path = Path(temp.name)
    try:
        info = zipfile.ZipInfo("yashik.exe", date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.create_system = 3
        info.external_attr = (stat.S_IFREG | 0o755) << 16
        with zipfile.ZipFile(temp_path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            archive.writestr(info, binary.read_bytes())
        with zipfile.ZipFile(temp_path) as archive:
            entries = archive.infolist()
            if len(entries) != 1 or entries[0].filename != "yashik.exe":
                raise ValueError("Windows ZIP must contain only root yashik.exe")
            if not stat.S_ISREG(entries[0].external_attr >> 16):
                raise ValueError("Windows ZIP member yashik.exe must be a regular file")
            if archive.testzip() is not None:
                raise ValueError("Windows ZIP failed its CRC check")
        os.replace(temp_path, output)
    finally:
        temp_path.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path, help="compiled yashik.exe")
    parser.add_argument("--output", required=True, type=Path, help="output ZIP path")
    parser.add_argument("--version", required=True, help="stable release version")
    parser.add_argument("--architecture", required=True, choices=sorted(MACHINES))
    parser.add_argument(
        "--run-probes",
        action="store_true",
        help="execute --version and --help probes (native build only)",
    )
    args = parser.parse_args()

    try:
        if not VERSION_RE.fullmatch(args.version):
            raise ValueError("version must be a stable MAJOR.MINOR.PATCH value")
        if args.binary.is_symlink() or not args.binary.is_file():
            raise ValueError(f"binary must be a regular file: {args.binary}")
        machine = pe_machine(args.binary)
        if machine != MACHINES[args.architecture]:
            raise ValueError(
                f"PE machine 0x{machine:04x} does not match {args.architecture}"
            )
        if args.run_probes:
            run_probe(args.binary, args.version, "--version")
            run_probe(args.binary, args.version, "--help")
        write_archive(args.binary, args.output)
        digest = hashlib.sha256(args.output.read_bytes()).hexdigest()
        print(f"{digest}  {args.output.name}")
    except (OSError, ValueError, zipfile.BadZipFile) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
