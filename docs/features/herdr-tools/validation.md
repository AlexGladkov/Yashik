# Validation: Herdr tools and Yashik 0.3.0

Status: final reviewed fallback revision passes 94 Rust tests, formatting and strict Clippy. Live Linux x86_64 first/repeat installation and read-only doctor checks passed. Actual primary and official-mirror requests were observed on the live server. CI release artifacts and publication are still pending; v0.3.0 is not yet published.

## Release integration checks

- Root and Windows Cargo versions/lockfiles and scripts/install.sh default updated to 0.3.0.
- `cargo test --locked --manifest-path tools/windows-launcher/Cargo.toml`: passed, 14 tests on Linux; these are mock WSL tests, not a real Windows/WSL execution.
- `cargo fmt --manifest-path tools/windows-launcher/Cargo.toml -- --check`: passed.
- `cargo clippy --locked --manifest-path tools/windows-launcher/Cargo.toml --all-targets -- -D warnings`: passed.
- `python3 -m unittest discover -s tests -p 'test_*.py'`: passed, 24 installer/bootstrap/package-generator tests.

The first bump run exposed stale 0.2.1 version assertions in installer fixtures and one Windows launcher test. They were corrected and all affected checks rerun successfully.

## Linux validation before fallback revision (2026-10-06)

Commands run against the current `0.3.0` working tree:

- `cargo fmt --all -- --check` — passed.
- `cargo test --locked` — passed, 88 tests: 29 library unit, 1 binary unit,
  58 integration; no failures. Doc tests had no cases.
- `cargo clippy --locked --all-targets -- -D warnings` — passed.
- `cargo fmt --manifest-path tools/windows-launcher/Cargo.toml -- --check` —
  passed.
- `cargo test --locked --manifest-path tools/windows-launcher/Cargo.toml` —
  passed, 14 Linux-hosted mock WSL tests.
- `cargo clippy --locked --manifest-path tools/windows-launcher/Cargo.toml --all-targets -- -D warnings` — passed.
- `python3 -m unittest discover -s tests -p 'test_*.py'` — passed, 24 tests.

Focused acceptance runs also passed: `cargo test --locked --test yaml_schema herdr`
(2 tests), `cargo test --locked --test cli check_accepts_herdr_without_harnesses_and_does_not_create_installer_state`
(1), `cargo test --locked install::tools::tests::` (8), and
`cargo test --locked --test installer_state state_without_tools_field_remains_backward_compatible`
(1). These cover closed schema/defaults and tool-only `check`; metadata and
platform restrictions; checksum/version checks before activation; bounded
unknown-length download overflow and child reap; pinned repeat preserving
inode/mtime; unmanaged and symlink collisions; declined removal; doctor
tamper detection; and loading pre-tools state. Focused tests overlap the 88
tests above and are not additional unique cases.

## Environment

Live SSH alias test-server-root: Linux x86_64, reachable, curl and tar present.
At initial inspection, setup directory `/root/opt/yashik-setup` was empty; no
Yashik/Herdr was found on the default SSH PATH. There is no `/opt/yashik-setup`.

## Remaining validation and release work

- Final release workflow and cross-platform CI, verification of all release assets, server confirmation with the CI-produced binary, publication and public installer confirmation are pending.
- The initial real metadata failure left no tool ownership or binaries, confirmed by a later remote state read. New fixtures cover transport failure and retry. The subsequent real installation succeeded with fallback; detailed evidence follows below.
- Linux local/server checks and mock WSL tests do not establish native macOS or Windows/WSL runtime support. macOS 13, Windows ARM64 runtime, and WSL2 remain unverified.

## Preliminary live server acceptance (before publication)

Built `cargo build --release --locked`, packaged with `scripts/package-release.sh`, transferred archive to /root/opt/yashik-setup and extracted into .validation. Initial archive SHA256: 8d079fcf2a24de197ec889be82c07f5ed95b28347961da06cd00cc414417e27d (this predates review fixes and is not a final release asset).

- Packaged `--version`: yashik 0.3.0.
- `check ./yashik-compose.yaml`: valid Herdr-only manifest with mandatory `harnesses: {}` and Herdr pin 0.9.3.
- First bare `init`: correctly reported `Failed: tool/herdr — official Herdr download failed`; metadata GET exceeded 120s. No successful installation was claimed. This is not a passing installation acceptance.
- Diagnostic server `curl -4` GET to https://herdr.dev/latest.json timed out after 20s, receiving 20508 of 177574 bytes; HEAD returned 200.
- GET to official source https://raw.githubusercontent.com/herdrdev/herdr/master/distribution/latest.json succeeded with 177574 bytes on the same server.
- Root fetched both endpoints locally: identical SHA256 daad8d6fe4524fbb78caa287b7936a6a7400262dbe872c8bc3532c3b371bf05e.

