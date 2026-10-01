# Plan: реальный installer

Основание: [Research](research.md), [adapters](research-adapters.md),
[engine](research-engine.md), [validation](research-validation.md),
[требования](../../requirements.md) и [design](../../design.md).
Этап Plan завершён; пользователь уже разрешил реализацию и испытания.
Дополнительное согласование плана не требуется.

## Результат и границы

`yashik init <manifest>` действительно устанавливает пользовательские CLI
Codex, Claude Code, OpenCode, Pi и OMP, разрешает Git/local sources, выполняет
явные MCP install steps и доставляет поддерживаемые skills, agents и rules.
Повторный запуск сверяет desired state с фактическим окружением; `doctor`
наблюдает записанное окружение без изменения файлов, загрузок или MCP запуска.
Старая проверка схемы переносится в `yashik check <manifest>`.

Первая обязательная вертикальная приёмка — Ubuntu 24.04 на
`<authorized Ubuntu host>`, простой YAML с Codex и двумя native VoltAgent agents:
`backend-developer` и `reviewer`. После неё обязательны повторный `init` и
`doctor`. Эта проверка является первым измеримым результатом, а не заменой
полного installer для пяти harnesses и остальных ресурсов.

Сохраняются YAML v1 и существующее наследование `EffectiveManifest`.
Установка остаётся интерактивной: conflicts, removal, sudo и unsupported
capabilities требуют решения в терминале. EOF не считается согласием.
Авторизация в CLI и сервисах остаётся действием пользователя. Установщик
не запускает пользовательские `run.command` во время `init`.

В эту работу не входят marketplace, remote MCP, URL-манифесты, экспорт машины,
модели/providers/permissions/hooks, новый unattended-режим, установщик
Homebrew, подписанный release pipeline или отдельный DAG framework.
Разработка и доставка собранного Yashik binary на разрешённый Ubuntu host
достаточны для текущей приёмки; они не требуют Rust на пользовательской машине.

## Решения, закрывающие расхождения Research

1. Источники, runtimes/CLI, adapters/state и orchestration имеют отдельные
   модули. Последовательное выполнение с memoization результатов достаточно:
   ошибка блокирует зависимые bindings, независимые операции продолжаются.
2. Отсутствующий `${env:NAME}` **не блокирует** загрузку публичного источника,
   build или регистрацию MCP wrapper. Значение понадобится при реальном
   запуске MCP; `doctor` сообщает только отсутствующие имена переменных.
   Это заменяет формулировки research-engine E7 и ранних research notes.
3. `run.env` хранит исходные `${env:NAME}` references. Literal values для
   `run.env` отклоняются installer validation до записи; значения секретов
   никогда не записываются в state, launch spec, rendered config или журнал.
   Старый schema-only `check` не обязан обращаться к текущему environment.
4. `${source}` означает постоянный корень **writable artifact**, содержащий
   snapshot source. `from` выбирает файл/каталог внутри него, но не меняет
   смысл `${source}`. Явные install steps выполняются из корня artifact;
   аргументы steps буквальные, interpolation в них отклоняется.
5. Native Codex agents — валидные `.toml` файлы с `name`, `description` и
   `developer_instructions`; после проверки копируются byte-for-byte.
   Настройки model/sandbox upstream не изменяются.
6. Native Codex rules не превращаются из Markdown в Starlark policy.
   Они, native OpenCode/Pi/OMP rules и Pi agents идут в явную ветку
   `unsupported → skip/cancel`. Это поддержка честного capability flow,
   а не успешная установка этих возможностей.
7. OpenCode выбирает формат по реальной major version: verified 1.18.34
   использует V1 direct `mcp.<name>` с `type: local`, command array,
   `environment`, `enabled`; V2 использует `mcp.servers`. V1 agent/skill roots
   имеют singular `agent` и `skill`. Нельзя записывать V2 schema в 1.x CLI.
   Неизвестная/unverified major version — unsupported с сохранением остальных.

## Runtime и CLI recipes

