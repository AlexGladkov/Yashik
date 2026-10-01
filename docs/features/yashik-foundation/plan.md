# Plan: спецификация и дорожная карта Yashik

Статус: Plan завершён. Основание: [интервью](../../requirements.md) и
[Research](research.md). При расхождении старого scaffold приоритет имеет интервью.

## Результат текущего прогона

Execute обновляет `README.md`, `docs/design.md`, `examples/yashik.yaml`:
цельная продуктовая спецификация, черновик Compose-подобного YAML и roadmap.
Команды описывают будущий Rust CLI; установщик в этом прогоне не реализуется.
Требования интервью сохраняются; Review и Validate выполняются отдельно.
Приёмка документации не подтверждает установку на Ubuntu/macOS.

## Поведение и границы будущего MVP

Пользователь отдельно устанавливает Yashik, вручную пишет YAML, запускает
`yashik init ./yashik.yaml`; `yashik doctor` проверяет состояние без изменений.
Rust CLI поддерживает Linux/macOS; первая ручная проверка — Ubuntu.
Харнессы: Codex, Claude Code, OpenCode, Pi, OMP. Только пользовательское
глобальное окружение: CLI, MCP, skills, субагенты и rules.
Общие agents/rules имеют переносимый формат; локально допустим нативный.
Источники: публичный/приватный Git с уже настроенной авторизацией,
локальные файлы/каталоги и явные инструкции установки. Missing runtime
устанавливаются автоматически по поддержанным рецептам, с sudo при необходимости.
Авторизация моделей/сервисов остаётся за пользователем и не блокирует
независимую установку публичных компонентов.

Повторный init сверяет YAML, state и машину; доставляет изменения.
При конфликте спрашивает merge/replace и делает backup; замена управляемой
записи не стирает чужие поля общего конфига. Перед удалением ранее управляемого
ресурса спрашивает; отказ сохраняет ресурс и отметку в state. Ручная правка
управляемого файла — конфликт, а не разрешение перезаписать/удалить его.
Unsupported объясняет причину и спрашивает пропустить/отменить установку
данного ресурса. Ошибка блокирует зависимые операции, независимые продолжаются.
MCP после failed build не регистрируется; итоговый отчёт показывает частичный успех.
Первая версия интерактивная. Не добавлять новых подтверждений плана/установки.

Unattended, экспорт окружения, модели, providers, permissions и hooks отложены.
Marketplace/handles, URL манифеста, release-binary sources и встроенные каталоги
не являются согласованными требованиями MVP. Remote MCP требует отдельного контракта.

## Контракт черновика YAML

Корень: `version: 1`, `harnesses`, `mcp`, `skills`, `agents`, `rules`.
Ресурсы — словари по имени. Корневые ресурсы наследуют все включённые харнессы;
`common` и `targets` не нужны. IDs: `codex`, `claude`, `opencode`, `pi`, `omp`.
Перечисленный harness включён по умолчанию; `enabled: false` выключает его.
Неперечисленный harness не устанавливается. Корневой `version: 1` — версия схемы;
`harnesses.<id>.version` — версия coding CLI, при отсутствии latest.
Локальные `mcp/skills/agents/rules` заменяют общий
ресурс того же типа/имени целиком, без неявного field merge.
Локальный `{enabled: false}` исключает общий ресурс только для этого harness.

| Поле | Семантика примера |
| --- | --- |
| `source.type: git` | `url`, необязательный `ref`; источник не определяет сборку |
| `source.type: local` | `path`; relative path отсчитывается от каталога YAML |
| `from` | Путь ресурса внутри источника; без него используется выбранный объект/корень |
| `install.requires` | Список runtime/tool IDs, например `[node]`; рецепты определяются до runtime-этапа |
| `install.steps` | Непустые массивы argv, cwd источника, без неявного shell |
| `transport: stdio` | Единственный транспорт в текущем примере MCP |
| `run.command/args/env` | Запуск MCP; `${source}` — абсолютный корень источника, `${env:NAME}` — env reference |

Skills — каталоги с `SKILL.md`. Portable agent: `format: portable`, `description`,
Markdown-инструкции из `source/from`; имя из ключа. Portable rule:
`format: portable`, Markdown из `source/from`. Native agent/rule:
`format: native` в локальном блоке harness, `source/from` в нативном формате.
Адаптер сохраняет portable описание/инструкции; невозможность выразить ресурс
проходит unsupported ветку, без молчаливой потери полей.

Git без ref разрешает текущий default branch; SHA закрепляет коммит.
Tag/branch допустимы, но могут перемещаться. Latest разрешается каждый init;
точные выбранные версии/коммиты записываются для воспроизводимости прогона,
а lock не замораживает latest навсегда. Произвольные steps не дают гарантии
закрепления транзитивных зависимостей без соответствующих рецептов/lockfiles.
`from` и symlink остаются внутри канонического корня источника;
`source.path` может указывать за пределы каталога YAML.
Непроверенные адреса/команды примера помечаются placeholders.

## Архитектурные решения

