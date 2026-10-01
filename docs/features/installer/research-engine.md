# Research: engine реального установщика

## Вывод

Этап schema уже завершён: текущий `init` намеренно только валидирует YAML и
показывает effective resources, а `doctor` завершается с кодом 2. Следующим
шагом нужен не ещё один слой парсинга, а исполняющий движок с изолированными
границами: source materialisation, bootstrap runtime/CLI, argv executor,
reconciliation/state и adapters. Это позволит сначала получить настоящий
вертикальный сценарий Codex, а не блокировать установщик ожиданием адаптеров
всех harnesses.

Предлагаемый первый реальный smoke на Ubuntu:

1. Yashik ставит Node 22 LTS (не ниже 22.19.0) в пользовательский runtime
   store, если нужной версии нет.
2. Через этот Node устанавливает закреплённый (или разрешённый `latest`) пакет
   `@openai/codex` в пользовательский npm prefix.
3. Берёт зафиксированный Git snapshot
   `VoltAgent/awesome-codex-subagents`, копирует выбранные native TOML agents в
   `~/.codex/agents/`, не трогая другие файлы, и запоминает владение.
4. Второй `init` ничего не скачивает, не переустанавливает CLI и не переписывает
   agent files. Изменённый пользовательский TOML становится conflict, а не
   молча перезаписывается.

Это сохраняет v1 YAML: `harnesses.codex.version` задаёт npm package version, а
native `agents` уже разрешены только локально в блоке harness. Не нужно вводить
специальный синтаксис для VoltAgent или ослаблять `from`.

## Проверенные исходные факты

- [Требования](../../requirements.md) требуют пользовательскую глобальную
  установку, Git/local sources, повторный reconciliation, интерактивные
  conflicts/removal и `doctor` без изменения машины.
- [Design](../../design.md) уже закрепляет `source → runtime/build → binding`,
  argv install steps без shell, containment `from`, state только после успеха
  и shared MCP с одинаковой effective specification.
- `src/effective.rs` строит нужную наследованную модель, а `src/main.rs` пока
  не читает source и не меняет HOME. Новые модули должны получать
  `EffectiveManifest`, не дублировать наследование и валидацию.
- Удалённый host доступен по SSH: Ubuntu 24.04, `x86_64`; на нём есть `git`,
  `curl`, `tar`, `sudo` и `build-essential`. На момент проверки не было
  `cargo`, `rustc`, `node` или `npm`. Это хороший чистый Linux smoke: он
  подтверждает именно bootstrap Node, а не неявную зависимость от Rust/npm.
- `https://github.com/VoltAgent/awesome-codex-subagents.git` существует; на
  момент исследования `main` и HEAD были commit
  `7add6913c53ccbbc250c481815c9c6afb02709c6`. В нём native TOML files лежат,
  например, в `categories/01-core-development/backend-developer.toml`.
  README этого проекта предписывает глобальный каталог `~/.codex/agents/`.
  Это соответствует проверенному target path для Codex; для smoke не требуется
  менять `~/.codex/config.toml`.

## Границы первой реализации

`init` остаётся интерактивным. Он не исполняет MCP, не выполняет `run.command`
и не авторизует пользователя в Codex или в сервисах. Он вправе выполнять
явные install steps, поскольку это заявленный контракт источника. `doctor` не
скачивает, не запускает install steps, не запускает MCP и не обращается к сети.

Первый adapter обязан реально поддержать Codex CLI и native agents. Установка
остальных четырёх harnesses проходит тем же `HarnessRecipe` интерфейсом, но
публиковать adapter как supported можно лишь после smoke его конкретной
версии/config format. Это не отменяет цель all-five: recipes для каждого
harness — npm packages в managed prefix, с version resolver и state, а format
adapters могут двигаться независимо. OMP — дополнительное runtime-исключение:
его npm-installed executable имеет shebang `bun` и требует Bun >= 1.3.14.

Секретом считается не только значение `${env:NAME}`, но и всё, что может
оказаться в URL с password/token. Логи, state, backups и конфиги harnesses не
должны содержать разрешённые значения environment variables.

## Разделение модулей и интерфейсов

Ниже — границы, по которым работу можно безопасно разделять. Модули зависят
вниз; adapter не вызывает Git, а source resolver не знает форматы TOML.

```text
schema/effective
      │
      ▼
planner ──► SourceStore ──► RuntimeRegistry ──► ArgvExecutor
  │              │                  │                 │
  ├────► Reconciler ◄──── StateStore/Journal ◄─────────┘
  │              │
  └────► HarnessAdapter (Codex first) ──► config files
                 │
                 └────► Doctor (read-only inspection)
```

