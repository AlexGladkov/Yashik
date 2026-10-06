# Orca review

Independent Engineer review against `docs/features/orca-tools/plan.md`. Code
review is closed with no remaining product findings. The final formatting and
test-only follow-up was reviewed against the already approved code paths.

## Open findings

None.

## Resolved during review

- O1 null-version parsing: the Orca visitor now rejects `version: null`
  explicitly. The schema suite passes.
- Removal rechecks bundle, launcher, and history fingerprints after the user
  confirms and before it writes the intent or deletes files. A fixture mutates
  a bundle file in the confirmation callback and verifies removal stays
  pending with the changed files intact.
- Orca-only initialization no longer requests `curl` from the shared system
  bootstrap. If `curl` is absent, the Orca installer reports it as a required
  prerequisite without installing OS packages.
- The requirements document now says system bootstrap recipes require user
  confirmation and explicitly requires Orca's `curl` and Electron libraries to
  be preinstalled for an Orca-only manifest.
- Version-probe process cleanup: `capture_child_output` now terminates the
  isolated process group after the direct child exits and before joining its
  pipe readers. The 15-second limit therefore still covers inherited pipes.
- Post-rename bundle cleanup: hash errors and fingerprint mismatches now run
  fingerprint-checked cleanup; altered trees are preserved.
- Tree traversal memory bound: `collect_tree_bounded` now iterates directory
  entries incrementally and checks the global entry limit before descending.
- Doctor read-only behavior: active and historical bundle/launcher probes use
  ephemeral temporary HOME directories instead of creating a persistent
  `tools/orca/probes` directory under Yashik data.
- Download byte bound: the AppImage downloader receives the official asset
  size and streams to its private staging file with an exact write-time cap;
  metadata response capture also stops when it exceeds its byte limit.
- Portability follow-up: the no-replace rename uses Linux's direct syscall
  number and ABI constants, which are present for the supported x86_64 and
  aarch64 GNU and musl targets; other OSes compile the explicit unsupported
  stubs. The regression test confirms an existing destination is preserved
  and an absent destination is moved atomically.

## Focused verification

- `cargo test --locked --test yaml_schema` — passed, 8 tests.
- `cargo test --locked --test cli check_` — passed, 7 tests.
- `cargo test --locked --test effective_manifest` — passed, 2 tests.
- `cargo test --locked --lib install::orca::tests::` — passed, 5 tests.
- `cargo test --locked --lib install::orca::reconciliation_fixtures::` —
  passed, 11 tests.
- `cargo test --locked --lib install::orca::tests::bundle_rename_uses_atomic_no_replace_semantics`
  — passed, 1 test.

The root reports `cargo test --locked`, `cargo fmt --check`, strict Clippy,
and Windows bridge mock tests passing. Real Linux server acceptance remains
tracked in `validation.md` and is not inferred from these local gates.
The pre-existing `.agents/skills/business-feature/SKILL.md` change is excluded
from this review.