Модули: typed manifest/inheritance; platform/runtime; git/local sources;
planner с DAG; argv executor; version-aware harness adapters; owned state,
backups и journal; интерактивный CLI. Использовать зрелый YAML parser,
не писать собственный из-за отсутствия offline crates.
Общий MCP с одинаковой эффективной спецификацией устанавливается один раз;
отличающееся локальное override не перезаписывает его установку.
State отдельно хранит результаты всех операций (успех, ошибка, пропуск,
блокировка) и managed state. В записи попытки сохраняется целевая спецификация
и resolved source revision, если они известны. Только после успешного изменения
managed state получает владельца, артефакт, установленную версию/revision,
fingerprint и привязки; одно совпадение имени не означает ownership.
Конфиги читать/править через форматные адаптеры, сохранять чужие данные,
атомарно заменять файлы после render и backup. Внешние установки не обещают
общую атомарность; journal фиксирует каждый исход, включая пропуск, блокировку
и сбой. Secrets не логировать.
Doctor проверяет binary/version, ресурсы, bindings, drift и прошлые ошибки;
наличие CLI не подтверждает auth или доступность внешней модели.
OpenCode требует привязки схемы к версии; OMP — учёта глобального профиля.
Pi subagents unsupported до выбора/проверки расширения. Матрица Research
не заменяет smoke test реально установленного CLI.

## Приёмка текущего результата

| ID | Критерий | Проверка |
| --- | --- | --- |
| D1 | README/design/example согласованы: будущие init/doctor и честная стадия проектирования | Review diff |
| D2 | Пять harnesses и только global scope | Сверка с интервью/Research |
| D3 | YAML демонстрирует четыре общих типа, полный local override и адресное отключение | PyYAML parse и ручное inheritance |
| D4 | Git/local, pin/latest, portable/native, explicit argv steps | Review примеров; placeholders названы |
| D5 | Reconciliation, backup, deletion, unsupported, independent failures описаны | Сверка поведения и M-критериев |
| D6 | Bootstrap отделён от init; roadmap имеет зависимости/выходы | Review; нет обещания готового бинарника |
| D7 | Отложенные решения и ограничения проверки названы | Нет неподтверждённых marketplace/remote/release требований |
| D8 | Blocking findings исправлены, проверки описаны честно | Независимые review.md и validation.md |

Для YAML доступен `/usr/bin/python3` с PyYAML 6.0.3; parser check не доказывает
работу будущего CLI. Отсутствие Cargo YAML crate относится к следующему этапу.

## Приёмка будущего рабочего MVP

| ID | Критерий и способ проверки |
| --- | --- |
| M1 | Invalid YAML/unknown fields/types не меняют машину; typed fixtures и HOME snapshot |
| M2 | Наследование/полный override/отключение; табличные effective-manifest tests |
| M3 | Git/local/relative path/containment; локальный Git, файл/каталог, symlink escape fixtures |
| M4 | Pin/latest/resolved state; управляемый Git с moving branch/tag и CLI resolver fixture |
| M5 | Shared MCP ставится один раз; failed build не регистрируется; installer calls и config snapshot |
| M6 | Второй init без дубликатов, drift и recovery после частичного сбоя; повторные запуски |
| M7 | Merge/replace/backup сохраняют чужие поля; scripted input и JSON/JSONC/TOML/Markdown fixtures |
| M8 | Removal только owned, с согласием/отказом и ручными правками; fixtures |
| M9 | Unsupported и failure блокируют только зависимости; Pi agent и падающая build ветвь |
| M10 | Doctor выявляет missing/drift/failed без изменений и ложного auth; HOME snapshot |
| M11 | Пять CLI и заявленные global capabilities; реальные versioned smoke tests |
| M12 | Yashik/runtime bootstrap на чистых Ubuntu и macOS; successful/denied sudo |

Тестировать в изолированном временном HOME. Mock не подтверждает настоящую
установку, Ubuntu не подтверждает macOS; для выпуска нужны обе платформы.

## Этапы и зависимости

| Этап | После | Результат и условие выхода |
| --- | --- | --- |
| 0. Текущий docs run | Интервью/Research | Execute → Review → Validate, D1–D8 |
| 1. Rust-каркас/schema | 0, проверенные parser crates | init/doctor интерфейс, M1–M2 |
| 2. Sources/state/planner | 1 | Git/local/lock/DAG, M3–M4 и базовый reconciliation |
| 3. Ubuntu vertical slice | 2, runtime/CLI recipes и adapter baseline | Один harness: CLI/stdio MCP/skill/rule/agent при capability, repeat/failure tests |
| 4. Пять адаптеров | 3, подтверждённые версии/форматы | Native overrides, version-aware config, M9/M11 |
| 5. Reconciliation/doctor | 2–4 | Merge/backup/removal/drift/journal/report, M5–M10 |
| 6. Bootstrap/macOS | 3–5, способ доставки Yashik | Чистые машины, runtime/sudo, M12 |
| 7. Выпуск MVP | 1–6 | Независимый Review и реальные Validate по всем обязательным M-критериям |

До этапа 3 определить рецепты runtime/CLI и latest policy; до 4 — baseline
версии OpenCode/Pi/OMP и merge/portable контракты; до 6 — публикацию Yashik
и платформенные источники/checksums. Remote MCP и marketplace требуют отдельного
решения. Эти вопросы не блокируют текущий docs Execute. Каждый этап будущей
реализации проходит профиль в нужном объёме; сроки оценить после вертикали.
