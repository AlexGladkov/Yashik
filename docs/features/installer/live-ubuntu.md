# Real Ubuntu execution evidence

Date: 2026-10-01. Root ran the implementation on the user's authorized SSH
host: Ubuntu 24.04.2 LTS, x86_64. Node and Bun were absent before the installer
test. Git, curl, tar, unzip, Python and build-essential were present.

Final corrected build: `yashik 0.2.0`, release binary SHA-256
`ad2602991a97bfebe3a15936bc3e6240a1fbf986bcb0da6275b633cd955f6c49`.
The initial install and a fresh all-five install were followed by final-build
init, protocol and doctor reruns after review fixes.

## Primary user-requested scenario

The binary was copied to `~/.local/bin/yashik`, and
`examples/codex-voltagent.yaml` was copied to
`~/yashik-tests/codex-voltagent.yaml`.

```sh
~/.local/bin/yashik check ~/yashik-tests/codex-voltagent.yaml
~/.local/bin/yashik init ~/yashik-tests/codex-voltagent.yaml
~/.local/bin/codex --version
~/.local/bin/yashik doctor
~/.local/bin/yashik init ~/yashik-tests/codex-voltagent.yaml
```

The first init exited 0 and reported Node ready, Codex 0.159.3 ready, and two
successfully installed native agents. The Codex probe returned
`codex-cli 0.159.3`. Python 3.12 `tomllib` parsed each installed agent and
confirmed its name, description and developer_instructions fields. Hashes
were byte-identical to the pinned VoltAgent commit:

| Installed file | SHA-256 |
| --- | --- |
| `~/.codex/agents/backend-developer.toml` | `59f3f9bef6bb222e90ce25ba6a6e0931e8f6a74afafa806202ca4ecb232b33c7` |
| `~/.codex/agents/reviewer.toml` | `d3aa577b51c5e7f5cb1afffb8782d619ddfda4c96e88f73d34064ffc04f1b131` |

Doctor exited 0 and reported CLI and both bindings unchanged. A before/after
recursive SHA-256, mtime and mode snapshot of `.codex`, Yashik state and data
was equal: doctor made no filesystem changes. The second init exited 0 and
reported both agent bindings unchanged. Final corrected-build reruns again
reported both agents unchanged and returned 0. Runtime/CLI readiness is reported
as `Succeeded` even when their pinned package cache is reused. A separate
before/after SHA-256, mtime and mode snapshot of the Codex package prefix,
launcher and both agents was identical across a further final-build init.

## All-five resource smoke

Root ran `examples/all-five.yaml` in fresh isolated HOME
`/tmp/yashik-final.PxRNZ8/home`, with XDG data/cache/state/config paths explicitly
set under it. This left the primary Codex setup separate from broader fixtures.

All CLI installs and version probes succeeded:

| CLI | Actual probe |
| --- | --- |
| Codex | `codex-cli 0.159.3` |
| Claude | `2.1.286 (Claude Code)` |
| OpenCode | `1.18.34` |
| Pi | `0.99.2` |
| OMP | `omp/18.4.8` |

Each harness received the echo MCP, `setup-check` skill and portable rule.
Codex, Claude, OpenCode and OMP received a portable `test-reviewer` agent;
Pi's agent was deliberately disabled in the manifest because this adapter
does not provide a Pi-core saved-subagent contract. All-five doctor exited 0.

Real client probes confirmed:

- `codex mcp list` listed the wrapper command as enabled.
- `claude mcp list` reported the echo MCP connected.
- `opencode mcp list` reported the echo MCP connected.
- `pi mcp list` reported connected and one echo tool.
- `opencode agent list` discovered `test-reviewer (subagent)` and its skill
  paths appeared in the discovered permissions.

An additional Python stdio client read each native MCP entry and started its
configured launcher with the same HOME/XDG environment. For all five entries,
JSON-RPC initialize, tools/list and tools/call returned the expected server,
tool and `installation-confirmed` echo result. State contained exactly **one**
shared MCP artifact for the five bindings.

The echo is a real dependency-free protocol fixture shipped in examples,
not evidence that an external API service is authenticated. No model request
or provider login was performed. Codex warns about helper aliases when its
HOME is under `/tmp`; that warning did not prevent its version/MCP probes.

## Remaining evidence boundaries

These are genuine downloads, package installations, binary probes and local
MCP handshakes on Ubuntu, rather than mocked package-manager results. This
does not validate macOS, private Git authentication, provider sign-in, live
model delegation or all possible third-party MCP install recipes. Independent
Review and final Tester validation are recorded separately.

## Final review-fix rerun

The final binary installed all five pinned clients in a fresh isolated HOME;
every supported resource binding succeeded. The final-build rerun left all
bindings unchanged. Updating the test HOME's earlier Yashik binary required
an explicit yes; a protected backup of that binary was retained.

All five final configured MCP launchers passed real `initialize`, `tools/list`
and `tools/call` JSON-RPC exchanges. Only one shared built artifact was present.
Final all-five doctor probed actual curated CLI versions and reported all
bindings and the artifact unchanged, exiting 0. The final primary Codex doctor
again passed the recursive hash/mtime/mode immutability snapshot.

No provider login or request to a remote model was performed. A clean Ubuntu
installation validates this Linux x86_64 path; it does not validate macOS.
