//! Interface language.
//!
//! The program is written with Russian wording in the source (that is the
//! language it grew up in), and this module turns those strings into English
//! when the user chose English.  Two rules keep the translation honest:
//!
//! * **Only what a user can read is translated.**  The lookup table holds
//!   interface text - labels, menus, dialogs, log lines, exported text.  It does
//!   *not* hold database keys, JSON field names or anything a comparison depends
//!   on, so switching the language can never change what the program stores.
//! * **Translation happens at the edge.**  The Win32 helpers (`win32::wide`,
//!   `win32::fill_wide`) run every string through [`tr`], and the places that
//!   build text by hand do the same, so the wording is decided exactly once.
//!
//! `tr` returns the English text for the current language, or the input
//! unchanged when the language is Russian or the string is unknown (a class
//! name, a path, a value from the journal - nothing of that belongs here).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// A language the interface can be shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    /// Code stored in `settings.json` (`settings::Settings::language`).
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    /// Read the code from the settings file; anything unknown means English.
    pub fn from_code(code: &str) -> Lang {
        if code.trim().to_ascii_lowercase().starts_with("ru") {
            Lang::Ru
        } else {
            Lang::En
        }
    }
}

const CODE_EN: u8 = 0;
const CODE_RU: u8 = 1;
/// Nothing chosen yet; the initial value depends on the build (see below).
const CODE_UNSET: u8 = 255;

static CURRENT: AtomicU8 = AtomicU8::new(CODE_UNSET);

/// The language before the settings are read.
///
/// The shipped program starts in English.  The unit tests, on the other hand,
/// assert the source wording, so they keep running in Russian - that is the
/// language the code itself is written in, and it means a test never has to
/// translate what it expects.
#[cfg(test)]
fn initial() -> Lang {
    Lang::Ru
}

#[cfg(not(test))]
fn initial() -> Lang {
    Lang::En
}

/// The language in force.
pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        CODE_EN => Lang::En,
        CODE_RU => Lang::Ru,
        _ => initial(),
    }
}

/// Choose the language (called once at start-up from the settings).
pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::En => CODE_EN,
            Lang::Ru => CODE_RU,
        },
        Ordering::Relaxed,
    );
}

/// True when the interface is being shown in English.
pub fn is_english() -> bool {
    current() == Lang::En
}

/// English wording of one interface string.
///
/// The Russian text is the key; an unknown string is returned unchanged, so a
/// value that is not interface text (a path, a class name, journal data) is
/// never touched - even when it happens to be passed through the same helpers.
pub fn tr(text: &str) -> &str {
    tr_for(current(), text)
}

/// [`tr`] for an explicit language (keeps the decision testable and free of the
/// global setting, so tests never disturb one another).
pub fn tr_for(lang: Lang, text: &str) -> &str {
    if lang != Lang::En {
        return text;
    }
    match table().get(text) {
        Some(english) => english,
        None => text,
    }
}

/// Owned form of [`tr`], for the places that need a `String`.
pub fn tr_owned(text: &str) -> String {
    tr(text).to_string()
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| PAIRS.iter().copied().collect())
}

