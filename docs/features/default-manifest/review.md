# Review: default manifest for `init`

Review date: 2026-10-05. Independent Engineer review of the feature diff and
its follow-up fixes. Execute reports 15/15 CLI tests, 1/1 binary unit test and
14/14 Windows launcher tests passing; those tests were not rerun in this
follow-up. `cargo fmt --all -- --check` and `git diff --check` passed here.

## Findings

| ID | Severity/status | Location | Follow-up review |
| --- | --- | --- | --- |
| R1 | P2, closed | `README.md:7-29,69-73,182-194` | README now says the bare command enters the next release and gives the explicit `yashik init ./yashik-compose.yaml` command for published v0.2.1. The Linux quick start and Windows steps also keep an explicit path, so users following current installation instructions do not invoke an unsupported command. |
| R2 | P2, closed | `tests/cli.rs:105-164`; `src/main.rs:315-344` | The CLI case now runs from the parent directory with `configs/custom.yaml` and a relative local resource under `configs/`. The new `load_effective` unit test directly asserts that the effective resource path resolves beside the selected manifest, independently of process cwd. Together they cover the plan's manifest-relative-resource condition. |
| R3 | P3, closed | `tools/windows-launcher/src/lib.rs:832-870` | The cfg(windows) conversion fixture now runs both explicit `init` and explicit `check` against the Windows path, preserving prior check coverage while adding init coverage. |

## Reviewed behavior and limits

The native CLI reads only `./yashik-compose.yaml` for bare `init`; an explicit
path still takes precedence. Missing-file errors name the default and show the
explicit-path hint. Loading and validation happen before `run_init` creates
installer directories, and `check` still requires exactly one path. The
Windows launcher forwards bare `init` without path conversion and preserves
strict operand parsing. No additional code correctness or error-handling
regression was found by inspection.

The launcher mock confirms bare `init` is forwarded, but actual Windows-to-WSL
current-directory mapping was not exercised. Native Windows/WSL validation is
still required.
