# Distribution validation

This report is in progress. Release v0.2.1 has not yet been published at this
checkpoint; README installation commands become usable after publication.
Windows bridge/WinGet review and validation remain pending.

## Completed local checks

- Linux host: cargo fmt --check, strict Clippy and all 68 Rust tests passed.
- POSIX bootstrap: 14 Python regression tests passed, covering checksum/archive
  rejection, unsupported platforms, safe replacement, mode repair and repeat
  no-op. Independent review also tested an actual HTTPS redirect to HTTP;
  transfer was refused without contacting the HTTP endpoint.
- Independent alias-root regression: 21 library tests and five state tests
  passed with TMPDIR pointing through a symbolic link.
- Native Darwin CI exposed compile errors and fixture path differences; narrow
  fixes were reviewed and a native rerun is underway. Linux results do not
  establish macOS compatibility.

## Acceptance still required

Final tag CI must pass native Linux/macOS builds, archive/static-link checks,
bootstrap probes, and four native Homebrew fixture installs/audits/tests.
Windows CI must build/probe both PE architectures and validate WinGet packaging.
A genuine WSL smoke, if unavailable on a hosted runner, must remain explicitly
unverified rather than being replaced by a mock claim.

After the relevant gates: verify all draft assets and checksums, publish the
release, perform real non-root Ubuntu network installation plus Codex/VoltAgent
init/doctor/repeat-init checks, publish the production formula and test installation
from the actual public tap. Submit final WinGet manifests using published ZIP
hashes. Community catalog acceptance is external; a submitted PR does not make
`winget install --id AlexGladkov.Yashik` immediately available.
