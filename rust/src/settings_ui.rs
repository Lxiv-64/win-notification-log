//! The settings window.
//!
//! Only options a normal user can actually judge are offered, grouped by what
//! they influence: collection, window behaviour, appearance and the data folder.
//! Everything technical (interval, retention, backfill) is written in plain
//! language next to the input field, with the allowed range taken from
//! `settings`, so the dialog can never produce a value the collector rejects.

use std::path::PathBuf;

use crate::export;
use crate::settings::{self, Settings, EXPORT_FORMATS};
use crate::storage;
use crate::style;
use crate::win32::*;

const WINDOW_CLASS: &str = "WinNotificationLogSettings";
const WINDOW_TITLE: &str = "Настройки";

const ID_TITLE_COLLECT: usize = 501;
const ID_COLLECT_ON_START: usize = 502;
const ID_INTERVAL_LABEL: usize = 503;
const ID_INTERVAL: usize = 504;
const ID_BACKFILL_LABEL: usize = 505;
const ID_BACKFILL: usize = 506;
const ID_RETENTION_LABEL: usize = 507;
const ID_RETENTION: usize = 508;
const ID_TITLE_WINDOW: usize = 509;
const ID_CLOSE_TO_TRAY: usize = 510;
const ID_MINIMIZE_TO_TRAY: usize = 511;
const ID_START_WITH_WINDOWS: usize = 512;
const ID_START_MINIMIZED: usize = 513;
const ID_TITLE_VIEW: usize = 514;
const ID_APP_ICONS: usize = 515;
const ID_TECHNICAL_COLUMNS: usize = 516;
const ID_TITLE_STORAGE: usize = 517;
const ID_HOME: usize = 518;
const ID_HOME_BROWSE: usize = 519;
const ID_HOME_OPEN: usize = 520;
const ID_HOME_HINT: usize = 521;
const ID_EXPORT_LABEL: usize = 522;
const ID_EXPORT_FORMAT: usize = 523;
const ID_RESET: usize = 524;
const ID_OK: usize = 525;
const ID_CANCEL: usize = 526;
const ID_FONT_LABEL: usize = 527;
const ID_FONT_SCALE: usize = 528;
/// Keep a copy of the pictures of a notification in the data folder.
const ID_KEEP_PICTURES: usize = 529;
/// Note about the rules of notification processing.
const ID_RULES_HINT: usize = 530;
/// Keep a log of the collector's work (`wnl.log`).
const ID_LOG: usize = 531;
/// Read the notification center through the documented WinRT API as well.
const ID_UNS: usize = 532;
/// Note about that source (where it is explained).
const ID_UNS_HINT: usize = 533;
/// Append every new notification to a plain text file.
const ID_FEED: usize = 534;
/// Folder that file is written to.
const ID_FEED_DIR: usize = 535;
/// Choose that folder.
const ID_FEED_BROWSE: usize = 536;
/// Note about the feed (which file it is).
const ID_FEED_HINT: usize = 537;
const ID_SIMPLE_VIEW: usize = 538;
/// The look of the program: day or night (`settings::Settings::theme`).
const ID_DARK_THEME: usize = 539;
/// Caption of the interface language list.
const ID_LANGUAGE_LABEL: usize = 540;
/// The interface language (`settings::Settings::language`).
const ID_LANGUAGE: usize = 541;

const PADDING: i32 = style::PADDING;
const GAP: i32 = style::GAP;
const ROW: i32 = style::ROW;
/// Gap between two rows of controls.
const ROW_GAP: i32 = style::ROW_GAP;
/// Gap between two sections.
const SECTION_GAP: i32 = style::SECTION_GAP;
/// One line of plain text.
const LINE: i32 = style::LINE;
/// Size of the window in logical pixels.
///
/// The height is what the sections need *plus* the row of buttons at the bottom;
/// a window that is too short grows a scroll bar that covers those buttons.
const WINDOW_WIDTH: i32 = 780;
const WINDOW_HEIGHT: i32 = 866;
const LABEL_WIDTH: i32 = 400;
const FIELD_WIDTH: i32 = 90;
const GROUP_TITLE_HEIGHT: i32 = style::HEADING;
const BUTTON_WIDTH: i32 = 130;

/// Read a whole number from a text field, clamped to `min..=max`.
///
/// Empty or unreadable input keeps the previous value instead of failing: the
/// user must never lose a setting because of a typo.
pub fn number_from_text(text: &str, min: i64, max: i64, previous: i64) -> i64 {
    match text.trim().parse::<i64>() {
        Ok(value) => value.clamp(min, max),
        Err(_) => previous.clamp(min, max),
    }
}

/// One line describing what the settings mean for the current run.
pub fn summary(current: &Settings, layout: &storage::Layout) -> String {
    let interval = current.collect_interval_seconds;
    let storage_text = storage::describe(layout);
    let retention = if current.retention_days == 0 {
        tr!("хранятся бессрочно").to_string()
    } else {
        tf!("хранятся {} дн.", "kept for {} days", current.retention_days)
    };
    let log = if current.log_enabled {
        tr!("журнал работы ведётся")
    } else {
        tr!("журнал работы выключен")
    };
    tf!(
        "Сбор каждые {interval} с · записи {retention} · {log} · данные: {storage_text}",
        "Collection every {interval} s · records {retention} · {log} · data: {storage_text}"
    )
}

