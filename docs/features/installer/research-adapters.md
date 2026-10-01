# Research: harness adapter contracts

Status: primary-source research completed 2026-10-01. This document describes
the current user-global formats to implement; it does not itself make product
changes. It supersedes the older foundation note where paths or client
capabilities differ.

## Scope and shared contract

Yashik v1 has one MCP shape: `transport: stdio`, `run.command`, `run.args` and
`run.env`. An adapter must turn it into its client-native stdio entry, retain
unmanaged siblings and recognized authorization data, and use an atomic
read/modify/write (or the client CLI). It must never expand `${env:NAME}` into a
config file, state, backup, or log. Resolve `${source}` to the materialised
absolute source root; defer `${env:NAME}` to the client/process environment.

The resource key is the identity used in client config and destination
filenames. Reject keys that are not valid for a target format or would escape a
target directory. A re-run must compare the adapter's normalized desired record
with the existing record and its ownership fingerprint before editing it.

Portable skill means a directory containing `SKILL.md`. Portable agent and rule
sources are Markdown. For clients with a single global instruction file,
portable rules are concatenated in a deterministic, Yashik-owned block:

```markdown
<!-- yashik:rules:start -->
## <resource-key>
<source Markdown>
<!-- yashik:rules:end -->
```

The reconciler owns only that block and preserves text before and after it.
Duplicate/missing markers or a manually changed owned block are a conflict,
not an invitation to overwrite a user's instructions.

## Capability and destination matrix

| Harness | User-global MCP | Skill | Portable agent | Native agent | Portable rule | Native rule |
| --- | --- | --- | --- | --- | --- | --- |
| Codex | `~/.codex/config.toml` | `~/.agents/skills/<key>/SKILL.md` | `~/.codex/agents/<key>.toml` | same | `~/.codex/AGENTS.md` managed block | special `.rules` policy only; see below |
| Claude Code | `~/.claude.json` top-level `mcpServers` | `~/.claude/skills/<key>/SKILL.md` | `~/.claude/agents/<key>.md` | same | `~/.claude/rules/<key>.md` | same Markdown file |
| OpenCode 1.18.34 | `~/.config/opencode/opencode.json(c)` direct `mcp.<key>` map | `~/.config/opencode/skill/<key>/SKILL.md` | `~/.config/opencode/agent/<key>.md` | same | `~/.config/opencode/AGENTS.md` managed block | unsupported |
| Pi | `<agent-dir>/mcp.json` | `<agent-dir>/skills/<key>/SKILL.md` | unsupported | unsupported | `<agent-dir>/AGENTS.md` managed block | unsupported |
| Oh My Pi (OMP) | `<agent-dir>/mcp.json` | `<agent-dir>/skills/<key>/SKILL.md` | `<agent-dir>/agents/<key>.md` | same | `<agent-dir>/AGENTS.md` managed block | unsupported |

`<agent-dir>` is Pi's default `~/.pi/agent` and OMP's default
`~/.omp/agent`. Both can be relocated by `PI_CODING_AGENT_DIR`; resolve the
effective directory at plan time and store the resolved destination in state.
For OMP, profiles normally use `~/.omp/profiles/<name>/agent`; profile selection
is not represented in YAML v1, so the initial adapter must either use the
effective active profile or stop with a clear unsupported-profile result. It
must not silently write the default profile while a different one is active.

The table distinguishes semantic portable rules from each client's own policy
language. No resource should be silently converted from prose instructions to
an executable permission policy.

## Install recipes and version probes

All package-manager recipes below install to a managed user prefix, never with
`sudo`. The install command is a recipe, not an arbitrary manifest command.
When `harnesses.<id>.version` is supplied, select the exact version where that
upstream package supports it; otherwise resolve `latest` and record the actual
version in state. Always probe the executable after installation.

