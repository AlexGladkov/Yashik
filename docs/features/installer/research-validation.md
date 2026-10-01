# Research: риски и validation установщика

Статус: **вход для Plan**. Это исследование критериев приёмки будущего
установщика; продуктовый код и пользовательские конфигурации не менялись.

## Что уже доказано, а что нет

Текущий Rust-каркас проверяет строгий YAML, семантику схемы и наследование
(`M1–M2`). Его CLI намеренно не читает источники, не запускает команды и не
пишет `HOME`. В частности, `effective.rs` пока лишь вычисляет
`local_source_path`; он не канонизирует источник или `from`. Поэтому успешные
schema/CLI tests нельзя считать доказательством ни одного из `M3–M12`.

Будущий validation должен работать в отдельном временном `HOME` и отдельном
XDG-каталоге. Он сравнивает дерево и содержимое конфигураций до и после, а не
только exit code Yashik. Mock адаптера полезен для unit-test, но не заменяет
запуск настоящего CLI в чистой Ubuntu.

## Внешние контракты, которые нужно проверять по версии

Первичные документы подтверждают, что у клиентов различаются global roots,
форматы MCP и discovery ресурсов. Например, Codex использует TOML-конфиг для
MCP и глобальные agent TOML; Claude Code поддерживает пользовательский scope
MCP и Markdown agents; OpenCode документирует глобальный
`~/.config/opencode`, JSON/JSONC и versioned MCP schema; Pi —
`~/.pi/agent`, `mcp.json` и Agent Skills; OMP имеет собственные profile-aware
roots. Нельзя превращать эту матрицу в вечные константы: validation фиксирует
реально установленную версию каждого CLI и проверяет её документированный
формат.

