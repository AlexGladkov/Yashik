# Validation: Orca tools and Yashik 0.4.0

Status: final local validation and real Ubuntu 24.04 x86_64 first/repeat installation acceptance passed. Independent review has no open findings. Release CI, final artifact verification and publication remain pending.

## Research reference package

Root fetched the official v1.4.221 Linux x86_64 AppImage, verified SHA256 7bf17b3619c2a4f2346b1894c652e2790b89277e501a78ce8911249a5e12f0f6, and extracted it in a separate local reference directory. `timeout 15 ./squashfs-root/resources/bin/orca-ide --version` returned `1.4.221`. Expanded size is about 647MiB; internal links are relative. This is upstream contract research, not installation acceptance.

## Release integration prechecks

Root prepared version 0.4.0 in root/Windows Cargo manifests and lockfiles and install.sh defaults/help. Installer fixture version expectations were updated.

- `python3 -m unittest discover -s tests -p 'test_*.py'`: 24 tests passed.
- `cargo test --locked --manifest-path tools/windows-launcher/Cargo.toml`: 14 tests passed on Linux (mock WSL).
- Windows launcher `cargo fmt ... -- --check` and strict Clippy: passed.

These checks do not establish real Windows/WSL runtime acceptance. The final Rust suite and independent review are now green. Real Linux server installation and CI packaging remain root-owned checks; no release publication is claimed.

## Final local validation

Run on 2026-10-06 from the repository root after the Orca implementation and schema fixture edits stabilized:

- `cargo fmt --all -- --check` — passed.
- `cargo test --locked` — passed: 126 tests, 0 failed, 0 ignored; 0 doc tests. Breakdown: 51 library, 1 binary, and 74 integration tests.
- `cargo clippy --locked --all-targets -- -D warnings` — passed.

The broad Rust run includes the acceptance coverage in O1–O5:

- **O1 schema/effective behavior:** `yaml_schema` (8) covers closed Orca schema, defaults, SemVer, malformed/unknown/duplicate/null inputs, required `harnesses`, and coexistence; `effective_manifest` (2) and CLI checks cover tool independence, disabled behavior, Orca-only manifests, and read-only `check`.
- **O2 resolver:** unit fixtures cover exact/latest official asset resolution and reject bad tag, URL, digest, and platform metadata. Digest failure is also exercised before ownership or launcher activation.
- **O3 install:** fixtures cover checksum-before-activation, exact version probing, first install and unchanged pinned repeat, wrong version probe without ownership, and failed-upgrade rollback. These use test fixtures rather than an Orca process connected to the server.
- **O4 reconcile/removal:** fixtures cover unmanaged launcher collision, bundle/launcher drift blocking doctor/upgrade/removal, confirmation-time drift, pending and confirmed removal with neighbor/user-data preservation, and recorded previous-version reactivation.
- **O5 doctor/state:** fixtures cover offline/read-only doctor, historical bundles, unknown tool IDs, old Herdr records, and existing Herdr behavior through the broad suite.

The focused test commands recorded by Review (`yaml_schema` 8, CLI `check_` 7, `effective_manifest` 2, Orca unit tests 5, reconciliation fixtures 11) are included in the final `cargo test --locked` run; the full run is the final result for this source state.

These fixtures are separate from the real Linux acceptance recorded below. The Windows launcher suite uses mock WSL on Linux; it is not native Windows runtime validation. No real ARM64, macOS, native Windows, or WSL runtime result is claimed here.

## Server source preflight

A direct HTTPS-only request from test-server-root to the official pinned GitHub release API succeeded. It returned v1.4.221 and exactly one x86_64 AppImage with size 219617397 and digest sha256:7bf17b3619c2a4f2346b1894c652e2790b89277e501a78ce8911249a5e12f0f6. This checks source reachability, not installer behavior or binary download acceptance.


## Real Ubuntu 24.04 x86_64 acceptance

Root built the reviewed source with `cargo build --release --locked`, packaged the executable, and verified archive SHA256 9083bbf1b3d7a32ca51589c54300e8f4dba5948afc211875bf4409c0e9b5ad1e locally and on test-server-root before extraction. This is a preliminary local build, not the CI release artifact.

The actual /root/opt/yashik-setup/yashik-compose.yaml now pins Herdr 0.9.3 and Orca 1.4.221 with harnesses: {}. The previous manifest was backed up, and the update preserved the expected original contents before atomic replacement. Packaged Yashik reported 0.4.0; manifest check, first bare init, orca-ide --version, read-only doctor, and repeat init all passed. First init installed Orca; repeat reported both tools Unchanged. Orca launcher inode/mtime/size remained 282868 / 1791285964 / 106. Herdr's inode/mtime/size were identical before and after the Orca installation. State.json SHA256 was identical before/after doctor.

Real State.tools records both tools, with Orca's canonical pinned API/asset URL and official AppImage SHA256 7bf17b3619c2a4f2346b1894c652e2790b89277e501a78ce8911249a5e12f0f6. The streaming bundle fingerprint is aba4221d43c5d0efdc824f848f5de74f6f03972a6958cd775ecbfd1897dd4cbe. State.clis remains empty. An inspection of /proc executable paths found no running packaged Orca process, and listening ports matched the pre-install baseline (SSH, existing HTTP/HTTPS services and local DNS; no Orca listener).

This validates actual provisioning on the Linux x86_64 server without FUSE or a desktop. It does not exercise serve, authentication, pairing, interactive agent calls, native ARM64, macOS or Windows runtimes. The requested harness/agent/MCP setup remains separate. CI-produced artifact and public installer acceptance are still required before release completion.
