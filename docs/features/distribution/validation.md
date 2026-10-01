# Distribution validation

Status: `v0.2.1` is published at the [GitHub release page](https://github.com/AlexGladkov/Yashik/releases/tag/v0.2.1).
The annotated tag points to source commit `419cd201`; the tag-triggered workflow
passed and its eight assets and checksums were verified. The anonymous Ubuntu
install and product smoke test also passed, with limits noted below.
The build-only workflow run [36914713358](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358)
has all six build jobs, WinGet install/uninstall, and real WSL1 smoke green.
All four Homebrew checks and release-asset assembly also passed; the draft step
was skipped because this was a manual dispatch. The tag-triggered run has all
build jobs, WinGet, WSL1, all four Homebrew checks, assembly, and draft
creation green. An earlier full run,
[36913365882](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882),
passed all six build jobs, all four Homebrew fixture jobs and the genuine
WSL1 smoke. Its WinGet uninstall failed; a later attempt exposed a macOS Intel
ENOENT error in concurrent temporary-directory fixtures; a timestamp collision
was suspected. That attempt also failed WinGet uninstall by package ID.
Both issues are resolved in the latest run. Homebrew tap and WinGet community
submission details are recorded below.

## Local checks

- Root crate: formatting, strict Clippy, and all 68 Rust tests passed.
- Installer bootstrap: all 14 Python tests passed. The independent review also
  tested the real curl-based installer against a trusted HTTPS-to-HTTP redirect;
  it refused the redirect without contacting the HTTP endpoint.
- Windows bridge: formatting, strict Clippy, and all 13 host-compatible Rust
  tests passed. The six Windows packaging/manifest tests and four Homebrew
  generator tests passed.
- A separate filesystem regression check passed 21 library tests and five
  state tests with `TMPDIR` pointing through a symbolic link.

## CI evidence

Run [36913365882](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882)
uses `workflow_dispatch`, so it builds and validates without publishing. Its
results are retained below as earlier complete-run evidence; the current
rerun is summarized after that table.

| Gate | Result in this run |
| --- | --- |
| Version agreement, format, strict Clippy, Rust tests, Python bootstrap/package-manager tests | Passed ([release check](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110541266761)) |
| Linux x86_64/aarch64 and macOS Intel/Apple Silicon native build, test, archive, and installer jobs | All four passed ([Linux x86_64](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110541579824), [Linux aarch64](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110541579789), [macOS Intel](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110541579890), [macOS Apple Silicon](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110541579808)) |
| Linux static ELF checks | Passed on both Linux jobs; direct inspection of their downloaded CI archives also found no ELF interpreter or dynamic `NEEDED` libraries. The extracted x86_64 binary reports `yashik 0.2.1`. |
| Native Homebrew audit/install/test | All four jobs passed ([Linux x86_64](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110542445677), [Linux aarch64](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110542445592), [macOS Intel](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110542445546), [macOS Apple Silicon](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110542445616)). |
| Windows x64 build/tests/probes and ARM64 cross-build | Passed ([x64](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110541579755), [ARM64](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110541579685)). Both builds use static CRT and pass the workflow's PE import gate. Direct inspection of both downloaded ZIPs confirmed one root `yashik.exe`, PE machines `0x8664`/`0xAA64`, and no `VCRUNTIME` or `MSVCP` imports. ARM64 runtime execution remains unverified. |
| WinGet production manifest validation | Passed with client `v1.11.510`; the three 1.10.0 manifests were accepted. |
| WinGet portable x64 install, alias, and fresh-shell probe | Package hash verification and installation passed. The exact `Links/yashik.exe` alias appeared in persisted user/machine PATH, and a new PowerShell process resolved bare `yashik --version` to `yashik 0.2.1`. Uninstall by ID first stopped at Store source agreements; after accepting them in run 36913977013, WinGet reported no installed package matching the ID. The newer successful result is recorded below. |
| Real WSL1 bridge smoke | Passed ([job](https://github.com/AlexGladkov/Yashik/actions/runs/36913365882/job/110542445487)). The runner imported official Ubuntu Base 24.04.5 amd64 into an isolated WSL1 distro after checking its SHA-256 against the [official checksum manifest](https://cdimage.ubuntu.com/ubuntu-base/releases/24.04.5/release/SHA256SUMS) (`e77b6f10c2590cef872b33ee9f635a0e3fd1f57fb074c0e52b5c7f56147a0c86`). It used the local Linux x86_64 release archive, checked a Windows manifest path containing spaces, `$`, `&`, an apostrophe and `λ`, then ran `init`, `doctor`, and repeated `init`; it unregistered the distro afterward. This does not validate `yashik setup` downloading from GitHub. |

## Latest build-only rerun

Run [36914713358](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358)
was dispatched against source commit `407e1d3` and does not publish a release.
Version agreement, formatting, strict Clippy, Rust tests, Python installer and
packaging tests, and Windows bridge checks all passed
([release check](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110545747714)).
All six build jobs passed (four native Unix builds, native Windows x64, and a Windows ARM64 cross-build): [Linux x86_64](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110546082126),
[Linux aarch64](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110546082263),
[macOS Intel](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110546082402),
[macOS Apple Silicon](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110546082239),
[Windows x64](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110546082130),
and [Windows ARM64](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110546081999).
Both Windows builds passed the PE import gate for avoiding the MSVC
redistributable. Linux static ELF checks, archive probes, and same-artifact
installer tests passed on their native runners.

WinGet client `v1.11.510` accepted the production manifests. The raw log for
the [WinGet job](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110546397970)
confirms the x64 ZIP hash, successful portable installation, creation of the
exact `yashik.exe` alias, and persisted WinGet Links directory in the User or
Machine PATH. A fresh PowerShell process resolved bare `yashik` to that alias
and reported `yashik 0.2.1`. Uninstall by exact package name then completed
successfully, and the workflow's final assertion confirmed the alias was gone.
The genuine isolated [WSL1 smoke](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110547766361)
also passed.

All four Homebrew jobs and release-asset assembly passed in the manual run:
[Linux x86_64](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110547766661),
[Linux aarch64](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110547766554),
[macOS Intel](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110547766804),
and [macOS Apple Silicon](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110547766603); the [assembly job](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110549854030)
completed successfully. I downloaded its `yashik-release-assets` artifact:
the seven-entry `SHA256SUMS` verified against `install.sh` and all six
archives, and the eight-file directory contained exactly those eight expected
release assets. Each tarball held one root `yashik` executable with mode
`0755`; each ZIP held one root `yashik.exe`. The extracted Linux x86_64 binary
reported `yashik 0.2.1`. The assembled Homebrew formula's four URLs and hashes
matched the release checksums, and both generated WinGet installer hashes
matched their Windows ZIP entries. This artifact came from a build-only run;
the [draft job](https://github.com/AlexGladkov/Yashik/actions/runs/36914713358/job/110549942594)
was correctly skipped.

## Tag workflow and published release

The tag-triggered [run 36915349771](https://github.com/AlexGladkov/Yashik/actions/runs/36915349771)
completed successfully. It passed version agreement, formatting, strict
Clippy, Rust/Python tests, all six build jobs,
WinGet manifest validation and x64 install/uninstall, the real WSL1 smoke, all
four native Homebrew audit/install/test jobs, and release assembly. The
[assembly job](https://github.com/AlexGladkov/Yashik/actions/runs/36915349771/job/110550793390)
and [draft creation job](https://github.com/AlexGladkov/Yashik/actions/runs/36915349771/job/110550900474)
both passed.

The draft initially reported `isDraft: true`, `tagName: v0.2.1`, and exactly
eight uploaded assets. After publication, the release API reports `isDraft: false`
at the [canonical release page](https://github.com/AlexGladkov/Yashik/releases/tag/v0.2.1).
I downloaded the draft assets and verified `SHA256SUMS` with `sha256sum -c`;
all seven entries matched `install.sh` and the six archives. The final tagged
WinGet manifest hashes also match the Windows ZIPs. They differ from the earlier
manual-dispatch ZIP hashes, so the tagged release checksums below are
authoritative. Anonymous fetches of the pinned `SHA256SUMS` and `install.sh`
returned HTTP 200 and matched the downloaded release bytes. The GitHub
`releases/latest/download/install.sh` URL also returned HTTP 200 and matched
the pinned script byte-for-byte.
Every final tarball contains one root `yashik` executable with mode `0755`, and
each ZIP contains one root `yashik.exe`. The extracted Linux x86_64 binary
reports `yashik 0.2.1`; the Windows executables identify as x64 and ARM64 PE
files, respectively.

| Published asset | SHA-256 |
| --- | --- |
| `install.sh` | `0bf0037ba211fed07562b0da7b50b6cc99c66766a82c8b2dbd0363358198a691` |
| `yashik-linux-x86_64.tar.gz` | `ca0e44ebe46acb2d6f170435d1ce53664a326d3ce48e7702b29cce48526cf200` |
| `yashik-linux-aarch64.tar.gz` | `3e3d167e356d4cc385c10878ddf338a5060886382d0cc14415f0bf7b320b6486` |
| `yashik-macos-x86_64.tar.gz` | `491a0d326012ed0e038d4652a6481f59bde526a8a8e65b05e769dff6b355f20c` |
| `yashik-macos-aarch64.tar.gz` | `cc11ec64fbdbbdc2498108d9de86799b19c5e1288c699edf02050b6bb454e1b0` |
| `yashik-windows-x86_64.zip` | `dedbaf98ed1816bf02103b7f9f3e5f5358248b8cce8898a3f03af6ae7f878258` |
| `yashik-windows-aarch64.zip` | `67ce7d8ca4e87ada7e192ca9258d58b4f8ee09091423aaff9cb73bc1bfaf5368` |

The public release metadata reports the pinned `/download/v0.2.1/` asset URLs.
Public asset byte checks and the fresh-home install flow passed.

## Previous rerun and artifact inspection

Run [36913977013](https://github.com/AlexGladkov/Yashik/actions/runs/36913977013)
passed version/lint/tests and the Linux x86_64/aarch64, macOS Apple Silicon,
and both Windows build jobs. Its macOS Intel job failed while concurrent CLI
tests shared a timestamp-derived temporary directory; the root owns a unique
counter fix in the later code ([failed job](https://github.com/AlexGladkov/Yashik/actions/runs/36913977013/job/110543564523)).
Its WinGet manifest validation, x64 install and fresh-shell alias probe passed,
but uninstall by package ID returned “No installed package found matching
input criteria” after source agreements were accepted ([job](https://github.com/AlexGladkov/Yashik/actions/runs/36913977013/job/110544242868)).
The workflow now targets `--name Yashik --exact`; this succeeded in the latest
build-only and tag-triggered runs above. WSL1, Homebrew and assembly were
skipped in this older run because required jobs failed.

The archives from this failed run were structurally inspected. Their Windows
ZIP hashes differ from the later build-only run and final tagged draft; use the
tagged draft's verified checksums above.

## Post-publication checks and remaining limits

The post-publication Ubuntu flow ran anonymously as UID 65534 with a fresh
home. The literal `curl .../releases/latest/download/install.sh | sh` command
installed Yashik 0.2.1; a repeat preserved the binary's inode, mtime and mode.
`check` accepted the manifest, and the initial `doctor` reported no recorded
installation. `init` installed the managed Node runtime, recognized Codex CLI
0.159.3, and installed two VoltAgent agent bindings from native TOML files.
Verification checked the TOML hashes and fields, CLI versions, read-only
`doctor`, and repeat-init preservation of file bytes, inode, mtime and mode.
The root user's existing Yashik 0.2.0 executable remained unchanged.

The public formula was pushed to [AlexGladkov/homebrew-tap at commit 10fe93d](https://github.com/AlexGladkov/homebrew-tap/commit/10fe93d8092330e0fb9d06dba41d1d21fff1f540).
It was generated from all seven final checksum rows. A real
`brew install AlexGladkov/tap/yashik`, `brew test --verbose`, and
`brew audit --strict` passed against the public formula and release archive;
the tag workflow also passed all four native formula jobs.

The final hash-pinned manifests are in [WinGet PR #445205](https://github.com/microsoft/winget-pkgs/pull/445205).
At last check, the PR was open and awaiting review. Pull request, manifest,
URL, URL-domain, manifest-policy, catalog-content and CLA checks had passed;
the installer scan was running, and installation validation and installer
metadata checks were queued. The WinGet command becomes available after the
community PR merges.

Authentication and remote model access were not checked during the Codex/VoltAgent
smoke. Codex warned that it would not create helper binaries because this
throwaway test home's `.codex` directory was under `/tmp`; package installation,
agent bindings, hashes, and repeat-init checks still passed. Windows ARM64
runtime and WSL2 are unverified. Native macOS CI ran on macOS 15 with deployment
target 13.0; macOS 13 itself was not tested.
