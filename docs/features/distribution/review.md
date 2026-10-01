# Distribution review

Review date: 2026-10-01. Reviewer: independent Engineer assignment,
`gpt-6.1-sol` / `xhigh`, using the user's explicitly approved fallback from
the profile's Terra/max assignment. The reviewer authored Plan but did not
implement the product changes. This checkpoint covers Unix distribution at
`09268e2` (including `9013a5b` and the Rust 1.88/lint corrections).
The Unix checkpoint is clear. Follow-up through `407e1d3` has no
remaining code-review blocker. Static-runtime artifacts, WinGet validation,
installation and exact command alias have passed; final uninstall acceptance
and native macOS CLI fixture acceptance remain pending in CI `36914713358`.
Follow-up Review includes the current installer regression
fixes and `src/install/util.rs` Darwin type corrections. All four native Unix
build/test jobs passed in `36909839543` and `36912709517`. Public release/tap
acceptance remains pending; local fixtures do not establish clean-host
acceptance.

## Findings

| ID | Severity/status | Location | Evidence and required change |
| --- | --- | --- | --- |
| R1 | P2, closed in follow-up | Original `scripts/install.sh:158`; fix now `:113,146` | GNU Wget's `--https-only` accepted a non-recursive HTTPS→HTTP redirect in the independent TLS fixture. Root amended Plan to curl-only. The fix removes Wget and restricts both curl transfer and redirect protocols to HTTPS. Re-review invoked the actual installer/curl against a trusted local HTTPS redirect: installer failed, the HTTP endpoint received no request, and the prior binary's bytes/inode/mtime were preserved. Missing-curl and protocol-argument regression tests pass. |
| R2 | P2, closed in follow-up | Original `scripts/install.sh:263`; fix now `:253` | Original byte-identical target with 0644 returned success while remaining unusable. The fix keeps executable identical targets unchanged and atomically replaces a non-executable identical payload with the verified 0755 stage, without force. Independent re-review confirmed changed inode, unchanged bytes, mode 0755, and no-op on the subsequent install. The new 0644 regression and existing inode/mtime preservation test pass. |

R1/R2 are closed. No open Unix code-review blocker remains at this checkpoint;
publication still requires successful native CI and external acceptance.
Both integration items identified before Windows landed are now addressed:
Homebrew checksum parsing permits only the two known optional Windows ZIPs,
requires all five Unix/script entries, and retains malformed/duplicate/unknown
entry rejection. Four generator tests pass independently. Production URLs
omit the redundant explicit version; local fixture URLs retain it. Root
reports production strict audit plus local install/test success. Workflow
assembly now requires the exact seven checksum rows, generates both package
manager manifests, and uploads exactly eight release assets after all gates.

## Checks and reasoning

Personally executed:

```sh
python3 -m unittest discover -s tests -p test_install_script.py
/bin/sh -n scripts/install.sh
bash -n scripts/package-release.sh
```

The initial 11 bootstrap tests and both syntax checks passed. After fixes,
all 14 bootstrap tests pass. Additional temporary
fixtures, kept outside the repository, confirmed:

- Before amendment, an isolated PATH without curl exercised Wget successfully;
  this branch is now intentionally removed. A bin directory containing an
  apostrophe produces a PATH hint which executes correctly.
- Formula generation uses all four exact tag-pinned URLs and the corresponding
  supplied hashes; missing, duplicate, malformed and unexpected manifest
  entries fail before generating a formula.
- Packaging rejects the stale local `0.2.0` release binary for requested 0.2.1.
  Packaging the local test-built 0.2.1 binary validates the one-member archive,
  mode 0755, extracted version/help/check probes and emitted SHA-256. This is
  packaging evidence, not proof that a released Linux binary is static.
- R1/R2 were reproduced independently of the initially passing suite and then
  rechecked against fixes as described above. The redirect recheck used real
  curl and trusted local TLS, with a test wrapper changing only the fixture URL.
- After Darwin utility fixes, `cargo test --locked --test installer_engine
  --test installer_sources --test installer_state` passed all 18 selected
  Linux tests, including filesystem containment/atomic write/state checks.

