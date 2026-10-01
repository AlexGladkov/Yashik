# Plan: Rust-каркас и YAML-схема

Основание: [Research](research.md), [требования](../../requirements.md),
[design](../../design.md), [пример](../../../examples/yashik.yaml).
Этап 1 реализует M1–M2 исходного roadmap; код и проверки уже разрешены.

## Результат и границы

`yashik init <manifest>` читает YAML, проверяет схему и выводит эффективные
ресурсы включённых harnesses в стабильном порядке. Вывод явно сообщает:
это проверка схемы, установка и изменение окружения не выполняются.
`doctor` печатает объяснение недоступности диагностики на этапе 1 и возвращает
ненулевой код. Help/version доступны; неверные аргументы завершаются ошибкой.
Нет скачивания, исполнения install/run, записи state, установки CLI,
изменения HOME или конфигов; bootstrap и остальные M-критерии остаются позже.

## Typed-контракт

Serde structs/enums с `deny_unknown_fields`; словари — `BTreeMap` с отдельным
десериализатором, отклоняющим повтор ключа до вставки. Обычного BTreeMap
недостаточно: нельзя молча сохранить последнее значение duplicate key.
Повторы полей structs и ключей всех maps, включая `run.env`, — ошибки.

| Узел | Поля и типы |
| --- | --- |
| Manifest | Обязательные `version: integer = 1`, `harnesses: map`; `mcp/skills/agents/rules: map` по умолчанию пусты. Пустой `harnesses: {}` допустим. |
| Harness ID | Закрытый enum: `codex`, `claude`, `opencode`, `pi`, `omp`. |
| Harness | `enabled: bool = true`, `version?: string`, четыре локальные resource maps по умолчанию пусты. |
| Source | Tagged enum `type: git` с обязательным `url: string`, `ref?: string`; `type: local` с обязательным `path: string`. |
| MCP | Обязательные `source`, `transport: stdio`, `run`; необязательные `from: string`, `install`. |
| Install | `requires: string[] = []`, `steps: string[][] = []`. |
| Run | Обязательный `command: string`; `args: string[] = []`, `env: map<string,string> = {}`. |
| Skill | Обязательный `source`; необязательный `from: string`. |
| Agent | `format: portable` требует `description: string`, `source`, необязательный `from`; `format: native` — `source`, необязательный `from`. |
| Rule | `format: portable/native`, обязательный `source`, необязательный `from`. |

Maps обязаны быть mappings, списки — sequences; `null` не заменяет пустую
коллекцию или обязательное значение. Корневые resources содержат только
полные specs. Локальный entry — полный spec или **точный** `{enabled: false}`.
`enabled: true`, `enabled: false` с другими полями и корневой sentinel — ошибки.
Не использовать permissive untagged fallback, скрывающий неизвестные поля.
Portable agents/rules допустимы глобально и локально; native — только локально.
Все заданные specs проверяются, включая блоки отключённого harness.

## Семантическая проверка и наследование

Непустыми после trim должны быть resource names, CLI version, Git url/ref,
local path, from, portable description, runtime IDs и `run.command`.
Каждый argv step содержит хотя бы command; command не пустой, остальные args
могут быть пустыми строками. `run.env` имеет непустые имена переменных;
значения и аргументы сохраняются как строки без исполнения/раскрытия секретов.
CLI version и Git ref — непробельные непрозрачные pins; не требовать semver
или существующего Git SHA. Отсутствие pin означает future latest resolution.
Адреса example.invalid и прочие placeholders синтаксически принимаются.

`from` — относительный непустой путь без абсолютного префикса и компонентов
`..`. Это лексическая проверка, а не подтверждение containment: доступ к
источнику, существование файла/SKILL.md, canonical paths и symlink escape
проверяет этап 2. `source.local.path` может быть абсолютным или содержать `..`;
относительный путь разрешается от абсолютного каталога YAML без canonicalize
самого источника. Никакого сетевого запроса или чтения placeholder источников.

Для каждого включённого harness клонировать общие maps, затем применить
локальные entries по типу/имени: полная замена, добавление или удаление sentinel.
Неперечисленный/отключённый harness не попадает в effective model/вывод.
Типы независимы; одинаковые имена в разных типах разрешены.
Вывод показывает harness, pin/latest и имена эффективных ресурсов;
значения `run.env` и весь исходный YAML не печатаются.
Ошибки чтения/парсинга/валидации возвращают ненулевой код и контекст файла,
строки/колонки где доступны, либо semantic path вида `harnesses.pi.agents.x`.

## Реализация и зависимости

Engineer владеет `Cargo.toml`, `src/`, `tests/`, README, узким `.gitignore`
и обновлением текущего статуса design; чужие изменения сохраняет.

1. Создать Rust crate и production dependency
   `serde_yaml = { package = "yaml_serde", version = "=0.10.7" }`, Serde derive.
2. Разделить typed schema/unique maps, semantic validation, effective model
   и CLI orchestration; ни один слой не выполняет установку.
3. Добавить parser/core/CLI tests; README описывает команду и пределы этапа 1.
4. Пройти независимые Review/Validate, исправить блокирующие замечания.

YAML dependency и libyaml-rs отсутствуют в cache, shell DNS crates.io недоступен.
Не заменять parser самодельным YAML/Python. Cargo.lock сохранять только если
Cargo реально разрешил граф и создал файл; не составлять lock вручную.

## Приёмка и проверки

M1: fixtures для malformed YAML, unknown fields/IDs/enums, неверных типов,
missing required, duplicates, invalid argv, sentinel и native в корне.
CLI tests запускают binary с отдельным временным HOME и сравнивают его снимок
до/после valid/invalid init, неправильных аргументов и unavailable doctor.
M2: табличные tests inheritance, полной замены без field merge, добавления,
отключения только для одного harness, disabled harness и независимости типов.
Пример разбирается parser и даёт ожидаемые effective resources/relative paths.

Запустить rustfmt и Cargo checks/tests, зафиксировав реальные результаты.
При блокировке Cargo допустим отдельный `/tmp` Cargo harness с cached
Serde/serde_json, компилирующий **те же production core modules**: JSON tests
проверяют types/unknown/sentinel/duplicates, core tests — validation/inheritance.
Использовать временный CARGO_HOME; настройки и cache пользователя не изменять.
Это частичная проверка core, которая не доказывает YAML parsing или CLI.
Review/Validate явно перечисляют build/parser/CLI gaps; этап нельзя объявлять
полностью проверенным до успешного обычного build и parser/CLI tests.