/// One line about the rules of notification processing.
///
/// The list itself is edited in the settings file (there is no rule editor in the
/// window yet), so the sentence has to say where it lives and how many rules are
/// actually in force.
pub fn rules_hint(settings: &Settings) -> String {
    let total = settings.rules.rules().len();
    if total == 0 {
        return tr!("Правила обработки: нет. Список правил задаётся в settings.json рядом с программой.")
            .to_string();
    }
    if !settings.rules_enabled {
        return tf!(
            "Правила обработки выключены (в файле правил: {total}).",
            "Processing rules are switched off ({total} in the rule file)."
        );
    }
    let active = settings.rules.enabled_count();
    tf!(
        "Правила обработки: в силе {active} из {total} — задаются в settings.json рядом с программой.",
        "Processing rules: {active} of {total} are in force — they are set in settings.json next to the program."
    )
}

/// Sentence under the WinRT switch: what the source brings.  Short enough for the
/// two lines the hint helper reserves (the state itself is shown by
/// `wnl-cli doctor`, `status.json` and the log).
pub fn uns_hint(settings: &Settings) -> String {
    if !settings.uns_enabled {
        return tr!(
            "Центр уведомлений через WinRT не опрашивается: записи идут только по базе Windows."
        )
        .to_string();
    }
    tf!(
        "Опрос через WinRT каждые {} с: тексты и сроки попадают в те же записи, что и данные из базы.",
        "WinRT is polled every {} s: its texts and times land in the same records as the data from the database.",
        settings.uns_poll_seconds
    )
}

/// Sentence under the feed switch: which file the new notifications go to and
/// what it will hold.
///
/// The effective file is named, not the folder the user typed: that is the only
/// thing that can be found afterwards.  The running counters are not repeated
/// here - `status.json`, `wnl-cli doctor` and the log carry them.
pub fn feed_hint(settings: &Settings, layout: &storage::Layout) -> String {
    let file = crate::feed::file_path(layout, settings);
    if !settings.feed_enabled {
        return tf!(
            "Выключено.\r\nФайл {} остаётся как есть.",
            "Switched off.\r\nFile {} is left as it is.",
            file.display()
        );
    }
    tf!(
        "Файл {}\r\nНовые уведомления после включения дописываются, старые строки не стираются.",
        "File {}\r\nNotifications that arrive after switching on are appended; old lines are not erased.",
        file.display()
    )
}

/// State of the settings window.
struct SettingsWindow {
    /// The window itself (set before the controls are created).
    hwnd: HWND,
    /// What the user is editing right now.
    draft: Settings,
    /// The settings in force (used by *Сбросить* and for the summary).
    current: Settings,
    layout: storage::Layout,
    dpi: u32,
    font: HFONT,
    bold_font: HFONT,
    /// Colours of the look the window was opened in (see `settings::Settings::theme`).
    ///
    /// The look is chosen *in* this window, so a change takes effect after «ОК»:
    /// repainting half of the controls while the user is typing would be a surprise.
    palette: style::Palette,
    controls: Vec<HWND>,
    home_value: HWND,
    /// Path field of the text feed (it gets the folder chosen with «Обзор…»).
    feed_value: HWND,
    /// Left edge and width of the column that is being filled right now.
    ///
    /// The window has two columns: twenty rows one under another are taller than
    /// the screen of a laptop, and the frame is wide enough for both.
    current_x: i32,
    current_width: i32,
    /// Filled in when the user saves; the caller reads it after the dialog.
    result: std::rc::Rc<std::cell::RefCell<Option<Settings>>>,
}

impl Drop for SettingsWindow {
    fn drop(&mut self) {
        for font in [self.font, self.bold_font] {
            if !font.is_null() {
                unsafe {
                    DeleteObject(font as HGDIOBJ);
                }
            }
        }
    }
}

/// A bold variant of the system font, used for the section headings.
fn bold_font(dpi: u32) -> HFONT {
    let mut log = message_font_log().unwrap_or_default();
    log.lf_weight = 700; // FW_BOLD
                         // The system metrics describe the primary monitor; correct for this one.
    if log.lf_height != 0 {
        let primary = 96i32;
        log.lf_height = -((-log.lf_height).max(1) * dpi.max(96) as i32 / primary);
    }
    create_font(&log).unwrap_or_else(|| system_font())
}

/// Create one child control (used before it is placed).
fn create_child(
    parent: HWND,
    instance: HINSTANCE,
    class: &str,
    text: &str,
    style: UINT,
    ex_style: UINT,
    id: usize,
) -> HWND {
    let class_w = wide(class);
    let text_w = wide(text);
    unsafe {
        CreateWindowExW(
            ex_style,
            class_w.as_ptr(),
            text_w.as_ptr(),
            style,
            0,
            0,
            0,
            0,
            parent,
            id as HMENU,
            instance,
            std::ptr::null_mut(),
        )
    }
}

/// Scale a logical value to the monitor of this window.
fn px(value: i32, dpi: u32) -> i32 {
    scale(value, dpi)
}

/// Labels of the export format list.
fn export_label(name: &str) -> &'static str {
    match export::Format::from_name(name) {
        export::Format::Csv => tr!("Таблица CSV (Excel)"),
        export::Format::Json => tr!("Данные JSON"),
        export::Format::Txt => tr!("Текст TXT"),
        export::Format::Html => tr!("Страница HTML"),
    }
}

