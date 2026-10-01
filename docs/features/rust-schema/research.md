# Research: Rust-каркас и YAML-схема

Consilium: Terra 5.6 xhigh, отдельные задания по контракту и сборке.

## Контракт

Этап 1 реализует M1–M2: typed schema, строгую валидацию и наследование.
`init` пока только разбирает/проверяет YAML и показывает эффективные ресурсы;
не запускает установщики, Git, сборку MCP и не создаёт состояние.
`doctor` доступен в CLI как явно недоступная на этом этапе команда с
ненулевым кодом, без притворной проверки установленного окружения.

Использовать Serde deny_unknown_fields, закрытые enums пяти харнессов и
источников git/local, BTreeMap для стабильного порядка. Локальный ресурс
заменяет общий целиком; точный `{enabled: false}` исключает ресурс.
Portable agents/rules разрешены глобально; native — только локально.
Относительный local path разрешается от каталога YAML, без скачивания и
проверки существования placeholders из примера на этапе schema.

Проверки: malformed YAML, неизвестные поля/харнессы, типы, обязательные поля,
duplicate keys, argv, sentinel; полная замена, наследование, добавление,
отключение для одного харнесса и отключённый харнесс. Ошибка не меняет HOME.

## Зависимость и сборка

Выбран поддерживаемый YAML Organization fork yaml_serde 0.10.7, MSRV 1.82:
https://github.com/yaml/yaml-serde и https://docs.rs/yaml_serde/.
Совместимый Cargo alias: serde_yaml = { package = "yaml_serde", version = "=0.10.7" }.
Исследователь подтвердил наличие serde/indexmap/itoa/ryu, но отсутствие
yaml_serde и libyaml-rs как в cache, так и в sparse index. Rust 1.98.1 подходит.
Shell DNS для crates.io недоступен; `cargo info --offline` подтверждает блокировку.
Локальные Cargo.toml/archives проверены: vendored parser не найден.

Не писать самодельный YAML parser и не запускать Python как production runtime.
Реализовать обычную Rust-зависимость, сохранить тесты для полноценного запуска.
Проверки rustfmt и независимые проверки core-модулей возможны отдельно, но
не доказывают работу parser/CLI. Cargo.lock создаётся Cargo при разрешении
зависимостей, не сочиняется вручную. Блокировку build честно отражать в Validate.