| Module | Owns | Does not own |
| --- | --- | --- |
| `paths` | XDG/macOS locations, private directory creation, one process lock | manifest-relative source paths |
| `sources` | Git/local resolve, cache/materialisation, source fingerprint and `from` containment | install commands or harness paths |
| `runtime` | closed recipes, Node archive checksum and managed npm prefix | arbitrary package-manager commands from YAML |
| `executor` | `Command` argv, cwd, curated child environment, captured/redacted status | interpolation grammar or state mutation |
| `state` | versioned state, backups, journal, atomic writes/recovery | interpretation of a harness config |
| `adapters` | inspect/plan/apply/remove one harness format, configuration fingerprint | Git, runtime installation and prompting |
| `planner`/`reconcile` | DAG, deduplication, dependency outcomes, prompts and final report | direct filesystem mutation except through interfaces |
| `doctor` | state/config/binary observation and drift report | repair, source refresh, network or runtime execution |

Core traits should expose explicit data, never a mutable global context:

```rust
trait SourceStore { fn resolve(&self, request: SourceRequest) -> Result<ResolvedSource>; }
trait RuntimeRegistry { fn ensure(&self, requirement: RuntimeId) -> Result<Runtime>; }
trait HarnessAdapter {
    fn inspect(&self) -> Result<ObservedHarness>;
    fn plan(&self, desired: &[Binding], observed: &ObservedHarness) -> Vec<ConfigOp>;
    fn apply(&self, op: &ConfigOp, backup: &Backup) -> Result<AppliedBinding>;
}
trait StateStore { fn begin_run(&mut self, plan: &Plan) -> RunId; /* outcome + commit */ }
```

`ResolvedSource` contains canonical source identity, exact commit/content
fingerprint, immutable snapshot path and a separate writable artifact path.
`AppliedBinding` returns its precise target path and post-write fingerprint.
This makes planner tests entirely fakeable while retaining an integration
surface for a real machine.

## Source store: Git, local and containment

### Directories and lifecycle

Use per-user locations, create them with mode 0700, and do not write in the
manifest directory:

| Purpose | Linux | macOS |
| --- | --- | --- |
| state, journal, backups, launch specs | `${XDG_STATE_HOME:-~/.local/state}/yashik` | `~/Library/Application Support/yashik` |
| immutable source objects/download cache | `${XDG_CACHE_HOME:-~/.cache}/yashik` | `~/Library/Caches/yashik` |
| installed Node/npm/artifacts | `${XDG_DATA_HOME:-~/.local/share}/yashik` | `~/Library/Application Support/yashik` |

Each source resolution holds a lock for its cache key. Write to a unique
same-filesystem staging directory, verify it, `fsync` file and parent where the
platform permits, then rename to its final object path. Final source objects
are immutable; install steps run only in a distinct writable artifact directory.
An interrupted staging directory has no state reference and is cleaned on the
next locked operation. A held/stale global init lock is reported instead of
guessing that concurrent writes are safe.

### Git

`GitSource { url, ref }` resolves to a commit before planning installation:

1. Accept only HTTPS and SSH Git remotes. Reject HTTP, `file:`, `ext::`, and
   URLs containing a password/token. SSH `git@host:path` is valid: authentication
   comes from the user’s SSH agent/config; HTTPS uses existing credential helper.
2. Cache by a cryptographic digest of the normalized source identity. Never use
   raw URL as a directory name or journal field. State stores a safe display
   URL only after its userinfo validation.
3. Use an internal bare repository and direct `git fetch <url> <ref>` commands;
   never save `<url>` as `remote.origin`. Scrub `GIT_DIR`, `GIT_WORK_TREE`,
   `GIT_INDEX_FILE` and related ambient Git control variables, disallow the
   `file` and `ext` protocols, but retain `HOME`, `SSH_AUTH_SOCK` and configured
   credential helpers.
4. Resolve absent `ref` from remote default HEAD on every `init`. Resolve a
   branch/tag to an exact commit. A 40-hex ref must name that commit. Record the
   result for this run; do not turn a manifest `latest` into a permanent lock.
5. Export the commit to a fresh object directory without `.git` metadata. Do
   not auto-initialize submodules. A source which needs a submodule is unsupported
   until it declares it through a separate source; silently executing recursive
   checkout changes source semantics.

A cache hit is valid only if the target commit object remains present. The
source artifact key includes the resolved commit, not a branch/tag name, so a
moving ref creates a new artifact while a pinned commit reuses it.

