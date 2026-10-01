# Installer validation

Date: 2026-10-01. This is the Tester-stage record after the engine and
adapter/state reviews. The E1 source/runtime work was implemented in this
shared checkout by the tester, so those code paths received functional test
coverage here; separate review records provide the independent code review.
See [engine review](review-engine.md), [adapter/state review](review-adapters.md),
and [real Ubuntu execution evidence](live-ubuntu.md).

## Result

The corrected build passed the repository checks and the recorded Linux
acceptance scenarios. The user's primary Codex plus VoltAgent agent scenario
installed, passed `doctor`, and converged on a second `init`. The all-five
Linux smoke installed and probed all five CLIs, exercised all five configured
MCP launchers through real JSON-RPC calls, and reported one shared MCP
artifact. The all-five rerun reported the bindings and artifact unchanged.

## Local checks and CLI probes

Local verification ran on Ubuntu 26.04.1 LTS, x86_64, with `rustc 1.98.1` and
`cargo 1.98.1`. The checked local binary reported `yashik 0.2.0` and had
SHA-256 `a0dea9a8a7bd74b1ba58c674289ca4c8972cc0199d0c9d28f28e67fa93692aa6`.
This is the debug binary used for local CLI probes; the real-host release
binary is identified separately below.

| Command | Result |
| --- | --- |
| `cargo fmt -- --check` | PASS |
| `cargo test --all-targets` | PASS: 68 tests (21 unit, 47 integration) |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS |
| `git diff --check` | PASS |
| `target/debug/yashik --version` and `--help` | PASS; version `0.2.0`, expected command surface shown |
| `target/debug/yashik check examples/codex-voltagent.yaml` | PASS; Codex `0.159.3`, two agents listed |
| `target/debug/yashik check examples/all-five.yaml` | PASS; all five harness versions and resource names listed |

I also ran the debug CLI under an isolated temporary HOME/XDG root. `doctor`
on absent state exited 0, identified its read-only mode, and created no Yashik
state directory. An `init` manifest containing a literal secret in `run.env`
failed before private roots were created; neither stdout nor stderr contained
the secret. These commands repeated the relevant `tests/cli.rs` assertions
against the built binary.

The 68-test suite includes the following acceptance and failure coverage:

- Manifest preflight rejects malformed exact versions, open-ended runtime
  requirements, unsafe Git URLs/refs, and literal environment values before
  creating private state. `check` and `doctor` have read-only coverage.
- Git URL policy, immutable local directory snapshots, standalone local file
  selection, `from` traversal, symlink escapes, dangling links, special files,
  atomic writes through linked parents, and checked artifact removal are
  exercised in `installer_sources`, `installer_runtime`, and CLI tests.
- State, backup, and private path symlinks are rejected without writing to
  their referents. Directory removal tests cover restoration after cleanup
  failure and preservation of complete active targets.
- Reconciliation tests cover ownership and final-reference artifact cleanup;
  the engine regression keeps an existing portable rules block when a native
  rule is skipped. Adapter tests cover all five MCP projections, native Codex
  agent validation, skill discovery metadata, JSONC preservation, file-to-
  directory replacement, and preservation of existing MCP auth fields.
- Doctor tests check that missing environment output contains names but not
  values, that version probes discard stderr and run with isolated HOME and
  cwd, and that the configured npm relative-bin symlink remains inside the
  managed version prefix. Launcher tests cover launch-time interpolation and
  safe errors for missing environment values.

## Corrected-build Ubuntu acceptance

Root ran the corrected release binary, SHA-256
`ad2602991a97bfebe3a15936bc3e6240a1fbf986bcb0da6275b633cd955f6c49`, on the
authorized Ubuntu 24.04.2 LTS x86_64 host. The host had neither Node nor Bun
before the installer run. Detailed commands, package probes, and installed
VoltAgent file hashes are in [live-ubuntu.md](live-ubuntu.md). I received the
final run summary from the operator; I did not independently log into or
repeat changes on that host.

For the primary `examples/codex-voltagent.yaml` scenario, the corrected build
completed both `init` reruns with exit status 0, confirming the Node runtime,
Codex `0.159.3`, and the two native VoltAgent TOMLs installed by the initial
bootstrap and pinned to commit
`7add6913c53ccbbc250c481815c9c6afb02709c6`. Python 3.12 parsed the installed
files, required fields were present, and both file hashes matched the pinned
upstream bytes. The Codex `--version` probe returned `codex-cli 0.159.3`.
The second run retained both agent bindings as unchanged. `doctor` exited 0;
the before/after SHA-256, mtime, and mode snapshot of `.codex`, Yashik state,
and data was identical.
An additional snapshot across a further final-build `init` covered the Codex
package prefix, common-bin launcher, and both agent files; hashes, mtimes, and
modes were unchanged.

For `examples/all-five.yaml`, the final isolated-HOME run reported all five
CLIs ready from the already verified package cache and all supported resource
bindings unchanged. Probes returned Codex `0.159.3`, Claude Code `2.1.286`,
OpenCode `1.18.34`, Pi `0.99.2`, and OMP `18.4.8`. The final `doctor` reported
all five CLIs, bindings, and the shared artifact unchanged. Each configured
wrapper passed JSON-RPC `initialize`, `tools/list`, and `tools/call`; state
contained one shared MCP artifact for the five bindings. Pi's saved agent was
disabled in the fixture because Pi-core agents are explicitly unsupported.

## Review status and limits

The independent engine and adapter/state reviews are clear. The adapter review
records one nonblocking crash-recovery limit: an uncatchable kill after an
owned directory has been atomically renamed to its private quarantine can
leave hidden `.yashik-remove-*` data on disk. The active target is absent as a
single atomic operation, and a later init re-inspects the missing binding;
repeated kills can consume disk until a future scavenger or recovery command
cleans the private residue.

This evidence validates the recorded Linux x86_64 paths. No macOS x64/arm64
machine run was performed. Private Git authentication, provider login, remote
model requests, and arbitrary third-party MCP services were not exercised.
The echo MCP is a local protocol fixture, not evidence of provider
authentication or model delegation. Checksum registry and platform recipes
have tests, but those tests do not substitute for real macOS installation and
discovery. OpenCode/V1 and the documented Linux client versions are covered;
unknown versions remain unsupported pending verification.
