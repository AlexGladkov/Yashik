# Research: конфиг по умолчанию

Запрос: `yashik init` ищет `yashik-compose.yaml` только в текущей папке;
явный путь остаётся доступен. Исследование выполнено двумя независимыми
Consilium agents, GPT-6 Luna / max: CLI и интеграции/риски.

## Подтверждённые факты

- `src/main.rs` вручную разбирает аргументы; init/check требуют путь.
  `load_effective` читает и проверяет YAML до создания install state.
  Относительные ресурсы разрешаются от папки манифеста.
- `tests/cli.rs` проверяет явные пути; helper не задаёт cwd дочернего процесса.
- Windows launcher (`tools/windows-launcher/src/lib.rs`) также требует путь
  для init/check, но общий runner уже принимает optional manifest.
  Явные Windows-пути canonicalize + wslpath переводятся в Linux-путь.
- Windows bridge не задаёт --cd. Mock tests не подтверждают отображение
  Windows cwd в WSL cwd. Microsoft документирует наследование cwd при
  запуске Linux-команд из CMD/PowerShell через wsl.exe:
  https://learn.microsoft.com/en-us/windows/wsl/filesystems .
  Передача bare init соответствует этому контракту; точную bridge-команду
  на Windows следует проверить отдельно.
- README и docs/design.md описывают старое имя yashik.yaml.

## Решения и риски

Область фичи — init: check продолжает требовать путь. Не искать в родителях,
не выбирать произвольный YAML и не добавлять fallback. Ошибка отсутствующего
конфига должна содержать имя и подсказку об explicit path без записи state.
Проверить implicit init, explicit override, invalid YAML, отсутствие файла,
изоляцию cwd, отсутствие поиска родителей и Windows аргументы.
Реальное Windows/WSL поведение не подтверждается Linux mock tests.
