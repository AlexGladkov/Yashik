# Validate: документация Yashik

Статус: **пройдена для документационного результата**. Сверены требования,
README, design, YAML-пример, Plan, Research и Review. Это не проверка реализации.

## Результаты D1–D8

| ID | Результат | Проверка |
| --- | --- | --- |
| D1 | pass | README, design и пример согласованы: `init`/`doctor` — будущие команды; установщик и бинарник не заявлены как существующие. |
| D2 | pass | Codex, Claude Code, OpenCode, Pi и OMP; scope ограничен глобальным пользовательским окружением. Пользователь авторизуется сам. |
| D3 | pass | PyYAML разбирает пример. Ручное применение правил общего наследования, полной замены и `enabled: false` дало таблицу ниже. |
| D4 | pass | Пример и контракт показывают Git/local, pin/latest, portable/native и явные argv steps; значения примера помечены placeholders. |
| D5 | pass | Описаны повторная сверка, merge/replace с backup, подтверждение удаления, unsupported-ветка, поддерживаемые runtime-рецепты и отчёт независимых ошибок. Журнал хранит все исходы; ownership появляется только после успеха. |
| D6 | pass | Установка Yashik/bootstrap отделена от `init`; этапы roadmap имеют зависимости и условия выхода. |
| D7 | pass | Unattended, модели/providers/permissions/hooks, экспорт и marketplace отложены по интервью; удалённый MCP и доставка Yashik требуют отдельного решения. Лишних продуктовых требований в MVP не добавлено. Linux/macOS и реальные smoke tests не представлены как проверенные. |
| D8 | pass | Review одобрил документационный результат и пометил P1 resolved; эта Validate не обнаружила блокирующих расхождений. |

## Фактические проверки

```sh
/usr/bin/python3 -c "import yaml; from pathlib import Path; manifest = yaml.safe_load(Path('examples/yashik.yaml').read_text()); print(f'PyYAML {yaml.__version__}; schema version {manifest[\"version\"]}; harnesses: {\", \".join(manifest[\"harnesses\"])}')"
```

Вывод: PyYAML **6.0.3**, schema version `1`, harnesses `codex`, `claude`,
`opencode`, `pi`, `omp`. Второй inline-checker на `/usr/bin/python3` разобрал YAML и вручную
применил документированное правило: локальный ресурс заменяет одноимённый общий
целиком, `{enabled: false}` удаляет его для целевого harness.

| Harness | MCP | Skills | Agents | Rules |
| --- | --- | --- | --- | --- |
| codex | knowledge-search | release-check | portable-reviewer | writing-style |
| claude | knowledge-search | release-check | portable-reviewer (native override) | writing-style |
| opencode | knowledge-search | release-check | portable-reviewer | writing-style |
| pi | knowledge-search | release-check | — (общий отключён) | writing-style |
| omp | knowledge-search | release-check | portable-reviewer | writing-style, terminal-notes |

Claude override содержит собственные `format`, `source` и `from`; поле
`description` общего agent в эффективную запись не перешло. У Pi общий agent
исключён только локально. OMP сохраняет общий rule и добавляет локальный.

Локальные Markdown-ссылки проверены inline-сканером `/usr/bin/python3` на README
и `docs/**/*.md`: **11 проверено, 0 отсутствующих**.

## Границы проверки

Проверены документы и синтаксис/пример YAML. CLI отсутствует, поэтому build,
install, `init`/`doctor`, конфигурации установленных harnesses и поведение при
сбоях не запускались. Ubuntu не проверялась; утверждений о подтверждённой работе
на Linux или macOS эта Validate не делает. Это закрывает D1–D8 документационного
прогона, но не критерии реализации M1–M12.
