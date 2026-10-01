# Yashik

[Yashik](https://github.com/AlexGladkov/Yashik) — Rust CLI для установки
личного окружения coding-агентов из YAML. Лицензия: [MIT](LICENSE).

`init` устанавливает CLI и поддерживаемые ресурсы, `check` проверяет YAML без
изменения машины, `doctor` проверяет записанное окружение без переустановки.
Пользователь самостоятельно входит в Codex, Claude и другие сервисы.

## Установка

Linux и macOS, x86_64 и ARM64:

```sh
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
  https://github.com/AlexGladkov/Yashik/releases/latest/download/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

Скрипт устанавливает только Ящик в `~/.local/bin`, проверяет SHA-256 архива и
не изменяет shell-профиль. Нужны `curl`, `tar` и `sha256sum` либо `shasum`; Rust не нужен. Для другой папки используйте
`sh -s -- --bin-dir /absolute/path`. Повторная установка идентичного бинарника
не меняет его; для замены другой версии нужен `--force`.

Homebrew:

```sh
brew install AlexGladkov/tap/yashik
```

Закреплённая версия:

```sh
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
  https://github.com/AlexGladkov/Yashik/releases/download/v0.2.1/install.sh | \
  sh -s -- --version 0.2.1
```

Чтобы сначала скачать и проверить сам скрипт, выполните в пустой папке:

```sh
release=https://github.com/AlexGladkov/Yashik/releases/download/v0.2.1
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSLO "$release/install.sh"
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSLO "$release/SHA256SUMS"
awk '$2 == "install.sh" {print}' SHA256SUMS > install.sha256
# Linux:
sha256sum -c install.sha256 && sh install.sh
# macOS: shasum -a 256 -c install.sha256 && sh install.sh
```

Скрипт каждого релиза содержит собственную версию по умолчанию: архив и
контрольные суммы скачиваются из одного закреплённого релиза. Архивы для Linux
собираются со статической musl, для macOS — с deployment target 13.0.
Результаты реальных проверок платформ описаны в
[distribution validation](docs/features/distribution/validation.md).

## Windows: WinGet и WSL

Windows-пакет содержит `yashik.exe`, который запускает Linux-версию в WSL.
Нужен уже установленный и инициализированный WSL-дистрибутив. Установка пакета
не включает Windows features и не создаёт дистрибутив.

После появления пакета в каталоге WinGet:

```powershell
winget install --id AlexGladkov.Yashik --exact
# В новой оболочке:
yashik --distro Ubuntu setup
yashik --distro Ubuntu check .\yashik.yaml
yashik --distro Ubuntu init .\yashik.yaml
yashik --distro Ubuntu doctor
```

Без `--distro` используется дистрибутив WSL по умолчанию. `setup` устанавливает
Linux Ящик из закреплённого релиза; для замены другой версии нужен
`setup --force`. `init` и `check` принимают Windows-пути, включая относительные,
или абсолютные Linux-пути. Все CLI, MCP, агенты, skills и rules устанавливаются
внутри выбранного дистрибутива, где пользователь входит в сервисы.

Для ручной установки Windows CLI скачайте ZIP своей архитектуры из
[релиза](https://github.com/AlexGladkov/Yashik/releases/tag/v0.2.1), извлеките
`yashik.exe` в отдельную папку и добавьте её в пользовательский PATH.
Статус проверки WinGet и WSL указан в
[distribution validation](docs/features/distribution/validation.md).

## Сборка из исходников

Нужны Rust 1.88 или новее и доступ к crates.io. На Ubuntu также нужен linker,
например из `build-essential`:

```sh
cargo build --release --locked
mkdir -p ~/.local/bin
install -m 755 target/release/yashik ~/.local/bin/yashik
export PATH="$HOME/.local/bin:$PATH"
```

## Простой тест: Codex и агенты VoltAgent

[examples/codex-voltagent.yaml](examples/codex-voltagent.yaml) устанавливает
Codex и два нативных субагента — `backend-developer` и `reviewer` — из
[VoltAgent/awesome-codex-subagents](https://github.com/VoltAgent/awesome-codex-subagents).
Версия CLI и ревизия Git-источника закреплены:

```sh
yashik check examples/codex-voltagent.yaml
yashik init examples/codex-voltagent.yaml
yashik doctor
```

После установки TOML-файлы лежат в `~/.codex/agents/`. Повторный `init`
проверяет текущее состояние и доставляет изменения. Вход в Codex и реальный
запрос к модели выполняются отдельно; наличие CLI не означает авторизацию.

## YAML

В `harnesses` перечисляются нужные клиенты: `codex`, `claude`, `opencode`,
`pi`, `omp`. Общие `mcp`, `skills`, `agents`, `rules` применяются ко всем
включённым клиентам. Внутри клиента одноимённый ресурс заменяется целиком,
а `{enabled: false}` отключает только эту привязку.

Источник — Git-репозиторий с необязательным `ref` или локальный файл/каталог.
Относительный `source.path` считается от YAML. `from` выбирает файл/каталог
внутри источника; выход через `..` и внешние symlinks не допускается.
Без версии CLI выбирается latest; без Git ref разрешается текущий HEAD.

MCP задаёт stdio `run.command`, `run.args`, `run.env` и необязательные
`install.requires`/`install.steps`. Шаги — массивы argv без неявного shell,
выполняются из постоянной копии источника. `${source}` в `run` указывает на
эту копию; `${env:NAME}` раскрывается при запуске MCP. Для env используйте
ссылки на переменные окружения, а не токены в YAML. В этой версии
`install.requires` поддерживает только `node` и `bun`; другие runtime нужно
установить самостоятельно.

[examples/all-five.yaml](examples/all-five.yaml) — расширенный тест с пятью
CLI, небольшим stdio MCP echo, скиллом, portable-агентом и правилом.
Исходники этих ресурсов лежат рядом в `examples/resources/`. Этот пример
проверяет локальную установку и протокол MCP; он не проверяет доступ к моделям.
[examples/yashik.yaml](examples/yashik.yaml) остаётся иллюстрацией схемы с
placeholder-адресами и не предназначен для реальной установки.

## Конфликты и ограничения

При конфликте с существующим ресурсом или ручном изменении Ящик спрашивает,
объединить, заменить или пропустить его. Нативные файлы нельзя безопасно
объединить автоматически; для них доступны замена с backup или пропуск.
Удаление ранее установленного ресурса требует ответа пользователя.
Ошибки независимых операций показываются в итоговом отчёте.

Поддержка ресурсов зависит от клиента. Pi core не предоставляет формат
сохранённых субагентов. Нативные rules Codex, OpenCode, Pi и OMP не считаются
portable Markdown: неподдерживаемая возможность явно пропускается или
отменяется по выбору пользователя. Адаптеры ресурсов проверены и включены
для Codex 0.159.3, Claude 2.1.286, OpenCode 1.18.34, Pi 0.99.2 и OMP 18.4.8.
Другие версии CLI можно установить, но их ресурсы потребуют отдельной проверки
адаптера и будут предложены к пропуску.

Целевые платформы — Linux и macOS. Реальные результаты тестов и платформенные
ограничения записываются в [validation](docs/features/installer/validation.md).
Тест на Ubuntu не подтверждает macOS. Marketplace, экспорт существующего
окружения и проектная установка остаются за пределами этой версии.

Подробнее: [требования](docs/requirements.md), [план установщика](docs/features/installer/plan.md),
[исследование форматов клиентов](docs/features/installer/research-adapters.md).
