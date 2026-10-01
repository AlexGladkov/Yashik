# Engine review

Review status: **clear**. No blocking engine findings remain after the focused
re-review on 2026-10-01.

Review scope: `src/install/{engine,doctor,launcher,runtime,paths,executor,sources,util}.rs`,
`src/main.rs`, installer validation, and their focused tests. The review covered
the untracked installer implementation as well as the normal diff. The reviewer
changed only this review document; remediation was performed by the execution
team and independently re-read here.

## Outstanding blockers

None.

## Closed findings

### Closed P0 — installer semantic validation now precedes every private write

`build_effective` calls full manifest validation before `run_init` discovers or
creates installer paths. Validation now accepts only `latest` or strict exact
SemVer harness versions, limits `install.requires` to the closed `node|bun`
registry, and calls the same pure Git URL/ref validators used by source
resolution. The runtime retains the same checks as defense in depth.

The CLI integration cases cover malformed versions, unsupported runtime
requirements, unsafe Git URLs, and unsafe refs. Each rejected `init` leaves the
isolated data, cache, and state roots absent. `check` remains read-only.

### Closed P1 — CLI and self-launcher conflicts are owned, backed up, and rechecked

An existing common-bin harness launcher is inspected with a bounded no-follow
read before installation. A foreign or drifted launcher requires an explicit
replace choice; skip preserves it. Confirmed replacement writes an exact `0600`
backup below the private state root and the final atomic swap succeeds only if
the launcher still has the approved hash. An identical managed wrapper is a
no-op and creates no extra backup.

The persistent `yashik` self-launcher now follows the same preservation rule:
checked read, explicit confirmation, protected backup, and hash-conditional
atomic replacement. It no longer changes permissions through a path after a
separate type check.

Obsolete CLIs now enter an explicit removal flow after their bindings are gone.
Confirmed removal verifies launcher ownership and package version, backs up and
removes only the recorded wrapper and exact version prefix, and retains shared
runtimes. Decline and remaining bindings produce `pending-removal`.

### Closed P1 — final-reference MCP cleanup removes the artifact and launch specs

Binding removal now recomputes artifact references after both an already-missing
binding and a confirmed adapter removal. An artifact remains while any retained
or pending binding references it. Once the final reference disappears, cleanup
validates the deterministic managed root, removes matching obsolete launch
specifications, removes the artifact and ownership index with checked removal,
and deletes the state record only after cleanup succeeds. A final sweep also
collects artifacts left unreferenced by a failed or changed binding operation.

The focused regression test proves that one retained binding preserves a shared
artifact and launch spec, while clearing the final binding removes the directory,
specification, and `state.artifacts` entry.

### Closed P1 — doctor reports missing env names from command, args, and env overrides

Doctor now uses the launcher's interpolation parser over `run.command`, every
`run.args` value, and `run.env` references. It de-duplicates missing variable
names, reports only those names, and never reads or prints their values. Invalid
saved interpolation is reported as a failed launch specification rather than
silently ignored. The integration test covers all three locations and a secret
marker that must not appear in stdout or stderr.

### Closed P1 — doctor verifies the actual installed CLI in a bounded isolated probe

Doctor first verifies the recorded common-bin wrapper path and fingerprint, then
queries the curated executable inside the recorded exact-version prefix. The
initial remediation rejected a legitimate npm installation because npm exposes
`prefix/bin/codex` as a relative symlink. The final implementation resolves that
link, requires its canonical regular-file target to remain inside the exact
version prefix, and rejects escaping or missing targets.

The `--version` process runs with a cleared environment, curated PATH, private
temporary HOME/XDG/TMP directories, and that temporary home as its working
directory. Stdout capture is limited, stderr is discarded, and the process has
a five-second deadline. The regression fixture matches npm's relative-bin-link
layout, verifies wrapper drift/version mismatch, confirms a secret is not
exposed, and proves HOME-relative and cwd-relative side effects do not reach the
real home or caller directory.

### Closed — private paths and source reads retain no-follow boundaries

Private directory creation, the init lock, checked reads, atomic writes, and
checked file/tree removal use descriptor-relative no-follow operations on Unix.
The reviewed conflict paths now use those helpers rather than a type-check then
plain-read sequence. Source hashing/copying rechecks file identity and content
generation, rejects escaping/dangling links and special files, and never follows
links during managed-tree removal. Focused tests cover linked private roots,
linked lock/state paths, linked atomic-write parents, source escapes, and
internal-link removal without touching the outside sentinel.

## Checks run

- `cargo fmt --check` — passed.
- `cargo clippy --all-targets --all-features -- -D warnings` — passed.
- `cargo test --quiet` — passed, 67 tests.
- Focused malformed-preflight and npm-link doctor regressions — passed.

This clears the engine Review stage. Live clean-machine behavior and platform
coverage remain evidence for the separate Validate stage; this Linux review does
not establish macOS behavior.