/// `(russian, english)` pairs; the Russian text is what the source holds.
const PAIRS: &[(&str, &str)] = &[
    // -- window titles, menus, headings -----------------------------------
    ("Настройки", "Settings"),
    ("Настройки…", "Settings…"),
    ("О программе", "About"),
    ("Выход", "Exit"),
    ("Файл", "File"),
    ("Вид", "View"),
    ("Справка", "Help"),
    ("Открыть журнал", "Open journal"),
    ("Открыть папку данных", "Open data folder"),
    ("Экспорт…", "Export…"),
    ("Экспорт…\tCtrl+S", "Export…\tCtrl+S"),
    ("Обновить список\tF5", "Refresh list\tF5"),
    ("Проверить сейчас\tF5", "Collect now\tF5"),
    ("Импорт сейчас", "Import now"),
    (
        "Прочитать базу уведомлений заново…",
        "Read the notification database again…",
    ),
    ("Приостановить сбор", "Pause collection"),
    ("Продолжить сбор", "Resume collection"),
    ("Удалить запись из журнала…", "Delete the record from the journal…"),
    ("Удалить за период…", "Delete by period…"),
    ("Удалить выбранные…\tDel", "Delete selected…\tDel"),
    ("Открыть целиком\tEnter", "Open the full record\tEnter"),
    ("Открыть выбранное\tEnter", "Open the selected record\tEnter"),
    ("Копировать заголовок", "Copy the title"),
    ("Копировать текст уведомления", "Copy the notification text"),
    ("Копировать ссылку", "Copy the link"),
    ("Открыть ссылку в браузере", "Open the link in a browser"),
    ("Открыть изображение", "Open the image"),
    ("Открыть изображение в браузере", "Open the image in a browser"),
    ("Показать только это приложение", "Show only this application"),
    ("Показать только этот тип", "Show only this type"),
    ("Сбросить фильтры", "Reset the filters"),
    ("Полный вид программы", "Full program view"),
    ("Простой вид (лента)", "Simple view (feed)"),
    (
        "Настройки: сбор, работа в фоне, вид, размер текста, папка данных.",
        "Settings: collection, background, view, text size, data folder.",
    ),
    (
        "Открыть папку, где лежит журнал, настройки и журнал работы.",
        "Open the folder that holds the journal, the settings and the log.",
    ),
    ("Экспорт новых уведомлений", "Export new notifications"),
    ("Экспорт отобранного…", "Export the selected records…"),
    ("Нечего экспортировать: список пуст.", "Nothing to export: the list is empty."),
    (
        "Сохранить то, что отобрано фильтрами, в файл: CSV, JSON, TXT или HTML.",
        "Save what the filters selected to a file: CSV, JSON, TXT or HTML.",
    ),
    ("Экспорт отменён.", "Export cancelled."),
    ("Обзор…", "Browse…"),
    ("Размер текста", "Text size"),
    ("Тема оформления", "Theme"),
    ("Папка с данными", "Data folder"),
    ("Папка", "Folder"),
    ("Размер текста:", "Text size:"),
    // -- list columns, filters, records -----------------------------------
    ("Время", "Time"),
    ("Приложение", "Application"),
    ("Заголовок", "Title"),
    ("Текст", "Text"),
    ("Тип", "Type"),
    ("Источник", "Source"),
    ("Источники", "Sources"),
    ("Группа", "Group"),
    ("Тег", "Tag"),
    ("Ключ", "Key"),
    ("Ссылка", "Link"),
    ("Действия", "Actions"),
    ("Содержание", "Content"),
    ("Повторов", "Repeats"),
    ("Сценарий", "Script"),
    ("Обработчик", "Handler"),
    ("подпись", "label"),
    ("Последний раз", "Last seen"),
    ("Хеш содержимого", "Content hash"),
    ("Идентификатор", "Identifier"),
    ("Идентификатор приложения", "Application identifier"),
    ("Тип уведомления", "Notification type"),
    ("Тип активации", "Activation type"),
    ("Дополнительно", "Additional"),
    ("Технические столбцы", "Technical columns"),
    ("Всё время", "All time"),
    ("Сегодня", "Today"),
    ("Последние 7 дней", "Last 7 days"),
    ("Последние 30 дней", "Last 30 days"),
    ("Свой диапазон", "Custom range"),
    ("Все приложения", "All applications"),
    ("Все типы", "All types"),
    ("Все источники", "All sources"),
    ("Фильтры:", "Filters:"),
    ("Поиск:", "Search:"),
    ("Сбросить", "Reset"),
    ("Копировать", "Copy"),
    ("Отмена", "Cancel"),
    ("Сохранить", "Save"),
    ("Закрыть", "Close"),
    ("Открыть", "Open"),
    ("Удалить", "Delete"),
    ("Удалить…", "Delete…"),
    ("Ещё", "More"),
    ("Действия:", "Actions:"),
    ("Изображения:", "Images:"),
    ("Изображения", "Images"),
    ("Основное", "Main"),
    ("Основные сведения", "Details"),
    ("Технические подробности", "Technical details"),
    ("Полностью", "Full"),
    ("Действия уведомления", "Notification actions"),
    ("Дневная тема (светлое оформление)", "Day theme (light look)"),
    ("Ночная тема (тёмное оформление)", "Night theme (dark look)"),
    (
        "Ночная тема (тёмное оформление окна и списка)",
        "Night theme (dark window and list)",
    ),
    ("Обычный (100 %)", "Normal (100 %)"),
    ("Крупнее (115 %)", "Larger (115 %)"),
    ("Крупный (130 %)", "Large (130 %)"),
    ("Очень крупный (150 %)", "Very large (150 %)"),
    ("Максимальный (175 %)", "Largest (175 %)"),
    ("Свой размер", "Custom size"),
    ("Одна строка", "One line"),
    ("Две строки", "Two lines"),
    ("Три строки", "Three lines"),
    // -- settings window ---------------------------------------------------
    ("Сбор уведомлений", "Notification collection"),
    ("Собирать уведомления при запуске", "Collect notifications at start-up"),
    ("Проверять новые уведомления, секунды (1–3600):", "Check for new notifications, seconds (1–3600):"),
    ("Прочитать прежних при первом запуске:", "Read existing ones on the first run:"),
    ("Удалять записи старше, дней (0 — бессрочно):", "Delete records older than, days (0 — keep forever):"),
    ("Окно и работа в фоне", "Window and background work"),
    ("При закрытии продолжать работу в области уведомлений", "Keep running in the notification area on close"),
    ("При сворачивании убирать окно в область уведомлений", "Minimise into the notification area"),
    ("Запускать автоматически при входе в Windows", "Start automatically at logon"),
    ("Запускать сразу свёрнутым (без окна)", "Start minimised (no window)"),
    ("Вид списка", "List view"),
    ("Показывать значки приложений", "Show application icons"),
    ("Показывать технические столбцы (источник, повторы)", "Show the technical columns (source, repeats)"),
    ("Простой вид — только лента (источник, содержание, значок)", "Simple view — the feed only (source, content, icon)"),
    ("Весы приложений", "Application weights"),
    ("Значки приложений", "Application icons"),
    ("Скрывать повторные уведомления", "Hide repeated notifications"),
    ("Столбцы ленты", "Feed columns"),
    ("Строк в строке ленты", "Lines per feed row"),
    ("Куда сохранять журнал уведомлений?", "Where should the journal be kept?"),
    ("Папка данных", "Data folder"),
    ("Дописывать новые уведомления в текстовый файл", "Append new notifications to a text file"),
    ("Куда дописывать новые уведомления?", "Where should new notifications be appended?"),
    ("Формат экспорта по умолчанию:", "Default export format:"),
    ("Данные JSON", "JSON data"),
    ("Таблица CSV (Excel)", "CSV table (Excel)"),
    ("Текст TXT", "TXT text"),
    ("Страница HTML", "HTML page"),
    ("Вести журнал работы (wnl.log)", "Keep the collector log (wnl.log)"),
    ("Опрашивать центр уведомлений через WinRT", "Read the notification centre through WinRT"),
    ("Сохранять картинки уведомлений рядом с журналом", "Keep notification pictures next to the journal"),
    ("Тип уведомления: toast (всплывающее), tile (плитка), badge (значок на плитке).", "Notification type: toast (pop-up), tile (live tile), badge (tile badge)."),
    // -- status line and collector text -----------------------------------
    ("Сбор", "Collection"),
    ("сбор запущен", "collection running"),
    ("сбор запущен, идёт первое чтение…", "collection running, first read in progress…"),
    ("сбор остановлен", "collection stopped"),
    ("сбор приостановлен", "collection paused"),
    ("сбор продолжается", "collection resumed"),
    ("сбор не выполняется", "collection is not running"),
    ("сбор ещё не запускался", "collection has not started yet"),
    ("сбор выключен в настройках", "collection is switched off in the settings"),
    ("Сбор выключен в настройках.", "Collection is switched off in the settings."),
    ("запрошен сбор…", "collection requested…"),
    ("коллектор запущен", "collector started"),
    ("коллектор остановлен", "collector stopped"),
    ("источник uns: выключен в настройках", "uns source: switched off in the settings"),
    ("источник uns: читаю центр уведомлений через WinRT", "uns source: reading the notification centre through WinRT"),
    (
        "лента новых уведомлений: выключена в настройках",
        "feed of new notifications: switched off in the settings",
    ),
    (
        "Сбор уведомлений идёт в фоне. Открыть окно — двойной щелчок по значку.",
        "Notifications are being collected in the background. Double-click the tray icon to open the window.",
    ),
    ("проблемы нет", "no problem"),
    ("без повторов", "without repeats"),
    ("без текста", "without text"),
    ("нет", "no"),
    ("по", "to"),
    ("с", "from"),
    ("записей", "records"),
    ("записи", "records"),
    ("запись", "record"),
    ("Ещё 428", "428 more"),
    ("Ай", "Ah"),
    ("вы уверены?", "are you sure?"),
    ("Проверить сейчас", "Collect now"),
    // -- collection sources, diagnostics ----------------------------------
    ("  •  имя программы из базы Windows", "  •  application name from the Windows database"),
    ("(без заголовка)", "(no title)"),
    ("(без имени)", "(no name)"),
    ("(уведомление без текста)", "(notification without text)"),
    ("в профиле пользователя", "in the user profile"),
    ("рядом с программой", "next to the program"),
    ("хранятся бессрочно", "kept forever"),
    ("бессрочно", "forever"),
    ("[ok     ] service: состояние не публиковалось (status.json отсутствует)", "[ok     ] service: no state was published (status.json is missing)"),
    ("[ok     ] uns    : выключен в настройках", "[ok     ] uns    : switched off in the settings"),
    ("[ok     ] uns    : состояние источника не записано", "[ok     ] uns    : the state of the source was not recorded"),
    ("Current вернул пустой указатель", "Current returned a null pointer"),
    ("GetNotificationsAsync вернул пустой указатель", "GetNotificationsAsync returned a null pointer"),
    ("RequestAccessAsync вернул пустой указатель", "RequestAccessAsync returned a null pointer"),
    ("асинхронный вызов WinRT не завершился вовремя", "the asynchronous WinRT call did not finish in time"),
    ("библиотека WinRT (combase.dll) недоступна", "the WinRT library (combase.dll) is unavailable"),
    ("вызов вернул пустой результат", "the call returned an empty result"),
    ("фабрика класса вернула пустой указатель", "the class factory returned a null pointer"),
    ("неизвестный ответ системы", "unknown answer from the system"),
    ("не удалось прочитать Id уведомления", "could not read the notification id"),
    ("слушатель уведомлений недоступен", "the notification listener is unavailable"),
    ("пауза после ошибок чтения", "pause after read errors"),
    ("ещё не читался", "has not been read yet"),
    ("доступ ещё не определён", "access is not decided yet"),
    ("доступ запрещён пользователем", "access was denied by the user"),
    ("доступ разрешён", "access is granted"),
    ("выключен в настройках", "switched off in the settings"),
    ("выключена в настройках", "switched off in the settings"),
    ("выключено", "switched off"),
    ("выключен", "switched off"),
    ("включён", "on"),
    ("отменён", "cancelled"),
    ("все правила выключены", "every rule is switched off"),
    ("правил нет", "no rules"),
    // -- feed, export and the collector log --------------------------------
    ("Экспорт журнала уведомлений", "Export of the notification journal"),
    ("Журнал уведомлений Windows", "Windows notification journal"),
    ("Все файлы (*.*)", "All files (*.*)"),
    ("журнал работы ведётся", "the collector log is written"),
    ("журнал работы выключен", "the collector log is switched off"),
    // -- deletion dialogs --------------------------------------------------
    ("Удаление за период", "Delete by period"),
    ("Дату не удалось прочитать. Примеры: 29.09.2026, 29.09.26, 2026-09-29.", "The date could not be read. Examples: 29.09.2026, 29.09.26, 2026-09-29."),
    ("Начало своего диапазона: 29.09.2026 или 2026-09-29.", "Start of your range: 29.09.2026 or 2026-09-29."),
    ("Конец своего диапазона (включительно).", "End of your range (inclusive)."),
    ("Укажите хотя бы одну дату: без границ будет удалён весь журнал.", "Give at least one date: without bounds the whole journal would be deleted."),
    ("сколько записей попадает в этот период", "how many records fall into this period"),
    ("сколько записей будет удалено", "how many records will be deleted"),
    ("удали их", "delete them"),
    ("Удалить 3 записи?", "Delete 3 records?"),
    ("Уведомление", "Notification"),
    // -- window messages ---------------------------------------------------
    ("Не удалось зарегистрировать класс окна.", "The window class could not be registered."),
    ("Не удалось создать окно приложения.", "The application window could not be created."),
    ("Не удалось открыть окно уведомления.", "The notification window could not be opened."),
    ("Не удалось открыть окно удаления за период.", "The delete-by-period window could not be opened."),
    ("Не удалось скопировать в буфер обмена.", "Could not copy to the clipboard."),
    ("В этой записи нечего копировать.", "There is nothing in this record to copy."),
    ("копировать нечего: ничего не выбрано", "nothing to copy: nothing is selected"),
    ("Сначала выберите уведомление в списке.", "Select a notification in the list first."),
    ("Под текущий отбор ничего не попало.", "Nothing matched the current filter."),
    ("Показаны все записи, отобранные фильтрами.", "All records selected by the filters are shown."),
    ("Показать следующую часть списка. Кнопка гаснет, когда показано всё.", "Show the next part of the list. The button goes grey once everything is shown."),
    ("У записи нет идентификатора приложения, фильтровать не по чему.", "The record has no application identifier, so there is nothing to filter by."),
    ("У записи нет типа уведомления, фильтровать не по чему.", "The record has no notification type, so there is nothing to filter by."),
    ("последний столбец ленты выключать нельзя", "the last column of the feed cannot be switched off"),
    ("удалять было нечего", "there was nothing to delete"),
    ("Продолжаю работу", "Still running"),
    ("Папка с данными. Её можно сменить в настройках — журнал при этом переносится.", "Data folder. It can be changed in the settings — the journal is moved along."),
    ("Правила обработки: нет. Список правил задаётся в settings.json рядом с программой.", "Processing rules: none. The rule list is set in settings.json next to the program."),
    ("Почта", "Mail"),
    ("остановить: Ctrl+C", "press Ctrl+C to stop"),
    ("Язык интерфейса:", "Interface language:"),
    ("№", "No."),
    (
        "Удаляются записи журнала за выбранные сутки (местное время). Даты: 29.09.2026, 29.09.26, 2026-09-29. Пустое поле — граница не задана.",
        "Journal records of the chosen day (local time) are deleted. Dates: 29.09.2026, 29.09.26, 2026-09-29. An empty field leaves that bound open.",
    ),
    ("Удалить эти записи?", "Delete these records?"),
    ("payload", "payload"),
    (
        "Центр уведомлений через WinRT не опрашивается: записи идут только по базе Windows.",
        "The notification centre is not read through WinRT: records come from the Windows database only.",
    ),
    (
        "Язык изменён. Надписи окна обновятся после перезапуска программы.",
        "The language is set. The window labels follow after the program is started again.",
    ),
];