/// Label of one text size step (the same words as in the «Вид» menu).
fn font_scale_label(scale: u32) -> &'static str {
    settings::font_scale_label(scale)
}

/// Read the whole state of the window into a `Settings` value.
fn collect_draft(state: &SettingsWindow) -> Settings {
    let mut draft = state.draft.clone();
    let checked = |control: HWND| unsafe {
        SendMessageW(control, BM_GETCHECK, 0, 0) as usize == BST_CHECKED as usize
    };
    draft.collect_on_start = checked(state.control(ID_COLLECT_ON_START));
    draft.hide_to_tray_on_close = checked(state.control(ID_CLOSE_TO_TRAY));
    draft.minimize_to_tray = checked(state.control(ID_MINIMIZE_TO_TRAY));
    draft.start_with_windows = checked(state.control(ID_START_WITH_WINDOWS));
    draft.start_minimized = checked(state.control(ID_START_MINIMIZED));
    draft.show_app_icons = checked(state.control(ID_APP_ICONS));
    draft.keep_pictures = checked(state.control(ID_KEEP_PICTURES));
    draft.log_enabled = checked(state.control(ID_LOG));
    draft.uns_enabled = checked(state.control(ID_UNS));
    draft.feed_enabled = checked(state.control(ID_FEED));
    let feed_dir = window_text(state.feed_value);
    draft.feed_dir = (!feed_dir.trim().is_empty()).then(|| PathBuf::from(feed_dir.trim()));
    draft.show_technical_columns = checked(state.control(ID_TECHNICAL_COLUMNS));
    draft.simple_view = checked(state.control(ID_SIMPLE_VIEW));
    // The night look is one setting with two values: the tick says which one.
    draft.theme = if checked(state.control(ID_DARK_THEME)) {
        settings::THEMES[1]
    } else {
        settings::THEMES[0]
    }
    .to_string();
    draft.collect_interval_seconds = number_from_text(
        &window_text(state.control(ID_INTERVAL)),
        i64::from(settings::MIN_INTERVAL_SECONDS),
        i64::from(settings::MAX_INTERVAL_SECONDS),
        i64::from(state.draft.collect_interval_seconds),
    ) as u32;
    draft.backfill_limit = number_from_text(
        &window_text(state.control(ID_BACKFILL)),
        0,
        settings::MAX_BACKFILL,
        state.draft.backfill_limit,
    );
    draft.retention_days = number_from_text(
        &window_text(state.control(ID_RETENTION)),
        0,
        settings::MAX_RETENTION_DAYS,
        state.draft.retention_days,
    );
    let selected = unsafe { SendMessageW(state.control(ID_EXPORT_FORMAT), CB_GETCURSEL, 0, 0) };
    if selected >= 0 {
        if let Some(name) = EXPORT_FORMATS.get(selected as usize) {
            draft.export_format = (*name).to_string();
        }
    }
    // The text size is a list of fixed steps, so a wrong index cannot happen.
    let font = unsafe { SendMessageW(state.control(ID_FONT_SCALE), CB_GETCURSEL, 0, 0) };
    if font >= 0 {
        draft.font_scale = settings::FONT_SCALES
            .get(font as usize)
            .copied()
            .unwrap_or(draft.font_scale);
    }
    let language = unsafe { SendMessageW(state.control(ID_LANGUAGE), CB_GETCURSEL, 0, 0) };
    if language >= 0 {
        if let Some(code) = settings::LANGUAGES.get(language as usize) {
            draft.language = (*code).to_string();
        }
    }
    let home = window_text(state.home_value);
    draft.data_dir = (!home.trim().is_empty()).then(|| PathBuf::from(home.trim()));
    draft.sanitize();
    draft
}

impl SettingsWindow {
    fn control(&self, id: usize) -> HWND {
        unsafe { GetDlgItem(self.hwnd, id as i32) }
    }

    fn window(&self) -> HWND {
        self.hwnd
    }

    /// Build every control of the settings window.
    ///
    /// The settings are laid out in two columns.  One column of twenty rows is
    /// taller than the screen of a laptop - the bottom row of buttons then sits
    /// below the visible area - and the frame is wide enough to hold both.
    fn create_children(&mut self, hwnd: HWND) {
        let instance = module_handle();
        let draft = self.draft.clone();
        let style = |extra: UINT| WS_CHILD | WS_VISIBLE | extra;
        let gap = px(GAP, self.dpi);
        let half = (self.inner_width() - gap) / 2;

        // ---- left column: what the collector does and how the program lives on.
        self.current_x = px(PADDING, self.dpi);
        self.current_width = half;
        let mut left = px(PADDING, self.dpi);
        left = self.group(hwnd, instance, left, "Сбор уведомлений");
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Собирать уведомления при запуске",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_COLLECT_ON_START,
            &mut left,
            draft.collect_on_start,
        );
        self.label_with_field(
            hwnd,
            instance,
            ID_INTERVAL_LABEL,
            "Проверять новые уведомления, секунды (1–3600):",
            ID_INTERVAL,
            &draft.collect_interval_seconds.to_string(),
            &mut left,
        );

        self.label_with_field(
            hwnd,
            instance,
            ID_BACKFILL_LABEL,
            "Прочитать прежних при первом запуске:",
            ID_BACKFILL,
            &draft.backfill_limit.to_string(),
            &mut left,
        );