| Harness | Current observed package/version | Ubuntu user installation | Probe and important constraint |
| --- | --- | --- | --- |
| Codex | `@openai/codex@0.159.3` | official `curl -fsSL https://chatgpt.com/codex/install.sh \| sh`; exact npm fallback: `npm install -g @openai/codex@<version>` | `codex --version`; npm package requires Node `>=16`, bin `codex` |
| Claude Code | `@anthropic-ai/claude-code@2.1.286` | recommended `curl -fsSL https://claude.ai/install.sh \| bash`; pin-capable fallback `npm install -g @anthropic-ai/claude-code@<version>` | `claude --version`, `claude doctor`; npm route requires Node `>=22` and is upstream-deprecated but current/installable |
| OpenCode | `opencode-ai@1.18.34`, matching source tag `v1.18.34` / commit `aec0b9a6d8898f68f923aaf08b7306d931fd9d76` | `curl -fsSL https://opencode.ai/install \| bash`, or pin with `npm i -g opencode-ai@<version>` | `opencode --version`; baseline is the validated direct-map V1 adapter below |
| Pi | `@earendil-works/pi-coding-agent@0.99.2` | `npm install -g --ignore-scripts @earendil-works/pi-coding-agent@<version>` | `pi --version`; Node `>=22.19.0` required |
| OMP | `@oh-my-pi/pi-coding-agent@18.4.8` | `curl -fsSL https://omp.sh/install \| sh`, or install Bun then `bun install -g @oh-my-pi/pi-coding-agent@<version>` | `omp --version`; package requires Bun `>=1.3.14` |

The pinned npm smoke installed and ran both `claude 2.1.286` and
`opencode 1.18.34` in isolated prefixes. Both packages use an install script
and platform optional dependency to expose their native executable, so this
recipe requires a Node 22+ npm which permits optional dependencies and
postinstall scripts. Do not add `--ignore-scripts` to either recipe. The native
installer remains Claude's upstream recommendation; the npm path is retained
because it is the current working way to request an exact version in Yashik's
managed prefix.

Do **not** use `npm install -g @oh-my-pi/pi-coding-agent` as an OMP recipe.
The published `omp` entrypoint has `#!/usr/bin/env bun`; npm can place the
files, but Node cannot execute it without Bun. Pi's maintained package is the
`@earendil-works` package above, not legacy `@mariozechner/pi-coding-agent`.

The observed current npm release is OpenCode **1.18.34**, not the separately
published V2 documentation surface. It was installed into an isolated prefix
and reported `1.18.34`; its package and the matching official Git tag use the
V1 direct-MCP schema below. The current adapter baseline must therefore be V1,
with a version gate before any V2 writer is introduced.

| Detected release/config generation | MCP write shape | File destinations | Adapter status |
| --- | --- | --- | --- |
| `opencode-ai@1.18.34`, tag `v1.18.34` | `mcp.<key>` direct map; local has `type`, `command`, `environment`, `enabled` | singular `agent/` and `skill/` targets, although reader also accepts plural | supported baseline; isolated npm smoke passed |
| V2 documentation surface | `mcp.servers.<key>`; uses `disabled`, not `enabled` | documented plural `agents/` and `skills/` | do not write until an actual V2 package/version is pinned and smokes |

The 1.18.34 source deliberately reads both `{agent,agents}/**/*.md` and
`{skill,skills}/**/SKILL.md`. Yashik writes the singular V1 paths to avoid two
owned copies of one resource. It must treat an existing matching plural path as
an unmanaged/conflicting resource rather than duplicate it.

## Adapter projections

### Codex

Official global MCP configuration is `~/.codex/config.toml`. Project config is
out of scope. The stdio projection is:

```toml
[mcp_servers.example]
command = "npx"
args = ["-y", "example-mcp"]

[mcp_servers.example.env]
API_KEY = "${env:API_KEY}"
```

Codex also exposes `codex mcp add <name> --env KEY=VALUE -- <command> ...` and
`codex mcp list`; use the CLI for smoke/inspection where it exposes the desired
operation, but the TOML adapter needs a parser/preserving writer for reliable
reconciliation. `cwd`, `enabled`, and timeout fields are client options that
Yashik v1 does not invent.

Global skills are officially discovered at `~/.agents/skills`, not
`~/.codex/skills`. Copy the resolved skill directory to
`~/.agents/skills/<key>/`, validate the `SKILL.md` frontmatter before writing,
and leave non-owned skills alone.