/// Translate one interface string: the Russian text in the source, English on
/// screen when English was chosen.
#[macro_export]
macro_rules! tr {
    ($ru:literal) => {
        $crate::lang::tr($ru)
    };
}

/// Like [`tr!`] for a sentence that carries values.
///
/// Both templates are written out, so the wording of each language stays where
/// it is used instead of hiding in the table (a `format!` template cannot be an
/// argument, it has to be a literal).
#[macro_export]
macro_rules! tf {
    ($ru:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::lang::is_english() {
            format!($en $(, $arg)*)
        } else {
            format!($ru $(, $arg)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_interface_text_is_translated() {
        assert_eq!(tr_for(Lang::En, "Настройки"), "Settings");
        assert_eq!(tr_for(Lang::En, "Выход"), "Exit");
        assert_eq!(tr_for(Lang::En, "сбор остановлен"), "collection stopped");
    }

    #[test]
    fn russian_comes_back_unchanged() {
        assert_eq!(tr_for(Lang::Ru, "Настройки"), "Настройки");
        assert_eq!(tr_for(Lang::Ru, "сбор остановлен"), "сбор остановлен");
    }

    #[test]
    fn values_that_are_not_interface_text_are_never_touched() {
        // A class name, an identifier or a path must survive every language.
        for text in [
            "WinNotificationLogNative",
            "Vendor.Chat_123!App",
            r"C:\Users\Public\history.db",
            "toast",
        ] {
            assert_eq!(tr_for(Lang::En, text), text, "{text}");
        }
    }

    #[test]
    fn the_language_code_round_trips() {
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::from_code("RU"), Lang::Ru);
        assert_eq!(Lang::from_code("ru-RU"), Lang::Ru);
        assert_eq!(Lang::from_code("de"), Lang::En, "unknown means English");
        assert_eq!(Lang::En.code(), "en");
        assert_eq!(Lang::Ru.code(), "ru");
    }
}
