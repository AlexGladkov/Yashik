# Review: Rust-каркас и YAML-схема

## Решение

**APPROVED WITH BUILD-REPRODUCIBILITY GAP.** B1 исправлена; открытых
блокирующих дефектов Stage 1 не найдено. Полный набор тестов выполнился с
временными path-patch зависимостями из проверенных upstream source snapshots.
Обычная Cargo-сборка через registry по-прежнему не воспроизводима в этой среде,
а macOS не проверялась.

## Повторная проверка B1

`Agent` теперь десериализуется через internal tagged `AgentWire`
(`src/schema.rs:199-232`). Вариант `Portable` требует non-null
`description: String`; вариант `Native` вообще не содержит этого поля и оба
варианта используют `deny_unknown_fields`. Поэтому native
`description: null` и `description: "..."`, а также portable отсутствующий или
null `description`, отклоняются ещё при deserialization.

Новые regression cases находятся в `src/core_tests.rs:99-115` и
`tests/yaml_schema.rs:37-50`. Повторно выполнены оба затронутых теста с
временным проверенным parser source:

- `core_tests::agent_variants_require_portable_description_and_reject_native_description` — passed;
- `yaml_parser_rejects_malformed_unknown_duplicate_and_null_collections` — passed.

## Проверенные части

По исходному коду M1/M2 в остальном согласованы с планом:

- `deserialize_unique_map` используется для всех resource maps и `run.env`;
  struct fields защищены Serde `deny_unknown_fields`.
- `Source` — закрытый tagged enum `git`/`local`; local sentinel ограничен
  строгим `DisabledEntry`, а `enabled: true` отклоняется validation.
- validation проверяет общие и локальные specs, включая отключённые harnesses;
  effective model сначала копирует common map, затем делает полную local
  замену или удаление по sentinel.
- CLI-код только читает manifest, выводит имена ресурсов и явно сообщает о
  schema-only этапе; `doctor` возвращает ненулевой код без диагностики.

## Проверки и незакрытая граница

| Проверка | Результат | Что это подтверждает |
| --- | --- | --- |
| `rustfmt --check src/schema.rs src/core_tests.rs tests/yaml_schema.rs` | passed | Форматирование изменённых файлов. |
| Targeted core regression через временный parser source | 1 passed | Native/portable variant schema больше не принимает B1-вход. |
| Targeted YAML regression через временный parser source | 1 passed | Реальный `yaml_serde` 0.10.7 отклоняет B1 и остальные parser fixtures. |
| Полный temporary-source suite | 15 passed: 10 core + 3 YAML + 2 CLI | Actual repo `src/`, `tests/` и `examples` были подключены symlink; выполнились parser и CLI HOME-snapshot tests. |
| Direct temporary-source binary smoke | passed: `init examples/yashik.yaml`, `--version` | Собранный `target/debug/yashik` запускается с реальным parser. |
| Обычный root `cargo test --offline` | blocked: `no matching package named yaml_serde found` | Registry cache по-прежнему не содержит production parser. |

Temporary suite использовал неизменённый production parser из
`yaml_serde` 0.10.7 (tree `22a7009e918620b523ea06e54ca75275eabe8027`) и
`libyaml-rs` 0.3.0 (tree `2b0da67109c8531d5a3b30e52a034c36f2bac6c9`),
полученные в `/tmp` и проверенные по upstream Git blobs; самодельная замена
parser не вводилась. Это подтверждает YAML/CLI поведение на текущем Linux
окружении, но не подтверждает registry resolution/checksums для обычной
Cargo-сборки и не заменяет macOS проверку.
`Cargo.toml` остаётся на предписанном exact alias
`serde_yaml = { package = "yaml_serde", version = "=0.10.7" }`; временный
`Cargo.lock` и path patches не опубликованы в репозитории.