A user-level custom subagent is exactly
`~/.codex/agents/<key>.toml`. Portable resources must render valid TOML with
the key as `name`, manifest `description`, and body as
`developer_instructions`; use a TOML serializer so triple quotes and newlines
cannot corrupt the document. Native resources must be a valid native TOML file
with at least `name`, `description`, and `developer_instructions`, and are
copied byte-for-byte after validation. Optional upstream fields such as
`model`, `model_reasoning_effort`, and `sandbox_mode` are preserved.

Portable rules belong in the global `~/.codex/AGENTS.md` managed block. Codex's
`~/.codex/rules/*.rules` is a different Starlark command-policy system; it is
not a Markdown instruction format. A local native rule may be supported only
when the source is an explicitly validated `.rules` policy and its semantics
are declared by the manifest/implementation. With the current v1 `Rule`
schema, which has no policy discriminator, report native Codex rules as
unsupported rather than guessing that arbitrary Markdown is safe Starlark.

### Claude Code

Use the documented user scope, never local or project scope:

```sh
claude mcp add --scope user --transport stdio <key> \
  --env KEY=VALUE -- <command> <args...>
```

The command writes a top-level user `mcpServers` entry in `~/.claude.json`.
The matching configuration record is the normal JSON stdio object:

```json
{
  "mcpServers": {
    "example": {
      "command": "npx",
      "args": ["-y", "example-mcp"],
      "env": {"API_KEY": "${env:API_KEY}"}
    }
  }
}
```

For regular installation pass environment *names/values* only after the user
has supplied them; Yashik must not turn a `${env:...}` reference into a stored
secret. For structural reconciliation, edit only that top-level map and
preserve other `~/.claude.json` keys, including project-specific records.

User skills are directories at `~/.claude/skills/<key>/SKILL.md`. User
subagents are Markdown files at `~/.claude/agents/<key>.md` with YAML
frontmatter. Render portable agents as:

```markdown
---
name: <key>
description: <manifest description>
---

<portable instructions>
```

Native agent input must be valid frontmatter Markdown, be copied unchanged,
and retain optional upstream fields (`tools`, `model`, `permissionMode`, hooks
and so on). Claude supports many named Markdown rule files under
`~/.claude/rules/`; copy/render each rule to `<key>.md`, so both portable and
native Markdown rule resources are representable.

### OpenCode 1.18.34 (current npm baseline)

The native global directory is XDG config `~/.config/opencode` (or the
equivalent `XDG_CONFIG_HOME/opencode`; `OPENCODE_CONFIG_DIR` is an explicit
override). The config reader chooses an existing `opencode.jsonc`,
`opencode.json`, or legacy `config.json`; if none exists, Yashik should create
`opencode.jsonc`. It must parse and edit it with a JSONC-preserving editor.

