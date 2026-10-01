# Distribution bootstrap research

Research date: 2026-10-01. This is a research-only record for the next
release. The checkout is `0.2.0`, has no Git tag or GitHub Release, and the
README explicitly says that a release installer has not been published. The
recommended first distributable version is `v0.2.1`.

## Recommended `v0.2.1` scope

Publish four native release archives from the tagged commit:

| OS and architecture | Rust target | Archive |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-musl` | `yashik-vVERSION-x86_64-unknown-linux-musl.tar.gz` |
| Linux aarch64 | `aarch64-unknown-linux-musl` | `yashik-vVERSION-aarch64-unknown-linux-musl.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `yashik-vVERSION-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `yashik-vVERSION-aarch64-apple-darwin.tar.gz` |

Each archive must contain exactly one regular executable named `yashik` at its
root. Release the archives with a `SHA256SUMS` asset generated from those exact
files, and make the GitHub Release immutable after all assets have been
attached. GitHub's immutable-release feature locks release assets and the tag,
and produces a release attestation; it is stronger than a mutable `latest`
redirect ([GitHub documentation](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)).

Ship `scripts/install.sh`, written for POSIX `sh`, as a release asset and
document its pinned-version form. It installs only Yashik; it does not install
Cargo, Node, Bun, Homebrew, or WinGet. `yashik init` already bootstraps its
curated Node/Bun runtimes after Yashik itself is present.

Homebrew is a separate channel: publish a formula in an `AlexGladkov/homebrew-tap`
tap for `brew install alexgladkov/tap/yashik`, then consider Homebrew/core after
the release flow is stable. A formula is designed to fetch a versioned URL with
a SHA-256 and place the executable in Homebrew's managed `bin`
([Formula Cookbook](https://docs.brew.sh/Formula-Cookbook)). The POSIX installer
must not silently run Homebrew's own remote bootstrap: that creates a second
package-manager trust and privilege flow within an install of a single CLI.

Do not include WinGet in this release. A POSIX shell cannot run it, and Yashik
does not yet support Windows. When Windows support exists, publish a native
Windows archive and a WinGet `portable` manifest with per-architecture
`InstallerUrl` and `InstallerSha256`; WinGet supports portable installers and
installs them beneath its configured user or machine package root
([WinGet overview](https://learn.microsoft.com/en-us/windows/package-manager/winget/),
[manifest specification](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest)).

## Installer contract

The installation command must name an immutable release version, for example:

```sh
curl -fsSLO https://github.com/AlexGladkov/Yashik/releases/download/v0.2.1/yashik-installer-v0.2.1.sh
sh yashik-installer-v0.2.1.sh --version v0.2.1
```

The release notes must give the script's SHA-256 and a two-step verification
example. Do not make a moving `releases/latest/download` URL the integrity
anchor: a script and its checksum fetched through separate moving redirects can
refer to different releases. A convenience `curl | sh` command is acceptable
only as an explicitly less-verifiable shortcut, never as the documented
verification path.

Required script behavior:

1. Accept only `--version vMAJOR.MINOR.PATCH`, `--install-dir ABSOLUTE_DIR`,
   and `--force`; reject unknown flags, non-absolute destinations, unsupported
   `uname -s`/`uname -m`, and a non-writable destination. Map `Linux`/`Darwin`
   and `x86_64`/`amd64`, `aarch64`/`arm64` to the four fixed asset names. Never
   interpolate an unvalidated version into a URL or a shell command.
2. Default to `~/.local/bin`, make no `sudo` call, and require `curl` plus a
   SHA-256 tool (`sha256sum` on Linux, `shasum -a 256` on macOS). Download the
   versioned archive and its versioned `SHA256SUMS` over HTTPS, select the one
   exact expected filename, and compare the hash before extraction. Do not
   accept a checksum by prefix or an asset selected from an unpinned API result.
3. Check the archive before use: it has one member named `yashik`; extraction
   may only stream that member into a private temporary file in the destination
   directory. Reject any additional entry, directory, link, device, or path
   traversal. Then set mode `0755` and require `stage/yashik --version` to
   report the requested Cargo version.
4. Reinstallation without `--force` succeeds only when the installed binary
   reports the requested version; another existing file is left untouched with
   a clear error. `--force` replaces only after all download, checksum, archive,
   and version checks pass. Atomically rename the completed same-directory
   staging file over the final path, clean temporary files on exit, and do not
   edit shell profiles. Print an exact `export PATH="$HOME/.local/bin:$PATH"`
   command when the chosen directory is absent from the current `PATH`.

The workflow verifies the release archive itself, not a source checkout. GitHub
exposes a SHA-256 asset digest through its release API, but a checked-in
`SHA256SUMS` asset remains portable to plain `sh` clients
([GitHub Releases REST API](https://docs.github.com/en/rest/releases/releases)).

## macOS findings and release gates

The Rust code is expected to compile for both Darwin targets, but that is not
evidence of a working macOS installation.

- `cfg(unix)` deliberately includes macOS. The secure file helpers use
  `openat`, `mkdirat`, `fstatat`, `renameat`, `unlinkat`, `fchmod`,
  `O_NOFOLLOW`, `O_DIRECTORY`, `O_CLOEXEC`, and Unix metadata/extensions.
  The current `libc` source declares `O_NOFOLLOW` in its BSD module and
  `O_DIRECTORY`/`O_CLOEXEC` for Apple, as well as these POSIX-at functions, so
  there is no identified compile-time missing-symbol issue.
- Rust currently lists `aarch64-apple-darwin` as Tier 1 and
  `x86_64-apple-darwin` as Tier 2. The latter must be built and tested on a
  real Intel macOS runner; neither a Linux cross check nor an Apple-Silicon
  Rosetta run substitutes for that. Apple targets may need the macOS SDK/Xcode
  to cross-link ([Rust target documentation](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html)).
- The runtime registry already has checksum-pinned Node 22.22.0 and Bun 1.4.2
  assets for `darwin-x64` and `darwin-arm64`. `paths::discover` intentionally
  uses `~/Library/Application Support/yashik` and `~/Library/Caches/yashik` on
  macOS. Exercise archive download, checksum verification, `tar`/`unzip`,
  atomic state write, `init`, rerun, and `doctor` on both architectures.
- `bootstrap_recipe` maps a missing `tar` to Homebrew's `gnu-tar`, but runtime
  extraction searches for `tar`, while Homebrew exposes GNU tar as `gtar`.
  Verify the system BSD tar supports the flags used by `extract_tar`, or correct
  the recipe and lookup together before claiming missing-tool bootstrap works.
- The test suite overstates macOS coverage. `tests/installer_engine.rs` has a
  crate-wide `#![cfg(unix)]`, yet its child-process fixtures write launch specs
  and state under `XDG_STATE_HOME` and manually construct `.local/bin` paths.
  On macOS, `paths::discover` ignores XDG and uses Library paths, so those CLI
  tests do not exercise the paths used at runtime. Its temporary directory also
  needs canonical-path expectations on macOS (`/tmp` resolves through
  `/private/tmp`). Adapt the fixture from `Paths::discover` and assert the
  Darwin layout before calling it macOS validation.