| Harness | Первичный источник для versioned fixture |
| --- | --- |
| Codex | [MCP](https://developers.openai.com/learn/docs-mcp), [skills](https://learn.chatgpt.com/docs/build-skills), [subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents) |
| Claude Code | [MCP](https://code.claude.com/docs/en/mcp), [skills](https://code.claude.com/docs/en/skills), [subagents](https://code.claude.com/docs/en/sub-agents), [memory/rules](https://code.claude.com/docs/en/memory) |
| OpenCode | [config](https://opencode.ai/v2/docs/config), [MCP](https://opencode.ai/v2/docs/mcp-servers), [skills](https://opencode.ai/docs/skills), [agents](https://opencode.ai/docs/agents) |
| Pi | [configuration](https://pi.dev/docs/latest/configuration), [MCP](https://pi.dev/docs/latest/mcp), [skills](https://pi.dev/docs/latest/skills) |
| OMP | [configuration discovery](https://github.com/can1357/oh-my-pi/blob/main/docs/config-usage.md), [MCP](https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md), [task agents](https://github.com/can1357/oh-my-pi/blob/main/docs/task-agent-discovery.md) |

Это также уточняет важную границу: Pi agent/subagent не считается работающим
только потому, что Yashik скопировал Markdown. До выбора и smoke test реального
Pi extension/adaptor результат должен быть `unsupported` с причиной. Если
релиз заявляет agents для всех пяти клиентов, отсутствие этого adapter —
blocking failure, а не допустимый пропуск.

## Инварианты, которые должен выбрать Plan

1. **Имена ресурсов.** Рекомендуемый переносимый контракт — ASCII
   `^[a-z][a-z0-9-]{0,62}$`. Имя — одновременно ключ конфигурации, будущий
   filename и идентификатор state, поэтому его нельзя «безопасно» нормализовать
   задним числом. Имена, которые после adapter-specific encoding совпадают,
   отклоняются до плана. Это включает `..`, `.`, slash/backslash, NUL,
   пробелы, управляющие символы, TOML/JSON syntax и case-fold collisions.
2. **Containment `from`.** Сначала канонизируется выбранный source root, затем
   разрешается `from` и его канонический target должен быть внутри этого root.
   Сам `source.local.path` по контракту может находиться вне каталога YAML;
   это не делает `from` свободным. Проверка одинакова для local и Git.
   Canonical root, являющийся пользовательской symlink, допустим, но symlink
   внутри source, ведущая наружу, — нет.
3. **State и secrets.** State/journal сохраняют spec fingerprint, URL/ref,
   resolved revision, ownership, bindings и outcome, но никогда развёрнутые
   environment values. Для переносимого v1 безопаснее принимать в `run.env`
   только `${env:NAME}`; literal secret должен быть ошибкой до какого-либо
   write. Existing user-auth fields в конфиге клиента могут пережить merge или
   backup, однако backup получает `0600` и не попадает в output/state/journal.
4. **Ownership не выводится из имени.** Управляемым становится только ресурс,
   чей write и fingerprint успешно записаны в state. Такой же foreign key в
   клиентском конфиге нельзя заменить или удалить без conflict flow.
5. **Versioned adapters.** Каждый adapter имеет fixture с точной версией CLI,
   входным config и ожидаемым config/discovery output. Обновление baseline
   версии требует заново прогнать fixture; «latest» не является validation
   baseline.

## Высокоценные acceptance tests

Все ниже запускаются как integration tests с изолированными `HOME`,
`XDG_CONFIG_HOME`, `XDG_STATE_HOME`, temporary source/cache и transcript,
который сканируется на marker secrets. Перед каждым сценарием создаются
sentinel files вне test root и checksum всего home. Неудачный сценарий обязан
проверять как отсутствие незапланированного конфиг-write, так и корректный
journal outcome.

| ID | Стимул | Приёмка |
| --- | --- | --- |
| V1 — Git/local и revision | Local source, bare local Git, fixed full SHA, moving branch/tag и no-ref source. | Local path считается от YAML. Fixed SHA повторно даёт тот же commit; branch/tag и latest повторно разрешаются, а state каждой попытки содержит фактический SHA. Source clone/fetch использует argv без shell и не считает ref из старого state вечным pin. |
| V2 — containment | Для local и Git fixture: `from` с `..`/absolute path, nested symlink в `/tmp/escape`, dangling link, source-root symlink и обычный путь внутри root. | Все escape/dangling cases отклоняются до install/config/state ownership. Root symlink и внутренний обычный файл проходят. При rejected path не создаются temp artifact, adapter config, backup или managed record. Отдельная race fixture меняет symlink между discovery и copy: implementation должна повторно проверить canonical target перед чтением/копированием либо использовать no-follow/open-at аналог; наружный sentinel не читается и не перезаписывается. |
| V3 — safe names | Сгенерировать допустимые и недопустимые имена, включая `../x`, `a/b`, `a\\b`, `.`, empty, leading/trailing whitespace, `a\u0000b`, `x]`, `x\"`, `Foo`/`foo` и длинное имя. Запустить для MCP, skill, agent и rule на каждом adapter. | Только контрактные имена порождают predictable config key/path. Недопустимые или adapter-colliding names отвергаются до plan. Никакой файл не появляется за designated resource/config roots и незнакомая конфигурационная запись не создаётся. |
| V4 — secrets | Set `YASHIK_TEST_SECRET` на уникальное значение; good MCP получает `${env:YASHIK_TEST_SECRET}` и сообщает лишь boolean `present`. Fixtures содержат literal secret, malformed YAML с secret и pre-existing auth token в клиентском config. | Child получает значение, но stdout/stderr/plan/error, state, journal, backups metadata и argv его не содержат. Literal env value отклоняется до write. Merge/replace сохраняет pre-existing auth field; byte backup может содержать исходный token только как защищённая копия original config (`0600`), не как новая запись Yashik. |
| V5 — независимые ошибки | Один MCP имеет successful source, но failing build (`exit 23`); другой skill/rule/agent имеет независимый good source; отдельный resource зависит от отсутствующего runtime. | Good independent resources устанавливаются. Failing MCP не регистрируется ни в одном client config, resource with missing runtime отмечен `blocked`, а failure — `failed`; report/journal содержит точную причину каждого outcome без secret. Планировщик не останавливает очередь после первой ошибки. |
| V6 — shared MCP и bindings | Одинаковый effective stdio MCP назначен двум harnesses; третий получает локальный override. | Общий build/artifact выполняется один раз и имеет две bindings; override имеет отдельный artifact и не переписывает common. Failed shared build не оставляет частичной binding ни у одного клиента. |
| V7 — merge, replace, backup | Prepopulate supported TOML/JSON/JSONC/Markdown config с foreign neighbours, auth field и managed entry, затем выбрать scripted `merge` и `replace`. | Меняется только managed entry. Foreign neighbours и recognised auth сохраняются; merge сохраняет отсутствующие в manifest поля, replace сбрасывает только managed resource. Перед каждой реальной модификацией есть readable exact backup, режим `0600`, атомарная final write и ни одного truncated config при injected write failure. |
| V8 — idempotence, drift, recovery | Successful init запускается второй и третий раз. Затем вручную меняется managed config/artifact; отдельно имитируется crash после artifact write до state commit. | Converged run не создаёт дубликаты, extra backups или повторный build; managed config/artifact остаются byte-equivalent. Manual edit обнаруживается как drift/conflict, не перезаписывается молча. Subsequent init или doctor корректно классифицирует незавершённую попытку и не приписывает ownership artefact, чей commit не состоялся. |
| V9 — deletion | Из YAML убрать owned resource, foreign same-name entry и manually modified owned entry; прогнать confirm и decline. | Только clean, positively-owned resource становится кандидатом. Decline не меняет config/artifact и оставляет pending-removal state. Foreign or drifted entry не удаляется без conflict decision; unrelated config and resources остаются. |
| V10 — doctor | Отсутствующий binary/artifact, missing binding, drifted config, failed/blocked history и no-auth MCP. До/после сравнить полный HOME. | `doctor` читает, но ничего не меняет; различает missing/drift/previous failure, не утверждает, что CLI login или remote model access подтверждены, и не печатает secret. |

`V1–V10` трассируются к M3–M10. M12 дополнительно запускает bootstrap в
чистой Ubuntu и macOS; Ubuntu result никогда не закрывает macOS.

## Реальный Ubuntu smoke: public Git и Codex primary flow

Это не placeholder `example.invalid`. Следующий manifest использует публичные
репозитории и полные Git revisions, проверенные в исследовании 2026-10-01:

- MCP test server: `modelcontextprotocol/servers`,
  `f46d9578190b476b3501923ea8977d899e8db2cb`, `src/everything`. В его
  `package-lock.json` и `start:stdio` есть настоящий protocol test server;
  build запускается явно, а не через неявный shell.
- Skill: `openai/skills`,
  `49f948faa9258a0c61caceaf225e179651397431`, `cli-creator/SKILL.md`.
- Codex agents: `VoltAgent/awesome-codex-subagents`,
  `7add6913c53ccbbc250c481815c9c6afb02709c6`, native TOML
  `backend-developer` и `reviewer`. Эти два файла имеют требуемые Codex
  `name`, `description` и `developer_instructions`; target discovery —
  `~/.codex/agents/*.toml`.
- Codex native rule берётся из pinned `AGENTS.md` рядом с MCP source. В
  disposable HOME это безопасный format/discovery probe; product rule fixture
  перед выпуском должна быть маленьким нейтральным Yashik-owned public Git
  source, чтобы можно было проверить marker instruction.

```yaml
version: 1
harnesses:
  codex:
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
    rules:
      mcp-everything-guidance:
        format: native
        source:
          type: git
          url: https://github.com/modelcontextprotocol/servers.git
          ref: f46d9578190b476b3501923ea8977d899e8db2cb
        from: src/everything/AGENTS.md
  claude: {}
  opencode: {}
  pi: {}
  omp: {}
mcp:
  protocol-everything:
    source:
      type: git
      url: https://github.com/modelcontextprotocol/servers.git
      ref: f46d9578190b476b3501923ea8977d899e8db2cb
    from: src/everything
    transport: stdio
    install:
      requires: [node]
      steps:
        - [npm, ci, --ignore-scripts]
        - [npm, run, build, --workspace=@modelcontextprotocol/server-everything]
    run:
      command: node
      args: ["${source}/src/everything/dist/index.js", stdio]
skills:
  cli-creator:
    source:
      type: git
      url: https://github.com/openai/skills.git
      ref: 49f948faa9258a0c61caceaf225e179651397431
    from: skills/.curated/cli-creator
```

The smoke runner must first run `git ls-remote`/clone verification that each
full SHA still resolves to the expected object, install a recorded version of
`yashik`, Node and each of the five CLIs in an Ubuntu VM, then capture
`--version` output. It runs `yashik init smoke.yaml` with only scripted answers
and verifies all of the following:

1. The MCP source builds once, a direct MCP client completes initialize,
   `tools/list` and an `echo` call, and each real CLI reports the corresponding
   global MCP as loaded/connected through its own versioned probe.
2. Each real CLI discovers the installed `cli-creator` skill through a native
   list/diagnostic/startup probe, not merely because `SKILL.md` exists on disk.
3. Codex discovers both native agents by name and can delegate a harmless
   read-only task to each. It discovers the native rule after a fresh process
   start. The transcript records only resource names and versions.
4. The same VM then runs the per-adapter MCP/skill/rule/agent fixture. A pass
   requires evidence from all five CLIs. An adapter that has no verified agent
   capability (currently Pi) returns explicit `unsupported`; this cannot close
   an acceptance item promising that all five have working agents.
5. A second `init` proves V8; a `doctor` snapshot proves V10. Test accounts,
   if a client needs authentication to perform an actual delegation, are
   separate disposable accounts. MCP handshake and installation are tested
   without claiming that service/model authentication was verified.

Do not publish an Ubuntu success as macOS support. Do not replace this smoke
with a test that merely parses configs: successful real discovery and MCP
handshake are the minimum evidence that resources work.
