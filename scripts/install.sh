#!/bin/sh

set -eu

DEFAULT_VERSION=0.2.1

fail() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

usage() {
    printf '%s\n' \
        'Usage: install.sh [--version VERSION] [--bin-dir ABSOLUTE_DIR] [--force]' \
        '       install.sh --help' \
        '' \
        'Install the verified Yashik binary for this platform.' \
        'VERSION accepts MAJOR.MINOR.PATCH or vMAJOR.MINOR.PATCH.' \
        'The default version is 0.2.1; the default directory is ~/.local/bin.'
}

requested_version=$DEFAULT_VERSION
bin_dir=
bin_dir_provided=no
force=no

while [ "$#" -gt 0 ]; do
    case "$1" in
        --help|-h)
            usage
            exit 0
            ;;
        --version)
            [ "$#" -ge 2 ] || fail 'missing value for --version'
            requested_version=$2
            shift 2
            ;;
        --bin-dir)
            [ "$#" -ge 2 ] || fail 'missing value for --bin-dir'
            bin_dir=$2
            bin_dir_provided=yes
            shift 2
            ;;
        --force)
            force=yes
            shift
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

case "$requested_version" in
    v*) version=${requested_version#v} ;;
    *) version=$requested_version ;;
esac

case "$version" in
    ''|.*|*.|*..*|*[!0-9.]*) fail 'version must be MAJOR.MINOR.PATCH or vMAJOR.MINOR.PATCH' ;;
esac

saved_ifs=$IFS
IFS=.
set -- $version
IFS=$saved_ifs
[ "$#" -eq 3 ] || fail 'version must have exactly three numeric components'
for version_component do
    case "$version_component" in
        ''|*[!0-9]*) fail 'version must have exactly three numeric components' ;;
        0) ;;
        0*) fail 'version components must not have leading zeroes' ;;
    esac
done

if [ "$bin_dir_provided" = no ]; then
    [ -n "${HOME-}" ] || fail 'HOME is unset; pass --bin-dir explicitly'
    case "$HOME" in
        /*) bin_dir=$HOME/.local/bin ;;
        *) fail 'HOME must be an absolute path' ;;
    esac
fi
case "$bin_dir" in
    /*) ;;
    *) fail '--bin-dir must be an absolute path' ;;
esac

if ! system_name=$(uname -s 2>/dev/null); then
    fail 'unable to determine operating system with uname'
fi
if ! machine_name=$(uname -m 2>/dev/null); then
    fail 'unable to determine architecture with uname'
fi

case "$system_name:$machine_name" in
    Linux:x86_64|Linux:amd64)
        asset_name=yashik-linux-x86_64.tar.gz
        ;;
    Linux:aarch64|Linux:arm64)
        asset_name=yashik-linux-aarch64.tar.gz
        ;;
    Darwin:x86_64|Darwin:amd64)
        asset_name=yashik-macos-x86_64.tar.gz
        ;;
    Darwin:aarch64|Darwin:arm64)
        asset_name=yashik-macos-aarch64.tar.gz
        ;;
    *)
        fail "unsupported platform: $system_name $machine_name"
        ;;
esac

if command -v curl >/dev/null 2>&1; then
    downloader=curl
elif command -v wget >/dev/null 2>&1; then
    downloader=wget
else
    fail 'curl or wget is required to download the release'
fi

if command -v sha256sum >/dev/null 2>&1; then
    checksum_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    checksum_tool=shasum
else
    fail 'sha256sum or shasum -a 256 is required to verify the release'
fi

command -v tar >/dev/null 2>&1 || fail 'tar is required to inspect and extract the release archive'
command -v awk >/dev/null 2>&1 || fail 'awk is required to read SHA256SUMS'
command -v cmp >/dev/null 2>&1 || fail 'cmp is required to compare verified binaries'
command -v mktemp >/dev/null 2>&1 || fail 'mktemp is required for private temporary files'

release_tag=v$version
release_base=https://github.com/AlexGladkov/Yashik/releases/download/$release_tag
temporary_root=${TMPDIR:-/tmp}
temp_dir=$(mktemp -d "${temporary_root%/}/yashik-install.XXXXXX") || fail 'cannot create a private temporary directory'
stage_file=

cleanup() {
    if [ -n "$stage_file" ]; then
        rm -f "$stage_file" 2>/dev/null || :
    fi
    rm -rf "$temp_dir" 2>/dev/null || :
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

download() {
    download_url=$1
    download_path=$2
    if [ "$downloader" = curl ]; then
        if ! curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
            "$download_url" --output "$download_path"; then
            fail "download failed: $download_url"
        fi
    else
        if ! wget --https-only --quiet --output-document "$download_path" "$download_url"; then
            fail "download failed: $download_url"
        fi
    fi
}

archive_path=$temp_dir/$asset_name
manifest_path=$temp_dir/SHA256SUMS
download "$release_base/$asset_name" "$archive_path"
download "$release_base/SHA256SUMS" "$manifest_path"

if ! expected_checksum=$(awk -v expected="$asset_name" '
    {
        filename = $NF
        sub(/^\*/, "", filename)
        if (filename == expected) {
            matches++
            if (NF != 2 || length($1) != 64 || $1 ~ /[^[:xdigit:]]/) {
                invalid = 1
            } else {
                checksum = tolower($1)
            }
        }
    }
    END {
        if (matches != 1 || invalid) {
            exit 1
        }
        print checksum
    }