For 1.18.34 server names are direct entries under `mcp`, not under
`mcp.servers`. A local stdio server is:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "example": {
      "type": "local",
      "command": ["npx", "-y", "example-mcp"],
      "environment": {"API_KEY": "{env:API_KEY}"},
      "enabled": true
    }
  }
}
```

The adapter maps `command` + `args` to the one command array and maps only
`${env:NAME}` to OpenCode's `{env:NAME}` syntax. In this generation
`enabled: false` disables a configured server. `opencode mcp add <key> --
<command> ...` writes this global record (there is **no** `--global` option),
and `opencode mcp list` is the smoke/inspection command. The 1.18.34 source
uses JSONC editing for this same operation, so Yashik needs JSONC-preserving
reconciliation rather than a strict JSON rewrite.

The portable destination is `~/.config/opencode/agent/<key>.md`; the runtime
also reads `agents/`, but singular is the owned target. Render a portable
subagent as YAML frontmatter with at least `name`, `description`, and
`mode: subagent`, followed by the Markdown body, which becomes the agent
prompt. Native sources are valid frontmatter Markdown, copied unchanged after
validation. Optional native fields include `model`, `variant`, `permission`,
`tools`, `temperature`, `top_p`, `hidden`, and `steps`.

The portable skill destination is
`~/.config/opencode/skill/<key>/SKILL.md`; `skills/` is also read, but is not
the v1 write target. The loader requires a string `name`; `description` is
optional in 1.18.34. A Yashik skill still validates both so it works across all
harnesses.

The global instruction file is `~/.config/opencode/AGENTS.md`. Portable rules
therefore use the managed block. No distinct native global rule artifact is
available, so native rules remain unsupported rather than merged into prose
whose source semantics may not survive composition.

### Pi

Resolve `agent-dir` from `PI_CODING_AGENT_DIR`, otherwise `~/.pi/agent`. The
native MCP document is `<agent-dir>/mcp.json`:

```json
{
  "mcpServers": {
    "example": {
      "command": "npx",
      "args": ["-y", "example-mcp"],
      "env": {"API_KEY": "${env:API_KEY}"}
    }
  }
}
```

The documented CLI is `pi mcp add <key> -- <command> ...`, `pi mcp list`, and
`pi mcp remove`; a name is limited to letters, numbers, `_`, and `-`. Use the
JSON map for full reconciliation while retaining every other server.

Skills are `<agent-dir>/skills/<key>/SKILL.md`; Pi also discovers the shared
`~/.agents/skills` location, but this adapter should install into Pi's native
root to make ownership and removal unambiguous. Global instructions are
`<agent-dir>/AGENTS.md`, which supplies the portable-rule managed block.

Pi core does not document a saved custom subagent file/discovery contract.
`pi-agents` is a separate extension/package and is not a Pi-core adapter. Thus
both portable and native `agents` are **unsupported** for Pi in this release:
the planner must offer skip/cancel before making any other dependent change.
Likewise no independent native rule resource exists; only portable prose can
be safely composed into `AGENTS.md`.

### Oh My Pi (OMP)

The default root is `~/.omp/agent`; `PI_CODING_AGENT_DIR` overrides it and a
named profile convention is `~/.omp/profiles/<name>/agent`. OMP's native
`<agent-dir>/mcp.json` uses the same `mcpServers` object but permits explicit
`type`, `cwd`, and `env`:

```json
{
  "mcpServers": {
    "example": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "example-mcp"],
      "env": {"API_KEY": "${env:API_KEY}"}
    }
  }
}
```

OMP skill discovery is one level deep at
`<agent-dir>/skills/<key>/SKILL.md`; validate required `name` and `description`
frontmatter. OMP agents are native Markdown files at
`<agent-dir>/agents/<key>.md` with required YAML `name` and `description`.
The Markdown body is the agent `systemPrompt`; optional native fields such as
`model`, `tools`, and `spawns` must survive. Portable agents render those two
fields plus the portable body. Native agent files validate then copy unchanged.

For portable rules use the managed block in `<agent-dir>/AGENTS.md`. OMP also
has `<agent-dir>/RULES.md`, but that file is its sticky native rulebook
semantics, not a composable generic Markdown-instructions directory. The v1
schema cannot preserve and merge several independently sourced rulebooks, so
native OMP rules are **unsupported**. The planner must explain this and offer
skip/cancel. Do not concatenate arbitrary native `RULES.md` source into it.

## Codex + VoltAgent end-to-end acceptance

The approved first vertical test has a directly usable native source. At
research time, `VoltAgent/awesome-codex-subagents` main resolved to
`7add6913c53ccbbc250c481815c9c6afb02709c6`. Its
`categories/01-core-development/backend-developer.toml` is a current Codex
custom-agent TOML with `name`, `description`, optional model/sandbox fields,
and `developer_instructions`.

The minimal manifest should pin that exact source and keep the native resource
inside `harnesses.codex.agents`:

```yaml
version: 1
harnesses:
  codex:
    version: "0.159.3"
    agents:
      backend-developer:
        format: native
        source:
          type: git
          url: https://github.com/VoltAgent/awesome-codex-subagents.git
          ref: 7add6913c53ccbbc250c481815c9c6afb02709c6
        from: categories/01-core-development/backend-developer.toml