## Acceptance evidence

The meaningful Linux acceptance is a clean, non-root user on the already
authorized Ubuntu host the authorized Ubuntu host, operated by the root task owner. It
must download the published `v0.2.1` archive, not copy a build output. Record:

1. OS, architecture, installer command, release tag, asset and checksum;
   `yashik --version` succeeds after a fresh install and the directory mode is
   user-owned.
2. A second identical install does not change the binary; a different existing
   executable is refused without `--force`; `--force` replaces it only after
   verification. Include a deliberately altered archive or checksum entry and
   prove the existing binary remains unchanged.
3. Run the existing `examples/codex-voltagent.yaml` acceptance: `check`, first
   `init`, Codex version probe, `doctor`, and a second `init`. The existing
   Ubuntu result proves the product path for `0.2.0`; repeat it against the
   downloaded release asset for distribution evidence.

GitHub Actions must additionally build all four archives, run repository checks
on Linux, run native macOS test jobs separately for Intel and Apple Silicon,
generate `SHA256SUMS`, and verify the archive shape plus `--version` for every
asset before publication. A Linux pass must remain documented as Linux-only.

## Sources consulted

- [GitHub: immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
- [GitHub: Releases REST API](https://docs.github.com/en/rest/releases/releases)
- [Homebrew Formula Cookbook](https://docs.brew.sh/Formula-Cookbook)
- [Rust: Apple Darwin platform support](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html)
- [Microsoft: WinGet package manifests](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest)
