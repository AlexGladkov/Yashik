# Installer research synthesis

The user requires real installation rather than schema validation. The primary
acceptance scenario is Ubuntu 24.04 at `<authorized Ubuntu host>`: install Codex and native
subagents from `VoltAgent/awesome-codex-subagents`, then rerun `init` and inspect
with `doctor`. Existing five-harness and resource requirements remain in scope.

Consilium details are in `research-adapters.md`, `research-engine.md`, and
`research-validation.md`. Contracts must follow current upstream documentation,
not the historical schema-stage assumptions.

Confirmed decisions:

- Codex native agents are TOML files discovered under `~/.codex/agents/`.
  VoltAgent supplies these directly; validate and copy without changing model,
  sandbox, or instruction fields. Portable agents require format adaptation.
- Pi's maintained CLI package is `@earendil-works/pi-coding-agent` and requires
  Node 22.19+. Its MCP configuration is supported; built-in native subagent
  discovery is not confirmed, so ask to skip unsupported agents.
- OMP's package is `@oh-my-pi/pi-coding-agent`, with Bun 1.3.14+ required.
  Installing its package through npm without a working Bun is insufficient.
- Sources, runtimes/execution, adapters, and state/reconciliation need separate
  modules. Git resources must resolve to exact revisions and prevent `from` or
  recursive symlinks escaping the selected source.
- Environment references for MCP are resolved at process launch, not serialized
  as credentials in state. A launcher lets all adapters share this behavior.
- Existing unrelated configuration and manual resource edits must be preserved;
  conflicts and removal require explicit interactive choices and backups.
- Existing `init` schema-only tests must move to `check`. Real `init` needs tests
  exercising successful filesystem writes, reruns, drift and partial failures.

Rust registry access has been restored and the original 15 tests pass using
ordinary `cargo test`. The Ubuntu server has Git, curl and Python; Node/Bun are
absent. `build-essential` was installed during the earlier build verification.
Testing on Ubuntu does not establish macOS support.
