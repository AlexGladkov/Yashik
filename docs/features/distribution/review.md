# Distribution review

Review date: 2026-10-01. Reviewer: independent Engineer assignment,
`gpt-6.1-sol` / `xhigh`, using the user's explicitly approved fallback from
the profile's Terra/max assignment. The reviewer authored Plan but did not
implement the product changes. This checkpoint covers Unix distribution at
`09268e2` (including `9013a5b` and the Rust 1.88/lint corrections).
Windows bridge/WinGet implementation is still in progress and is not approved
by this checkpoint. Follow-up Review includes the current installer regression
fixes and `src/install/util.rs` Darwin type corrections. Native GitHub CI and
public release/tap acceptance remain pending; local fixtures do not establish
macOS or clean-host acceptance.

## Findings

| ID | Severity/status | Location | Evidence and required change |
| --- | --- | --- | --- |
| R1 | P2, closed in follow-up | Original `scripts/install.sh:158`; fix now `:113,146` | GNU Wget's `--https-only` accepted a non-recursive HTTPS→HTTP redirect in the independent TLS fixture. Root amended Plan to curl-only. The fix removes Wget and restricts both curl transfer and redirect protocols to HTTPS. Re-review invoked the actual installer/curl against a trusted local HTTPS redirect: installer failed, the HTTP endpoint received no request, and the prior binary's bytes/inode/mtime were preserved. Missing-curl and protocol-argument regression tests pass. |
| R2 | P2, closed in follow-up | Original `scripts/install.sh:263`; fix now `:253` | Original byte-identical target with 0644 returned success while remaining unusable. The fix keeps executable identical targets unchanged and atomically replaces a non-executable identical payload with the verified 0755 stage, without force. Independent re-review confirmed changed inode, unchanged bytes, mode 0755, and no-op on the subsequent install. The new 0644 regression and existing inode/mtime preservation test pass. |

R1/R2 are closed. No open Unix code-review blocker remains at this checkpoint;
publication still requires successful native CI and external acceptance.
Two integration items must be reassessed when Windows lands:

- `generate-homebrew.py:19,45` currently permits exactly the four Unix archives
  and `install.sh`; the final shared SHA256SUMS will also contain Windows ZIPs.
  Permit the two known Windows entries while continuing to require all Unix
  entries and reject malformed/duplicate/unrecognised names.
- Workflow assembly/upload currently has six Unix-only release assets.
  Final Windows integration must include its builds, both ZIPs and actual
  hashes, version agreement with the separate crate, eight final release
  assets, and validated WinGet manifests before community publication.

## Checks and reasoning

Personally executed:

```sh
python3 -m unittest discover -s tests -p test_install_script.py
/bin/sh -n scripts/install.sh
bash -n scripts/package-release.sh
```

The initial 11 bootstrap tests and both syntax checks passed. After fixes,
all 14 bootstrap tests pass. Additional temporary
fixtures, kept outside the repository, confirmed:

- Before amendment, an isolated PATH without curl exercised Wget successfully;
  this branch is now intentionally removed. A bin directory containing an
  apostrophe produces a PATH hint which executes correctly.
- Formula generation uses all four exact tag-pinned URLs and the corresponding
  supplied hashes; missing, duplicate, malformed and unexpected manifest
  entries fail before generating a formula.
- Packaging rejects the stale local `0.2.0` release binary for requested 0.2.1.
  Packaging the local test-built 0.2.1 binary validates the one-member archive,
  mode 0755, extracted version/help/check probes and emitted SHA-256. This is
  packaging evidence, not proof that a released Linux binary is static.
- R1/R2 were reproduced independently of the initially passing suite and then
  rechecked against fixes as described above. The redirect recheck used real
  curl and trusted local TLS, with a test wrapper changing only the fixture URL.
- After Darwin utility fixes, `cargo test --locked --test installer_engine
  --test installer_sources --test installer_state` passed all 18 selected
  Linux tests, including filesystem containment/atomic write/state checks.

Code inspection confirmed archive checksum verification precedes extraction,
only the root member is streamed into a private same-directory stage, version
comparison happens before replacement, existing unknown programs are not
executed, non-regular targets are refused, and failed verification retains the
previous binary. The installer does not invoke sudo or edit shell profiles.

`tests/installer_engine.rs` now canonicalises the fixture root and uses the
same Darwin Library layout as `src/install/paths.rs`; child processes receive
matching HOME/XDG variables without modifying the test runner's global
environment. Linux layout is preserved. The Rust 1.88 requirement matches the
locked JSONC parser's let-chain syntax; the lint-only fixture change preserves
its formatted string.

Native CI run `36906969648` was reported by root as successful for both Linux
builds and failed on both Darwin builds with four existing type errors. The
follow-up utility diff changes the three variadic `openat` creation-mode
arguments to `libc::c_uint`, avoiding Darwin's narrow `mode_t` variadic
restriction. Fixed-arity mkdir/fchmod retain `mode_t`; security flags and
creation permissions are unchanged. `metadata_dev` casts the standard-library
device value back to the platform's `libc::dev_t`, matching the `stat.st_dev`
comparison on Darwin. Inspection found no weakening of symlink/identity checks.
The selected Linux checks pass; corrected native Darwin compile/test results
are still required from the next CI run.

Workflow inspection confirmed read-only default permissions, write access only
for draft creation, strict tag/Cargo/script agreement, explicit native Unix
runners and architecture checks, target tests before packaging, static ELF
checks on Linux, macOS deployment target 13.0, checksums from final archive
bytes, production formula audit followed by same-artifact local Homebrew
install/test, and build-only `workflow_dispatch`. A retry leaves an already
published release unchanged. Actual native job outcomes must be recorded by
Validate rather than inferred from this inspection. The root-reported 68 Rust
tests pass locally; this reviewer did not rerun the entire Rust suite.

The final validation record must separately identify native macOS/Linux CI,
real non-root Ubuntu installation, public release asset downloads, production
tap install/audit/test, and the pending Windows/WSL conditions. README's release
commands and its validation link need the completed publication/report before
being treated as publicly usable installation evidence.