Рецепты закрыты и управляются кодом, а не произвольными package-manager
полями YAML. Runtime baseline — Node 22, не ниже 22.19.0, и Bun не ниже
1.3.14. Execute фиксирует выбранные версии и официальные SHA-256 для
`linux/darwin × x64/arm64` в registry; проверяет archive до extraction.
Не использовать интернет shell installers. Уже подходящий runtime можно
использовать после version probe; недостающий — установить в managed data dir.

| Harness | Package | Установка в собственный user prefix | Bin |
| --- | --- | --- | --- |
| Codex | `@openai/codex` | managed Node/npm | `codex` |
| Claude | `@anthropic-ai/claude-code`, verified 2.1.286 | managed Node/npm, Node >=22 | `claude` |
| OpenCode | `opencode-ai` | managed Node/npm | `opencode` |
| Pi | `@earendil-works/pi-coding-agent` | managed Node/npm, Node >=22.19.0 | `pi` |
| OMP | `@oh-my-pi/pi-coding-agent` | managed Bun, Bun >=1.3.14 | `omp` |

Для Claude npm recipe — подтверждённый version-pinnable fallback, несмотря
на рекомендацию upstream использовать native installer. Для OMP использовать
Bun global recipe с отдельными `BUN_INSTALL_GLOBAL_DIR` и
`BUN_INSTALL_BIN`; npm без рабочего Bun не является успешной установкой.
Каждый harness имеет собственный prefix, чтобы пакеты и их executable не
конфликтовали. Managed `bin` содержит рабочие launchers с нужным runtime PATH;
после установки CLI обязан пройти свой `--version` probe.

Явная версия означает точную npm/Bun package version; отсутствие или `latest`
разрешается через package metadata заново на каждом `init`. Install получает
точную разрешённую версию, state — version/integrity и executable path.
Если exact version уже установлена и probe согласуется со state, package
install не повторяется. PATH shell profile автоматически не редактируется;
вывод показывает один необходимый PATH export, если managed bin отсутствует.

Если отсутствуют bootstrap tools, Linux Debian/Ubuntu использует только
фиксированные argv apt recipes для Git/curl/archive tools/CA certificates;
перед sudo требуется choice. macOS использует доступные системные tools либо
уже установленный Homebrew с фиксированными recipes. Неизвестная платформа
или отказ от dependency не превращаются в успех; независимые ресурсы продолжают
работу. Проверка Linux не закрывает macOS acceptance.

## Sources, исполнение и MCP launcher

Sources не изменяют исходный local directory. Relative local path берётся
из готового `EffectiveResource.local_source_path`; canonical root и
детерминированный tree digest дают immutable cache snapshot. Git HTTPS/SSH
разрешается в полный commit, экспортируется без `.git`; source key включает
commit. Пин полного SHA повторно использует cache. Moving ref/default HEAD
повторно разрешается, поэтому изменение upstream создаёт новый artifact.
Git URLs с credentials и небезопасные protocols отклоняются до Git invocation.
Приватный Git использует уже существующие SSH agent/credential helpers.

`from` и вся копируемая source tree проверяются на containment: absolute или
escaping symlinks, dangling links, special files и traversal не допускаются.
Source-root symlink допустим после canonicalization. Проверка повторяется
при чтении/copy, чтобы изменение symlink не открыло наружный файл.
Не следовать symlinks через обычный рекурсивный `path.is_dir()` copier.

Artifact key включает resolved source revision/digest и effective install/run
spec. Равные effective MCP в разных harnesses имеют один build/artifact,
разные overrides — отдельные. Install steps используют `Command` и отдельные
argv, artifact cwd и runtime PATH; shell/eval отсутствуют. Failed build не
создаёт MCP bindings, successful independent skill/agent/rule остаётся
кандидатом на установку даже при ошибке MCP или другого CLI.

Внутренняя команда `yashik mcp-launch <launch-id>` читает private launch spec,
раскрывает `${source}` и `${env:NAME}` при запуске, задаёт env overrides и
исполняет argv непосредственно. Launch spec содержит исходный `Run`, artifact
root и runtime bins, но не развёрнутые env values. Все adapters регистрируют
абсолютный persistent Yashik launcher и launch-id. Отсутствующий env при
запуске даёт безопасную ошибку с именем переменной. Unknown/unclosed tokens
отклоняются перед binding; нельзя делать recursive expansion.