' "$manifest_path"); then
    fail "SHA256SUMS must contain exactly one valid checksum for $asset_name"
fi

if [ "$checksum_tool" = sha256sum ]; then
    if ! checksum_output=$(sha256sum "$archive_path"); then
        fail 'unable to calculate archive SHA-256'
    fi
else
    if ! checksum_output=$(shasum -a 256 "$archive_path"); then
        fail 'unable to calculate archive SHA-256'
    fi
fi
actual_checksum=${checksum_output%% *}
[ "$actual_checksum" = "$expected_checksum" ] || fail "SHA-256 mismatch for $asset_name"

member_list=$temp_dir/members
if ! tar -tzf "$archive_path" >"$member_list"; then
    fail 'release archive is not a readable tar.gz file'
fi
member_count=0
while IFS= read -r archive_member || [ -n "$archive_member" ]; do
    member_count=$((member_count + 1))
    [ "$archive_member" = yashik ] || fail 'release archive must contain only the root member yashik'
done <"$member_list"
[ "$member_count" -eq 1 ] || fail 'release archive must contain exactly one member named yashik'

verbose_list=$temp_dir/member-details
if ! tar -tvzf "$archive_path" >"$verbose_list"; then
    fail 'unable to inspect release archive member type'
fi
detail_count=0
while IFS= read -r archive_detail || [ -n "$archive_detail" ]; do
    detail_count=$((detail_count + 1))
    case "$archive_detail" in
        -*) ;;
        *) fail 'release archive member yashik must be a regular file' ;;
    esac
done <"$verbose_list"
[ "$detail_count" -eq 1 ] || fail 'release archive must contain exactly one regular file'

if ! mkdir -p "$bin_dir"; then
    fail "cannot create installation directory: $bin_dir"
fi
target_path=$bin_dir/yashik
if [ -L "$target_path" ]; then
    fail "refusing to replace symlink: $target_path"
elif [ -e "$target_path" ] && [ ! -f "$target_path" ]; then
    fail "refusing to replace non-regular target: $target_path"
fi

if ! stage_file=$(mktemp "$bin_dir/.yashik-install.XXXXXXXX"); then
    fail "cannot create a private staging file in: $bin_dir"
fi
if ! tar -xOzf "$archive_path" yashik >"$stage_file"; then
    fail 'unable to extract the verified yashik member'
fi
if ! chmod 0755 "$stage_file"; then
    fail 'unable to mark the staged Yashik binary executable'
fi

version_output=$temp_dir/version-output
expected_version_output=$temp_dir/expected-version
if ! "$stage_file" --version >"$version_output" 2>/dev/null; then
    fail 'verified archive binary failed to run with --version'
fi
printf 'yashik %s\n' "$version" >"$expected_version_output"
if ! cmp -s "$expected_version_output" "$version_output"; then
    fail "archive binary did not report exactly: yashik $version"
fi

if [ -L "$target_path" ]; then
    fail "refusing to replace symlink: $target_path"
elif [ -e "$target_path" ]; then
    [ -f "$target_path" ] || fail "refusing to replace non-regular target: $target_path"
    if cmp -s "$target_path" "$stage_file"; then
        rm -f "$stage_file"
        stage_file=
        install_status=already
    else
        [ "$force" = yes ] || fail "existing file differs; pass --force to replace: $target_path"
        install_status=installed
    fi
else
    install_status=installed
fi

if [ "$install_status" = installed ]; then
    if ! mv -f "$stage_file" "$target_path"; then
        fail "unable to atomically install Yashik at: $target_path"
    fi
    stage_file=
fi

if [ "$install_status" = already ]; then
    printf 'Yashik %s is already installed at %s\n' "$version" "$target_path"
else
    printf 'Installed Yashik %s at %s\n' "$version" "$target_path"
fi

path_contains_bin_dir() {
    remaining_path=${PATH-}
    while :; do
        case "$remaining_path" in
            *:*) path_entry=${remaining_path%%:*}; remaining_path=${remaining_path#*:}; has_more=yes ;;
            *) path_entry=$remaining_path; has_more=no ;;
        esac
        [ "$path_entry" = "$bin_dir" ] && return 0
        [ "$has_more" = yes ] || break
    done
    return 1
}

quote_shell_word() {
    quote_value=$1
    printf "'"
    while :; do
        case "$quote_value" in
            *"'"*)
                quote_prefix=${quote_value%%"'"*}
                printf "%s'\\''" "$quote_prefix"
                quote_value=${quote_value#*"'"}
                ;;
            *)
                printf '%s' "$quote_value"
                break
                ;;
        esac
    done
    printf "'"
}

if ! path_contains_bin_dir; then
    printf 'Add this directory to PATH for this shell with:\n  export PATH='
    quote_shell_word "$bin_dir"
    printf ':"$PATH"\n'
fi
