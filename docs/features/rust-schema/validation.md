# Validate: Rust schema stage

## Result

**PASS on Linux for stage 1 schema validation.** The full suite passed against
the repository's current production source, real YAML parser, tests, and example.
This validates the parser/core/CLI scope described in the plan; it does not
validate installation behavior or the later stages.

## Full production-source suite

Ran from the project root using `/tmp/yashik-full-check/Cargo.toml`. Its package
manifest matches the repository manifest; `src`, `tests`, and `examples` are
symlinks to the actual project directories. Cargo created a temporary lockfile
under `/tmp`; no root `Cargo.lock` exists.

```sh
CARGO_HOME=/tmp/yashik-build-probe-qqzqp0cl/cargo-home \
CARGO_TARGET_DIR=/home/neura/Документы/Projects/Yashik/target \
cargo test --offline --manifest-path /tmp/yashik-full-check/Cargo.toml \
  --config 'patch.crates-io.yaml_serde.path="/tmp/yashik-upstream-deps/yaml_serde"' \
  --config 'patch.crates-io.libyaml-rs.path="/tmp/yashik-upstream-deps/libyaml-rs"'
```

**15 passed, 0 failed:** 10 core, 3 YAML parser, and 2 CLI tests. Coverage
includes malformed/unknown/duplicate/null YAML cases, semantic validation,
inheritance, manifest-relative local paths, the checked-in example, secret-free
output, and HOME snapshots around valid/invalid `init`, invalid arguments, and
unavailable `doctor`. The captured Cargo output is at
`/tmp/yashik-final-schema-cargo-test.log`.

The path-patched dependencies are upstream source snapshots: `yaml_serde`
0.10.7 tree `22a7009e918620b523ea06e54ca75275eabe8027` and `libyaml-rs` 0.3.0
tree `2b0da67109c8531d5a3b30e52a034c36f2bac6c9`. Their 48 upstream Git blob
hashes are recorded in `/tmp/yashik-upstream-deps/provenance.json`. These are
source snapshots, not verified Cargo registry archives. The build emitted 63
warnings from `libyaml-rs`; tests completed successfully.

## CLI and formatting

- `./target/debug/yashik init examples/yashik.yaml` — passed; printed all five
  enabled harnesses and confirmed schema-only behavior.
- `./target/debug/yashik --version` — passed; printed `yashik 0.1.0`.
- `./target/debug/yashik doctor` — returned expected exit code 2 and stated no
  diagnostics were run.
- `rustfmt --check src/schema.rs src/core_tests.rs tests/yaml_schema.rs` — passed.

Manifest-relative local-path behavior is directly asserted by the passing core
test. A separate cross-working-directory CLI check would not expose the
resolved path: stage 1 does not print or access source paths.

## Limits

The ordinary registry build was not reproduced: the full suite used temporary
path patches, and there is no root lockfile. Registry archive checksums and
macOS behavior remain unverified. No resources were installed and no user
configuration was changed.