Install logs ограничиваются по размеру и очищаются от потенциальных env
secret values. Не включать raw source, YAML values, credential URLs и
child environment в report/error. Запуск MCP является действием harness,
а не способом `init` доказать наличие токена.

## Adapter и reconciliation contract

Точные projections и native validation берутся из
[research-adapters](research-adapters.md). Минимальная карта destinations:

| Harness | MCP | Skills | Agents | Portable rules |
| --- | --- | --- | --- | --- |
| Codex | `~/.codex/config.toml`, `mcp_servers` | `~/.agents/skills/<name>` | `~/.codex/agents/<name>.toml` | managed block `~/.codex/AGENTS.md` |
| Claude | `~/.claude.json`, `mcpServers` | `~/.claude/skills/<name>` | `~/.claude/agents/<name>.md` | `~/.claude/rules/<name>.md` |
| OpenCode | version-selected XDG `opencode/opencode.json(c)` | V1 `opencode/skill/<name>`, V2 `skills` | V1 `opencode/agent/<name>.md`, V2 `agents` | managed block global `AGENTS.md` |
| Pi | `<agent-dir>/mcp.json`, `mcpServers` | `<agent-dir>/skills/<name>` | unsupported | managed block `<agent-dir>/AGENTS.md` |
| OMP | `<agent-dir>/mcp.json`, `mcpServers`, `type: stdio` | `<agent-dir>/skills/<name>` | `<agent-dir>/agents/<name>.md` | managed block `<agent-dir>/AGENTS.md` |

Pi/OMP honor `PI_CODING_AGENT_DIR`; default roots are `~/.pi/agent` and
`~/.omp/agent`. OMP active profile must be detected/resolved or explicitly
unsupported, never silently written into an inactive default. Native Markdown
agents retain optional fields; portable ones render required frontmatter.
Skills validate `SKILL.md` and required discovery metadata before write.

Resource names used as filenames/config keys accept ASCII
`^[a-z][a-z0-9-]{0,62}$`; validate before source/runtime/config work. Never
silently normalize names. Preserve unrelated TOML, JSON, Markdown and JSONC
content. JSONC edits must preserve comments; a lossy parse/write fallback
requires unsupported flow rather than silently stripping them.

An unowned occupied target or drifted owned entry requires merge/replace/skip.
Native-file merge cannot safely combine instructions: expose replace/skip
with an explanation rather than pretend to merge. Structured MCP merge retains
unrequested fields; replace changes only that managed entry. Managed rules
blocks preserve surrounding Markdown; ambiguous/multiple markers conflict.
All writes and confirmed removals receive exact protected backups and atomic
replacement. Compare selected entry/block fingerprints, not whole config hash,
so an unrelated foreign edit does not become managed-resource drift.

Ownership comes only from a successful recorded write, never from name/path.
Desired-removed owned resources require a specific removal answer. Decline
records pending-removal; drifted removal defaults to retain. Whole-source
artifacts are removed only when no retained binding references them.
No converged rerun creates backups, rewrites bindings or repeats build/install.

Private state/journal/launch specs/backups live under Linux XDG state/cache/data
roots or macOS Library roots. Files containing specs/backups get 0600 and
private directories 0700. One init lock prevents concurrent writes. `doctor`
loads without directory creation or journal repair. Durable mutation intents
and per-operation outcomes expose interrupted runs; unknown result does not
become automatic ownership. Atomic state commits retain successful independent
operations when later ones fail.

## Shared Rust API and ownership

Execute uses at most three Engineer agents with the required Luna/max profile.
Each receives only its scope, this plan and needed Research. They work in a
shared checkout, must preserve other engineers' changes, and do not edit files
outside their ownership. Root owns docs and real-host validation.

