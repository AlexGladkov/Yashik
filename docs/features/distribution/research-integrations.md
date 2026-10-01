# Distribution integrations research

## Scope and repository facts

This research covers the requested release distribution for `yashik`: a POSIX
shell installer, an upstream-owned Homebrew tap, and the requested "winglet",
interpreted as [WinGet](https://learn.microsoft.com/en-us/windows/package-manager/winget/).
It does not change the product or add a release workflow.

The repository currently has no GitHub release (the public `releases/latest`
endpoint returns 404), no `.github` workflow, and no prebuilt artifacts.
`Cargo.toml` declares Yashik 0.2.0, Rust 1.85, and MIT. The CLI has stable,
non-interactive probes: `yashik --version` prints the Cargo package version and
`yashik --help` exits successfully.

The product boundary is material. The README and requirements name Linux and
macOS as the supported platforms. The implementation uses Unix-specific file,
permission, process, and `libc` APIs in `src/install/{util,sources,launcher}.rs`
and related modules; it is not a Windows build today. The owned
`https://github.com/AlexGladkov/homebrew-tap` repository exists, and its current
formulae are root-level Ruby files with architecture-specific binary URLs.

## Recommended first release pipeline

Use annotated release tags named `v<semver>` as the only release trigger. A
tagged commit first builds and tests, then publishes one GitHub Release whose
assets are the source of truth for the shell installer and Homebrew formula.
GitHub describes releases as deployable packages based on tags and supports
binary release assets [directly](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).

Build the four native targets below rather than relying on cross compilation.
GitHub currently provides the required x64 Linux, ARM64 Linux, Intel macOS, and
ARM64 macOS hosted runners [in its runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
Pin the runner names, rather than using a moving `*-latest` label, and build
with `cargo build --release --locked` after `cargo test --locked`.

| Release asset target | GitHub-hosted runner | Archive name |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` | `yashik-v<version>-linux-amd64.tar.gz` |
| `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` | `yashik-v<version>-linux-arm64.tar.gz` |
| `x86_64-apple-darwin` | `macos-15-intel` | `yashik-v<version>-darwin-amd64.tar.gz` |
| `aarch64-apple-darwin` | `macos-15` | `yashik-v<version>-darwin-arm64.tar.gz` |

Each archive should contain only a top-level executable named `yashik`, and the
workflow should run its `--version` and `--help` probes before upload. A final
release job downloads all four build artifacts, creates a sorted `SHA256SUMS`
file covering every archive **and** `install.sh`, and calls `gh release create
"$GITHUB_REF_NAME" --generate-notes` with all archives, the checksum file, and
the release `install.sh`. GitHub's Rust
workflow guide shows the artifact handoff and `gh release create` pattern
[here](https://docs.github.com/en/actions/tutorials/build-and-test-code/rust).
Give only the final job `permissions: { contents: write }`; GitHub recommends
explicitly limiting `GITHUB_TOKEN` permissions to the minimum needed
[here](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token).

This starts with GNU/Linux artifacts built on Ubuntu 22.04. It establishes a
glibc baseline only after a clean-machine test; it must not claim support for
older distributions merely because the binary builds. If the intended Linux
range includes older or musl distributions, add a separately tested musl
artifact before advertising that compatibility.

## POSIX shell installation

Add a versioned `scripts/install.sh` to the source tree and publish the exact
same file as a release asset. The user-facing default can be:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/AlexGladkov/Yashik/main/scripts/install.sh | sh
```

For reproducible support instructions, document the tag-pinned equivalent:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/AlexGladkov/Yashik/v0.2.0/scripts/install.sh | \
  sh -s -- --version v0.2.0
```

The script contract should be deliberately small and POSIX `sh` compatible:

1. Accept `--version vX.Y.Z` and default to the current stable release;
   resolve the stable tag before constructing any asset URL.
2. Map `uname -s` plus `uname -m` only to the four release names above. Fail
   without writing anything on any other OS or architecture.
3. Download the selected archive and `SHA256SUMS` over HTTPS to a temporary
   directory, verify the selected archive with `sha256sum` or `shasum -a 256`,
   and extract with `tar -xzf` only after the match.
4. Install atomically into `${YASHIK_INSTALL_DIR:-$HOME/.local/bin}`. Do not
   invoke `sudo`, edit shell profile files, or overwrite a non-regular target.
   Print the exact PATH export when that directory is absent from `PATH`.
5. Run the newly installed binary's `--version`; fail while preserving the
   previous executable if that probe is wrong.

The script's checksum guards the artifact against accidental or transport
corruption. It is not a signed provenance system: release signing or Sigstore
attestations can be a later hardening step. Versioned instructions above are
preferable for an incident report or a reproducible installation.

The source fallback should stay available and explicitly pin the tag:

```sh
git clone --branch v0.2.0 --depth 1 https://github.com/AlexGladkov/Yashik.git
cd Yashik
cargo build --release --locked
install -m 755 target/release/yashik "$HOME/.local/bin/yashik"
```

This is an alternative for users who already have Rust, not a replacement for
prebuilt archives.

## Homebrew: binary formula in the owned tap

Make `yashik.rb` a binary formula in the root of
`AlexGladkov/homebrew-tap`, consistent with the existing `harnest.rb` style.
Users can then run:

```sh
brew install AlexGladkov/tap/yashik
```

Homebrew documents this direct `user/repository/formula` installation form for
upstream taps [here](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap).
The tap repository must retain its `homebrew-tap` name for that short form.

The formula should select one immutable GitHub Release archive and its actual
SHA-256 per OS and CPU. Homebrew's supported conditional DSL is `on_macos`,
`on_linux`, `on_arm`, and `on_intel`; its cookbook documents those blocks and
the in-method `Hardware::CPU` alternative
[here](https://docs.brew.sh/Formula-Cookbook#handling-different-system-configurations).

```ruby
class Yashik < Formula
  desc "Install and reconcile personal AI client environments from a manifest"
  homepage "https://github.com/AlexGladkov/Yashik"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/AlexGladkov/Yashik/releases/download/v0.2.0/yashik-v0.2.0-darwin-arm64.tar.gz"
      sha256 "RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/AlexGladkov/Yashik/releases/download/v0.2.0/yashik-v0.2.0-darwin-amd64.tar.gz"
      sha256 "RELEASE_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/AlexGladkov/Yashik/releases/download/v0.2.0/yashik-v0.2.0-linux-arm64.tar.gz"
      sha256 "RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/AlexGladkov/Yashik/releases/download/v0.2.0/yashik-v0.2.0-linux-amd64.tar.gz"
      sha256 "RELEASE_SHA256"
    end
  end

  def install
    bin.install "yashik"
  end

  test do
    assert_match "yashik #{version}", shell_output("#{bin}/yashik --version")
  end
end
```

The placeholders must be replaced only with the checksum of the published
release artifact. `bin.install` is the standard Homebrew primitive for adding a
binary to a formula's `bin` directory
[as documented here](https://docs.brew.sh/Formula-Cookbook#bininstall-foo).
For each later tag, update `version`, all four URLs, and all four hashes in a
pull request to the owned tap, then run `brew audit --strict yashik` and
`brew test yashik` on the relevant platforms. Homebrew says the tap owner is
responsible for maintaining its formulae and their updates
[here](https://docs.brew.sh/Versions).

A Homebrew formula that compiles from a Git tag is not recommended as the
primary path. Cargo dependencies are not vendored in this repository, while
Homebrew builds must not fetch them at build time. A true source formula would
therefore require a versioned, vendored source release plus resource management
and a Rust build dependency. The tag-pinned Cargo command above is the useful
source alternative until such a vendor release exists.

## WinGet portable Windows-to-WSL shim

The chosen Windows scope is a bridge, not a port of Yashik's Unix installer.
Ship a separate `std`-only Rust crate, for example `crates/yashik-wsl`, that
builds `yashik.exe` for `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc`. It owns only Windows argument parsing, WSL discovery,
Windows-to-WSL path conversion, and process exit propagation. The existing
Linux binary remains responsible for `init`, `check`, `doctor`, state, and all
resource changes inside the selected WSL distribution.

This boundary avoids trying to compile the current Unix-specific product on
Windows while giving Windows users one `yashik` command. It must be documented
as managing the WSL environment, not Windows-native Codex/Claude/OpenCode/Pi
configuration.

### Shim contract

The shim exposes `setup`, `init`, `check`, and `doctor`.

* `setup [--distro NAME]` uses its compiled package version to select the exact
  release tag (`v<shim-version>`). It downloads that version's `install.sh` and
  `SHA256SUMS` from the GitHub Release by HTTPS inside the chosen distribution,
  verifies the script's checksum, then invokes `sh install.sh --version
  v<shim-version>`. It must never use a `latest` URL.
* Before `setup` or forwarding a command, call `wsl.exe --status` and
  `wsl.exe --list --quiet`, and require a user-selected installed distribution.
  Persist the selected name under `%LOCALAPPDATA%\\Yashik` using the standard
  library; `--distro` overrides it. A missing WSL executable, no registered
  distribution, an uninitialised Linux user, missing `curl`/`tar`, or a missing
  Linux Yashik binary is a clear non-zero error with the next command to run.
* Do not call `wsl --install`, DISM, PowerShell feature commands, or request
  elevation. Microsoft's WSL installation flow needs an elevated PowerShell
  and a restart, and the first distribution launch creates a Linux user
  ([official installation guide](https://learn.microsoft.com/en-us/windows/wsl/install)).
  The shim may print `wsl --install -d Ubuntu`, ask the user to restart and
  initialise Ubuntu, then ask them to rerun `yashik setup --distro Ubuntu`.
* After setup, `init <manifest>` and `check <manifest>` convert the sole
  manifest path to a Linux path with an argument-vector call to `wslpath`, then
  execute the Linux binary using `wsl.exe --distribution NAME -- ...`. `doctor`
  needs no conversion. Do not concatenate user arguments into a `sh -c` string;
  pass them as process arguments. Reserve a small, explicit forwarding grammar
  rather than forwarding unknown future flags.
* Probe the installed Linux `yashik --version` before forwarding. If it differs
  from the shim's compiled version, fail with an instruction to rerun `setup`;
  do not silently update the Linux binary.

The bootstrap pipeline necessarily uses a shell to connect the fixed release
asset to its SHA-256 verification, but the version is compile-time controlled
and values supplied by the user are not interpolated into that shell command.
The implementation should validate both the semantic version and the selected
distribution name before construction. Unit tests must cover malicious-looking
arguments as well as normal paths.

### WinGet package and manifest

Publish two *direct executable* release assets alongside the four Unix assets:

| Target | WinGet asset |
| --- | --- |
| `x86_64-pc-windows-msvc` | `yashik-v<version>-windows-amd64.zip`, containing `yashik.exe` |
| `aarch64-pc-windows-msvc` | `yashik-v<version>-windows-arm64.zip`, containing `yashik.exe` |

Use a multi-file `AlexGladkov.Yashik` manifest in `microsoft/winget-pkgs`: a
version YAML, an `en-US` default-locale YAML, and this installer YAML shape
with the real per-asset hashes:

```yaml
PackageIdentifier: AlexGladkov.Yashik
PackageVersion: "0.2.0"
InstallerType: zip
Commands:
  - yashik
Installers:
  - Architecture: x64
    NestedInstallerType: portable
    NestedInstallerFiles:
      - RelativeFilePath: yashik.exe
        PortableCommandAlias: yashik
    InstallerUrl: https://github.com/AlexGladkov/Yashik/releases/download/v0.2.0/yashik-v0.2.0-windows-amd64.zip
    InstallerSha256: RELEASE_SHA256
  - Architecture: arm64
    NestedInstallerType: portable
    NestedInstallerFiles:
      - RelativeFilePath: yashik.exe
        PortableCommandAlias: yashik
    InstallerUrl: https://github.com/AlexGladkov/Yashik/releases/download/v0.2.0/yashik-v0.2.0-windows-arm64.zip
    InstallerSha256: RELEASE_SHA256
ManifestType: installer
ManifestVersion: 1.12.0
```

The payload binary is named `yashik.exe`. WinGet's portable installer type is
designed for a standalone executable: it places the file in the
portable-package location, creates its command link, and supports uninstall and
upgrade. The upstream portable-app specification documents that behaviour
[here](https://github.com/microsoft/winget-cli/blob/master/doc/specs/%23182%20-%20Support%20for%20installation%20of%20portable%20standalone%20apps.md),
and the current community schema explicitly supports `portable` starting with
Windows Package Manager 1.3
[here](https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.28.0/installer.md).

Do not use two differently named direct `.exe` assets in this manifest.
WinGet permits a direct `InstallerType: portable` executable, but then its
command derives from that executable's filename. `PortableCommandAlias` is
valid for a nested portable file in a ZIP, while `Commands` is only metadata.
The ZIP therefore guarantees that both architectures install as `yashik`.
There are no `InstallerSwitches`: WinGet extracts and copies the nested portable
executable rather than running it as an installer, so `Silent` or
`SilentWithProgress` arguments would be incorrect. A portable install itself is
non-interactive. This is also why the executable must not launch `setup` during
installation; the WSL bootstrap remains an explicit `yashik setup` command. A
new terminal may be needed before the portable command link is on `PATH`.

The `InstallerUrl` must remain the immutable, direct GitHub Release URL and
each hash must be computed after upload. Microsoft requires the installer URL
and SHA-256, as well as non-interactive install and uninstall validation
([manifest](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest),
[repository validation](https://learn.microsoft.com/en-us/windows/package-manager/package/repository)).
Use `wingetcreate new` to generate the submission, then review the result
against the published asset rather than generating hashes in advance.

### Why the manifest must not provision WSL

Do not add `Dependencies.ExternalDependencies: [WSL]`: the current schema says
external dependencies have no WinGet behaviour. `PackageDependencies` can refer
only to packages from the same source. `WindowsFeatures` describes optional
features, but those can require a restart; it cannot ensure that a Linux
distribution has been installed and initialised. The relevant schema limitations
are explicit [here](https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.28.0/installer.md).

Even where a current WinGet client processes Windows-feature dependencies, that
would conflict with the selected product behaviour: enabling WSL is privileged,
may need a reboot, and must not occur as a side effect of `winget install`.
The runtime preflight is therefore the reliable and transparent requirement
check. A separate, opt-in `winget configure` file could later model WSL and an
Ubuntu package, but is a distinct configuration workflow, not a package
dependency.

### Windows CI and validation

The release workflow adds native Windows build jobs:

| Check | Runner | Evidence produced |
| --- | --- | --- |
| x64 build/unit/mock integration | `windows-2022` | `yashik-v<version>-windows-amd64.zip` and passing command-contract tests |
| ARM64 build/unit/mock integration | `windows-11-arm` | `yashik-v<version>-windows-arm64.zip` and the same tests |
| WinGet local-manifest test | `windows-2022` | `winget validate`, portable install, command probe in a new shell, upgrade, and uninstall |
| Real WSL bridge smoke | dedicated self-hosted Windows 11 runner with pre-provisioned WSL 2 and Ubuntu | `setup`, `check`, `init`, `doctor`, a converted manifest path, and exit propagation |

The mocked integration test injects a `wsl.exe` process test double and records
its argument vector. It proves that the shim invokes `--distribution`, uses the
exact versioned release URLs during `setup`, converts the one manifest path
before forwarding, never attempts feature installation, and returns the Linux
process exit code. It is the normal CI gate for the shim.

Do not claim a GitHub-hosted Windows runner proves the real bridge. The current
Windows Server 2022 runner image enables WSLv1 but ships no distribution, while
GitHub says nested virtualization on hosted runners is not officially supported
([current image contents](https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md),
[GitHub runner limitation](https://docs.github.com/en/actions/concepts/runners/github-hosted-runners)).
The self-hosted WSL 2 smoke test is the platform proof; it should be manually
available or scheduled, and retain its logs as release evidence.

## Execution acceptance checks

1. A protected `vX.Y.Z` tag produces four Unix archives, two Windows portable
   ZIPs, `SHA256SUMS`, and `install.sh`; every included binary reports exactly
   `yashik X.Y.Z` and succeeds for `--help`.
2. Fresh Linux x64 and ARM64 environments, and fresh macOS Intel and ARM64
   environments, install through the script, report the release version, and
   can run `yashik check` on the shipped example. Linux results do not stand in
   for macOS results.
3. The tagged script rejects unsupported platform combinations and a bad
   checksum without replacing an existing binary.
4. Each supported Homebrew platform passes `brew audit --strict yashik`,
   `brew install AlexGladkov/tap/yashik`, `brew test yashik`, and
   `yashik --version`.
5. Windows x64 and ARM64 portable executables pass their unit and mocked WSL
   command-contract tests. Each matching-architecture WinGet local-manifest
   test validates, installs, upgrades and uninstalls the shim without running
   setup.
6. A pre-provisioned WSL 2 runner proves the fixed-version `setup` flow and
   forwarded `check`, `init`, and `doctor`; a GitHub-hosted Windows mock test is
   never presented as that proof.