        self.label_with_field(
            hwnd,
            instance,
            ID_RETENTION_LABEL,
            "Удалять записи старше, дней (0 — бессрочно):",
            ID_RETENTION,
            &draft.retention_days.to_string(),
            &mut left,
        );

        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Вести журнал работы (wnl.log)",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_LOG,
            &mut left,
            draft.log_enabled,
        );

        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Опрашивать центр уведомлений через WinRT",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_UNS,
            &mut left,
            draft.uns_enabled,
        );
        let uns = uns_hint(&draft);
        self.hint(hwnd, instance, ID_UNS_HINT, &uns, &mut left);

        left = self.group(hwnd, instance, left, "Окно и работа в фоне");
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "При закрытии продолжать работу в области уведомлений",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_CLOSE_TO_TRAY,
            &mut left,
            draft.hide_to_tray_on_close,
        );
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "При сворачивании убирать окно в область уведомлений",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_MINIMIZE_TO_TRAY,
            &mut left,
            draft.minimize_to_tray,
        );
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Запускать автоматически при входе в Windows",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_START_WITH_WINDOWS,
            &mut left,
            draft.start_with_windows,
        );
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Запускать сразу свёрнутым (без окна)",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_START_MINIMIZED,
            &mut left,
            draft.start_minimized,
        );

        let left_bottom = left;

        // ---- right column: how the list looks and where the data lives.
        self.current_x = px(PADDING, self.dpi) + half + gap;
        self.current_width = half;
        let mut right = px(PADDING, self.dpi);
        right = self.group(hwnd, instance, right, "Вид списка");
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Показывать значки приложений",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_APP_ICONS,
            &mut right,
            draft.show_app_icons,
        );
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Показывать технические столбцы (источник, повторы)",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_TECHNICAL_COLUMNS,
            &mut right,
            draft.show_technical_columns,
        );
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Простой вид — только лента (источник, содержание, значок)",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_SIMPLE_VIEW,
            &mut right,
            draft.simple_view,
        );
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Ночная тема (тёмное оформление окна и списка)",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_DARK_THEME,
            &mut right,
            draft.theme == settings::THEMES[1],
        );
        let formats: Vec<(String, bool)> = EXPORT_FORMATS
            .iter()
            .map(|name| {
                (
                    export_label(name).to_string(),
                    *name == draft.export_format.as_str(),
                )
            })
            .collect();
        self.combo(
            hwnd,
            instance,
            ID_EXPORT_LABEL,
            "Формат экспорта по умолчанию:",
            ID_EXPORT_FORMAT,
            &formats,
            &mut right,
        );
        let scales: Vec<(String, bool)> = settings::FONT_SCALES
            .iter()
            .map(|scale| (font_scale_label(*scale).to_string(), *scale == draft.font_scale))
            .collect();
        self.combo(
            hwnd,
            instance,
            ID_FONT_LABEL,
            "Размер текста:",
            ID_FONT_SCALE,
            &scales,
            &mut right,
        );
        // The language entries are never translated: each one is written the way
        // its own speakers write it, so somebody who cannot read the current
        // language can still find their own.
        let languages: Vec<(String, bool)> = settings::LANGUAGES
            .iter()
            .map(|code| {
                (
                    settings::language_label(code).to_string(),
                    *code == draft.language.as_str(),
                )
            })
            .collect();
        self.combo(
            hwnd,
            instance,
            ID_LANGUAGE_LABEL,
            "Язык интерфейса:",
            ID_LANGUAGE,
            &languages,
            &mut right,
        );

        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Сохранять картинки уведомлений рядом с журналом",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_KEEP_PICTURES,
            &mut right,
            draft.keep_pictures,
        );

        let rules = rules_hint(&draft);
        self.hint(hwnd, instance, ID_RULES_HINT, &rules, &mut right);

        right = self.group(hwnd, instance, right, "Папка данных");
        let home = draft
            .data_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.home_value = self.field(
            hwnd,
            instance,
            "EDIT",
            &home,
            style(WS_TABSTOP | ES_AUTOHSCROLL),
            WS_EX_CLIENTEDGE,
            ID_HOME,
            &mut right,
            ROW,
        );
        let buttons_y = right;
        let column_x = self.column_x();
        let small_button = px(BUTTON_WIDTH, self.dpi);
        self.button_at(
            hwnd,
            instance,
            "Обзор…",
            ID_HOME_BROWSE,
            column_x,
            buttons_y,
            small_button,
        );
        self.button_at(
            hwnd,
            instance,
            "Открыть",
            ID_HOME_OPEN,
            column_x + small_button + px(GAP, self.dpi),
            buttons_y,
            small_button,
        );
        right += px(34, self.dpi);
        self.hint(
            hwnd,
            instance,
            ID_HOME_HINT,
            &storage::describe(&self.layout),
            &mut right,
        );

        right = self.group(hwnd, instance, right, "Экспорт новых уведомлений");
        self.add(
            hwnd,
            instance,
            "BUTTON",
            "Дописывать новые уведомления в текстовый файл",
            style(WS_TABSTOP | BS_AUTOCHECKBOX),
            0,
            ID_FEED,
            &mut right,
            draft.feed_enabled,
        );
        let feed_dir = draft
            .feed_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.feed_value = self.field_with_button(
            hwnd,
            instance,
            &feed_dir,
            ID_FEED_DIR,
            "Обзор…",
            ID_FEED_BROWSE,
            &mut right,
        );
        let feed = feed_hint(&draft, &self.layout);
        self.hint_lines(hwnd, instance, ID_FEED_HINT, &feed, 3, &mut right);

        // The window is as tall as the taller column plus the row of buttons, so
        // nothing sits below the screen and nothing is wasted either.
        self.resize_to_content(hwnd, left_bottom.max(right));
        self.create_bottom_row(hwnd, instance);
    }

    /// Give the window the height the content asks for.
    fn resize_to_content(&self, hwnd: HWND, content_bottom: i32) {
        let dpi = self.dpi;
        let client_height = content_bottom
            + px(GAP, dpi)
            + px(ROW, dpi)
            + px(PADDING, dpi);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: px(WINDOW_WIDTH, dpi),
            bottom: client_height,
        };
        unsafe {
            AdjustWindowRectEx(
                &mut rect,
                WS_CAPTION | WS_SYSMENU,
                0,
                WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            );
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                rect.width(),
                rect.height(),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// The last row of the window: reset on the left, save and cancel on the right.
    fn create_bottom_row(&mut self, hwnd: HWND, instance: HINSTANCE) {
        let button = px(BUTTON_WIDTH, self.dpi);
        let bottom = self.client_height() - px(PADDING, self.dpi) - px(ROW, self.dpi);
        self.button_at(
            hwnd,
            instance,
            "Сбросить",
            ID_RESET,
            px(PADDING, self.dpi),
            bottom,
            button,
        );
        self.button_right(hwnd, instance, "Отмена", ID_CANCEL, BUTTON_WIDTH, 0, bottom);
        self.button_right(hwnd, instance, "Сохранить", ID_OK, BUTTON_WIDTH, 1, bottom);
    }

    fn client_height(&self) -> i32 {
        let mut rect = RECT::default();
        unsafe { GetClientRect(self.window(), &mut rect) };
        rect.height()
    }

    fn place(&self, control: HWND, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            MoveWindow(control, x, y, width, height, 1);
        }
    }

    fn client_width(&self) -> i32 {
        let mut rect = RECT::default();
        unsafe { GetClientRect(self.window(), &mut rect) };
        rect.width()
    }

    fn inner_width(&self) -> i32 {
        (self.client_width() - 2 * px(PADDING, self.dpi)).max(200)
    }

    /// Left edge of the column being filled.
    fn column_x(&self) -> i32 {
        self.current_x
    }

    /// Width of the column being filled.
    fn column_width(&self) -> i32 {
        self.current_width
    }

    /// Section heading; returns the y of the next row.
    ///
    /// A section is opened by air and by a heading in bold - no frame around it:
    /// a box around every group of four rows adds a line per group and tells the
    /// eye nothing, which is what makes a settings window look busy.
    fn group(&mut self, hwnd: HWND, instance: HINSTANCE, y: i32, title: &str) -> i32 {
        let control = create_child(
            hwnd,
            instance,
            "STATIC",
            title,
            WS_CHILD | WS_VISIBLE | style::LABEL,
            0,
            0,
        );
        unsafe {
            SendMessageW(control, WM_SETFONT, self.bold_font as WPARAM, 1);
        }
        self.controls.push(control);
        let top = y + px(SECTION_GAP - ROW_GAP, self.dpi);
        self.place(
            control,
            self.column_x(),
            top,
            self.column_width(),
            px(GROUP_TITLE_HEIGHT, self.dpi),
        );
        top + px(GROUP_TITLE_HEIGHT + GAP, self.dpi)
    }

    /// A check box or a caption on a line of its own.
    ///
    /// `checked` is ignored for labels; a caption is centred in its row so that
    /// it lines up with the field on the same row (see `label_with_field`).
    fn add(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        class: &str,
        text: &str,
        style: UINT,
        ex_style: UINT,
        id: usize,
        y: &mut i32,
        checked: bool,
    ) {
        let controls_style = if class == "STATIC" {
            style | style::LABEL
        } else {
            style
        };
        let control = create_child(hwnd, instance, class, text, controls_style, ex_style, id);
        unsafe {
            SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1);
            if class == "BUTTON" && checked {
                SendMessageW(control, BM_SETCHECK, BST_CHECKED, 0);
            }
        }
        self.controls.push(control);
        let height = px(ROW, self.dpi);
        self.place(
            control,
            self.column_x(),
            *y,
            self.column_width(),
            height,
        );
        *y += height + px(ROW_GAP, self.dpi);
    }

    /// A caption and the small number field that belongs to it, on one line.
    fn label_with_field(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        label_id: usize,
        caption: &str,
        field_id: usize,
        value: &str,
        y: &mut i32,
    ) {
        let height = px(ROW, self.dpi);
        let field_width = px(FIELD_WIDTH, self.dpi);
        let label = create_child(
            hwnd,
            instance,
            "STATIC",
            caption,
            WS_CHILD | WS_VISIBLE | style::LABEL,
            0,
            label_id,
        );
        let field = create_child(
            hwnd,
            instance,
            "EDIT",
            value,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL | ES_NUMBER,
            WS_EX_CLIENTEDGE,
            field_id,
        );
        unsafe {
            SendMessageW(field, WM_SETFONT, self.font as WPARAM, 1);
            SendMessageW(field, EM_SETLIMITTEXT, 12, 0);
            SendMessageW(label, WM_SETFONT, self.font as WPARAM, 1);
        }
        self.controls.push(label);
        self.controls.push(field);
        // The label gets the very box of the field: `SS_CENTERIMAGE` centres its
        // text inside it, so the two share one line at any text size.
        self.place(
            label,
            self.column_x(),
            *y,
            self.column_width() - field_width - px(GAP, self.dpi),
            height,
        );
        self.place(
            field,
            self.column_x() + self.column_width() - field_width,
            *y,
            field_width,
            height,
        );
        *y += height + px(ROW_GAP, self.dpi);
    }

    /// A note under the control it explains.
    fn hint(&mut self, hwnd: HWND, instance: HINSTANCE, id: usize, text: &str, y: &mut i32) {
        self.hint_lines(hwnd, instance, id, text, 2, y);
    }

    /// The same note with an explicit number of lines.
    ///
    /// A note that carries a path needs the third line: what does not fit is cut
    /// off by the control instead of making the window taller.
    fn hint_lines(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        id: usize,
        text: &str,
        lines: i32,
        y: &mut i32,
    ) {
        let control = create_child(
            hwnd,
            instance,
            "STATIC",
            text,
            WS_CHILD | WS_VISIBLE | style::HINT,
            0,
            id,
        );
        unsafe {
            SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1);
        }
        self.controls.push(control);
        // Room for the lines the note needs; it wraps by itself when a line is
        // longer than the column.
        let height = px(lines * LINE, self.dpi);
        self.place(control, self.column_x(), *y, self.column_width(), height);
        *y += height + px(ROW_GAP, self.dpi);
    }

    /// A full width control of one row.
    fn field(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        class: &str,
        text: &str,
        style: UINT,
        ex_style: UINT,
        id: usize,
        y: &mut i32,
        height: i32,
    ) -> HWND {
        let control = create_child(hwnd, instance, class, text, style, ex_style, id);
        unsafe {
            SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1);
        }
        self.controls.push(control);
        self.place(
            control,
            self.column_x(),
            *y,
            self.column_width(),
            px(height, self.dpi),
        );
        *y += px(height, self.dpi) + px(4, self.dpi);
        control
    }

    /// A text field with its own button on the same line.
    ///
    /// Used where a path and the action that changes it belong together: the
    /// field keeps the rest of the column and the button stays a standard width.
    fn field_with_button(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        value: &str,
        field_id: usize,
        button_text: &str,
        button_id: usize,
        y: &mut i32,
    ) -> HWND {
        let height = px(ROW, self.dpi);
        let button = px(BUTTON_WIDTH, self.dpi);
        let gap = px(GAP, self.dpi);
        let field_width = (self.column_width() - button - gap).max(px(FIELD_WIDTH, self.dpi));
        let control = create_child(
            hwnd,
            instance,
            "EDIT",
            value,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
            WS_EX_CLIENTEDGE,
            field_id,
        );
        unsafe {
            SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1);
        }
        self.controls.push(control);
        self.place(control, self.column_x(), *y, field_width, height);
        self.button_at(
            hwnd,
            instance,
            button_text,
            button_id,
            self.column_x() + field_width + gap,
            *y,
            button,
        );
        *y += height + px(ROW_GAP, self.dpi);
        control
    }

    /// A button at an explicit position (physical pixels, as `place` wants them).
    fn button_at(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        text: &str,
        id: usize,
        x: i32,
        y: i32,
        width: i32,
    ) {
        let control = create_child(
            hwnd,
            instance,
            "BUTTON",
            text,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            id,
        );
        unsafe {
            SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1);
        }
        self.controls.push(control);
        self.place(control, x, y, width, px(ROW, self.dpi));
    }

    /// A button at the right edge, `order` buttons away from it.
    fn button_right(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        text: &str,
        id: usize,
        width_logical: i32,
        order: i32,
        y: i32,
    ) {
        // The whole calculation stays in one unit and is converted once:
        // converting a pixel position back to a logical one (and back again) is
        // what used to shift the buttons of this window off the right edge on a
        // scaled display.
        let width = px(width_logical, self.dpi);
        let gap = px(GAP, self.dpi);
        let x = px(PADDING, self.dpi) + self.inner_width() - (width + gap) * (order + 1) + gap;
        self.button_at(hwnd, instance, text, id, x, y, width);
    }

    /// A caption and a drop down list on one line.
    ///
    /// The caption is created here as well, so the two cannot end up on
    /// different rows: the list sits in the very box of its caption.
    fn combo(
        &mut self,
        hwnd: HWND,
        instance: HINSTANCE,
        label_id: usize,
        caption: &str,
        id: usize,
        entries: &[(String, bool)],
        y: &mut i32,
    ) {
        let height = px(ROW, self.dpi);
        let width = px(200, self.dpi);
        let label = create_child(
            hwnd,
            instance,
            "STATIC",
            caption,
            WS_CHILD | WS_VISIBLE | style::LABEL,
            0,
            label_id,
        );
        let control = create_child(
            hwnd,
            instance,
            "COMBOBOX",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
            WS_EX_CLIENTEDGE,
            id,
        );
        unsafe {
            SendMessageW(label, WM_SETFONT, self.font as WPARAM, 1);
            SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1);
            for (index, (text, selected)) in entries.iter().enumerate() {
                let text = wide(text);
                SendMessageW(control, CB_ADDSTRING, 0, text.as_ptr() as LPARAM);
                if *selected {
                    SendMessageW(control, CB_SETCURSEL, index, 0);
                }
            }
            if SendMessageW(control, CB_GETCURSEL, 0, 0) < 0 {
                SendMessageW(control, CB_SETCURSEL, 0, 0);
            }
        }
        self.controls.push(label);
        self.controls.push(control);
        self.place(
            label,
            self.column_x(),
            *y,
            self.column_width() - width - px(GAP, self.dpi),
            height,
        );
        // The dropped down list needs the extra room below the field.
        self.place(
            control,
            self.column_x() + self.column_width() - width,
            *y,
            width,
            height + px(200, self.dpi),
        );
        *y += height + px(ROW_GAP, self.dpi);
    }
}