| Engineer | Exclusive files | Responsibility |
| --- | --- | --- |
| E1 sources/runtime/util | `src/install/paths.rs`, `sources.rs`, `runtime.rs`, `executor.rs`, `util.rs`, `tests/installer_sources.rs`, `tests/installer_runtime.rs` | paths, source containment/cache, fixed runtimes/CLI recipes, argv execution, shared hashing/copy helpers |
| E2 state/adapters | `src/install/state.rs`, `src/install/adapters.rs` or `adapters/*`, `tests/installer_adapters.rs`, `tests/installer_state.rs` | native formats, capability probes, ownership records, backup/atomic mutation and journal |
| E3 orchestration | `src/install/mod.rs`, `api.rs`, `engine.rs`, `launcher.rs`, `doctor.rs`, `src/main.rs`, `src/lib.rs`, `src/schema.rs`, `src/validation.rs`, `Cargo.toml`, `Cargo.lock`, `tests/cli.rs`, `tests/installer_engine.rs`, necessary existing core tests | shared contracts, installer validation, prompts, sequential dedup execution, CLI/launch/doctor integration |

Before E1/E2 dependent implementation, E3/root scaffolds `api.rs` and dependencies
and sends its exact declarations to both. E1/E2 send dependency additions to
E3; they never edit Cargo or exports concurrently. Shared contract changes are
communicated before callers are changed. Module-local tests stay with owners.

The following is the fixed interface vocabulary; avoid parallel incompatible
`Paths`, `Runtime` or state definitions:

```rust
// src/install/api.rs; shared derives: Clone/Debug, Serialize/Deserialize
// where state needs them. InstallResult errors contain sanitized messages.
pub type InstallResult<T> = Result<T, String>;
pub struct Paths {
    pub home: PathBuf, pub data: PathBuf, pub cache: PathBuf,
    pub state: PathBuf, pub bin: PathBuf,
}
pub struct ResolvedSource { pub key: String, pub revision: String, pub root: PathBuf }
pub struct RuntimeEnv { pub bin_dirs: Vec<PathBuf> }
pub struct InstalledCli {
    pub harness: HarnessId, pub version: String, pub executable: PathBuf,
    pub integrity: Option<String>, pub runtime_bins: Vec<PathBuf>,
}
pub enum ResourceKind { Mcp, Skill, Agent, Rule }
pub struct BindingRequest {
    pub harness: HarnessId, pub kind: ResourceKind, pub name: String,
    pub format: Option<ResourceFormat>, pub description: Option<String>,
    pub source_path: Option<PathBuf>, pub artifact_key: Option<String>,
    pub launch_id: Option<String>,
}
pub enum BindingPayload {
    File(Vec<u8>), Directory(PathBuf),
    Mcp(serde_json::Value), RulesBlock(String),
}
pub struct PreparedBinding {
    pub id: String, pub harness: HarnessId, pub kind: ResourceKind,
    pub names: Vec<String>, pub target: PathBuf, pub selector: Option<String>,
    pub desired_fingerprint: String, pub payload: BindingPayload,
    pub artifact_keys: Vec<String>,
}
pub struct ManagedBinding {
    pub id: String, pub harness: HarnessId, pub kind: ResourceKind,
    pub names: Vec<String>, pub target: PathBuf, pub selector: Option<String>,
    pub fingerprint: String, pub desired_fingerprint: String,
    pub artifact_keys: Vec<String>,
}
pub enum WriteChoice { Merge, Replace, Skip }
pub enum Observation { Missing, Present { fingerprint: String } }
pub enum Outcome { Succeeded, Unchanged, Failed, Blocked, Skipped, PendingRemoval, Interrupted }
pub struct LaunchSpec { pub artifact_root: PathBuf, pub run: Run, pub runtime_bins: Vec<PathBuf> }
pub struct BootstrapRecipe {
    pub commands: Vec<Vec<String>>, pub needs_sudo: bool,
    pub missing_tools: Vec<String>,
}
```

