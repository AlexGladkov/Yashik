# Research: спецификация и план Yashik

Первый прогон профиля business-feature завершает исходную задачу интервью:
спецификация, согласованный пример YAML и план разработки. Это не реализация
установщика. Consilium разделён на архитектуру, интеграции и reconciliation.

## Архитектура

Rust CLI с типизированным YAML-контрактом, отдельными источниками, установщиками,
планировщиком, адаптерами харнессов и журналом управляемых ресурсов. Повторный
init сравнивает желаемое состояние с реальным, а не только с прошлым манифестом.
Удалять можно только ранее управляемые ресурсы и после вопроса пользователю.
Изменения конфигурации требуют сохранения остальных полей и резервных копий.
Пути из источников разрешаются внутри корня источника, включая ссылки.

Установка зависимостей и пользовательские build steps выполняют код; массив
аргументов исключает неявный shell, но не делает скачанный код безопасным.
Рецепты должны содержать воспроизводимые источники и команды, ошибки должны
останавливать зависимые операции, но не независимые установки.
Отсутствующие credentials не должны блокировать установку публичных ресурсов;
авторизация моделей и MCP остаётся за пользователем.

## Интеграции

Матрица — результат исследования первичной документации, не проверка на
установленных CLI. Точные версии/форматы подтверждаются перед кодированием
адаптера. Проектные ресурсы не входят в требования.

| Харнесс | Глобальный MCP | Skills | Субагенты | Rules |
| --- | --- | --- | --- | --- |
| Codex | TOML mcp_servers в config.toml | SKILL.md | TOML agents | AGENTS.md |
| Claude Code | JSON mcpServers в ~/.claude.json | SKILL.md | Markdown frontmatter | rules/*.md / CLAUDE.md |
| OpenCode | JSON/JSONC, зависит от версии | SKILL.md | Markdown frontmatter | AGENTS.md |
| Pi | mcp.json, подтвердить по версии | SKILL.md | Нужен пакет/расширение; нативный формат не подтверждён | AGENTS.md |
| OMP | JSON mcpServers в mcp.json | SKILL.md | Markdown frontmatter | RULES.md / rules |

Нельзя обещать полностью одинаковую функциональность пяти харнессов.
Pi subagents проходят ветку вопроса о неподдерживаемой возможности, пока
адаптер соответствующего расширения не определён. OpenCode требует привязки
формата к версии. OMP может использовать профиль с другим пользовательским
корнем. ~/.claude.json содержит постороннее состояние: изменять только
управляемые MCP-поля, даже если выбран режим замены управляемых ресурсов.

Первичные источники:

- Codex: https://github.com/openai/codex,
  https://learn.chatgpt.com/docs/agent-configuration/subagents,
  https://developers.openai.com/learn/docs-mcp.
- Claude: https://code.claude.com/docs/en/setup,
  https://code.claude.com/docs/en/mcp,
  https://code.claude.com/docs/en/skills,
  https://code.claude.com/docs/en/sub-agents,
  https://code.claude.com/docs/en/memory.
- OpenCode: https://opencode.ai/v2/docs/config,
  https://opencode.ai/v2/docs/mcp-servers,
  https://opencode.ai/v2/docs/agents.
- Pi: https://pi.dev/docs/latest/configuration,
  https://pi.dev/docs/latest/mcp,
  https://pi.dev/docs/latest/packages.
- OMP: https://github.com/can1357/oh-my-pi,
  https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md,
  https://github.com/can1357/oh-my-pi/blob/main/docs/task-agent-discovery.md,
  https://github.com/can1357/oh-my-pi/blob/main/docs/context-files.md.

## Проверка среды

Запуск gpt-5.6-terra с reasoning xhigh инструментом делегирования успешен.
Предыдущее предположение о недоступности модели опровергнуто запуском.
Rust/cargo доступны. crates.io не разрешается через shell DNS; YAML-crate
не найден в локальном Cargo cache. Сборка будущего YAML CLI требует доступа
к зависимостям или проверенного vendoring. Самодельный YAML parser не предлагается.

## Открытые решения

Каталоги/marketplace, URL манифеста, детали remote MCP, версия OpenCode,
пакет для Pi subagents и способы доставки runtime уточняются до реализации
соответствующего этапа. Их нельзя считать согласованными требованиями лишь
на основании примеров или рекомендаций исследователей.

## Сведение Consilium

Исследователь reconciliation предложил DAG установок, владение артефактами,
атомарные изменения, doctor без запуска авторизации и проверки идемпотентности,
конфликтов, удаления, дрейфа и частичных ошибок. Эти рекомендации приняты.

Отклонены предложения, расходящиеся с интервью: явные targets вместо общих
блоков, только нативные agents вместо переносимых и нативных, apply вместо
init, запрет относительных локальных путей и проектный scope. Также отсутствие
MCP-токена не должно запрещать установку; пользователь настраивает доступ позже.
latest должен повторно разрешаться при init, с записью точной версии для данного
прогона; прошлый lock не превращает latest в вечный pin.

Исследователи разошлись в актуальном рекомендуемом пути Codex skills
(~/.codex/skills и ~/.agents/skills), а также версии OpenCode. Это открытые
вопросы проверки адаптеров, не основание записать один путь как универсальный.
Нужны version-specific fixtures и проверка первичных источников при реализации.
