# Independent review: adapters and state

Review date: 2026-10-01. Re-review scope: `src/install/adapters.rs`,
`src/install/state.rs`, `src/install/api.rs`, the adapter/state tests,
examples, and installer evidence. This was a review-only pass; this reviewer
changed this review record only.

The installer implementation is untracked in this checkout, so ordinary
`git diff` does not show the reviewed installer files. The review inspected
the untracked source and tests directly.

## Review gate

**Adapter/state code: CLEAR.** No blocking adapter or state defect remains in
the re-reviewed tree. The former backup-symlink and aggregate-rule blockers
have dedicated regressions, and the additional auth-field loss found during
this re-review is fixed and covered.

The corrected-build Ubuntu evidence is now recorded in `live-ubuntu.md`:
Codex plus both VoltAgent agents remained unchanged on the second init and
doctor snapshot, while all five MCP protocol/doctor checks passed. It does not
change the adapter/state code gate.

## Closed findings

| Priority | Finding | Current evidence |
| --- | --- | --- |
| P1 | Backup could escape through `state/backups` or a linked state parent. | Closed. `backup_target_snapshot` now validates and creates the private state/backup roots before reserving a child ([adapters.rs](../../../src/install/adapters.rs:1769)); the Unix helper opens each directory component with `openat(...O_NOFOLLOW)` before private writes ([util.rs](../../../src/install/util.rs:945)). `backup_refuses_a_symlinked_backup_root_without_writing_outside_state` and `backup_refuses_a_symlinked_state_parent_without_writing_to_its_target` pass. |
| P1 | An unsupported rule in a portable aggregate could remove a prior rule. | Closed. `process_rules` marks capability rejection as `portable_failed`, blocks the remaining portable members, and retains the aggregate ([engine.rs](../../../src/install/engine.rs:486)). `skipped_native_rule_preserves_the_entire_portable_rules_block` passes. |
| P2 | `Replace` could not change a foreign file into a skill directory. | Closed. The directory path now rejects `Merge` for a file but allows protected `Replace`, stages the directory, and swaps it ([adapters.rs](../../../src/install/adapters.rs:966)). `explicit_replace_swaps_a_foreign_file_for_a_skill_directory` covers apply, inspect, backup, remove, and no leftover normal quarantine. |
| P2 | Error during owned-directory deletion could expose a partially removed active target. | Closed for recoverable errors. Removal first atomically quarantines the target, then restores the intact tree or its protected snapshot if cleanup fails ([adapters.rs](../../../src/install/adapters.rs:1833)). Both injected-failure tests pass. |
| P2 | Replace dropped a recognized `environment` auth field for Claude, Pi, and OMP. | Closed. `preserve_auth_fields` now carries every key declared in `AUTH_FIELDS` regardless of harness ([adapters.rs](../../../src/install/adapters.rs:1499)). The new `json_mcp_replace_preserves_existing_auth_environment_for_all_clients` regression passes; an isolated public-API Claude repro also retained both `environment.TOKEN` and `auth`. |

## Verified adapter and state behavior

- The exact adapter gate is active before preparation: Codex `0.159.3`, Claude
  `2.1.286`, OpenCode `1.18.34`, Pi `0.99.2`, and OMP `18.4.8`
  ([adapters.rs](../../../src/install/adapters.rs:94)). Unknown versions are
  rejected rather than applying an unverified configuration shape. The
  all-five fixture confirms the expected MCP destinations and native shapes.
- Native Codex agents require valid TOML with matching `name`, `description`,
  and `developer_instructions`; native source bytes are installed unchanged.
  The byte-copy and mismatched-name tests pass. Skill frontmatter and safe
  destinations are likewise covered, including rejection of destination links.
- TOML, JSON, and JSONC changes are limited to the selected MCP entry. The
  OpenCode 1.18.34 direct `mcp.<key>` writer preserves the exercised foreign
  comment, sibling entry, and authentication data; the test also verifies that
  no V2 `mcp.servers` map is introduced.
- `state::save` and `state::journal` reject linked state paths and linked leaf
  files, use bounded checked reads, and write through the descriptor-relative
  atomic helper ([state.rs](../../../src/install/state.rs:95)). The state tests
  verify absent loads are read-only and that parent/target links do not modify
  their referents.
- Successful ownership is committed only after adapter mutation returns. The
  durable intent is recorded first, so an incomplete operation is visible to
  `doctor` rather than becoming inferred ownership.

## Nonblocking limitation

An uncatchable process kill after the quarantine rename and before cleanup can
leave a hidden `.yashik-remove-*` directory. An isolated public-API skill
fixture with 60,001 files was killed in that window: the visible target was
absent atomically and the full tree remained in the private quarantine. On the
next init, the missing desired-removed binding is re-inspected and cleared;
there is no foreign-file deletion or false successful ownership. This is
private temporary garbage, comparable to the intentionally retained partial
quarantine on an injected cleanup error, but repeated interrupted removals can
consume disk. A future startup scavenger or explicit recovery command can
reclaim it. It is not a release blocker under the current removal contract.

## Validation run

```text
cargo test --test installer_adapters              PASS: 12 tests
cargo test --all-targets                          PASS: 68 tests
cargo clippy --all-targets -- -D warnings         PASS
cargo fmt --check                                 PASS
git diff --check                                  PASS
```

## Published Ubuntu evidence

The final corrected-build record is now in `live-ubuntu.md`: SHA-256
`ad2602991a97bfebe3a15936bc3e6240a1fbf986bcb0da6275b633cd955f6c49`, the
primary Codex/VoltAgent rerun, all-five protocol handshakes, idempotence, and
doctor checks ([live-ubuntu.md](live-ubuntu.md:7)). It verifies Linux x86_64;
the recorded macOS, authentication, and live-model boundaries remain unchanged.