`PreparedBinding.names` has one name normally, all portable-rule names for an
aggregated harness block. Its stable `id` is `harness/kind/name`, or
`harness/rules-block` for that aggregate. `selector` identifies a normalized MCP
entry or managed rules block inside a shared file; whole files/directories use
`None`. `desired_fingerprint` hashes canonical desired payload, while a
record's `fingerprint` hashes the actual managed entry after merge/apply.
`BindingRequest.launch_id` lets adapters derive the persistent Yashik launcher
path from `Paths`; no resolved env is passed into adapter code.

Required module entry points (details may add private helpers):

```rust
paths::discover() -> InstallResult<Paths>; // no writes
paths::create_private(&Paths) -> InstallResult<()>;
sources::resolve(&Paths, &Source, Option<&Path>) -> InstallResult<ResolvedSource>;
sources::select(&ResolvedSource, Option<&str>) -> InstallResult<PathBuf>;
sources::materialize_artifact(&Paths, &ResolvedSource, &str) -> InstallResult<PathBuf>;
runtime::ensure(&Paths, &str) -> InstallResult<RuntimeEnv>; // closed node/bun
runtime::install_cli(&Paths, HarnessId, Option<&str>) -> InstallResult<InstalledCli>;
runtime::bootstrap_recipe(&[String]) -> InstallResult<Option<BootstrapRecipe>>;
executor::run(&[String], &Path, &RuntimeEnv) -> InstallResult<()>;
adapters::capability(HarnessId, ResourceKind, Option<ResourceFormat>, &str)
    -> InstallResult<()>; // unsupported checked per resource before prepare
adapters::prepare(&Paths, &InstalledCli, &[BindingRequest])
    -> InstallResult<Vec<PreparedBinding>>;
adapters::inspect(&PreparedBinding) -> InstallResult<Observation>;
adapters::inspect_managed(&ManagedBinding) -> InstallResult<Observation>;
adapters::apply(&Paths, &PreparedBinding, WriteChoice) -> InstallResult<ManagedBinding>;
adapters::remove(&Paths, &ManagedBinding) -> InstallResult<()>;
state::load(&Paths) -> InstallResult<State>; // absent state is empty, no writes
state::save(&Paths, &State) -> InstallResult<()>; // atomic
state::journal(&Paths, &OperationRecord) -> InstallResult<()>;
engine::init(&EffectiveManifest, &Paths) -> InstallResult<RunReport>;
doctor::run(&Paths) -> InstallResult<RunReport>;
launcher::run(&Paths, &str) -> InstallResult<()>;
```

E2 defines public `State` and `OperationRecord` in `state.rs` immediately and
shares declarations with E3. State contains keyed sources/artifacts/CLIs,
`BTreeMap<String, ManagedBinding>` and operation outcomes; original source URL
is stored only after credential validation. E3 defines `RunReport`, prompts
and operation orchestration. State load never repairs state silently.
E1 runtime/executor APIs never prompt: E3 supplies required tool names to
`bootstrap_recipe`, owns its sudo choice and invokes only the returned fixed
commands. `bootstrap_recipe` returns `None` if tools already exist; unsupported
platform/dependency produces a sanitized error. Declined recipe blocks only
operations requiring those tools.

`prepare` must not let one unsupported resource discard all independent
requests: E3 calls per resource, with portable rules as one explicit aggregate,
and handles unsupported errors separately. Adapters do no downloading,
install/build execution, prompting or state success commit. `apply/remove`
use E2's backup/write helpers; E3 journals intent before mutation and commits
returned ownership only after success.

## Execute order

1. E3/root publishes common API/dependencies; preserve parser/effective behavior.
   E1/E2 then implement independently. Cargo dependencies stay small: existing
   Serde/YAML plus JSON, preserving TOML, hashing and locking; JSONC support
   must use a comment-preserving editor or a bounded equivalent.
2. E1 first delivers real Git/local and Node + Codex recipe; E2 first delivers
   native Codex TOML validation/copy, state, backup and observation. E3 connects
   `check`, real `init`, summaries and read-only `doctor` immediately.
3. Root builds and runs the minimal Codex+VoltAgent YAML on Ubuntu. Failures
   return to the responsible engineer; correct that vertical path before
   broadening smoke to other resources.
