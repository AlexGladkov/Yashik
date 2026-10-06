# Архитектура `tools.orca`

Исследование расширения независимых инструментов Yashik под Orca. Продуктовый
код не менялся. Внешние сведения сверены с официальной документацией Orca и
официальными release endpoints на 2026-10-06.

## Вывод

Для первой версии рекомендуется Linux x86_64, установка Orca и доступность
`orca-ide`, без запуска `serve` и без регистрации системного сервиса. Это
соответствует модели `tools.herdr`: `init` управляет версией бинарного
инструмента, но не запускает его длительную работу. На целевом Ubuntu 24.04
сервере есть 3,6 ГБ свободного места, но нет `Xvfb`, `orca-ide` и `unsquashfs`.
Встроенный AppImage extractor не требует FUSE или отдельного `unsquashfs`.

Расширение на macOS разумно вынести в следующую платформенную итерацию. Orca
официально распространяется для macOS, включая подписанный и нотариально
заверенный пакет и Homebrew cask, но проверенная headless-инструкция, путь к
встроенному CLI и процесс извлечения здесь относятся к Linux. Установку для
macOS нельзя считать подтверждённой тем же Linux acceptance.

Решение по постоянному серверу: по умолчанию только установка. `serve` создаёт
сетевой listener и pairing URL с учётными данными; service lifecycle, адрес
объявления, доступ к сети, пользователь systemd и сохранность терминальных
процессов требуют отдельной конфигурации и отдельного согласования. Установка
не должна молча запускать Orca, GUI или сервер.

## Точки расширения Yashik

- В [schema.rs](../../../src/schema.rs) `Manifest.tools` по умолчанию пустой,
  а `Tools` — закрытая структура с `deny_unknown_fields`; сейчас в ней только
  `herdr`. Десериализатор Herdr явно проверяет карту, дубликаты и неизвестные
  поля, отклоняет `null` и применяет `enabled: true`, `version: latest`.
  Добавление `orca` должно сохранить это поведение и совместимость манифестов,
  где `tools` отсутствует.
- В [effective.rs](../../../src/effective.rs) `EffectiveManifest` имеет
  отдельное поле для каждого инструмента; включённая запись преобразуется в
  effective-конфигурацию. Семантическая проверка версий живёт в
  [validation.rs](../../../src/validation.rs), а `check` печатает эффективные
  инструменты в [main.rs](../../../src/main.rs).
- [engine.rs](../../../src/install/engine.rs) запускает `tools::reconcile`
  до обработки harness CLI. Поэтому Orca может устанавливаться при
  `harnesses: {}` и не нуждается в harness.
- [tools.rs](../../../src/install/tools.rs) содержит полный рецепт Herdr,
  несмотря на общее имя модуля: URL metadata и assets, схема release JSON,
  имя бинарника, пути, platform mapping, проверка записей, операции удаления
  и сообщения зашиты под Herdr. Нужен отдельный рецепт Orca либо общий слой
  только для повторно используемых проверенных операций с явной моделью
  разных типов пакетов; механическая замена строк Herdr на Orca здесь опасна.

## Пакет Orca и проверка CLI

Официальная инструкция для headless Linux говорит, что FUSE необязателен:
AppImage поддерживает `--appimage-extract`. При извлечении создаётся
`squashfs-root` с режимом `0700`; для запуска под отдельным пользователем
дерево надо сделать читаемым и проходимым. При пользовательской установке в
каталогах `Paths.data` достаточно оставить дерево доступным тому же
пользователю Yashik.

Команда Linux CLI называется `orca-ide`, чтобы не конфликтовать с GNOME
screen reader `/usr/bin/orca`. В извлечённом bundle проверенный entrypoint —
`squashfs-root/resources/bin/orca-ide`; `AppRun` запускает приложение, а
`orca-linux.AppImage --version` может показать версию Electron. Для проверки
версии после распаковки следует вызывать именно bundled `orca-ide --version`.
Команда не должна запускать `serve` или GUI и не должна обращаться к данным
пользователя Orca.

Официальный `latest-linux.yml` содержит версию, размер, имя AppImage и SHA-512.
На 2026-10-06 latest metadata указывала версию `1.4.221`, AppImage размером
219,617,397 байт. GitHub release API также публиковал SHA-256 asset digest для
этого официального файла. Для exact pin release metadata доступна по
versioned release URL. Резолвер должен ограничивать URL официальным upstream,
проверять ожидаемую версию и точный asset, а размер и время загрузки ограничить.
SHA-512 потребует расширить digest-представление; альтернативно можно
использовать проверяемый SHA-256 digest официального GitHub release asset API.

Установщик должен сначала проверить архив по официальному digest и только
потом передать AppImage команду извлечения. Загруженный архив и распакованное
дерево — оба исполняемый payload: до проверки архива нельзя запускать extractor.
До реализации на целевом сервере остаётся проверить, какие системные
библиотеки нужны именно для bundled `orca-ide --version`; официальный список
библиотек и Xvfb относится к запуску Electron `serve`, поэтому его нельзя
автоматически переносить на install-only режим.

## Владение извлечённым деревом

Обычная модель `InstalledTool` в [api.rs](../../../src/install/api.rs)
представляет один бинарный файл и SHA-256. Orca хранит versioned AppImage,
распакованный bundle со множеством файлов и исполняемый launcher. Для этого
нужна запись, которая точно хранит и проверяет:

