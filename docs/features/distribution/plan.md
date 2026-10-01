# Plan: установка и публикация Yashik 0.2.1

Основание: [bootstrap research](research-bootstrap.md),
[integrations research](research-integrations.md) и
[профиль business-feature](../../../.agents/skills/business-feature/SKILL.md).
Установка через `sh` и публикация Homebrew в принадлежащий пользователю
`AlexGladkov/homebrew-tap` уже разрешены. Дополнительное подтверждение плана
не требуется. Plan: Strategist, `gpt-6.1-sol`, `xhigh`.

## Результат и границы

Первый распространяемый релиз — `v0.2.1`. Пользователь устанавливает готовый
Yashik одной командой, без Rust и прав администратора:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/AlexGladkov/Yashik/releases/latest/download/install.sh | sh
```

Закреплённая установка:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/AlexGladkov/Yashik/releases/download/v0.2.1/install.sh | \
  sh -s -- --version 0.2.1
```

Homebrew устанавливает те же архивы:

```sh
brew install AlexGladkov/tap/yashik
```

Релиз включает Linux x86_64/aarch64 и macOS Intel/Apple Silicon. Существующий
`yashik init` продолжает устанавливать выбранные CLI, runtimes и ресурсы;
bootstrap-скрипт устанавливает только исполняемый файл Yashik. Автоматическая
установка Homebrew, изменения shell profiles и подписывание артефактов в эту
работу не входят.

Пользователь выбрал WinGet + WSL. Отдельная стадия выпускает
Windows portable bridge, который запускает Linux Yashik в уже установленном
WSL-дистрибутиве. Её контракт описан ниже на основании supplement research;
она не блокирует самостоятельные SH/Homebrew работы. Продуктовый Rust
installer не портируется на Windows. Root сохраняет решение в run record.

## Решения, заменяющие варианты Research

