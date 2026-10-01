#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 BINARY OUTPUT_ARCHIVE VERSION" >&2
  exit 2
}

[[ $# -eq 3 ]] || usage

binary=$1
archive=$2
version=$3

if [[ ! "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "error: version must be a stable MAJOR.MINOR.PATCH value" >&2
  exit 2
fi

case "$(basename "$archive")" in
  yashik-linux-x86_64.tar.gz | \
  yashik-linux-aarch64.tar.gz | \
  yashik-macos-x86_64.tar.gz | \
  yashik-macos-aarch64.tar.gz) ;;
  *)
    echo "error: output filename is not a supported release archive" >&2
    exit 2
    ;;
esac

if [[ ! -f "$binary" || -L "$binary" || ! -x "$binary" ]]; then
  echo "error: binary must be a regular executable file: $binary" >&2
  exit 1
fi

binary=$(cd "$(dirname "$binary")" && pwd -P)/$(basename "$binary")
archive_parent=$(dirname "$archive")
mkdir -p "$archive_parent"
archive_parent=$(cd "$archive_parent" && pwd -P)
archive="$archive_parent/$(basename "$archive")"
temp_dir=$(mktemp -d)
trap 'rm -rf "$temp_dir"' EXIT

expected="yashik $version"
actual=$("$binary" --version)
if [[ "$actual" != "$expected" ]]; then
  echo "error: binary version mismatch; expected '$expected', got '$actual'" >&2
  exit 1
fi
"$binary" --help >/dev/null
cat > "$temp_dir/probe.yaml" <<'YAML'
version: 1
harnesses:
  codex: {}
YAML
"$binary" check "$temp_dir/probe.yaml" >/dev/null

python3 - "$binary" "$archive" <<'PY'
import gzip
import os
import sys
import tarfile
import tempfile

binary, archive = sys.argv[1:]
parent = os.path.dirname(archive)
fd, temporary = tempfile.mkstemp(prefix=".yashik-release-", suffix=".tar.gz", dir=parent)
try:
    with os.fdopen(fd, "wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as output:
                stat = os.stat(binary, follow_symlinks=False)
                if not os.path.isfile(binary) or os.path.islink(binary):
                    raise SystemExit("error: binary must be a regular file")
                info = tarfile.TarInfo("yashik")
                info.size = stat.st_size
                info.mode = 0o755
                info.uid = 0
                info.gid = 0
                info.uname = ""
                info.gname = ""
                info.mtime = 0
                with open(binary, "rb") as source:
                    output.addfile(info, source)
    with tarfile.open(temporary, mode="r:gz") as check:
        members = check.getmembers()
        if len(members) != 1 or members[0].name != "yashik" or not members[0].isreg():
            raise SystemExit("error: generated archive must contain only a regular root yashik file")
        if members[0].mode & 0o777 != 0o755:
            raise SystemExit("error: generated archive does not mark yashik executable")
    os.replace(temporary, archive)
except BaseException:
    try:
        os.unlink(temporary)
    except FileNotFoundError:
        pass
    raise
PY

mkdir "$temp_dir/extracted"
tar -xzf "$archive" -C "$temp_dir/extracted"
extracted="$temp_dir/extracted/yashik"
[[ -f "$extracted" && -x "$extracted" ]]
[[ "$("$extracted" --version)" == "$expected" ]]
"$extracted" --help >/dev/null
"$extracted" check "$temp_dir/probe.yaml" >/dev/null

python3 - "$archive" <<'PY'
import hashlib
import sys

path = sys.argv[1]
digest = hashlib.sha256()
with open(path, "rb") as archive:
    for chunk in iter(lambda: archive.read(1024 * 1024), b""):
        digest.update(chunk)
print(f"{digest.hexdigest()}  {path}")
PY
