# Win Notification Log

A local, private journal of Windows notifications. The program reads the
notifications the notification centre shows, keeps them in a single SQLite
database next to itself, and lets you browse, search, filter and export that
history later. It runs in the tray, keeps collecting while the window is closed,
and never sends anything anywhere.

The interface is available in **English (default)** and **Russian**; the
language is chosen in the settings window and stored in `settings.json`.

> **How this project was made.** Win Notification Log was built with an
> **agentic system based on DeepSeek** (an AI/LLM assistant working in the
> repository: reading the code, writing it, building and testing it). The
> architecture, the wording of the interface and the code were produced in that
> collaboration. See "Credits" below.

## Contents

- [Features](#features)
- [Requirements](#requirements)
- [Build](#build)
- [Run](#run)
- [Where the data lives](#where-the-data-lives)
- [Command line](#command-line)
- [Language](#language)
- [Privacy](#privacy)
- [Project layout](#project-layout)
- [License](#license)
- [Credits](#credits)

## Features

- **Two sources in one pass.** The Windows notification database (`wpndb`)
  gives the payload, tags, actions and pictures; the documented WinRT
  `UserNotificationListener` API (`uns`) gives the texts and times Windows
  shows in the action centre. Both observations merge into a single record.
- **One file, full-text search.** The journal is a single SQLite database in
  WAL mode with an FTS5 index and configurable retention.
- **A window and a console.** `wnl.exe` is the graphical program (a full
  journal view and a narrow "feed" strip); `wnl-cli.exe` is the same collector
  and journal for scripts, diagnostics and machines without a desktop.
- **Text feed.** Every new notification can be appended to a plain text file
  (handy for a chat window or `tail -f`).
- **Rules.** A small rule set can skip, flag or pop up notifications.
- **Trays, autostart, dark and light look, DPI awareness** — the usual comfort
  of a native Windows program, without a framework.
- **Idle does not touch the disk.** Nothing is written while no notification
  arrives; the state file is rewritten only when something a reader can act on
  changes.

## Requirements

- Windows 10 or 11 (64-bit).
- To **build**: a Rust toolchain (`rustup`). The default toolchain is the GNU
  target (`stable-x86_64-pc-windows-gnu`), which links with the `ld.exe` that
  ships with the `rust-mingw` component — no Visual Studio, Build Tools or
  Windows SDK are needed. MSVC works too; delete `rust/rust-toolchain.toml` (or
  change its channel) to use it.
- To **run**: nothing. SQLite comes from the in-box `winsqlite3.dll`, the
  interface is built directly on Win32, and there is no C dependency at all.

## Build

```powershell
.\rust\build.ps1            # run the tests, build the release, copy to rust\dist
.\rust\build.ps1 -Run       # the same, then start the program
.\rust\build.ps1 -SkipTests # skip the tests
```

The executables end up in `rust\dist`: `wnl.exe` (the program) and
`wnl-cli.exe` (diagnostics and scripting).

### Ready-to-run binaries

The `dist\` folder of this repository holds the compiled programs
(`wnl.exe`, `wnl-cli.exe`) for 64-bit Windows — no toolchain is needed to run
them.

They are built with the source paths rewritten (`--remap-path-prefix`), so the
files carry **no path, user name or other trace of the machine that produced
them**: the only source names inside are neutral and relative, such as
`rust\src\ui.rs`. The build script above sets the same remappings itself, so a
binary you build on your own machine is just as anonymous.

## Run

Double-click `wnl.exe`. The window shows the journal; closing it keeps the
program in the notification area (that is a setting, not a surprise).
`wnl.exe --background` starts straight into the tray, which is what the
autostart entry uses.

## Where the data lives

The data directory is chosen in this order:

1. the folder chosen in the settings (`data_dir`, `--home` or the
   `WINNOTIFYLOG_HOME` environment variable);
2. otherwise **`data` next to the executable** — portable mode: the journal
   travels with the program;
3. otherwise `%LOCALAPPDATA%\WinNotificationLog`.

It holds `history.db` (the SQLite journal, WAL mode, plus its `-wal`/`-shm`
files), `pictures\` (copies of notification images), `wnl.log` (the optional
collector log), `status.json` (collector state for external tools),
`feed.json` (state of the text feed) and `exports\`.

## Command line

```powershell
.\rust\dist\wnl-cli.exe doctor                 # what the program sees on this machine
.\rust\dist\wnl-cli.exe stats                  # journal statistics
.\rust\dist\wnl-cli.exe search --text invoice  # full-text search
.\rust\dist\wnl-cli.exe export --format html --out journal.html
.\rust\dist\wnl-cli.exe watch --interval 5     # collect in the console (Ctrl+C stops)
```

Run `wnl-cli.exe help` for the full list.

## Language

The interface is bilingual. English is the default; Russian is selected in the
settings window (**Interface language**). The wording lives in the source in
Russian (the language the program grew up in) and is translated at the edges of
the program — see `rust/src/lang.rs`, which holds the table and explains the
two rules that keep the translation honest:

- only text a user can read is translated (never database keys, JSON field
  names or anything a comparison depends on), so switching the language cannot
  change what the program stores;
- translation happens once, at the point where text leaves the program.

## Privacy

- Everything stays on the machine. The program makes **no network requests**.
- The journal, the log, the state file and the exported files are ordinary
  local files in the data directory; nothing is sent to a server.
- The repository contains **no personal data**: no journal, no settings, no
  paths of any particular machine and no real application identifiers. The
  example identifiers in code and tests are synthetic.

## Project layout

```
rust/
  Cargo.toml            crate and binaries
  build.ps1             tests, release build, copy to rust\dist
  src/
    lang.rs             interface language (English/Russian) and the table
    ui.rs               the main window (Win32)
    settings_ui.rs      the settings window
    detail.rs, purge.rs, confirm.rs, export.rs, feed.rs, view.rs
    service.rs          the background collector (thread, status.json, log)
    store.rs, model.rs, sqlite*.rs   the SQLite journal and the record model
    wpndb.rs, payload.rs, uns.rs     the two notification sources
    settings.rs, storage.rs          settings and where data lives
    appname.rs, appicon.rs, packages.rs, imaging.rs   names, icons, pictures
    win32.rs, style.rs, theme.rs     the Win32 layer
  tools/                developer helpers (UI driver, WinMD dump)
```

## License

MIT — see [LICENSE](LICENSE).

## Credits

Created with an **agentic AI system based on DeepSeek**: the assistant worked
inside the repository, reading the existing code, writing the native (Rust)
implementation and the English translation, and building and testing the
result. All decisions about what the program should do, and the final review,
belong to the human maintainer.

---

# Win Notification Log (по-русски)

Локальный, приватный журнал уведомлений Windows. Программа читает то, что
показывает центр уведомлений, складывает это в одну базу SQLite рядом с собой и
позволяет позже просматривать, искать, фильтровать и экспортировать эту историю.
Работает в области уведомлений, продолжает сбор при закрытом окне и никуда
ничего не отправляет.

Интерфейс доступен на **английском (по умолчанию)** и **русском** языках; язык
выбирается в настройках и хранится в `settings.json`.

> **Как создан проект.** Win Notification Log сделан с помощью **агентной
> системы на базе DeepSeek** (ИИ/LLM-ассистент работал прямо в репозитории:
> читал код, писал его, собирал и тестировал). Архитектура, формулировки
> интерфейса и код появились в этом сотрудничестве. См. «Благодарности».

## Возможности

- **Два источника в одном проходе.** База уведомлений Windows (`wpndb`) даёт
  payload, теги, действия и картинки; документированный WinRT-интерфейс
  `UserNotificationListener` (`uns`) — тексты и время из центра уведомлений. Оба
  наблюдения сливаются в одну запись.
- **Один файл, полнотекстовый поиск.** Журнал — одна база SQLite в режиме WAL с
  индексом FTS5 и настраиваемым сроком хранения.
- **Окно и консоль.** `wnl.exe` — графическая программа (полный вид журнала и
  узкая «лента»); `wnl-cli.exe` — тот же сборщик и журнал для скриптов,
  диагностики и машин без рабочего стола.
- **Лента уведомлений.** Каждое новое уведомление можно дописывать в текстовый
  файл (удобно для окна чата или `tail -f`).
- **Правила.** Небольшой набор правил умеет пропускать, помечать или всплывать
  оповещением.
- **Трей, автозапуск, тёмное и светлое оформление, DPI** — обычный комфорт
  программы для Windows, без фреймворков.
- **Простой не трогает диск.** Пока не пришло уведомление, ничего не пишется;
  файл состояния перезаписывается только когда меняется то, на что может
  отреагировать читатель.

## Требования

- Windows 10 или 11 (64 бита).
- Для **сборки**: инструментарий Rust (`rustup`). По умолчанию GNU-цель
  (`stable-x86_64-pc-windows-gnu`), которая линкуется `ld.exe` из компонента
  `rust-mingw` — Visual Studio, Build Tools и Windows SDK не нужны. MSVC тоже
  подходит: удалите `rust/rust-toolchain.toml` (или смените канал).
- Для **запуска**: ничего. SQLite берётся из системной `winsqlite3.dll`,
  интерфейс построен прямо на Win32, зависимостей на C нет.

## Сборка

```powershell
.\rust\build.ps1            # тесты, release, копирование в rust\dist
.\rust\build.ps1 -Run       # то же и сразу запуск
.\rust\build.ps1 -SkipTests # без тестов
```

Готовые файлы появляются в `rust\dist`: `wnl.exe` (программа) и `wnl-cli.exe`
(диагностика и скрипты).

### Готовые к запуску файлы

В папке `dist\` репозитория лежат собранные программы (`wnl.exe`,
`wnl-cli.exe`) для 64-битной Windows — для запуска инструментарий не нужен.

Они собраны с перезаписью путей исходников (`--remap-path-prefix`), поэтому в
файлах **нет ни пути, ни имени пользователя, ни других следов машины, на
которой они сделаны**: внутри остаются только нейтральные относительные имена
вроде `rust\src\ui.rs`. Скрипт сборки выше сам задаёт те же переназначения, так
что собранная у вас копия так же анонимна.

## Запуск

Запустите `wnl.exe`. Окно показывает журнал; закрытие оставляет программу в
области уведомлений (это настройка, а не сюрприз). `wnl.exe --background`
стартует сразу в трей — так делает запись автозапуска.

## Где лежат данные

Папка данных выбирается по порядку:

1. выбранная в настройках папка (`data_dir`, `--home` или переменная окружения
   `WINNOTIFYLOG_HOME`);
2. иначе **`data` рядом с exe** — portable-режим: журнал едет вместе с
   программой;
3. иначе `%LOCALAPPDATA%\WinNotificationLog`.

В ней лежат `history.db` (журнал SQLite в режиме WAL и его файлы `-wal`/`-shm`),
`pictures\` (копии картинок из уведомлений), `wnl.log` (необязательный журнал
работы), `status.json` (состояние сборщика для внешних инструментов),
`feed.json` (состояние текстовой ленты) и `exports\`.

## Командная строка

```powershell
.\rust\dist\wnl-cli.exe doctor                 # что программа видит на этой машине
.\rust\dist\wnl-cli.exe stats                  # статистика журнала
.\rust\dist\wnl-cli.exe search --text invoice  # полнотекстовый поиск
.\rust\dist\wnl-cli.exe export --format html --out journal.html
.\rust\dist\wnl-cli.exe watch --interval 5     # сбор в консоли (Ctrl+C — стоп)
```

Полный список — `wnl-cli.exe help`.

## Язык

Интерфейс двуязычный. Английский — по умолчанию, русский выбирается в настройках
(**Язык интерфейса**). Формулировки хранятся в исходном коде по-русски (на этом
языке программа выросла) и переводятся на границах вывода — см.
`rust/src/lang.rs`, где лежит таблица и объяснены два правила, которые делают
перевод честным:

- переводится только то, что читает пользователь (никогда — ключи базы, имена
  полей JSON или то, от чего зависит сравнение), поэтому смена языка не может
  изменить то, что программа записывает;
- перевод происходит один раз, там, где текст покидает программу.

## Приватность

- Всё остаётся на машине. Программа **не делает сетевых запросов**.
- Журнал, лог, файл состояния и экспортируемые файлы — обычные локальные файлы в
  папке данных; никуда ничего не отправляется.
- В репозитории **нет персональных данных**: ни журнала, ни настроек, ни путей
  конкретной машины, ни реальных идентификаторов приложений. Идентификаторы в
  коде и тестах — синтетические.

## Состав проекта

```
rust/
  Cargo.toml            крейт и бинарники
  build.ps1             тесты, release и копирование в rust\dist
  src/
    lang.rs             язык интерфейса (EN/RU) и таблица перевода
    ui.rs               главное окно (Win32)
    settings_ui.rs      окно настроек
    detail.rs, purge.rs, confirm.rs, export.rs, feed.rs, view.rs
    service.rs          фоновый сборщик (поток, status.json, журнал)
    store.rs, model.rs, sqlite*.rs   журнал SQLite и модель записи
    wpndb.rs, payload.rs, uns.rs     два источника уведомлений
    settings.rs, storage.rs          настройки и размещение данных
    appname.rs, appicon.rs, packages.rs, imaging.rs   имена, значки, картинки
    win32.rs, style.rs, theme.rs     слой Win32
  tools/                вспомогательные скрипты разработчика (драйвер UI, WinMD)
```

## Лицензия

MIT — см. [LICENSE](LICENSE).

## Благодарности

Проект создан с помощью **агентной системы на базе DeepSeek**: ассистент работал
внутри репозитория — читал существующий код, писал нативную (Rust) реализацию и
английский перевод, собирал и тестировал результат. Решения о том, что должна
делать программа, и финальная проверка — за человеком.