Code inspection confirmed archive checksum verification precedes extraction,
only the root member is streamed into a private same-directory stage, version
comparison happens before replacement, existing unknown programs are not
executed, non-regular targets are refused, and failed verification retains the
previous binary. The installer does not invoke sudo or edit shell profiles.

`tests/installer_engine.rs` now canonicalises the fixture root and uses the
same Darwin Library layout as `src/install/paths.rs`; child processes receive
matching HOME/XDG variables without modifying the test runner's global
environment. Linux layout is preserved. The Rust 1.88 requirement matches the
locked JSONC parser's let-chain syntax; the lint-only fixture change preserves
its formatted string.

Native CI run `36906969648` was reported by root as successful for both Linux
builds and failed on both Darwin builds with four existing type errors. The
follow-up utility diff changes the three variadic `openat` creation-mode
arguments to `libc::c_uint`, avoiding Darwin's narrow `mode_t` variadic
restriction. Fixed-arity mkdir/fchmod retain `mode_t`; security flags and
creation permissions are unchanged. `metadata_dev` casts the standard-library
device value back to the platform's `libc::dev_t`, matching the `stat.st_dev`
comparison on Darwin. Inspection found no weakening of symlink/identity checks.
The selected Linux checks pass; corrected native Darwin compile/test results
are still required from the next CI run.

The subsequent native CI run `36908444762`, as reported by root, compiles on
both Darwin targets but fails four library fixtures whose temporary roots
contain the platform's `/var` → `/private/var` alias. Follow-up Review on top
of `0415ef7` covers only root canonicalisation after directory creation in
the two `src/install/engine.rs` test fixtures, the
`src/install/adapters.rs::removal_tests::TempTree` helper, and
`tests/installer_state.rs::TempTree`. These changes affect test scaffolding
only; no production path validation, identity check, or symlink rejection is
removed. Resources intentionally created as symlinks inside the canonical
root remain symlinks and the negative assertions remain active.

Independent regression check created a Linux temporary directory with a
symlink alias, set only the child test process's `TMPDIR` to that alias, and
ran `cargo test --locked --lib --test installer_state`. All 21 library tests
and five state tests passed, including quarantine failure preservation,
artifact cleanup, rules preservation, and linked state/parent rejection.
No open review finding results from the narrow fixture diff. This reproduces
the aliased-temp-root condition on Linux; the next native Darwin CI run is
still required for macOS acceptance.

CI run `36909421287`, as reported by root, passed the Darwin library,
adapter, and engine checks before `installer_sources` reached a Unix socket
fixture longer than the platform's socket pathname limit. Review of
`a0e5a1a` confirms it only shortens the source test's temporary-directory
prefix and the special-file subdirectory, with its `Source::Local.path`
updated consistently. PID/counter uniqueness, canonicalisation, cleanup,
socket creation and the rejection assertion remain unchanged. The specific
`local_snapshot_rejects_dangling_and_external_symlinks_and_special_files`
test passed independently on Linux; root reports all nine source tests pass.
No new review finding. Native Darwin CI must still confirm the shorter path
on its actual runner before platform acceptance.

Workflow inspection confirmed read-only default permissions, write access only
for draft creation, strict tag/Cargo/script agreement, explicit native Unix
runners and architecture checks, target tests before packaging, static ELF
checks on Linux, macOS deployment target 13.0, checksums from final archive
bytes, production formula audit followed by same-artifact local Homebrew
install/test, and build-only `workflow_dispatch`. A retry leaves an already
published release unchanged. Actual native job outcomes must be recorded by
Validate rather than inferred from this inspection. The root-reported 68 Rust
tests pass locally; this reviewer did not rerun the entire Rust suite.

The final validation record must separately identify native macOS/Linux CI,
real non-root Ubuntu installation, public release asset downloads, production
tap install/audit/test, and the pending Windows/WSL conditions. README's release
commands and its validation link need the completed publication/report before
being treated as publicly usable installation evidence.

