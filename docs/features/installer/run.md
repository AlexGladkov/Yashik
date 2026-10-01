# Installer execution record

User request: implement the actual installer and validate a simple manifest
installing Codex plus VoltAgent native subagents on `<authorized Ubuntu host>`.

Profile applied from `.agents/skills/business-feature/SKILL.md`:

| Stage | Assignments |
| --- | --- |
| Research | Three Consilium researchers, `gpt-5.6-terra`, `xhigh` |
| Plan | Strategist, `gpt-6.1-sol`, `xhigh` |
| Execute | Three Engineers, `gpt-6-luna`, `max`; complete, including review fixes |
| Review | Initial independent Engineers: `gpt-5.6-terra/max`; final adapters: Terra/max, engine: `gpt-5.6-sol/max` after thread reuse failed |
| Validate | Acceptance Tester, `gpt-6-luna/max`; reused source engineer thread, independent Review remains separate |

Pre-change checks: ordinary `cargo test` passed all 15 existing tests. The
previous remote schema-only binary was built at
`/tmp/yashik-check.OE43oj/repo/target/release/yashik`. This was **not** an
installation of the requested coding setup; the new acceptance test must
demonstrate installed CLI and subagent files.

Final results are recorded in [review-engine](review-engine.md),
[review-adapters](review-adapters.md), [validation](validation.md), and
[live Ubuntu evidence](live-ubuntu.md).

Initial Review found blocking preflight, CLI ownership, backup symlink,
aggregate rule and diagnostic/cleanup defects. Follow-up fixes added
regression coverage. Final checks pass 68 tests, formatting and strict Clippy.
Root repeated real Ubuntu Codex/VoltAgent and all-five MCP acceptance against
the corrected release build. See linked reports for precise limits and gates.
