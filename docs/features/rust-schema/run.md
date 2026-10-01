# Прогон Rust schema

| Этап | Назначение | Результат |
| --- | --- | --- |
| Research | Consilium, Terra 5.6 xhigh | Контракт и отдельная проверка доступности зависимостей |
| Plan | Strategist, Sol xhigh | plan.md, этап 1 / M1–M2 |
| Execute | Engineer, Luna max | Cargo.toml, src, tests, статусы README/design |
| Review | Engineer, Terra 5.6 max | B1 исправлен; одобрено с ограничением registry-воспроизводимости |
| Validate | Tester, Luna max | Полные 15 tests, CLI smoke и rustfmt прошли на Linux |

Предварительно: rustfmt check прошёл, 9 тестов точных production core modules
скомпилированы и прошли в отдельном offline Cargo harness с Serde/serde_json.
Это не замена production parser: основная зависимость yaml_serde сохранена.

Обычное разрешение registry dependencies блокируется отсутствием
yaml_serde/libyaml-rs в cache и недоступным crates.io DNS.

## Полная проверка с upstream-исходниками

Root получил 48 файлов через read-only GitHub connector и проверил Git blob SHA:

- yaml/yaml-serde, version 0.10.7, tree 22a7009e918620b523ea06e54ca75275eabe8027.
- yaml/libyaml-rs, version 0.3.0, tree 2b0da67109c8531d5a3b30e52a034c36f2bac6c9.

Исходники и лицензии находятся в `/tmp/yashik-upstream-deps`; provenance.json
содержит хеши. Это pinned upstream snapshots, не проверенные registry archives.
Временный manifest `/tmp/yashik-full-check/Cargo.toml` копирует Cargo.toml;
src/tests/examples ссылаются на настоящие файлы проекта. Cargo patches
применялись только аргументами проверки; production dependencies не заменены.
Временный lockfile создан Cargo в /tmp, root Cargo.lock не сочинялся.

Полный cargo test с реальным parser прошёл: 10 core, 3 YAML, 2 CLI, всего 15.
CLI tests проверили неизменность HOME и отсутствие секретов в выводе.
Бинарник `target/debug/yashik` запускается: `init examples/yashik.yaml` и
`--version` проверены. Форматирование прошло.

Review нашёл B1: native agent принимал description:null. Execute ввёл строгие
wire-варианты Portable/Native; 10 core tests и YAML regression подтвердили исправление.

Установка программ, MCP и запись в пользовательские конфиги не выполнялись.
macOS и обычная registry-сборка не проверены.

Прогон этапа 1 завершён с этими ограничениями воспроизводимости и платформы.
Следующий этап — источники, состояние и планировщик; установка ещё не реализована.