## Windows bridge, packaging and workflow review

Scope: `tools/windows-launcher/**` excluding build output,
`scripts/package-windows.py`, `scripts/generate-winget.py`,
`tests/test_windows_release.py`, Homebrew generator follow-up, and the complete
release workflow. Initial Windows checkpoint was on top of `a0e5a1a`;
follow-up covers `ed028fa`, `3477e42`, `6be860c`, and the one-flag CI
uninstall correction `38bf083` and exact-name selection in `407e1d3`.

| ID | Severity/status | Location | Evidence and correction |
| --- | --- | --- | --- |
| W1 | P1, closed | Windows-only path fixture in `tools/windows-launcher/src/lib.rs` | Original fixture selected version-probe argv instead of wslpath; the first fix also exposed its explicit-distro mock/default-command mismatch in native CI `36911723568`. The fixture now supplies `--distro Ubuntu` and selects the actual conversion call by command. Native x64 tests/build/probes pass in `36912709517`. |
| W2 | P2, closed | `parse_default_distro` in launcher | Original parser truncated a valid registered default `Ubuntu Stopped Dev` at its name's state word. Independently reproduced through public `run`. Current parser matches registered names with a whitespace boundary, requires trailing state/version columns, and chooses the longest name. Same-state-name, embedded spaces, UTF-16, and localized-state fixtures pass. Current Microsoft WSL source hard-codes English state text; the localized fixture establishes parser robustness, not a claim that current WSL localizes that column. |
| W3 | P2, closed | WinGet generator digest fields | Unquoted accepted all-numeric hex digests became YAML integers and failed Microsoft's string schema. Both hashes are now quoted, with regression assertions. Independently parsed all-numeric hashes and validated all three generated documents against official schemas, first 1.12.0 and then final 1.10.0. |
| W4 | P2, alias acceptance closed; uninstall pending | WinGet portable install workflow | Prefix search `yashik*` and full-path invocation could accept a wrong/suffixed alias. An intermediate fix appended Links to PATH and could mask absent persistent registration. Final workflow requires exact `Links/yashik.exe`, asserts its directory is in expanded user/machine registry PATH, refreshes process PATH without adding a fallback, and launches a fresh PowerShell process which resolves and runs bare `yashik`. CI `36913365882` reaches uninstall after passing exact alias, persisted registry PATH, fresh-shell bare command and version assertions. Uninstall still must remove that exact alias in the final CI. |
| W5 | P2, closed with real WSL1 evidence | WSL1 rootfs fixture URL in workflow | Original moving Noble WSL index no longer contains the requested tarball, causing download to skip the real smoke. The fix uses existing official Ubuntu Base 24.04.5 amd64 rootfs and its exact SHA256SUMS entry. Independent source inspection confirms the URL/hash; CI `36912709517` genuinely imports it and reaches the final bridge-smoke-passed line. |
| W6 | P2, closed with both shipped-architecture artifacts | Windows release build and runtime dependency gate | Independent parsing of the actual ARM64 ZIP from `36911723568` found `VCRUNTIME140.dll` imports, an undeclared runtime prerequisite for the single portable executable. Both release builds now use `-C target-feature=+crt-static`, configure their corresponding MSVC developer environment, and reject VCRUNTIME/MSVCP DLL dependencies with successful `dumpbin /DEPENDENTS` before packaging. Both dependency gates pass in CI `36913365882`. The reviewer downloaded both produced ZIPs and independently parsed their PE import tables: only api-ms-win-core-synch-l1-2-0.dll, bcryptprimitives.dll, KERNEL32.dll and ntdll.dll remain; no VCRUNTIME/MSVCP/CONCRT imports. |
| W7 | P2, closed with actual client validation | WinGet manifest version and portable Scope fields | Actual client 1.11.510 in CI `36912709517` returns nonzero with unsupported portable Scope and 1.12.0 header warnings. Final generator omits Scope and emits 1.10.0 headers/ManifestVersion. All three final manifests independently pass official 1.10.0 schemas; six Python tests pass. Workflow retains strict nonzero failure, with no warning bypass. Client 1.11.510 now reports `Manifest validation succeeded.` with zero exit in CI `36913365882`; subsequent portable ZIP download hash verification and installation also pass. Uninstall acceptance is recorded separately below. |

