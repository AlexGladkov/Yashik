# Independent review: managed Herdr tools

Review scope: the implementation diff for H1–H5, including manifest handling,
tool state, install/activation, doctor, removal, and the release version bump.
The review did not edit product code.

## Finding history

### P2 — The download size limit was not strict on older curl versions — resolved

Initial location: `src/install/tools.rs:607-633` (`fetch`).

The original `fetch` buffered curl stdout with `Command::output()` and checked
`max_bytes` only after curl exited. Since curl before 8.4.0 does not enforce
`--max-filesize` on unknown-length responses, this left H3's bounded response
requirement open.

The Execute change replaces that capture with `bounded_child_output` and
`read_bounded_stream` in `src/install/tools.rs:653-762`. The reader retains at
most the configured byte limit, reports overflow to the parent, and the parent
kills and waits for curl before returning. The same loop enforces a 125-second
hard deadline and kills/waits on timeout. Curl's own 120-second timeout remains
an additional bound.

Regression coverage: `unknown_length_stream_over_limit_is_killed_and_reaped`
feeds continuous output from a child without a known content length and checks
that overflow returns promptly and the child has been reaped. Independent
rerun: `cargo test --locked unknown_length_stream_over_limit_is_killed_and_reaped`
passed (1 test). Execute reports `cargo fmt`, Clippy, and the full 88-test suite
passed. The direct timeout branch was inspected; no separate stalled-child
timeout regression test is present.

## Criteria reviewed

- **H1 Schema:** the new root tool is closed to unknown names and fields,
  defaults to enabled/latest, validates exact versions, and is included in
  effective-manifest reporting. The tool-only `harnesses: {}` path is supported.
- **H2 Install:** Herdr is reconciled independently of harnesses. Metadata
  version, exact platform asset URL, checksum, and staged `--version` are
  checked before activation. A failed preflight preserves the old activation.
- **H3 Safety:** recorded paths are constrained to the expected Yashik paths;
  final symlinks and non-regular files are rejected; existing unmanaged
  activation collisions are preserved. The bounded-download finding above is
  resolved in the current tree.
- **H4 Reconcile:** recorded active and versioned binaries are fingerprinted
  before repeat/update/removal; an identical pinned repeat returns unchanged;
  removal requires confirmation and retains the state when it is declined.
- **H5 Doctor/state:** missing `tools` deserializes as an empty map for old
  state. Doctor does not fetch metadata, checks both recorded binaries, and
  probes the staged binary with a timeout, isolated home, and bounded captured
  stdout. Tool operation intents are surfaced through the existing interrupted
  operation reporting.
- **Release integration:** root and Windows crate versions, lockfiles, and
  `scripts/install.sh` are aligned at `0.3.0`. The release workflow has native
  Linux and macOS target test jobs; those CI results and the requested SSH
  installation remain external validation evidence, not conclusions of this
  code review.

## Review disposition

R1 is resolved. The streaming cap handles unknown-length output independently
of curl's version, and the overflow regression test passed. No open P0/P1 or
P2 finding remains in the reviewed Herdr code. The caller reports download
failure and does not proceed to activation when the bounded reader rejects an
oversized, failed, or timed-out transfer.

The earlier transient fixture spawn failure was reproduced by Execute as
Linux `ETXTBSY` (errno 26) when tests staged executables under the default
tmpfs-backed `TMPDIR` (6 failures in 30 runs); the same runs were clean when
staging on the repository filesystem. `TempTree` now creates unique temporary
roots beside the current test executable, keeping `TMPDIR` unchanged and
avoiding the tmpfs execution race. The current focused Herdr suite passed all
14 tests; Execute reports the full 94-test suite passed.

## Re-review: metadata transport fallback

The amended H3 fallback is implemented as planned. `fetch_metadata_using`
tries the fixed `METADATA_FALLBACK_URL` only after a typed
`DownloadError::Transport` from the primary metadata GET. Local failures return
without retry; `resolve_metadata` runs after the completed response and its
JSON/version/asset/checksum failures also return without retry. The fetched
asset remains a separate call and does not enter metadata fallback handling.
Only the two exact metadata constants and the official Herdr GitHub release
asset prefix are accepted by `fetch`; the installed state keeps the canonical
`https://herdr.dev/latest.json` identity.

The real downloader maps curl HTTP/network transfer errors, including HTTP
failure and partial-transfer codes, to `Transport`. Size overflow, allocation
or capture errors, spawn/wait failures, and other curl exit codes are `Local`.
Metadata requests use a 20-second curl timeout and a 25-second hard child
deadline; release assets use 120 and 125 seconds. The hard-deadline path kills
and waits for curl. The earlier streaming byte limit remains active for both
metadata endpoints and the release asset.

Fixture coverage exercises successful primary-timeout fallback with verified
install, both endpoints failing without an asset request, complete but invalid
primary JSON without fallback, local process/size errors without fallback,
retry without false ownership, and curl transfer-versus-local code
classification. Independent rerun: `cargo test --locked install::tools::tests`
passed (14 tests). No code-review finding remains for the amended H3 behavior.
The timeout termination path was inspected; there is still no dedicated
stalled-child timeout test.

## Release integration re-review

The pinned Rust 1.88 Clippy failure was a test-only `format!` interpolation
style issue in the Windows launcher. The one-line change now uses
`format!("releases/download/v{VERSION}'")`, preserving the assertion while
satisfying `uninlined_format_args`. No product behavior changed. Execute
reports Windows launcher tests (14), formatting, and strict Clippy passing
locally; the release integration finding is closed.

## Platform fixture re-review

The doctor tampering fixture now builds its recorded release asset URL from
`platform_key(std::env::consts::OS, std::env::consts::ARCH)`, matching the
platform-specific ownership validation in `verify_record`. This removes its
previous Linux x86_64 hardcode and addresses the Linux ARM64/macOS fixture
failures from CI run 37447470384. The focused doctor test passed locally on
Linux x86_64. Cross-target CI reruns remain validation evidence for the
respective runners.

### P1 — Strict Clippy gate briefly failed on staged-probe diagnostics — resolved

An intermediate staged-probe diagnostic edit bound an error used only inside
`#[cfg(test)]`, causing an `unused variable` failure in the normal library
build. The final tree removes that temporary diagnostic and restores the
ordinary error mapping. Independent rerun:
`cargo clippy --locked --all-targets -- -D warnings` passed, closing this H6
gate finding. The final `TempTree` change is test-only; the current focused
Herdr suite also passed all 14 tests.