### Local

Resolve relative `source.path` against the manifest directory exactly once,
then `canonicalize`; absolute paths stay permitted as required. The cache key is
the canonical root plus a deterministic content tree digest (relative name,
type, link target, mode and file bytes in sorted order). The resolved local
tree is snapshotted into immutable cache and then copied to a writable artifact
directory. This prevents an install step from modifying the user’s source and
ensures a later harness invocation still sees the artifact that was actually
built.

The copier never follows symbolic links. It rejects an absolute link or a link
whose canonical target leaves the source root; this is the necessary stronger
rule for a portable cached snapshot. It is deliberately applied to the whole
tree, not merely `from`, otherwise a copied tree could change behaviour by
following an external live path after fingerprinting. A user can make an
explicit separate local source instead.

For both source kinds, resolve `from` only after materialisation; canonicalize
`source_root/from` and require that it starts with the canonical source root.
`from` is never allowed to be absolute or contain `..` (the current validator
already handles the lexical part). Type checks are resource-specific: a skill
must select a directory containing `SKILL.md`; a native Codex agent must select
one regular `.toml` file; an MCP build root must be a directory. The user-visible
error names the manifest path and offending resolved path, but not secret data.

## Runtime and CLI bootstrap

`install.requires` is a closed registry lookup, never an argv escape hatch.
The v1 initial registry contains `node` and `bun`. Harness recipes install their
known npm package in Yashik’s private prefix and supply package name, target
semver/latest policy, executable name, required runtime and a verified
`--version` parser. The Codex recipe is `@openai/codex` → `codex`; the currently
maintained Pi package is `@earendil-works/pi-coding-agent` and requires Node >=
22.19.0. OMP’s package still uses the managed npm prefix, but its `bun` shebang
adds the Bun >= 1.3.14 dependency. Exact mappings for Claude/OpenCode and the
remaining package metadata live in individual recipes and are acceptance-tested
before an adapter is marked supported.

1. Determine `{linux,darwin} × {x64,arm64}` from Rust’s platform APIs. Other
   pairs become an interactive unsupported result; no guessed archive URL.
2. If the selected Node 22 LTS version is absent or its recorded integrity does
   not match, download its official `node-v<V>-<triplet>.tar.xz` to staging via
   `curl --fail --location --proto =https --tlsv1.2`. Compare bytes to the
   published `SHASUMS256.txt` entry that is compiled/pinned with the selected
   runtime recipe; do not trust only a freshly downloaded checksum file. Extract
   after verification and atomically activate it in the data store.
3. Bootstrap Bun similarly from the versioned official release ZIP for the
   exact `{linux,darwin} × {x64,arm64}` target. Its URL, expected SHA-256 and
   minimum version are part of the shipped registry; verify before extraction
   and atomically activate its single `bun` binary. Do not use Bun's internet
   shell installer, and do not treat a download-time metadata file as the sole
   trust root. A recipe requesting OMP depends on this activation.
4. Prepend managed Node and Bun `bin` directories to executor `PATH`; do not modify `.profile`,
   `/usr/local`, system npm prefix or the project. Install harnesses with
   `npm install --global --prefix <managed-prefix> <package>@<exact-version>`.
   For `latest`, resolve metadata once per `init`, record exact semver and
   package integrity in state, then call npm with that exact version.
5. If bootstrap tools are missing, a **curated** platform recipe may install
   `ca-certificates`, `curl`, `tar`, `xz-utils` and `git`: Ubuntu/Debian via
   `apt-get`, macOS via already-installed Homebrew. Ask before a recipe needs
   `sudo`; it must run the fixed argv only. If neither recipe is applicable,
   explain and offer skip/cancel. Do not install Homebrew, Rust or any unknown
   package manager by executing an internet shell script.

The destination needs a stable launcher directory on `PATH` for users. The
Yashik self-bootstrap should therefore be released as a signed/checksummed
Linux/macOS binary archive installed in `~/.local/bin` (Linux) or
`~/Library/Application Support/yashik/bin` with a one-time PATH instruction
(macOS). Building the installer itself with `cargo` on the target is a developer
fixture, not the supported user bootstrap; the clean Ubuntu host demonstrates
why that distinction matters.

## argv execution and `${...}`

All child processes use `std::process::Command`: program and each argument stay
separate, current directory is the resource’s writable artifact root, and there
is no `sh -c`, `eval`, `PATH` concatenation from the manifest, or shell quote
interpretation. Capture bounded stdout/stderr for the report, redact values
matching resolved environment values, and never print entire source files.