- ID, pinned/resolved version, ожидаемый официальный metadata/asset URL,
  алгоритм и значение digest исходного AppImage;
- строго ожидаемый version directory под
  `Paths.data/tools/orca/versions/<version>`;
- детерминированный fingerprint распакованного дерева;
- путь и fingerprint активного launcher `Paths.bin/orca-ide`;
- предыдущие версии только если выбранная политика оставляет их для отката.

Существующий [util::hash_tree](../../../src/install/util.rs) включает в
fingerprint пути, типы объектов, режимы, содержимое файлов и байты относительных
symlink targets. Он разрешает только относительные symlinks, которые
канонизируются внутрь корня; dangling, cyclic, absolute или внешние ссылки и
special files отклоняются. Это полезный базис для Orca: его bundle содержит
внутренние symlinks, и без проверки нельзя запрещать все ссылки или допускать
выход за корень. Однако `hash_tree` удерживает содержимое всех файлов в памяти
до завершения хеша. Для потенциально большого Electron bundle следует либо
добавить потоковый вариант, либо задать и контролировать лимиты на число
файлов, глубину, размер одного файла и суммарный распакованный размер.

После извлечения следует проверить типы, пути и ссылки, наличие ожидаемого CLI,
дерево целиком fingerprint-ировать и только затем активировать стабильный
`Paths.bin/orca-ide` launcher. При повторной установке, doctor и удалении
сверять fingerprint дерева и launcher; при drift оставить запись и файлы,
зафиксировав failure или `PendingRemoval`. Удалять только дерево с точным
ожидаемым version path после успешной проверки ownership. Утилита
`remove_tree_checked` удаляет каталог без обхода symlink-целей, но fingerprint
и точную границу пути должен проверить вызывающий код до удаления. Проверка
должна сохранить корректные внутренние symlinks, не следуя за ними при обходе
и удалении.

Версионированный AppImage плюс распакованное дерево требует места для обоих
файлов на время распаковки, новой и предыдущей версий при обновлении, а также
временной staging-копии. Текущие 3,6 ГБ свободного места выглядят достаточными
для первого x86_64 install по размеру сжатого файла, но это не подтверждает
расширенный размер, повторный upgrade или удержание нескольких версий.
Нужны ограничение скачивания, безопасная временная директория, preflight
свободного места/контролируемый отказ и cleanup только staging-объектов,
принадлежащих текущей операции. Не удалять старую активную версию до успешной
проверки новой и commit записи состояния.

## State, doctor и обратная совместимость

[state.rs](../../../src/install/state.rs) хранит
`tools: BTreeMap<String, InstalledTool>` с `#[serde(default)]`; state без
поля `tools` уже поддержан тестом
`state_without_tools_field_remains_backward_compatible` в
[installer_state.rs](../../../tests/installer_state.rs). Не менять смысл
существующих Herdr-записей. Добавление нового вида bundle требует либо
отдельной backward-compatible записи/слота для Orca, либо явного расширения
формата без потери чтения прежнего JSON; исходный Herdr JSON должен продолжать
десериализоваться и проходить прежнюю проверку.

[doctor.rs](../../../src/install/doctor.rs) сейчас передаёт каждый элемент
`State.tools` в Herdr-only `tools::doctor` и печатает для всех успешных записей
сообщение Herdr. Если Orca окажется в той же карте, doctor обязан dispatch-ить
по ID; неизвестный ID или несовпадающий state key должен fail closed. Doctor
должен оставаться локальным, read-only и не запускать `serve`.

Повторно применимые тестовые seams находятся в `tests/yaml_schema.rs`,
`tests/cli.rs`, `tests/installer_state.rs` и модульных тестах в
`src/install/tools.rs`. Acceptance для Orca должна покрыть:

- совместную и раздельную конфигурацию Herdr/Orca, defaults, pin, disabled,
  unknown/duplicate/null/wrong-type rejection и манифест без `tools`;
- `check` Orca-only с `harnesses: {}` без создания state; эффективное
  представление включённого/отключённого инструмента;
- checksum до запуска extractor, границы размера/пути/времени, правильный
  bundled `orca-ide --version`, install и повторный запуск без изменения
  активного payload;
- отсутствие ownership при ошибке, сохранение unmanaged collision, drift при
  doctor и `PendingRemoval` без подтверждения; подтверждённое удаление только
  fingerprint-проверенных файлов/дерева с сохранением пользовательских данных;
- read-only doctor для Orca и Herdr, загрузку старого state без `tools` и
  регрессионный прогон действующих Herdr acceptance tests.

## Официальные источники

- [Orca install](https://www.onorca.dev/docs/install) — платформы, Linux
  AppImage/пакеты, конфликт имени CLI и macOS cask.
- [Headless Linux Server](https://github.com/stablyai/orca/blob/main/docs/reference/headless-linux-server.md)
  — AppImage extraction без FUSE, путь CLI, версия и требования к `serve`.
- [Latest Linux release metadata](https://github.com/stablyai/orca/releases/latest/download/latest-linux.yml)
  и [GitHub latest release API](https://api.github.com/repos/stablyai/orca/releases/latest)
  — текущая версия, размер и официальные digest assets.