/// Show the settings window modally; returns the new settings when saved.
pub fn show(
    parent: HWND,
    current: &Settings,
    layout: &storage::Layout,
    dpi: u32,
) -> Option<Settings> {
    unsafe {
        register_class();
        let result: std::rc::Rc<std::cell::RefCell<Option<Settings>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let state = Box::new(SettingsWindow {
            hwnd: std::ptr::null_mut(),
            draft: current.clone(),
            current: current.clone(),
            layout: layout.clone(),
            dpi,
            font: system_font(),
            bold_font: bold_font(dpi),
            palette: style::Palette::from_setting(&current.theme),
            controls: Vec::new(),
            home_value: std::ptr::null_mut(),
            feed_value: std::ptr::null_mut(),
            current_x: 0,
            current_width: 0,
            result: result.clone(),
        });
        let raw = Box::into_raw(state);
        let class = wide(WINDOW_CLASS);
        let title = wide(WINDOW_TITLE);
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT | WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            px(WINDOW_WIDTH, dpi),
            px(WINDOW_HEIGHT, dpi),
            parent,
            std::ptr::null_mut(),
            module_handle(),
            raw as *mut std::ffi::c_void,
        );
        if hwnd.is_null() {
            drop(Box::from_raw(raw));
            return None;
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let state = &mut *raw;
        state.hwnd = hwnd;
        state.create_children(hwnd);
        style::apply_look_to_window(hwnd, &state.palette, &state.controls.clone());
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        if !parent.is_null() {
            EnableWindow(parent, 0);
        }

        // Modal loop: the caller waits until the window is gone.
        let mut message: MSG = std::mem::zeroed();
        while IsWindow(hwnd) != 0 {
            let status = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
            if status <= 0 {
                break;
            }
            if IsDialogMessageW(hwnd, &mut message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if !parent.is_null() {
            EnableWindow(parent, 1);
            SetForegroundWindow(parent);
        }
        let outcome = result.borrow().as_ref().cloned();
        outcome
    }
}

/// Register the window class of the settings window once.
fn register_class() {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| unsafe {
        let class_name = wide(WINDOW_CLASS);
        let class = WNDCLASSEXW {
            lpfn_wnd_proc: Some(window_proc),
            h_instance: module_handle(),
            h_icon: crate::appicon::application_icon(),
            h_cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbr_background: COLOR_WINDOW_BRUSH,
            lpsz_class_name: class_name.as_ptr(),
            ..Default::default()
        };
        RegisterClassExW(&class);
    });
}

/// State of a window from its `GWLP_USERDATA`.
unsafe fn state_of(hwnd: HWND) -> Option<&'static mut SettingsWindow> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindow;
    raw.as_mut()
}