Independent Review R1 identified an unbounded curl stdout buffer on old curl
versions; the fix is present and independently reviewed. The final Linux test
suite passed, including the regression test that overflows an unknown-length
child stream and confirms the child is reaped. The subsequent Sol plan revision and implemented official metadata fallback resolve that site transport failure; final evidence follows below.

## Follow-up: failed-install ownership and plan revision

2026-10-06: root re-read actual remote state after the failed metadata download. `State.tools` keys are empty, `operations["tool/herdr"].outcome` is `failed`, and neither /root/.local/bin/herdr nor /root/.local/share/yashik/tools/herdr/versions/0.9.3/herdr exists. This directly confirms no tool ownership or binaries were left by that real failed attempt.

The Sol/xhigh Strategist became available again and approved the fixed official metadata transport fallback in research.md and plan.md. That revision has since been implemented and independently reviewed; no model substitution or additional approval was needed.

## Final local validation after metadata fallback (2026-10-06)

Ran against the updated fallback implementation:

- `cargo fmt --all -- --check` — passed.
- `cargo test --locked` — passed, 94 tests: 35 library unit, 1 binary unit,
  58 integration; no failures. Doc tests had no cases. This includes fixtures
  for primary transport failure followed by the fixed mirror, preserving the
  canonical metadata identity, retry after failed install without claiming
  ownership, both metadata endpoints failing, invalid primary metadata with no
  mirror retry, local size/process errors with no retry, and HTTP-versus-local
  curl error classification.
- `cargo clippy --locked --all-targets -- -D warnings` — passed.

Windows launcher checks (14 tests), Python checks (24 tests), and independent
review of the fallback (14 Herdr-tool tests; no open findings) were completed
by their respective owners and were not rerun in this pass. The live server
acceptance is tracked separately by root; this local result does not stand in
for it.

## Successful live Linux x86_64 acceptance after fallback

Rebuilt the final reviewed fallback code with `cargo build --release --locked`, packaged and transferred to test-server-root. Preliminary updated archive SHA256: 31e64e29ee28cc53aef90aa6e663152404420579523f19bf7552c7d02afb36da (local build; CI release artifact verification remains pending).

In /root/opt/yashik-setup, the actual Herdr-only yashik-compose.yaml pins 0.9.3. Packaged `--version` reported yashik 0.3.0 and `check` passed. First bare `init` succeeded: Herdr 0.9.3 installed and verified. Active `herdr --version` reported herdr 0.9.3. Second `init` reported Unchanged. Active binary inode/mtime/size before and after repeat were identical: 278109 / 1791280864 / 29962088. `doctor` reported verified Herdr; state.json SHA256 was identical before/after doctor. Both active and versioned binaries matched official SHA256 18a8dc65f1c2fa485884344356dea1cfd911c6f06cf46fa78e193f4087f4dba7. Real remote State.tools records Herdr 0.9.3, operation outcome unchanged, and State.clis is empty. No harness CLI was installed by this tool-only manifest.

This is a real Linux server installation, not a mocked fixture. It does not establish macOS, native Windows, WSL2 or Windows ARM64 runtime acceptance. Authenticated model calls and Herdr interactive sessions are outside this provisioning smoke test.


A third real pinned init was observed through /proc without altering curl or the network. Both the primary metadata URL and fixed official GitHub mirror were actually requested; the run reported Unchanged and preserved the active binary inode/mtime/size again. Observed endpoint labels were recorded remotely in .validation/metadata-observed.txt. This confirms the real fallback branch, not only fixture coverage.

## First tag CI and pinned-toolchain test fix

First tag run https://github.com/AlexGladkov/Yashik/actions/runs/37447212569 passed root Rust checks and Windows tests, but Rust 1.88 Clippy rejected one version assertion's uninlined format argument at tools/windows-launcher/src/lib.rs:752. Changed that test-only format to the exact recommended `{VERSION}` capture; independent re-review found unchanged semantics. Local Windows 14 tests, fmt and strict Clippy passed again. No release was created or published by the failed workflow; the unpublished tag will be updated with an expected-old-tag lease, retaining the prior source commit.