All findings were sent promptly to root and the Windows Engineer. No additional
code-review blocker remains through `407e1d3`. W6/W7 and the W4 alias checks
are closed by native evidence; uninstall is the remaining WinGet gate in CI
`36914713358`.

Personally executed after launcher fixes:

```sh
cargo test --locked --manifest-path tools/windows-launcher/Cargo.toml
cargo clippy --locked --manifest-path tools/windows-launcher/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s tests -p test_windows_release.py
python3 -m unittest discover -s tests -p test_homebrew_generator.py
```

Thirteen host-compatible Rust tests, six Windows packaging/manifest Python
tests, four Homebrew generator tests, and strict Clippy pass. Six Windows
Python tests were repeated after the final metadata amendment. These host
checks alone do not establish native Windows or ARM64 execution.

Independent temporary Rust executor captured the actual emitted setup script,
then `/bin/sh` executed it with an isolated fixture downloader. Bad, duplicate,
and missing `install.sh` checksum entries fail without executing the script;
a correct hash runs it with exactly `--version 0.2.1 --force` when explicitly
selected. New committed tests execute the same emitted script for correct/bad
hash and missing curl. Both HTTPS protocol restrictions are present and the
downloader URLs are compiled-version pinned.

Argument inspection confirms supplied distro/path values stay separate process
arguments. Windows paths are canonicalised and have verbatim namespace
prefixes removed before `wslpath -a -u`; absolute Linux paths skip conversion.
The fixed Linux launcher shell uses quoted `"$@"` and a fixed
`$HOME/.local/bin/yashik`; no user value is inserted into shell source.
`Command::status` inherits stdin/stdout/stderr for interactive forwarding and
propagates its exit code. Local help/version require no WSL call. Unknown
options, duplicate distro flags, missing/wrong Linux version, and conversion
failure prevent forwarding. No WSL installation, Windows feature enabling,
elevation, or implicit Linux setup occurs in forwarded commands.

ZIP inspection confirms one regular root `yashik.exe`, deterministic metadata,
CRC validation, 64-bit PE header and x64/ARM64 machine checks. Native executable
version/help probes run for x64; ARM64 packaging does not claim runtime
validation. WinGet manifests contain both pinned ZIP URLs, nested portable
command alias `yashik`, and an explicit WSL bridge description, without silent
switches or WSL feature dependencies. Portable Scope is omitted following
actual client validation; installation uses WinGet's portable defaults.