| Платформа | Native GitHub runner | Rust target | Имя архива |
| --- | --- | --- | --- |
| Linux x86_64 | `ubuntu-24.04` | `x86_64-unknown-linux-musl` | `yashik-linux-x86_64.tar.gz` |
| Linux aarch64 | `ubuntu-24.04-arm` | `aarch64-unknown-linux-musl` | `yashik-linux-aarch64.tar.gz` |
| macOS Intel | `macos-15-intel` | `x86_64-apple-darwin` | `yashik-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `macos-15` | `aarch64-apple-darwin` | `yashik-macos-aarch64.tar.gz` |

Имена архивов постоянны; версия находится в release URL. Linux использует
`musl-tools` и соответствующий Rust target на native архитектуре, со статической
линковкой. Это устраняет зависимость бинарника от glibc runner. Workflow
проверяет фактический статический результат, а не только название target.
Darwin собирается с `MACOSX_DEPLOYMENT_TARGET=13.0`; запуск на macOS 15
подтверждает этот runner, но не заменяет отдельную проверку macOS 13.

Каждый tar.gz содержит ровно один обычный executable `yashik` в корне.
Публикуются четыре Unix-архива, два Windows ZIP, `SHA256SUMS` и `install.sh`.
`SHA256SUMS` содержит SHA-256 шести окончательных архивов и релизного скрипта,
отсортированные по
имени; generated formula — результат сборки/CI для root, не дополнительный
пользовательский release asset.

`scripts/install.sh` содержит `DEFAULT_VERSION=0.2.1`. Скрипт, полученный через
`latest/download/install.sh`, скачивает остальные файлы только из
`releases/download/v0.2.1/`. Так две загрузки не перескакивают между релизами;
API GitHub и JSON parser клиенту не нужны. `--version` переопределяет эту
версию после проверки формата. Tag, Cargo version и DEFAULT_VERSION должны
совпадать до создания release.

Tag-triggered workflow создаёт **draft** GitHub Release после проверок. Root
завершает внешнюю приёмку и публикует draft по имеющемуся разрешению. После
публикации root обновляет только `yashik.rb` и относящийся к нему README в
существующем tap; другие формулы не меняются. Формула содержит реальные
контрольные суммы ровно опубликованных архивов.

## Контракт POSIX installer

- Интерфейс: `--version VERSION`, `--bin-dir ABSOLUTE_DIR`, `--force`, `--help`.
  VERSION принимает `MAJOR.MINOR.PATCH` и `vMAJOR.MINOR.PATCH`, нормализует их
  в точный tag и Cargo version; pre-release, shell characters, неизвестные
  аргументы и относительные пути отклоняются. По умолчанию — `$HOME/.local/bin`.
- `uname` выбирает только Linux/Darwin и x86_64/amd64/aarch64/arm64. Остальные
  сочетания завершаются с ошибкой до записи destination. Используется только
  `curl` с HTTPS-only transfer и redirects; отсутствие downloader, SHA-256 tool или
  archive tool даёт понятную ошибку. Поддержать `sha256sum` и `shasum -a 256`.
- Скачать закреплённый архив и `SHA256SUMS` в приватный temporary directory.
  Выбрать единственную строку с точным именем ожидаемого архива и валидным
  SHA-256; отсутствующая/дублирующаяся строка и несовпадение hash — ошибка.
  Checksums контролируют целостность; они не являются отдельной подписью.
- До extraction проверить, что archive содержит только один обычный файл
  `yashik`, без вложенных путей, ссылок, устройств и дополнительных entries.
  Извлечь только этот member в приватный staging file внутри destination,
  установить `0755` и выполнить `--version`. Требуется точный ответ
  `yashik VERSION`; проба проходит до изменения установленного executable.
- Byte-identical executable успешно завершает повторный запуск без
  изменения бинарника. Byte-identical файл без executable permissions
  атомарно заменяется проверенным staging file с mode 0755 без `--force`;
  другой payload не допускается в этой repair-ветке.
  Любой другой обычный файл сохраняется без `--force`;
  `--force` разрешает заменить его только после всех проверок. Не запускать
  существующий неизвестный файл для определения его происхождения. Symlink,
  directory, device и другие non-regular targets не заменяются даже с force.
- Использовать same-directory atomic rename; при любой ошибке прежние байты
  сохраняются, temporary files очищаются через trap. Не запускать `sudo` и
  не менять профили shell. Вывести установленную версию, путь и корректно
  процитированную PATH-команду, если bin dir отсутствует в текущем PATH.

Документация содержит удобную и закреплённую команды, download/verify/run
пример для скрипта с `SHA256SUMS`, Homebrew-команду, архитектуры, directory
override и source fallback. Переменная `DEFAULT_VERSION` обновляется вместе с
Cargo version при каждом следующем релизе.

Review amendment (2026-10-01): root согласовал curl-only bootstrap после
воспроизведённого HTTPS→HTTP redirect через GNU Wget `--https-only`.
Пользователь не требовал Wget; небезопасный fallback удаляется. Также
согласован atomic permission repair только byte-identical payload с 0644;
корректный identical executable сохраняет inode/mtime. Это заменяет ранние
варианты downloader/idempotence и не требует нового подтверждения.

## Workflow, формула и portability

1. Trigger — `push` tags `v*`; job проверяет строгий release version и совпадение
   tag/Cargo/script. `workflow_dispatch` запускает те же builds/tests для
   pre-tag проверки всех native платформ, без создания release.
   Repository permissions по умолчанию — `contents: read`;
   только draft-release job получает `contents: write`.
2. Четыре native jobs выполняют `cargo test --locked` и
   `cargo build --release --locked --target TARGET`. Linux job устанавливает
   musl toolchain; оба Darwin jobs выполняют тесты без Rosetta. Отдельный
   обычный Linux check запускает `cargo fmt --check` и strict Clippy.
3. Поправить только test fixtures, которые путают macOS layout с XDG layout.
   В `tests/installer_engine.rs` helper формирует platform-aware Paths,
   согласованные с child HOME/XDG environment; временный root канонизируется
   для `/tmp` → `/private/tmp`. Не менять global environment параллельных
   tests. Проверить launch specs, state, doctor и real child CLI paths.
4. Упаковать executable, проверить форму архива и запуск извлечённого файла
   (`--version`, `--help`, `check` с небольшим manifest). Каждый native job
   проверяет installer на своём архиве через локальный fixture transport и
   получает SHA-256 фактически упакованного файла.
5. Генератор Homebrew принимает version и checksum manifest, требует все
   четыре фиксированных архива и порождает `on_macos/on_linux` ×
   `on_arm/on_intel`, MIT, homepage, `bin.install "yashik"`, test версии.
   Production URL всегда закреплён на GitHub tag. Ошибочные/недостающие hashes
   не должны порождать формулу с placeholders.
6. Каждый native job устанавливает generated formula в локальный временный
   tap из своего **того же** упакованного архива, затем запускает `brew test`
   и version/help probes. Local fixture URL/base override используется только
   тестом. Он не попадает в production formula. Выполнить подходящий
   `brew audit --strict`; различать ограничения локального fixture URL и
   ошибки production metadata. Не загружать недоступные draft assets ради CI.
7. Final job скачивает четыре Unix и два Windows artifacts, формирует `SHA256SUMS` из
   окончательных файлов и release install.sh, повторяет checksum/version
   consistency checks и сохраняет production `yashik.rb` как CI artifact.
   Создать или обновить draft с восемью release assets. Не перезаписывать
   опубликованный релиз; tag retry после публикации завершается безопасно.

## Windows bridge и WinGet

Отдельный `tools/windows-launcher/` — std-only Rust crate версии `0.2.1`,
не зависящий от Unix crate. Executable называется `yashik.exe`; `--version`
локально выводит `yashik 0.2.1`, `--help` локально объясняет WSL и команды.
Оба работают без WSL и без network. WinGet install только доставляет bridge;
он не запускает Linux setup как side effect.

- Команды: `setup [--force]`, `init MANIFEST`, `check MANIFEST`, `doctor`;
  общий `--distro NAME` выбирает зарегистрированный distro. Без флага
  используется configured WSL default, без persistent selection file.
  NAME не пустой, не начинается с `-` и не содержит control characters;
  пробелы допустимы и передаются одним argument.
  Неизвестные flags и лишние operands — понятная ошибка. Проверить `wsl.exe`
  и наличие установленного/инициализированного distro. Нет WSL или distro —
  вывести ручные install/initialise/setup инструкции и завершиться non-zero.
  Не вызывать WSL install, DISM, Windows features, elevation или reboot.
- `setup` внутри выбранного Linux скачивает только tag-pinned `install.sh`
  и `SHA256SUMS` версии bridge, проверяет единственную точную строку checksum
  скрипта, затем выполняет `sh install.sh --version 0.2.1`. Отказ/ошибка hash
  не запускают скрипт. Temporary files очищаются. `--force` передаётся только
  при явно указанном пользователем флаге; без него действует сохранение
  любого другого установленного файла. Не использовать moving latest.
- `init/check/doctor` требуют установленный Linux Yashik той же версии;
  missing/wrong version выводит `setup` или `setup --force`, без автоматической
  установки. Linux исполняемый файл выбирается из `$HOME/.local/bin` внутри
  distro, а не из произвольной Windows PATH.
- Для `init/check` уже absolute Linux path (`/home/...`) передаётся напрямую;
  Windows relative path превращается в absolute Windows path. После Windows
  canonicalization удалить drive-path verbatim prefix `\\?\` перед
  отдельным argv-вызовом `wslpath -a -u PATH` внутри того же distro.
  Conversion failure блокирует forwarding. Пробелы, Unicode
  и shell characters остаются данными. User args передаются через process argv;
  bootstrap shell имеет фиксированный текст, без интерполяции distro/path.
  Если нужен fixed shell wrapper для HOME, все operands передаются positional
  arguments и используются через quoted `"$@"`. Вернуть Linux exit status;
  stdin/stdout/stderr forwarded commands сохраняют интерактивный `init`.

| Windows artifact | Target | Build/check |
| --- | --- | --- |
| `yashik-windows-x86_64.zip` | `x86_64-pc-windows-msvc` | native build, tests, mock WSL, extracted version/help на `windows-2022` |
| `yashik-windows-aarch64.zip` | `aarch64-pc-windows-msvc` | cross-build std-only crate средствами Windows MSVC ARM64 toolchain на `windows-2022`; PE architecture/ZIP check, без заявления native запуска |

ZIP содержит только regular root `yashik.exe`; SHA-256 вычисляется после
окончательного packaging и подтверждается скачиванием опубликованного asset.
Версия отдельного crate сверяется с tag и root Cargo version. Mock WSL tests
проверяют default/explicit distro, missing prerequisites, pinned setup/hash
failure, force forwarding, version mismatch, path conversion/quoting и exit
propagation. ARM cross-build не считается ARM64 runtime validation.

На hosted Windows runner попробовать настоящий WSL1 smoke: импортировать
официальный Ubuntu rootfs в изолированный временный distro, если host это
поддерживает; записать источник/hash, setup, converted-path check, init,
doctor/rerun и cleanup. Если host не позволяет импорт/запуск, сохранить
конкретную причину и real-WSL gate как непроверенный. Не называть такой runner
проверкой WSL2; отдельного доступного WSL2 host пока нет.

Генератор создаёт три YAML файла для `AlexGladkov.Yashik` версии `0.2.1` в
`manifests/a/AlexGladkov/Yashik/0.2.1/`: version, installer и `en-US` default
locale. Installer: `InstallerType: zip`, x64/arm64 entries с
`NestedInstallerType: portable`, `RelativeFilePath: yashik.exe`,
`PortableCommandAlias: yashik`, точными tag URL и фактическими SHA-256 двух ZIP.
Нет silent switches или WSL feature dependencies. Description явно говорит,
что Yashik управляет Linux окружением внутри уже установленного WSL.

Root после публикации скачивает публичные ZIP, сверяет hashes и выполняет
`winget validate` с локальными manifests; при доступном WinGet client проверяет
portable install/version/help/uninstall без запуска setup. После успешной
проверки подаёт community PR в `microsoft/winget-pkgs` по разрешению пользователя.
Ссылка/статус PR фиксируются. До внешнего merge не утверждать, что
`winget install --id AlexGladkov.Yashik --exact` уже работает из community source.

## Владение и порядок Execute

| Владелец | Файлы/ответственность |
| --- | --- |
| Engineer 1 — Luna/max | `scripts/install.sh`, отдельный installer test harness и его fixtures; POSIX options, platform mapping, download/checksum/archive validation, atomic install, error cases |
| Engineer 2 — Luna/max | `.github/workflows/release.yml`, release/formula generator и его проверки, `tests/installer_engine.rs` portability; четыре native jobs, packaging, brew CI, draft assets |
| Windows Engineer — Luna/max | `tools/windows-launcher/**`, Windows ZIP packager, WinGet manifest generator и их проверки; bridge, std-only builds, mock WSL и artifact/schema checks; `.github/workflows/release.yml` остаётся у Engineer 2 |
| Root | `Cargo.toml`/`Cargo.lock` version 0.2.1, README, distribution docs/run record, интеграция, commit/tag/push, наблюдение CI, Ubuntu acceptance, публикация draft, scoped tap commit/push, WinGet community PR |

Engineer работают одновременно и сохраняют чужие изменения. Общий
контракт выше фиксирует asset names и flags; helper scripts не должны
редактировать файл другого владельца. Если нужен общий helper, сначала
согласовать владение с root. Execute модель — `gpt-6-luna`, reasoning `max`;
отдельный Review — Terra/max по профилю; Validate — Luna/max.

Последовательность: реализация → интеграционные локальные проверки →
независимый Review и исправления → tag/push и четыре native CI jobs → draft
asset download и внешняя приёмка → публикация release → tap update и
публичные install probes. Создание draft не заменяет опубликованный URL в
приёмке: до публикации root может скачивать draft assets через авторизованный
GitHub API, после публикации обязательна повторная публичная загрузка.

## Приёмка и доказательства

| ID | Критерий и проверка |
| --- | --- |
| D1 release | Tag/root Cargo/Windows crate/script — 0.2.1; успешны fmt, tests, Clippy; четыре native Unix build/test jobs; Linux binary статический; Darwin обе архитектуры запускались native. Windows x64 tests/build и ARM64 cross-build успешны. Draft/final содержит четыре tar.gz, два ZIP, SHA256SUMS и install.sh. |
| D2 archive | Каждый архив имеет только regular root executable yashik. Hash matches manifest; извлечённый файл отвечает `yashik 0.2.1`, проходит `--help` и `check`. |
| D3 sh success | POSIX sh с curl и HTTPS-only redirects устанавливает в default/custom absolute bin dir; без curl завершается с понятной ошибкой. Повторный запуск сохраняет executable binary/inode/mtime; identical 0644 file атомарно чинится до 0755 без force. PATH hint корректен, sudo/profile edits отсутствуют. Native CI проверяет Linux x64/arm64 и macOS Intel/arm64. |
| D4 preservation | Foreign regular file отказан без force и заменён с force после проверки. Unsupported OS/arch, malformed options/version, duplicate/missing/bad checksum, invalid/link/traversal archive и wrong binary version сохраняют прежний binary и outside sentinel. Symlink/non-regular destination отказан. |
| D5 brew | Generated production formula имеет четыре tag-pinned URL и точные hashes окончательных assets. На каждом native CI platform local-tap install/test/probes используют тот же release archive. После публикации публичный `brew install AlexGladkov/tap/yashik` проходит как минимум на доступной реальной Linux машине; audit/test results и остальные публичные platform limits записаны отдельно. |
| D6 Ubuntu | Root проверяет чистого non-root пользователя на разрешённом Ubuntu host `<authorized Ubuntu host>`: публичный installer скачивает v0.2.1, --version/--help, повторный install, foreign/force и corrupted fixture сохраняют ожидаемые файлы. С release binary повторить examples/codex-voltagent.yaml: check, init, codex --version, doctor, повторный init. |
| D7 publication | Root публикует прошедший проверки draft, проверяет публичные pinned/latest script URLs, checksum и version, затем обновляет только yashik.rb/README в owned tap. Installation docs используют реально доступные URL. |
| D8 bridge | Windows x64 executable/version/help и mock WSL tests проходят; ARM64 ZIP/PE architecture подтверждены cross-build. setup проверяет pinned script checksum; forwarding имеет version gate, safe path conversion и exit/stdin propagation. Real WSL1 attempt/результат отдельно от mocks; WSL2 и ARM64 runtime без evidence остаются unverified. |
| D9 WinGet | Два ZIP опубликованы; downloaded hashes совпадают с 3-file AlexGladkov.Yashik manifests; winget validate проходит. Доступный Windows client проверяет portable install/probes/uninstall. Community PR подан после validation; до merge package availability остаётся pending. |

`review.md` содержит severity, место и основание независимых замечаний;
`validation.md` — команды, platform/architecture, URLs, hashes, результаты
CI и внешней приёмки, а также непроверенные условия. Linux fixture не
подтверждает macOS; native macOS CI не объявляется проверкой macOS 13.
Checksum test с mock не подменяет скачивание опубликованного релиза.

SH/Homebrew завершены после D1–D7 и закрытия блокирующего Review. Windows
доставка оценивается отдельно по D8–D9; непроверенные real WSL/ARM64 runtime
conditions и внешнее ожидание community PR не превращаются в успешную проверку.
При сбое CI или внешней приёмки draft не публиковать, tap не обновлять;
исправить затронутый код и повторить соответствующие Review/Validate.
