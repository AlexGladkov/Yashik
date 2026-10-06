# Validation: Orca tools and Yashik 0.4.0

Status: complete. Local validation, all release CI gates, verified final assets, final-source server acceptance, public installer upgrade and Homebrew publication pass. Yashik 0.4.0 is published.

## Research reference package

Root fetched the official v1.4.221 Linux x86_64 AppImage, verified SHA256 7bf17b3619c2a4f2346b1894c652e2790b89277e501a78ce8911249a5e12f0f6, and extracted it in a separate local reference directory. `timeout 15 ./squashfs-root/resources/bin/orca-ide --version` returned `1.4.221`. Expanded size is about 647MiB; internal links are relative. This is upstream contract research, not installation acceptance.

## Release integration prechecks

Root prepared version 0.4.0 in root/Windows Cargo manifests and lockfiles and install.sh defaults/help. Installer fixture version expectations were updated.

- `python3 -m unittest discover -s tests -p 'test_*.py'`: 24 tests passed.
- `cargo test --locked --manifest-path tools/windows-launcher/Cargo.toml`: 14 tests passed on Linux (mock WSL).
- Windows launcher `cargo fmt ... -- --check` and strict Clippy: passed.

These checks do not establish real Windows/WSL runtime acceptance. The final Rust suite and independent review are now green. Real Linux server installation and CI packaging remain root-owned checks; no release publication is claimed.

## Initial local validation before the portability patch

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

## Revalidation after the Linux portability patch

Run on 2026-10-06 against the current working-tree patch that calls the Linux `renameat2` syscall directly (for musl compatibility) and limits the filesystem-space helper to Linux:

- `cargo fmt --all -- --check` — passed.
- `cargo check --locked` — passed.
- `cargo test --locked --lib install::orca::tests::` — passed, 6 tests. This includes `bundle_rename_uses_atomic_no_replace_semantics`.
- `cargo test --locked` — passed, 127 tests, 0 failed, 0 ignored; 0 doc tests. Breakdown: 52 library, 1 binary, and 74 integration tests.
- `cargo clippy --locked --all-targets -- -D warnings` — passed.

Cross-target attempts on the available Linux x86_64 GNU runner:

- `cargo check --locked --target=x86_64-unknown-linux-musl` — could not compile because this Rust installation lacks the `x86_64-unknown-linux-musl` target standard library (`can't find crate for core`). The runner has no `rustup`, musl compiler, or Zig.
- `cargo check --locked --target=x86_64-apple-darwin` — could not compile because this Rust installation lacks the Apple target standard library (`can't find crate for core` / `std`). This Linux runner has no macOS SDK or Apple linker.

No musl or macOS cross-build/release CI runner was invoked, and no real ARM64, macOS, or native Windows runtime result is established by these checks. The Ubuntu server acceptance above was recorded before this portability patch; this validation turn did not repeat provisioning on the server, so that result does not cover the patched atomic rename path.

## Final-source CI Linux artifact acceptance

Release workflow https://github.com/AlexGladkov/Yashik/actions/runs/37457841143 builds source ad0b65678ef4278663a7e539455834e7f03ff3e4. Its Linux x86_64 musl archive SHA256 is 20054f552918620befc8ce8092fbfaeec5302b9df7699b48ca5e7ac48b8d8b75, verified locally and remotely before extraction. On the actual Ubuntu test server this artifact reports Yashik 0.4.0. Manifest check, two repeat init runs, Orca 1.4.221 version probe, and doctor pass. Both tools remain Unchanged, launcher inode/mtime/size remain identical, and state SHA256 before/after doctor is identical. No listener was added. This is final-source repeat/doctor acceptance; the initial live Orca installation predates the portability fix. Native Linux CI fixtures exercise the patched atomic activation path, including destination collision refusal.

## Published release and distribution validation

Release CI https://github.com/AlexGladkov/Yashik/actions/runs/37457841143 completed successfully for source ad0b65678ef4278663a7e539455834e7f03ff3e4: pinned Rust format/lint/tests, all four Unix native build/test/installer jobs, Windows x86_64/ARM64 packages, WinGet manifest and portable installation validation, and all four native Homebrew checks. The isolated WSL1 smoke actually passed with an imported official Ubuntu rootfs; this validates the Yashik bridge, not an Orca serve session.

All eight assembled release files were verified: seven SHA256 checksums, four single-executable Unix tar archives and two single-executable Windows ZIP archives. Draft assets matched CI assets byte-for-byte before publishing https://github.com/AlexGladkov/Yashik/releases/tag/v0.4.0. The release is public and latest. Both pinned and latest public install.sh match the verified artifact, SHA256 8071d66ec3f1976b9648362ffa2a114b4270cd24eb901f11aab11f11fb7f1a20.

On test-server-root the verified public installer upgraded /root/.local/bin/yashik from 0.3.0 to 0.4.0. Manifest check, bare init, read-only doctor, Herdr 0.9.3 and Orca 1.4.221 probes passed. Both tool launchers retained their inode/mtime/size; state SHA256 remained identical before and after doctor. The managed compose remains /root/opt/yashik-setup/yashik-compose.yaml. No serve service, authentication, pairing or agent/model session was started.

The owned AlexGladkov/homebrew-tap main branch now carries the byte-identical CI-tested production formula in its existing root yashik.rb, commit 13ab790. A briefly added Formula/yashik.rb duplicate was removed in that commit. No community WinGet publication is claimed.

The requested four-harness/agent/MCP configuration remains unfinished and is separate from this tool release. Orca live runtime acceptance is Linux x86_64 installation/version/doctor only; Linux ARM64 Orca runtime and serve remain untested. macOS/native Windows Orca backends are explicitly unsupported.