4. E1 completes remaining four CLI recipes including Bun; E2 completes all
   supported MCP/skill/portable agent/rule adapters and unsupported branches;
   E3 completes MCP launch, shared build dedup, prompt/removal/recovery paths.
5. Module tests and integration tests pass; root updates README/examples/design
   to describe actual capabilities and observed platform limits. Separate
   Engineer Review uses the profile's Terra/max mapping; fixes return to
   Execute. Separate Tester Validate uses Luna/max and records actual evidence.
6. Root repeats approved Ubuntu primary scenario after fixes, then checks broader
   real installation/discovery and documents remaining macOS/auth limitations.

## Acceptance and verification

| ID | Required evidence |
| --- | --- |
| I1 schema/check | Existing schema/effective tests pass; malformed/unsafe installer manifest creates no state/cache/config. `check` is read-only and replaces old schema-only CLI tests. |
| I2 sources | Git exact/moving/default refs and local changes produce correct snapshots; local path is manifest-relative; containment, symlink race and credential/protocol rejection do not touch outside sentinels. |
| I3 runtimes/CLIs | Exact/latest version recorded, correct runtime PATH, real version probe. Node/Bun archive checksum failure retains previous installation. Pi minimum Node and OMP Bun enforced. Platform recipes have fixed argv and sudo choices. |
| I4 primary Ubuntu | Real Node/Codex install, both VoltAgent native TOMLs at `~/.codex/agents`, required fields parsed, bytes equal pinned source, ownership recorded. `codex --version` succeeds. |
| I5 rerun/doctor | Second primary `init` makes no binding rewrite/build/package install or extra backup. `doctor` returns deterministic installed status and compares read-only filesystem snapshot. Authentication is reported as not checked. |
| I6 partial failure | Failed MCP build gets no bindings; unrelated successful agent/skill/rule and other CLI continue. Equal shared MCP builds once; override separately. Report distinguishes failure, blocked, skipped and success. |
| I7 MCP/secrets | Each supported adapter has wrapper stdio config. Fake/direct MCP client verifies argv and launch-time env expansion; marker secret absent from report/state/journal/config/spec. Missing env permits registration and is listed by name in doctor; launch fails safely until exported. |
| I8 preservation | Foreign config/auth fields and JSONC comments survive; selected native replace has exact private backup. User-edited owned target prompts and skip preserves bytes. Atomic failure never truncates target. |
| I9 removal/recovery | Removed YAML owned resource prompts; decline keeps resource and pending-removal. Confirm removes only owned entry, retains foreign neighbours/shared artifacts. Interrupted intent becomes visible without falsely claiming ownership. |
| I10 all-five/resources | Real CLI installation/version evidence for all five, version-specific adapter fixtures and supported MCP/skill/agent/rule discovery probes. Explicit unsupported Pi-agent/native-rule cases prompt and remain skipped/cancelled outcomes. Unsupported is never counted as successful discovery. |
| I11 platform honesty | Linux/macOS recipe/unit coverage passes. Actual clean Ubuntu proves Linux only; macOS x64/arm64 remain unverified until run. No claim of live model delegation without user auth evidence. |

Root creates this exact primary manifest on the Ubuntu host; it is deliberately
small and uses no auth-dependent calls:

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
      reviewer:
        format: native
        source:
          type: git
          url: https://github.com/VoltAgent/awesome-codex-subagents.git
          ref: 7add6913c53ccbbc250c481815c9c6afb02709c6
        from: categories/04-quality-security/reviewer.toml
```

Use an isolated HOME/XDG area for automated writes and deliberate conflict
fixtures. Real server acceptance records Yashik/CLI/runtime versions, commands,
checksums, first/second-run summaries and doctor output in validation.md.
Tests never replace genuine bootstrap with a preinstalled Node or mock CLI and
then claim clean-host acceptance. Run `cargo fmt --check`, `cargo test` and
appropriate compiler/lint checks once the complete integration compiles; rerun
affected checks after fixes. Review/Validate limitations remain explicit until
their acceptance evidence exists.