`install.steps` are literal argv by contract. They receive the curated child
environment and artifact cwd but **do not interpolate** `${...}`; cwd removes
the proposed need for `${source}` there. Reject interpolation tokens in steps
instead of inventing undocumented semantics.

For `run.command`, `run.args` and `run.env` values, parse only these tokens,
left-to-right and without recursive expansion:

| Token | Result |
| --- | --- |
| `${source}` | absolute persistent artifact root |
| `${env:NAME}` | current process environment value for POSIX name `[A-Za-z_][A-Za-z0-9_]*` |

Unknown/unclosed tokens, an invalid environment name, and an absent referenced
variable are errors for that resource before adapter mutation. The state stores
the original token form, never expanded values. The harness config also must
not get expanded values: an internal `yashik mcp-launch <binding-id>` launcher
reads the private launch spec, expands at actual MCP process start, sets only
the requested environment overrides, and `exec`s the argv directly. Adapters
point stdio MCP bindings at this launcher. Thus a configuration backup cannot
accidentally disclose an API key, while starting the MCP after the user exports
its variable still works.

## State, journal and reconciliation

Use a versioned `state.json` plus append-only `journal.jsonl`; all paths are
under the private state directory. The state schema contains:

- `schema_version`, generator version and run sequence;
- immutable source records (`git` safe URL + requested ref + resolved commit,
  or local canonical root + content digest), runtime/CLI exact versions and
  artifact paths;
- `resources` keyed by canonical effective resource fingerprint; a shared MCP
  with the same effective spec has one artifact and many bindings;
- each binding’s logical identity (`harness/kind/name`), adapter target,
  previous and desired fingerprints, ownership marker, source/runtime keys and
  last successful result;
- explicit outcomes `succeeded`, `failed`, `skipped`, `blocked`, `pending-removal`
  with safe error category/message, timestamps and dependency IDs; and
- backup metadata (private relative file, source/target checksum, original
  file mode). No secret, resolved environment value or credential-bearing URL
  is serialised.

Before every mutation, append a durable `intent` record; after it, append the
outcome. Save state through temp-file + rename only after operations that
succeeded. On startup recovery turns unfinished intents into a visible
`interrupted` outcome and re-inspects actual targets; it does not assume a
previously launched process failed or succeeded. State is ownership evidence,
not proof that the current file is still unmodified.

Reconciliation follows this order:

1. Parse and validate the full manifest before source, runtime or config work.
2. Build `EffectiveManifest`, resolve source revisions/digests and construct a
   deterministic DAG: source → runtime → artifact build → CLI → adapter binding.
   Deduplicate equal node keys. An upstream failure blocks only descendants;
   independent nodes continue.
3. Inspect every adapter target and compare its actual fingerprint with the
   state’s prior fingerprint and desired fingerprint. An identical desired,
   owned, undrifted binding is a no-op.
4. An unowned occupied target, a manually edited owned target, or a desired
   change requires an interactive `merge`, `replace`, `skip` choice before a
   write. Save a backup immediately before applying. `merge` is adapter-specific
   and must preserve recognised authorization and unrelated fields; native
   Codex agent files are whole-file values, so its safe merge is only an exact
   no-op or `replace` after backup.
5. A state-owned binding absent from desired YAML is `pending-removal`. Explain
   target and drift, ask, then delete only the recorded managed target. If it
   was manually edited, default to retain and require an explicit removal
   answer. Remove an artifact only after its final binding disappears.
6. Commit managed state only for successful installation/binding. Record every
   skipped/blocked/failed result separately and print a deterministic summary.

The Codex native-agent adapter writes only
`~/.codex/agents/<safe-resource-name>.toml`. It rejects unsafe resource names
for path construction, verifies that the selected cached source is a regular
TOML file, backs up an existing target on the selected replace path, writes a
temp sibling and renames atomically. It neither scans/modifies project
`.codex/agents` nor rewrites `config.toml`; unlisted global agent files remain
unowned and untouched.

## Doctor

`yashik doctor` acquires a shared/read lock and emits a deterministic table plus
non-zero status if any managed item is unhealthy. It observes only:

- state/journal consistency and abandoned intents;
- cache/artifact existence and recorded Git commit/content digests;
- Node and managed npm executable existence/version (known curated binaries
  may be queried with bounded `--version`; no arbitrary source command);
- adapter target existence, type and fingerprint; and
- missing, drifted, failed, blocked, pending-removal and orphaned artifacts.

