# Validation: default manifest for `init`

Validation date: 2026-10-05. Environment: Linux x86_64. The Windows launcher
suite ran on Linux with its mock WSL executor; Windows/WSL and macOS were not
available for live validation.

## Results

- `cargo test --locked` — passed: 76 unit and integration tests, including
  `tests/cli.rs` (15/15) and the `src/main.rs` unit test (1/1); doc tests had
  no cases. This covers default selection from the process cwd, missing-file
  diagnostics and no-write behavior, explicit-path precedence, invalid YAML,
  relative resources, and strict argument counts.
- `cargo test --locked --manifest-path tools/windows-launcher/Cargo.toml` —
  passed: 14/14 launcher tests, including bare `init` forwarding and explicit
  path conversion/forwarding.
- `cargo fmt --all -- --check` — passed.
- `cargo fmt --manifest-path tools/windows-launcher/Cargo.toml -- --check` —
  passed.
- `git diff --check` — passed.

An isolated manual smoke run used the built Linux binary with temporary `HOME`
and XDG directories. `yashik init` with an empty `yashik-compose.yaml` in the
cwd exited 0 and selected that file. With a manifest only in the parent
directory, bare `init` from a child exited 1, named the missing default and
gave the explicit-path hint; it created no data, cache, state, or bin paths.
`yashik init custom.yaml` exited 0 when the cwd also contained an invalid
default, confirming the explicit path takes precedence. The empty manifests
had no configured resources, so the smoke run made no network or server calls.

The launcher tests verify argument forwarding and Windows-path conversion
through mocks. Actual Windows-to-WSL cwd inheritance and installation behavior
on Windows or macOS remain unverified.