/// Remove every control (used before rebuilding the window contents).
fn destroy_children(state: &mut SettingsWindow) {
    for control in state.controls.drain(..) {
        unsafe {
            DestroyWindow(control);
        }
    }
}

/// Show a folder in Explorer.
pub fn open_folder(hwnd: HWND, folder: &str) {
    let operation = wide("open");
    let path = wide(folder);
    unsafe {
        ShellExecuteW(
            hwnd,
            operation.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Secondary text of the settings window: the note that explains the data
/// folder.  It is drawn in the system's grey, on the window background.
fn is_muted(id: i32) -> bool {
    matches!(
        id as usize,
        ID_HOME_HINT | ID_RULES_HINT | ID_UNS_HINT | ID_FEED_HINT
    )
}

/// Window procedure of the settings window.
unsafe extern "system" fn window_proc(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        // A label keeps no background of its own (see `style::paint_label`).
        WM_CTLCOLORSTATIC => {
            let control = l as HWND;
            match state_of(hwnd).map(|state| state.palette) {
                Some(palette) => {
                    let muted = is_muted(GetDlgCtrlID(control));
                    match style::paint_control(w as HDC, control, &palette, muted) {
                        Some(background) => background as LRESULT,
                        None => DefWindowProcW(hwnd, message, w, l),
                    }
                }
                None => DefWindowProcW(hwnd, message, w, l),
            }
        }
        WM_ERASEBKGND => match state_of(hwnd) {
            Some(state) => style::erase_background(hwnd, w as HDC, &state.palette),
            None => DefWindowProcW(hwnd, message, w, l),
        },
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => match state_of(hwnd) {
            // The fields of this window and its buttons: in the night look a white
            // field on a dark window would be the first thing the eye sees.
            Some(state) => style::paint_surface(message, w as HDC, &state.palette) as LRESULT,
            None => DefWindowProcW(hwnd, message, w, l),
        },
        WM_COMMAND => {
            let id = (w & 0xFFFF) as usize;
            if let Some(state) = state_of(hwnd) {
                match id {
                    ID_OK => {
                        let draft = collect_draft(state);
                        *state.result.borrow_mut() = Some(draft);
                        DestroyWindow(hwnd);
                    }
                    ID_CANCEL => {
                        DestroyWindow(hwnd);
                    }
                    ID_RESET => {
                        // Defaults, but the folder in use stays where it is.
                        let home = collect_draft(state).data_dir;
                        state.draft = Settings {
                            data_dir: home,
                            ..Settings::default()
                        };
                        destroy_children(state);
                        state.create_children(hwnd);
                    }
                    ID_HOME_BROWSE => {
                        if let Some(chosen) =
                            pick_folder(hwnd, "Куда сохранять журнал уведомлений?")
                        {
                            set_window_text(state.home_value, &chosen.display().to_string());
                        }
                    }
                    ID_HOME_OPEN => {
                        let folder = window_text(state.home_value);
                        let folder = if folder.trim().is_empty() {
                            state.layout.data_dir.display().to_string()
                        } else {
                            folder
                        };
                        open_folder(hwnd, &folder);
                    }
                    ID_FEED_BROWSE => {
                        if let Some(chosen) =
                            pick_folder(hwnd, "Куда дописывать новые уведомления?")
                        {
                            set_window_text(state.feed_value, &chosen.display().to_string());
                        }
                    }
                    // The note under the switch follows the switch, so the window
                    // never claims "выключено" about a feed the user just turned
                    // on (the dialog stays open while the setting is decided).
                    ID_FEED => {
                        let draft = collect_draft(state);
                        set_window_text(
                            state.control(ID_FEED_HINT),
                            &feed_hint(&draft, &state.layout),
                        );
                    }
                    _ => {}
                }
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_NCDESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindow;
            if !raw.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(raw));
            }
            DefWindowProcW(hwnd, message, w, l)
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn the_rules_note_names_what_is_in_force() {
        let mut settings = Settings::default();
        assert!(rules_hint(&settings).contains("нет"), "правил нет — так и сказано");

        settings.rules = crate::rules::RuleSet::new(vec![
            crate::rules::Rule {
                name: "тишина".to_string(),
                action: crate::rules::ACTION_SKIP.to_string(),
                ..Default::default()
            },
            crate::rules::Rule {
                name: "выкл".to_string(),
                enabled: false,
                ..Default::default()
            },
        ]);
        assert!(rules_hint(&settings).contains("в силе 1 из 2"));
        settings.rules_enabled = false;
        assert!(rules_hint(&settings).contains("выключены"));
    }

    #[test]
    fn the_winrt_switch_explains_itself() {
        let mut settings = Settings::default();
        assert!(settings.uns_enabled, "источник включён по умолчанию");
        let hint = uns_hint(&settings);
        assert!(hint.contains("WinRT"), "{hint}");
        assert!(
            hint.contains(&settings.uns_poll_seconds.to_string()),
            "интервал виден в подсказке: {hint}"
        );
        settings.uns_enabled = false;
        assert!(uns_hint(&settings).contains("не опрашивается"));
    }

    #[test]
    fn numbers_are_clamped_and_typos_keep_the_old_value() {
        assert_eq!(number_from_text("15", 1, 3600, 3), 15);
        assert_eq!(number_from_text("  0 ", 1, 3600, 3), 1, "below the range");
        assert_eq!(
            number_from_text("99999", 1, 3600, 3),
            3600,
            "above the range"
        );
        assert_eq!(
            number_from_text("", 1, 3600, 7),
            7,
            "empty keeps the old value"
        );
        assert_eq!(
            number_from_text("двенадцать", 1, 3600, 7),
            7,
            "garbage keeps the old value"
        );
        assert_eq!(number_from_text("-4", 0, 3650, 30), 0);
    }

    #[test]
    fn the_feed_note_names_the_file() {
        let layout = storage::Layout::build(
            std::path::Path::new(r"D:\данные"),
            std::path::PathBuf::from(r"D:\данные\settings.json"),
            false,
            false,
        );
        let off = Settings::default();
        let text = feed_hint(&off, &layout);
        assert!(text.contains("Выключено"), "{text}");
        assert!(text.contains("notifications.txt"), "{text}");
        assert!(
            text.contains(&layout.exports.display().to_string()),
            "выключенная лента всё равно называет файл: {text}"
        );

        let on = Settings {
            feed_enabled: true,
            feed_dir: Some(std::path::PathBuf::from(r"D:\на рабочий стол")),
            ..Settings::default()
        };
        let text = feed_hint(&on, &layout);
        assert!(text.contains("после включения"), "{text}");
        assert!(text.contains("на рабочий стол"), "{text}");
    }

    #[test]
    fn summary_describes_the_current_run() {
        let settings = Settings {
            collect_interval_seconds: 10,
            retention_days: 30,
            ..Settings::default()
        };
        let layout = storage::Layout::build(
            std::path::Path::new(r"D:\журнал"),
            std::path::PathBuf::from(r"D:\журнал\settings.json"),
            false,
            false,
        );
        let text = summary(&settings, &layout);
        assert!(text.contains("каждые 10 с"));
        assert!(text.contains("30 дн."));
        assert!(text.contains(r"D:\журнал"));

        let forever = Settings::default();
        assert!(summary(&forever, &layout).contains("бессрочно"));
    }

    #[test]
    fn export_labels_match_the_formats() {
        assert!(export_label("csv").contains("CSV"));
        assert!(export_label("json").contains("JSON"));
        assert!(export_label("txt").contains("TXT"));
        assert!(export_label("html").contains("HTML"));
    }
}