It never consults remote Git/npm, changes files, invokes install steps, starts
an MCP or treats missing Codex authentication as an error. CLI presence/version
is therefore reported as `installed`; login/service availability is
`not checked`, not `healthy` by implication.

## Acceptance matrix

Unit/integration tests use a fake command runner, local Git fixture and HTTP
archive/npm fixture. The final rows run against a fresh temporary HOME on the
actual Ubuntu host; no Linux run claims macOS validation.

| ID | Scenario and expected evidence |
| --- | --- |
| E1 | Existing schema/effective tests still pass; invalid manifest causes no cache/state/HOME changes. |
| E2 | Git full SHA, branch/tag and no-ref resolve to exact commits; a moving branch produces a new artifact, while a pinned SHA reuses one. |
| E3 | Git with password/userinfo, `file:` or `ext::` is rejected before invocation; `git@host:path` with fake SSH agent is accepted. |
| E4 | Local relative path resolves from YAML directory; changes produce a new digest; absolute/escaping symlink and `from` escape fail before install. |
| E5 | Cache staging failure/interrupted process leaves no final object or false state; two attempts contend on the source/init lock safely. |
| E6 | `install.steps` reaches the artifact cwd as exact argv; a semicolon, `$()` and space remain literal arguments; `${source}` in a step is rejected. |
| E7 | Launcher expands source/env only at run time; state, rendered config, backup, journal and CLI report omit a sentinel secret. Missing env fails that MCP binding before config registration. |
| E8 | Node tarball and Bun ZIP fixtures with correct checksum activate; corrupted archive/checksum leaves prior runtime active. Linux/macOS URL-triplet mapping covers x64/arm64 and rejects unknown pair. |
| E9 | Missing `curl`/`git` selects only the appropriate fixed apt/brew recipe and asks before sudo; declined sudo is a skipped dependency, not a partial state success. |
| E10 | npm `latest` stores resolved exact semver/integrity; second run invokes neither Node download nor npm install. A pinned harness version installs that exact package. |
| E10a | Pi fails planning when the selected Node is below 22.19.0; OMP activates Bun >= 1.3.14 and its npm-installed bin resolves the `bun` shebang through the curated PATH. |
| E11 | Equal effective MCP across two harnesses yields one source/artifact install and two bindings; failed build creates no MCP binding. |
| E12 | Independent failure and success are both reported; dependency failure blocks only descendant nodes. |
| E13 | Second identical init produces no filesystem/config changes. A user edit to an owned target offers merge/replace/skip and never silently overwrites it. |
| E14 | Existing unowned Codex TOML is backed up for replace; a declined choice is unchanged. Native-file merge is correctly refused as unsupported. |
| E15 | Removing a managed YAML agent prompts; decline yields `pending-removal`; confirmed removal removes only its target and retains unrelated `~/.codex/agents/*.toml`. |
| E16 | Crash between intent and config/state commit is visible as `interrupted`; next init/doctor re-inspects rather than treating it as owned success. |
| E17 | `doctor` detects missing Node/CLI/artifact/binding, config drift, failed history and pending removal with no writes, downloads, install steps or MCP execution. |
| E18 | Real clean Ubuntu HOME: bootstrap Node 22, install Codex npm CLI and at least one fixed VoltAgent TOML such as `backend-developer`; prove `codex --version`, `~/.codex/agents/backend-developer.toml`, state ownership and a no-op second init. |
| E19 | Real Ubuntu conflict/removal: modify that TOML, decline replacement, then remove it from YAML and decline deletion; verify contents persist and doctor reports drift/pending removal. |
| E20 | Fresh macOS x64 and arm64 validation repeats E8, E10 and E18 after platform-specific tool discovery. Until both run, report macOS as unverified. |

## Decisions handed to Plan

1. Implement the generic engine modules and Codex adapter first; keep other
   harness package/config recipes in separate files/fixtures to avoid one
   monolithic installer.
2. Add a schema validation rule for safe names used as native target filenames;
   do not overload resource name with a destination path.
3. Add a private, generated MCP launch-spec format and hidden internal launcher
   command before shipping an MCP adapter; storing expanded `run.env` in a
   harness config violates the stated secret contract.
4. Treat source snapshots/artifacts, user-owned config bindings and runtime
   versions as distinct state objects. This is what makes idempotence, safe
   removal and `doctor` possible after partial failure.
5. Release Yashik binary bootstrap remains a delivery artifact with signed
   checksums. It must not be substituted with an undocumented `curl | sh` or a
   requirement that the user already has Rust.