Official schema checks used OS-provided PyYAML/jsonschema and Microsoft
[version](https://raw.githubusercontent.com/microsoft/winget-cli/master/schemas/JSON/manifests/v1.10.0/manifest.version.1.10.0.json),
[installer](https://raw.githubusercontent.com/microsoft/winget-cli/master/schemas/JSON/manifests/v1.10.0/manifest.installer.1.10.0.json),
and [default locale](https://raw.githubusercontent.com/microsoft/winget-cli/master/schemas/JSON/manifests/v1.10.0/manifest.defaultLocale.1.10.0.json)
schemas. Actual client compatibility is a separate check; the prior 1.12.0
schema success did not imply client 1.11.510 acceptance.

Independently read CI `36912709517` job logs establish native x64 launcher
unit tests/build/probes and a real WSL1 bridge smoke. The rootfs download
matches the official
[Ubuntu Base checksum manifest](https://cdimage.ubuntu.com/ubuntu-base/releases/24.04.5/release/SHA256SUMS)
SHA-256 `e77b6f10c2590cef872b33ee9f635a0e3fd1f57fb074c0e52b5c7f56147a0c86`.
The job imports an isolated distro, chooses it as default, forwards doctor,
checks a Windows manifest path containing spaces, `$`, `&`, an apostrophe and
`λ`, then forwards init, doctor and repeated init. The final pass line is
present at 19:17:12 UTC and the distro is unregistered afterward. This smoke
installs the local Linux release artifact directly; it does not exercise
network `yashik setup`, WSL2, or ARM64 runtime behavior.

Full workflow review confirms version agreement across both crates, tag and
installer; six build artifacts; checksum generation from final archive bytes;
strict exact final checksum asset set; generated production Homebrew/WinGet
artifacts; all-gates dependency before assembly; eight release assets; and
tag-only draft creation. Dispatch builds and validates without publishing.
No published release is overwritten. Validate must record the passed static
CRT, WinGet validation/install/registry PATH/alias evidence and the remaining
uninstall result. Public
asset hash verification, tap installation, and the community PR remain later
external acceptance; WinGet availability remains pending until external merge.

### Narrow uninstall follow-up (`38bf083`)

Independent review confirms the only product/workflow diff adds
`--accept-source-agreements` to the isolated CI `winget uninstall` command.
Package ID, exact matching, disabled interactivity, strict exit-code failure,
and exact-alias removal assertion remain unchanged. Microsoft's
[official uninstall options](https://learn.microsoft.com/en-us/windows/package-manager/winget/uninstall)
explicitly support this flag and describe Store agreement prompts arising from
source queries during uninstall. It addresses the observed fixture prerequisite
without bypassing validation warnings or uninstall failure.

Independently read `36913365882` job `110541914672` confirms client 1.11.510
validates the production 1.10.0 manifests without warnings, verifies the actual
local ZIP hash, installs it, adds alias `yashik`, and changes PATH. Execution
passes the exact-alias/persisted-registry-PATH/fresh-shell/version assertions
before failing at uninstall because the `msstore` source agreements were not
accepted. This is partial acceptance, not successful uninstall. Both Windows
artifact builds and their static-runtime dependency gates pass; independent
PE import inspection of their downloaded ZIPs closes W6. CI `36913977013` passed source agreements but then reported no installed
package matching the catalog ID. The later exact-name correction and
its still-pending native acceptance are recorded below.

### Concurrent CLI fixture and local uninstall follow-up (`407e1d3`)

Independent review of the two-file commit is clear. CI `36913977013` records
Darwin Intel's `init_rejects_literal_env_values_without_leaking_them_or_writing_state`
failing at the fixture's `fs::canonicalize` with ENOENT. The prior helper
combined one process ID with wall-clock nanoseconds, which do not guarantee
unique concurrent names. A name collision could allow one fixture's Drop to
remove another's directory. This is a plausible diagnosis from source and the
observed failure, not an independently reproduced collision or proven clock
resolution on that runner. Rust documents
[platform-dependent SystemTime precision](https://doc.rust-lang.org/std/time/struct.SystemTime.html#platform-specific-behavior).

The helper now uses a process-global `AtomicU64::fetch_add` counter plus PID;
Relaxed ordering is sufficient for unique counter allocation. Directory
creation, canonicalisation, fixture cleanup, per-child HOME/XDG isolation and
all safety assertions remain unchanged. The diff contains no production code
or weakened assertion. Independently ran `cargo test --locked --test cli`
(all eight tests pass) and `cargo fmt --all -- --check` on Linux. Native Darwin
confirmation remains the CI gate; the Linux result alone does not establish it.

The second one-line change selects `winget uninstall --name Yashik --exact`
for the unique package installed from the isolated local manifest. Its
`PackageName` is exactly `Yashik`; Microsoft's
[uninstall options](https://learn.microsoft.com/en-us/windows/package-manager/winget/uninstall)
support exact name selection. There is no primary-source claim here that a
local manifest must always be queried by name; the previous actual ID lookup
failed and the next native run must establish this fixture's name lookup.
Accepted source agreements, disabled interactivity, strict nonzero rejection
and exact alias-removal assertion remain. User-facing catalog-ID commands are
unchanged. CI `36914713358` must confirm both the Mac fixture and successful
WinGet uninstall/removal before acceptance is complete.