```

Expected result is the validated, byte-identical source at
`~/.codex/agents/backend-developer.toml`. The non-authenticated smoke checks
are `codex --version`, TOML parsing/required-field validation, target checksum
equality, and a second `init` with no rewrite/download. Proving a live model
can invoke the agent additionally requires the user's Codex sign-in and is not
an installer failure. `doctor` should report this distinction.

## Implementation decisions for the modular engine

Implement a `HarnessAdapter` boundary with at least `probe`, `install`,
`inspect`, `plan_resources`, `apply`, and `remove_owned`. `inspect` returns
normalized client records and preserves uninterpreted siblings; `plan_resources`
declares every destination and capability before mutations. Format parsing and
rendering belong below this boundary (TOML for Codex, JSON for Claude/Pi/OMP,
JSONC for OpenCode, Markdown/frontmatter for agents/rules/skills).

Do not use shell strings for MCP or install commands. Build argv explicitly.
The resource source resolver validates git/local `from` containment before an
adapter reads a file. The runtime layer owns Node/Bun provisioning and user
package-prefix PATH registration. The adapter owns no downloading and does not
run an MCP server as validation.

Capability errors are resource-level plan results, not panics or partial writes.
The resulting prompt must name the harness, resource key, and unsupported
feature, with `skip` and `cancel` choices. The Pi-agent, native OMP-rule,
native OpenCode-rule, and native Codex-rule cases above require that path.

## Primary references

- [OpenAI Codex repository and installation](https://github.com/openai/codex)
- [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
- [Codex skills](https://learn.chatgpt.com/docs/build-skills)
- [Codex AGENTS.md configuration](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- [Codex subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents)
- [Codex rules policy language](https://learn.chatgpt.com/docs/agent-configuration/rules)
- [Claude Code install](https://code.claude.com/docs/en/overview)
- [Claude Code official repository/install options](https://github.com/anthropics/claude-code)
- [Claude Code npm package metadata](https://www.npmjs.com/package/@anthropic-ai/claude-code)
- [Claude Code MCP scopes and format](https://code.claude.com/docs/en/mcp)
- [Claude Code skills](https://code.claude.com/docs/en/skills)
- [Claude Code subagents](https://code.claude.com/docs/en/sub-agents)
- [Claude Code global instructions and rules](https://code.claude.com/docs/en/memory)
- [OpenCode repository/install](https://github.com/anomalyco/opencode)
- [OpenCode v1.18.34 source tag](https://github.com/anomalyco/opencode/tree/v1.18.34)
- [OpenCode v1.18.34 MCP schema](https://github.com/anomalyco/opencode/blob/v1.18.34/packages/core/src/v1/config/mcp.ts)
- [OpenCode v1.18.34 MCP CLI JSONC writer](https://github.com/anomalyco/opencode/blob/v1.18.34/packages/opencode/src/cli/cmd/mcp.ts)
- [OpenCode v1.18.34 agent discovery](https://github.com/anomalyco/opencode/blob/v1.18.34/packages/opencode/src/config/agent.ts)
- [OpenCode v1.18.34 skill discovery](https://github.com/anomalyco/opencode/blob/v1.18.34/packages/opencode/src/skill/index.ts)
- [OpenCode v1.18.34 global instructions](https://github.com/anomalyco/opencode/blob/v1.18.34/packages/opencode/src/session/instruction.ts)
- [OpenCode V2 config](https://opencode.ai/v2/docs/config)
- [OpenCode V2 MCP servers](https://opencode.ai/v2/docs/mcp-servers/)
- [OpenCode V2 skills](https://opencode.ai/v2/docs/skills/)
- [OpenCode V2 agents](https://opencode.ai/v2/docs/agents/)
- [OpenCode V2 instructions](https://opencode.ai/v2/docs/instructions)
- [Pi documentation](https://pi.dev/docs/latest/)
- [Pi MCP documentation](https://pi.dev/docs/latest/mcp/)
- [Pi skills](https://pi.dev/docs/latest/skills/)
- [Pi configuration](https://pi.dev/docs/latest/configuration/)
- [Oh My Pi repository](https://github.com/can1357/oh-my-pi)
- [OMP MCP configuration](https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md)
- [OMP skills](https://github.com/can1357/oh-my-pi/blob/main/docs/skills.md)
- [OMP task-agent discovery](https://github.com/can1357/oh-my-pi/blob/main/docs/task-agent-discovery.md)
- [OMP context files and rules](https://github.com/can1357/oh-my-pi/blob/main/docs/context-files.md)
- [VoltAgent native Codex subagents](https://github.com/VoltAgent/awesome-codex-subagents)
