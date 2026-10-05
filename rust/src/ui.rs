//! The window of Win Notification Log.
//!
//! The visible surface is deliberately plain: a search box, a handful of buttons,
//! a list that shows what the notification said and who sent it, and a status
//! line that explains what the program is doing.  Technical details live in the
//! details window and in the optional technical columns.
//!
//! Collection happens in a background thread (`service`), which means the window
//! never freezes while the Windows notification database is scanned; the window
//! only reads the journal.  Closing the window keeps the process alive in the
//! notification area (a setting, not a surprise), *Выход* in the tray menu quits.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::appname::NameResolver;
use crate::settings::Settings;
use crate::sqlite::Row;
use crate::storage::Layout;
use crate::store::{Journal, SearchQuery};
use crate::view::{self, Column};
use crate::win32::*;
use crate::{
    appicon, autostart, detail, export, imaging, purge, service, settings_ui, storage, style, theme,
    time, VERSION,
};

const WINDOW_CLASS: &str = "WinNotificationLogNative";
const TRAY_ID: UINT = 1;
/// Rows fetched per listing request; *Ещё* appends the next page.
const PAGE_SIZE: i64 = 300;
/// Rows written by an export.
const EXPORT_LIMIT: i64 = 5000;
/// How often the window looks at the journal and at the collector state.
const REFRESH_MS: u32 = 1000;
const SEARCH_DELAY_MS: u32 = 250;

const TIMER_REFRESH: usize = 1;
const TIMER_SEARCH: usize = 2;
/// Debounce of the custom date fields.
const TIMER_FILTER: usize = 3;

// Control identifiers (also the child window ids).
const ID_SEARCH_LABEL: usize = 101;
const ID_SEARCH: usize = 102;
/// The button that reads new notifications right now (*Проверить сейчас*).
const ID_IMPORT: usize = 103;
/// Kept free: the toolbar button that merely re-read the journal was removed
/// (the list refreshes itself; the menu keeps the command as *Обновить список*).
const ID_MORE: usize = 105;
const ID_EXPORT: usize = 106;
const ID_SETTINGS: usize = 107;
const ID_FOLDER: usize = 108;
const ID_LIST: usize = 109;
/// Status line: three plain labels instead of the common-controls status bar.
///
/// The status bar of `comctl32` was the one component whose painting
/// occasionally crashed the process (see the README on diagnostics), and the
/// three texts are all the application needs.  Plain labels are cheaper, look
/// the same and can be read back for the diagnostics.
const ID_STATUS: usize = 110;
const ID_STATUS_MIDDLE: usize = 111;
const ID_STATUS_RIGHT: usize = 112;
/// Second line of the status area: what is currently filtered.
const ID_STATUS_FILTERS: usize = 123;
/// Thin rules that separate the header, the list and the status line.
const ID_RULE_FILTERS: usize = 124;
const ID_RULE_STATUS: usize = 125;

// Filter row: the criteria the journal can answer directly.
const ID_FILTER_APP_LABEL: usize = 113;
const ID_FILTER_APP: usize = 114;
const ID_FILTER_KIND: usize = 115;
const ID_FILTER_SOURCE: usize = 116;
const ID_FILTER_PERIOD: usize = 117;
const ID_FILTER_FROM: usize = 118;
const ID_FILTER_TO: usize = 119;
const ID_FILTER_RESET: usize = 120;
const ID_FILTER_FROM_LABEL: usize = 121;
const ID_FILTER_TO_LABEL: usize = 122;

// Menu commands, above the control range so the two never collide.
const ID_MENU_IMPORT: usize = 201;
const ID_MENU_RESET: usize = 202;
const ID_MENU_EXPORT: usize = 203;
const ID_MENU_FOLDER: usize = 204;
const ID_MENU_REFRESH: usize = 205;
const ID_MENU_ABOUT: usize = 206;
const ID_MENU_EXIT: usize = 207;
const ID_MENU_SETTINGS: usize = 208;
const ID_MENU_PAUSE: usize = 209;
const ID_MENU_ICONS: usize = 210;
const ID_MENU_TECHNICAL: usize = 211;
const ID_MENU_OPEN: usize = 212;
const ID_MENU_HIDE_DUPLICATES: usize = 213;
/// The feed: the window without the toolbar, the filters and the status lines.
const ID_MENU_SIMPLE_VIEW: usize = 217;
/// Deletion commands: one record selection, everything filtered, a period.
const ID_MENU_DELETE_SELECTED: usize = 214;
const ID_MENU_DELETE_MATCHING: usize = 215;
const ID_MENU_DELETE_PERIOD: usize = 216;
/// Font sizes occupy `ID_MENU_FONT_FIRST .. ID_MENU_FONT_FIRST + FONT_SCALES.len()`.
const ID_MENU_FONT_FIRST: usize = 230;
/// `Вид → Столбцы ленты`: one entry per column of the feed (`view::FEED_COLUMN_KEYS`).
const ID_MENU_FEED_COLUMN_FIRST: usize = 240;
/// `Вид → Строк в строке ленты`: one entry per offered number (`settings::FEED_LINES`).
const ID_MENU_FEED_LINES_FIRST: usize = 250;
/// `Вид → Тема оформления`: one entry per look (`settings::THEMES`).
const ID_MENU_THEME_FIRST: usize = 260;

/// One group of menu entries that share a range of ids.
///
/// The range is what makes such a group small: one entry per value of a table, and
/// the id says which value is meant.  The ranges have to stay apart - a range that is
/// too wide silently swallows the ids of the next group, and the entry then does
/// nothing at all (`the_menu_group_ids_do_not_overlap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuGroup {
    Font,
    FeedColumn,
    FeedLines,
    Theme,
}

/// The groups, their first id and how many entries each one offers.
fn menu_groups() -> [(MenuGroup, usize, usize); 4] {
    [
        (
            MenuGroup::Font,
            ID_MENU_FONT_FIRST,
            crate::settings::FONT_SCALES.len(),
        ),
        (
            MenuGroup::FeedColumn,
            ID_MENU_FEED_COLUMN_FIRST,
            view::FEED_COLUMN_KEYS.len(),
        ),
        (
            MenuGroup::FeedLines,
            ID_MENU_FEED_LINES_FIRST,
            crate::settings::FEED_LINES.len(),
        ),
        (
            MenuGroup::Theme,
            ID_MENU_THEME_FIRST,
            crate::settings::THEMES.len(),
        ),
    ]
}

impl MenuGroup {
    /// Which group an id belongs to, and which entry of it.
    fn of(id: usize) -> Option<(MenuGroup, usize)> {
        for (group, first, count) in menu_groups() {
            if (first..first + count).contains(&id) {
                return Some((group, id - first));
            }
        }
        None
    }
}

// Context menu of the list (right click on a notification).
const ID_CTX_OPEN: usize = 401;
const ID_CTX_COPY_TEXT: usize = 402;
const ID_CTX_COPY_TITLE: usize = 403;
const ID_CTX_COPY_LINK: usize = 404;
const ID_CTX_OPEN_LINK: usize = 405;
const ID_CTX_FILTER_APP: usize = 406;
const ID_CTX_FILTER_KIND: usize = 407;
const ID_CTX_EXPORT: usize = 408;
const ID_CTX_REFRESH: usize = 410;
const ID_CTX_RESET_FILTERS: usize = 411;
const ID_CTX_DELETE_SELECTED: usize = 412;
const ID_CTX_DELETE_MATCHING: usize = 413;
const ID_CTX_DELETE_PERIOD: usize = 414;
/// Open the picture a notification carried (a file, or the address in a browser).
const ID_CTX_IMAGE: usize = 415;
/// Switch between the feed and the whole program from the list itself.
const ID_CTX_SIMPLE_VIEW: usize = 416;

const ID_TRAY_OPEN: usize = 301;
const ID_TRAY_IMPORT: usize = 302;
const ID_TRAY_EXIT: usize = 303;
const ID_TRAY_PAUSE: usize = 304;
const ID_TRAY_SETTINGS: usize = 305;
const ID_TRAY_FOLDER: usize = 306;
/// The feed, straight from the notification area.
const ID_TRAY_SIMPLE_VIEW: usize = 307;

/// Margins and heights in logical pixels (96 DPI).
///
/// The numbers come from `style`, so this window keeps the same rhythm as the
/// dialogs: one outer margin, one gap, one height for a row of controls.  The
/// only sizes that belong here are the widths of the controls themselves.
const PADDING: i32 = style::PADDING;
const GAP: i32 = style::GAP;
const ROW_GAP: i32 = style::ROW_GAP;
const ROW_HEIGHT: i32 = style::ROW;
const LINE_HEIGHT: i32 = style::LINE;
const RULE_HEIGHT: i32 = 1;
/// Room above and below the text inside one row of the list.
const ROW_PADDING: i32 = 4;
/// Room between the text and the edge of its column, and between the icon and the
/// text after it.
const CELL_PADDING: i32 = 5;
const SEARCH_LABEL_WIDTH: i32 = 50;
// The widths are chosen so that the whole filter row still fits on one line on a
// 1280 pixel wide display at 125 % text size; when it does not fit it wraps, and
// a wrapped «Сбросить» on a line of its own looks like a mistake.
const FILTER_APP_WIDTH: i32 = 188;
const FILTER_KIND_WIDTH: i32 = 104;
const FILTER_SOURCE_WIDTH: i32 = 104;
const FILTER_PERIOD_WIDTH: i32 = 136;
const FILTER_DATE_WIDTH: i32 = 90;
const FILTER_RESET_WIDTH: i32 = 96;
/// Width of the word that opens the filter row (`Фильтры:`).
const FILTER_WORD_WIDTH: i32 = 52;
const FILTER_DATE_LABEL_WIDTH: i32 = 14;
const FILTER_DATE_TO_LABEL_WIDTH: i32 = 22;
/// The words `с` and `по` belong to the field that follows them, so they are
/// separated from it by less than the standard gap.
const FILTER_PAIR_GAP: i32 = 4;
/// Width of the buttons in the toolbar row.
///
/// These are the widths a caption needs when it cannot be measured (see
/// `App::button_width`): the layout asks the font for the real width of every
/// caption, so a longer word does not get cut off and a short one does not leave a
/// hole next to it.
const BUTTON_FOLDER_WIDTH: i32 = 84;
const BUTTON_SETTINGS_WIDTH: i32 = 120;
const BUTTON_EXPORT_WIDTH: i32 = 116;
const BUTTON_MORE_WIDTH: i32 = 96;
const BUTTON_IMPORT_WIDTH: i32 = 172;
/// Room a caption needs inside its frame, and the limits of a measured button.
const BUTTON_PADDING: i32 = 18;
const BUTTON_MIN_WIDTH: i32 = 56;
const BUTTON_MAX_WIDTH: i32 = 220;
/// Least width of the search field.  Below this it moves to a row of its own
/// instead of being squeezed to a slit.
const SEARCH_MIN_WIDTH: i32 = 140;

/// Which part of a record the context menu copies to the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyPart {
    Everything,
    Title,
    Link,
}

/// Effective scaling of the window: monitor DPI combined with the text size.
///
/// Everything the window lays out goes through this one number, so a bigger
/// font also means taller rows, wider columns and larger icons - not text that
/// only fits half of the interface.
fn effective_scale(dpi: u32, font_scale: u32) -> u32 {
    let percent = font_scale.clamp(
        crate::settings::MIN_FONT_SCALE,
        crate::settings::MAX_FONT_SCALE,
    );
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((dpi as u64 * percent as u64) / 100) as u32
}

/// Label of the *Ещё* button: the number of records left to show.
fn more_button_label(remaining: i64) -> String {
    if remaining > 0 {
        tf!("Ещё {remaining}", "{remaining} more")
    } else {
        tr!("Ещё").to_string()
    }
}

/// `1 запись`, `3 записи`, `428 записей` - a count with the right word.
///
/// Deletion is irreversible, so the question has to say how many records it is
/// about, and a number followed by "записей" in every case reads like a machine.
fn records_word(count: i64) -> String {
    let word = match (count % 10, count % 100) {
        (1, rest) if rest != 11 => tr!("запись"),
        (2..=4, rest) if !(12..=14).contains(&rest) => tr!("записи"),
        _ => tr!("записей"),
    };
    format!("{count} {word}")
}

/// A short reminder of which records a deletion means: `«Письмо», «Счёт» и ещё 4`.
fn sample_titles(titles: &[String]) -> String {
    /// How many titles still fit into a confirmation without becoming a wall.
    const SHOWN: usize = 3;
    let mut parts: Vec<String> = titles
        .iter()
        .take(SHOWN)
        .map(|title| format!("«{title}»"))
        .collect();
    if titles.len() > SHOWN {
        parts.push(tf!(
            "и ещё {}",
            "and {} more",
            titles.len() - SHOWN
        ));
    }
    parts.join(", ")
}

/// How long a just-finished action keeps the middle status field.
const NOTE_SECONDS: u64 = 5;

/// Placement of the toolbar: the width of the search field and the row every
/// button lands on.
///
/// Everything stays on one line as long as the field keeps `search_min` beside the
/// buttons; otherwise the field takes the first row and the buttons flow
/// underneath it.  A window that is dragged narrow therefore re-arranges the
/// toolbar instead of drawing one button over the next.
fn toolbar_rows(
    inner: i32,
    label: i32,
    gap: i32,
    search_min: i32,
    buttons: &[i32],
) -> (i32, Vec<usize>) {
    let mut rows = vec![0usize; buttons.len()];
    if inner <= 0 {
        return (0, rows);
    }
    let after_label = label + gap;
    let buttons_width: i32 = buttons.iter().sum::<i32>() + gap * buttons.len() as i32;
    if inner - after_label - search_min - gap >= buttons_width {
        return (inner - after_label - gap - buttons_width, rows);
    }
    let mut row = 1usize;
    let mut x = 0;
    for (index, width) in buttons.iter().enumerate() {
        if x > 0 && x + width > inner {
            row += 1;
            x = 0;
        }
        rows[index] = row;
        x += width + gap;
    }
    ((inner - after_label).max(0), rows)
}

/// Widths of the three fields of the status line (left, middle, right).
///
/// The three share the line proportionally; when the window is so narrow that a
/// field would only show an ellipsis, it is dropped (`0`) instead, and the fields
/// that remain take its place.
fn status_widths(inner: i32, gap: i32, least: i32) -> (i32, i32, i32) {
    if inner <= 0 {
        return (0, 0, 0);
    }
    let left = inner * 36 / 100;
    let middle = inner * 34 / 100;
    let right = inner - left - middle - 2 * gap;
    if right >= least {
        return (left, middle, right);
    }
    let left = inner * 52 / 100;
    let middle = inner - left - gap;
    if middle >= least {
        return (left, middle, 0);
    }
    (inner, 0, 0)
}

/// Width of the source column of the feed: about a third of the row, kept between
/// a readable minimum and a width that still leaves the message room.
///
/// The two columns always add up to what the list has: a horizontal scroll bar in a
/// narrow strip would hide more than it shows, so a very small window lets the
/// minimum give way.
fn feed_source_width(inner: i32, least: i32, most: i32) -> i32 {
    let (least, most) = if least <= most { (least, most) } else { (most, least) };
    let share = inner * 30 / 100;
    // The message keeps at least a quarter of the row.
    let room = inner - (inner / 4).max(0);
    share.clamp(least.min(room), most).min(room).max(0)
}

/// Widths of the columns of the feed: the fixed ones in front of the message and the
/// message column itself, which takes the room that is left.
///
/// A table that ends in the middle of the window looks broken, so the last column is
/// always the one that grows.  In a window that is too narrow for the fixed columns
/// and the least room of the message together, they shrink by the same share - a
/// horizontal scroll bar in a narrow strip would hide more than it shows.
fn feed_column_widths(inner: i32, fixed: &[i32], least: i32) -> Vec<i32> {
    let inner = inner.max(0);
    let least = least.max(0);
    let wanted: i32 = fixed.iter().sum();
    let budget = (inner - least).max(0);
    let mut widths: Vec<i32> = Vec::with_capacity(fixed.len() + 1);
    if wanted <= budget || wanted == 0 {
        widths.extend_from_slice(fixed);
    } else {
        for width in fixed {
            widths.push((width * budget) / wanted);
        }
    }
    let used: i32 = widths.iter().sum();
    widths.push((inner - used).max(0));
    widths
}

/// Least size a window of this shape may be dragged to (logical pixels).
///
/// The feed is a strip and is allowed to look like one; the full program needs room
/// for the toolbar, the filter row, the list and the status line.
fn minimum_window_size(simple_view: bool) -> (i32, i32) {
    if simple_view {
        (
            crate::settings::MIN_FEED_WIDTH,
            crate::settings::MIN_FEED_HEIGHT,
        )
    } else {
        (
            crate::settings::MIN_WINDOW_WIDTH,
            crate::settings::MIN_WINDOW_HEIGHT,
        )
    }
}

/// Text of the middle status field: what just happened, or what the collector says.
///
/// Pure, so the "the message disappears again" rule can be checked without a
/// window.
fn middle_status_text(note: Option<(&str, Duration)>, collector: &str) -> String {
    match note {
        Some((text, age)) if age < Duration::from_secs(NOTE_SECONDS) => text.to_string(),
        _ => collector.to_string(),
    }
}

/// Menu entry for deleting the selection; one record is named as such.
fn delete_selected_label(count: i64) -> String {
    if count <= 1 {
        "Удалить запись из журнала…".to_string()
    } else {
        tf!("Удалить выбранные ({count})…", "Delete the selected ({count})…")
    }
}

/// Menu entry for deleting everything the filters select.
///
/// The listing stops counting at 5000 rows, so a menu that says how much is about
/// to go has to be honest about not knowing the exact number.
fn delete_matching_label(filtered: bool, count: i64) -> String {
    let shown = if count >= crate::store::COUNT_CAP {
        format!("{}+", crate::store::COUNT_CAP)
    } else {
        count.to_string()
    };
    if filtered {
        tf!(
            "Удалить отобранное ({shown})…",
            "Delete everything selected ({shown})…"
        )
    } else {
        tf!(
            "Удалить все записи ({shown})…",
            "Delete every record ({shown})…"
        )
    }
}

/// One phrase naming the active filters, or an empty text when there are none.
///
/// Pure, so the wording and the "nothing filtered" case can be checked without a
/// window; the window only collects the criteria.
fn filter_summary_text(parts: &[String]) -> String {
    if parts.is_empty() {
        String::new()
    } else {
        tf!("фильтры: {}", "filters: {}", parts.join(", "))
    }
}

/// Name of an application in the filter list, with the number of its records.
///
/// The count is a useful hint, but a long identifier would push it out of the
/// list and leave the user with an ellipsis instead of a name - so a long name
/// is shown on its own.
fn application_label(label: &str, count: i64) -> String {
    const SHORT_ENOUGH: usize = 26;
    if count > 1 && label.chars().count() <= SHORT_ENOUGH {
        format!("{label}  ({count})")
    } else {
        label.to_string()
    }
}

/// Command line options of the graphical build.
#[derive(Debug, Clone, Default)]
struct Options {
    /// `--home <dir>`: data directory (otherwise settings decide).
    home: Option<String>,
    /// `--background` / `--minimized`: start in the notification area.
    minimized: bool,
    /// `--selftest [file]`: describe the window and exit.
    selftest: Option<PathBuf>,
}

impl Options {
    fn from_env() -> Options {
        let mut options = Options::default();
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--home" => {
                    index += 1;
                    options.home = args.get(index).cloned();
                }
                "--background" | "--minimized" | "--minimized-to-tray" => options.minimized = true,
                "--selftest" => {
                    let next = args.get(index + 1).filter(|value| !value.starts_with("--"));
                    options.selftest = Some(match next {
                        Some(path) => {
                            index += 1;
                            PathBuf::from(path)
                        }
                        None => std::env::temp_dir().join("wnl-gui-selftest.txt"),
                    });
                }
                _ => {}
            }
            index += 1;
        }
        options
    }
}

/// Everything the window procedure needs; owned by the window (`GWLP_USERDATA`).
struct App {
    hwnd: HWND,
    options: Options,
    settings: Settings,
    layout: Layout,
    journal: Journal,
    resolver: NameResolver,
    collector: Option<service::Control>,
    collector_params: service::Params,
    icons: Option<appicon::IconCache>,
    columns: Vec<Column>,
    /// Width of every column in pixels, as the list has it right now.
    ///
    /// The rows paint themselves (`LVS_OWNERDRAWFIXED`), so the drawing has to know
    /// where a column begins and ends - the list view no longer tells it by drawing.
    column_widths: std::cell::RefCell<Vec<i32>>,
    /// Icon index of every listed row (`-1` when it has none).
    row_images: Vec<i32>,
    /// Colours of the look in use (`settings::Settings::theme`).
    palette: style::Palette,
    font: HFONT,
    dpi: u32,
    search_label: HWND,
    search: HWND,
    import_button: HWND,
    more_button: HWND,
    export_button: HWND,
    settings_button: HWND,
    folder_button: HWND,
    /// Filter row: application, kind, source, period and the custom range.
    filter_app_label: HWND,
    filter_app: HWND,
    filter_kind: HWND,
    filter_source: HWND,
    filter_period: HWND,
    filter_from: HWND,
    filter_to: HWND,
    filter_from_label: HWND,
    filter_to_label: HWND,
    filter_reset: HWND,
    /// Tooltip window explaining the controls.
    tooltip: HWND,
    list: HWND,
    status: HWND,
    status_middle: HWND,
    status_right: HWND,
    /// Filter summary under the three status fields (hidden when nothing is filtered).
    status_filters: HWND,
    /// Thin rule between the filter row and the list.
    rule_filters: HWND,
    /// Thin rule between the list and the status line.
    rule_status: HWND,
    /// Whether that second status line is currently shown (drives the layout).
    status_extra_line: bool,
    rows: Vec<Row>,
    matches: i64,
    total: i64,
    /// Index of the row selected in the list (-1: nothing selected).
    selected: i32,
    text: String,
    sort: String,
    descending: bool,
    /// Active filters (`None` means "everything").
    application_filter: Option<String>,
    kind_filter: Option<String>,
    source_filter: Option<String>,
    period_filter: view::Period,
    /// Applications, kinds and sources offered by the filter lists.
    applications: Vec<crate::store::ApplicationInfo>,
    kinds: Vec<(String, i64)>,
    sources: Vec<(String, i64)>,
    /// Text size of the interface in percent (`100` = system default).
    font_scale: u32,
    /// Journal total seen at the last refresh (new rows trigger a reload).
    seen_total: i64,
    /// True when `--home` overrode the storage folder for this run only.
    ///
    /// The switch is a switch, not a preference: saving the settings afterwards
    /// must not turn one run with another folder into the new default.
    home_override: bool,
    /// The folder `settings.json` carried before the override.
    stored_data_dir: Option<PathBuf>,
    /// Last report of the collector thread.
    collector_status: Option<service::Status>,
    /// Text of the middle status field.
    collector_text: String,
    /// Last completed action and when it happened; shown on the status line for a
    /// few seconds instead of the collector report, because "я нажал «Удалить», и
    /// что?" needs an answer that does not vanish within the same second.
    note: Option<(String, Instant)>,
    tray_added: bool,
    tray_balloon_shown: bool,
    /// Highest pop-up number already shown; `None` until the status file was read
    /// once, so that pop-ups from before this start are not repeated.
    shown_alert_seq: Option<u64>,
    about_to_quit: bool,
    /// Last text of every status field (avoids pointless repaints).
    status_texts: std::cell::RefCell<std::collections::BTreeMap<usize, String>>,
}

impl App {
    fn new(
        options: Options,
        settings: Settings,
        layout: Layout,
        journal: Journal,
        resolver: NameResolver,
        home_override: bool,
        stored_data_dir: Option<PathBuf>,
    ) -> App {
        let params = service::Params::from_layout(&layout, &settings);
        let columns = if settings.simple_view {
            view::feed_columns(&settings.feed_columns)
        } else {
            view::columns(settings.show_technical_columns)
        };
        let settings_font_scale = settings.font_scale;
        let palette = style::Palette::from_setting(&settings.theme);
        App {
            hwnd: std::ptr::null_mut(),
            options,
            settings,
            layout,
            journal,
            resolver,
            collector: None,
            collector_params: params,
            icons: None,
            columns,
            column_widths: std::cell::RefCell::new(Vec::new()),
            row_images: Vec::new(),
            palette,
            font: std::ptr::null_mut(),
            dpi: 96,
            search_label: std::ptr::null_mut(),
            search: std::ptr::null_mut(),
            import_button: std::ptr::null_mut(),
            more_button: std::ptr::null_mut(),
            export_button: std::ptr::null_mut(),
            settings_button: std::ptr::null_mut(),
            folder_button: std::ptr::null_mut(),
            filter_app_label: std::ptr::null_mut(),
            filter_app: std::ptr::null_mut(),
            filter_kind: std::ptr::null_mut(),
            filter_source: std::ptr::null_mut(),
            filter_period: std::ptr::null_mut(),
            filter_from: std::ptr::null_mut(),
            filter_to: std::ptr::null_mut(),
            filter_from_label: std::ptr::null_mut(),
            filter_to_label: std::ptr::null_mut(),
            filter_reset: std::ptr::null_mut(),
            tooltip: std::ptr::null_mut(),
            list: std::ptr::null_mut(),
            status: std::ptr::null_mut(),
            status_middle: std::ptr::null_mut(),
            status_right: std::ptr::null_mut(),
            status_filters: std::ptr::null_mut(),
            rule_filters: std::ptr::null_mut(),
            rule_status: std::ptr::null_mut(),
            status_extra_line: false,
            rows: Vec::new(),
            matches: 0,
            total: 0,
            selected: -1,
            text: String::new(),
            sort: "time".to_string(),
            descending: true,
            application_filter: None,
            kind_filter: None,
            source_filter: None,
            period_filter: view::Period::All,
            applications: Vec::new(),
            kinds: Vec::new(),
            sources: Vec::new(),
            font_scale: settings_font_scale,
            seen_total: -1,
            home_override,
            stored_data_dir,
            collector_status: None,
            collector_text: tr!("сбор ещё не запускался").to_string(),
            tray_added: false,
            tray_balloon_shown: false,
            shown_alert_seq: None,
            about_to_quit: false,
            note: None,
            status_texts: std::cell::RefCell::new(std::collections::BTreeMap::new()),
        }
    }
}

/// Diagnostic switches used while bisecting a defect: environment variables that
/// switch one visual feature off (`WNL_NO_THEME`, `WNL_NO_LIST_THEME`,
/// `WNL_NO_ICONS`, `WNL_NO_STATUS_TEXT`).  They are documented in the README and
/// cost nothing when unset.
fn feature_enabled(name: &str) -> bool {
    std::env::var_os(name).is_none()
}

/// Verbose trace into the log file (`WNL_TRACE=1`), used to locate a defect in
/// a build that cannot be debugged step by step.
fn trace_enabled() -> bool {
    std::env::var_os("WNL_TRACE").is_some()
}

/// Entry point of `wnl.exe`; returns the process exit code.
pub fn run() -> i32 {
    let options = Options::from_env();
    make_dpi_aware();

    // One copy per session: a second start brings the first window forward.
    let Some(_instance) = service::acquire_instance(service::INSTANCE_NAME) else {
        if let Some(hwnd) = find_main_window() {
            unsafe {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
                PostMessageW(hwnd, WM_APP + 10, 0, 0);
            }
        }
        return 0;
    };

    // Settings live next to the executable (or in the profile when that folder
    // is read-only); the data folder follows from them.
    let exe_dir = executable_directory();
    let profile = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let settings_path = Layout::settings_path(&exe_dir, profile.as_deref());
    let mut settings = Settings::load(&settings_path);
    // The interface language is part of the settings; it is chosen before any
    // window is built, so every label is created in the right language.
    crate::lang::set(crate::lang::Lang::from_code(&settings.language));
    // `--home` applies to this run: what the file carried is kept aside so that a
    // save afterwards does not make the temporary switch the new default.
    let stored_data_dir = settings.data_dir.clone();
    let home_override = options
        .home
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if let Some(home) = options
        .home
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        settings.data_dir = Some(PathBuf::from(home));
    }
    let layout = storage::resolve(&settings, &exe_dir, profile.as_deref());
    // Pictures of notifications are kept next to the journal (see `imaging`).
    imaging::configure(layout.pictures.clone(), settings.keep_pictures);
    // The visual style has to be activated before the first window exists.
    if feature_enabled("WNL_NO_THEME") {
        theme::activate(&layout.data_dir);
    }
    // A crash has to leave a trace: the window is invisible to the console.
    install_panic_log(&layout, settings.log_enabled);

    let journal = match Journal::open(&layout.journal) {
        Ok(journal) => journal,
        Err(error) => {
            message_box(
                std::ptr::null_mut(),
                &format!(
                    "Не удалось открыть журнал\n{}\n\n{error}",
                    layout.journal.display()
                ),
                MB_ICONERROR,
            );
            return 1;
        }
    };
    let cached = journal
        .app_name_map()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let resolver = NameResolver::with_cache(cached);
    // The picture folder belongs to the journal: copies of records that are gone
    // (deleted, or dropped by the retention rule) are removed here.
    if let Ok(references) = journal.image_references() {
        let removed = imaging::prune(&layout.pictures, &references);
        if removed > 0 {
            service::write_log(
                &service::Params::from_layout(&layout, &settings),
                &tf!(
                    "удалено картинок без записей: {removed}",
                    "pictures without records deleted: {removed}"
                ),
            );
        }
    }

    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dw_size: std::mem::size_of::<INITCOMMONCONTROLSEX>() as DWORD,
            dw_icc: ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES,
        };
        InitCommonControlsEx(&controls);

        let state = Box::new(App::new(
            options.clone(),
            settings,
            layout.clone(),
            journal,
            resolver,
            home_override,
            stored_data_dir,
        ));
        let instance = module_handle();
        let class_name = wide(WINDOW_CLASS);
        let class = WNDCLASSEXW {
            lpfn_wnd_proc: Some(window_proc),
            h_instance: instance,
            h_icon: appicon::application_icon(),
            h_cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbr_background: COLOR_WINDOW_BRUSH,
            lpsz_class_name: class_name.as_ptr(),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            message_box(
                std::ptr::null_mut(),
                "Не удалось зарегистрировать класс окна.",
                MB_ICONERROR,
            );
            return 1;
        }

        // The title names the shape the window starts in: the feed is not the journal
        // window with fewer buttons, it is the feed.
        let title = wide(&state.window_title());
        let (x, y, width, height) = if state.settings.simple_view {
            feed_window_geometry(&state.settings)
        } else {
            window_geometry(&state.settings)
        };
        // The state is handed over *before* the window exists: the window keeps it from
        // its very first message on, so every question about itself (the least size
        // among them) is answered by the shape that is being created.
        let raw = Box::into_raw(state);
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            raw as *mut std::ffi::c_void,
        );
        if hwnd.is_null() {
            drop(Box::from_raw(raw));
            message_box(
                std::ptr::null_mut(),
                "Не удалось создать окно приложения.",
                MB_ICONERROR,
            );
            return 1;
        }
        // From here the window owns the state: it was given to it in `WM_NCCREATE`.
        let app = &mut *raw;
        app.hwnd = hwnd;
        app.dpi = window_dpi(hwnd);

        app.install_menu();
        app.create_children();
        app.apply_fonts();
        app.layout();
        // The look of the stored settings reaches every window at once: dark title
        // bar, dark menus where the system supports them, dark controls.
        app.apply_look();
        app.add_tray();
        app.start_collector();
        app.reload();
        SetTimer(hwnd, TIMER_REFRESH, REFRESH_MS, std::ptr::null_mut());

        if app.options.minimized || app.settings.start_minimized {
            ShowWindow(hwnd, SW_HIDE);
        } else {
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
        }

        if let Some(path) = app.options.selftest.clone() {
            let report = app.selftest(Duration::from_millis(1200));
            let _ = std::fs::write(&path, report);
            app.shutdown();
            return 0;
        }
        message_loop(hwnd);
        app.shutdown();
    }
    0
}

/// Write the reason of a panic into the log before the process disappears.
///
/// The graphical build has no console, so without this hook a crash would leave
/// nothing but an entry in the Windows event log.  With the log switched off the
/// hook is not installed at all - the setting says "no file", and a file that
/// appears only when something goes wrong would be a surprise.
fn install_panic_log(layout: &Layout, log_enabled: bool) {
    if !log_enabled {
        return;
    }
    let path = layout.log.clone();
    std::panic::set_hook(Box::new(move |info| {
        let stamp = time::format_local_iso(Some(&time::now_iso()));
        let line = tf!(
            "{stamp}  паническое завершение: {info}\n",
            "{stamp}  panic: {info}\n"
        );
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }));
}

/// Handle of the running window, if there is one (second start of the program).
fn find_main_window() -> Option<HWND> {
    let class = wide(WINDOW_CLASS);
    let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
    (!hwnd.is_null()).then_some(hwnd)
}

/// Position and size of a new full window: the remembered one, or a generous
/// default.
fn window_geometry(settings: &Settings) -> (i32, i32, i32, i32) {
    match settings.window {
        Some([x, y, width, height]) if width > 0 && height > 0 => (x, y, width, height),
        _ => (CW_USEDEFAULT, CW_USEDEFAULT, 1280, 800),
    }
}

/// Position and size of a new feed window: the remembered strip, or a narrow
/// default that shows what the shape is good for.
///
/// The two shapes remember their own geometry, so looking at the full program does
/// not cost the user the narrow, tall window the feed was arranged in.
fn feed_window_geometry(settings: &Settings) -> (i32, i32, i32, i32) {
    match settings.feed_window {
        Some([x, y, width, height]) if width > 0 && height > 0 => (x, y, width, height),
        _ => (CW_USEDEFAULT, CW_USEDEFAULT, 380, 720),
    }
}

/// Standard message pump; `IsDialogMessageW` provides Tab navigation.
///
/// The shortcuts are handled here rather than in the window procedure: a window
/// procedure only sees the keys pressed while the window itself has the focus,
/// and in this program the focus almost always sits in the list or in the search
/// field.  `IsDialogMessageW` goes second, so a key the application takes care of
/// never turns into a dialog command.
unsafe fn message_loop(hwnd: HWND) {
    let mut message: MSG = std::mem::zeroed();
    loop {
        let status = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
        if status <= 0 {
            break;
        }
        if handle_hotkey(hwnd, &message) {
            continue;
        }
        if IsDialogMessageW(hwnd, &mut message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Handle a key that belongs to the application; true when it was consumed.
unsafe fn handle_hotkey(hwnd: HWND, message: &MSG) -> bool {
    if message.message != WM_KEYDOWN {
        return false;
    }
    // A key goes to the window that has the focus, so `message.hwnd` is the
    // authoritative answer to "where is the user typing" - more reliable than
    // asking `GetFocus`, which reports nothing while the thread is not the
    // foreground thread.
    handle_key(hwnd, message.hwnd, message.w_param as UINT)
}

/// The shortcuts themselves.
///
/// Ctrl+A is the one plain edit controls do not implement (a single line field
/// inserts a control character instead of selecting its text), so it is done by
/// hand - for the text fields and for the list.
unsafe fn handle_key(hwnd: HWND, target: HWND, key: UINT) -> bool {
    let Some(app) = app_ref(hwnd) else {
        return false;
    };
    let ctrl = GetKeyState(VK_CONTROL) < 0;
    // The list owns the keys whenever it is focused: that is where a deletion, a
    // copy or "open" is expected to act on the selection.
    let in_list = target == app.list;
    match key {
        VK_F5 => app.import_now(),
        VK_F if ctrl => {
            // Ctrl+F: to the search field, with the old text selected so that
            // typing simply replaces it.
            SetFocus(app.search);
            SendMessageW(app.search, EM_SETSEL, 0, -1);
        }
        VK_S if ctrl => app.export_rows(),
        VK_R if ctrl => app.reset_filters(),
        VK_A if ctrl => {
            if in_list {
                app.select_all_rows();
            } else if style::is_edit(target) {
                SendMessageW(target, EM_SETSEL, 0, -1);
            } else {
                return false;
            }
        }
        VK_C if ctrl && in_list => app.copy_selection(),
        VK_DELETE if in_list => app.delete_selected(),
        VK_RETURN if in_list => app.open_selected(),
        VK_ESCAPE => {
            // Escape undoes the two things it is expected to undo - the search,
            // then the filters - and never closes the window.
            if !window_text(app.search).is_empty() {
                set_window_text(app.search, "");
            } else if app.filters_active() {
                app.reset_filters();
            }
        }
        _ => return false,
    }
    true
}

impl App {
    /// Menu bar of the window.
    ///
    /// One bar for both shapes of the window.  The feed is the same program with
    /// less on screen, and the menu is where the settings open, where the text size
    /// is chosen and where the shape is switched - all three are wanted in a narrow
    /// strip as well.  The bar is built from scratch, so a mixture of two menus
    /// cannot survive; the menu that was there is destroyed first.
    fn install_menu(&mut self) {
        let previous = unsafe { GetMenu(self.hwnd) };
        if !previous.is_null() {
            unsafe {
                SetMenu(self.hwnd, std::ptr::null_mut());
                DestroyMenu(previous);
            }
        }
        self.install_full_menu();
    }

    /// The menu of the window: every pop-up the program offers, in both shapes.
    fn install_full_menu(&mut self) {
        unsafe {
            let bar = CreateMenu();
            let file = CreatePopupMenu();
            let view_menu = CreatePopupMenu();
            let service_menu = CreatePopupMenu();
            let help = CreatePopupMenu();
            let item = |menu: HMENU, id: usize, text: &str| {
                let text_w = wide(text);
                AppendMenuW(menu, MF_STRING, id, text_w.as_ptr());
            };
            item(file, ID_MENU_EXPORT, "Экспорт…\tCtrl+S");
            item(file, ID_MENU_OPEN, "Открыть выбранное\tEnter");
            item(file, ID_MENU_FOLDER, "Открыть папку данных");
            AppendMenuW(file, MF_SEPARATOR, 0, std::ptr::null());
            // Deletion: by selection, by the filters, and by a period.
            item(file, ID_MENU_DELETE_SELECTED, "Удалить выбранные…\tDel");
            item(file, ID_MENU_DELETE_MATCHING, "Удалить отобранное…");
            item(file, ID_MENU_DELETE_PERIOD, "Удалить за период…");
            AppendMenuW(file, MF_SEPARATOR, 0, std::ptr::null());
            item(file, ID_MENU_EXIT, "Выход");

            item(view_menu, ID_MENU_REFRESH, "Обновить список\tF5");
            AppendMenuW(view_menu, MF_SEPARATOR, 0, std::ptr::null());
            item(view_menu, ID_MENU_ICONS, "Значки приложений");
            item(view_menu, ID_MENU_TECHNICAL, "Технические столбцы");
            item(
                view_menu,
                ID_MENU_HIDE_DUPLICATES,
                "Скрывать повторные уведомления",
            );
            AppendMenuW(view_menu, MF_SEPARATOR, 0, std::ptr::null());

            item(view_menu, ID_MENU_SIMPLE_VIEW, "Простой вид (лента)");
            AppendMenuW(view_menu, MF_SEPARATOR, 0, std::ptr::null());

            // Text size: the option users ask for first on a large monitor.
            let font_menu = CreatePopupMenu();
            for (index, scale) in crate::settings::FONT_SCALES.iter().enumerate() {
                item(
                    font_menu,
                    ID_MENU_FONT_FIRST + index,
                    crate::settings::font_scale_label(*scale),
                );
            }
            let font_label = wide("Размер текста");
            AppendMenuW(view_menu, MF_POPUP, font_menu as usize, font_label.as_ptr());

            // The feed has its own columns and its own row height: the reader decides
            // how much of the message fits into the strip.
            AppendMenuW(view_menu, MF_SEPARATOR, 0, std::ptr::null());
            let columns_menu = CreatePopupMenu();
            for (index, column) in view::FEED_COLUMNS.iter().enumerate() {
                // The menu and the keys stand in the same order (a test holds them
                // together), so the position in the menu names the column.
                item(columns_menu, ID_MENU_FEED_COLUMN_FIRST + index, column.title);
            }
            let columns_label = wide("Столбцы ленты");
            AppendMenuW(
                view_menu,
                MF_POPUP,
                columns_menu as usize,
                columns_label.as_ptr(),
            );
            let lines_menu = CreatePopupMenu();
            for (index, lines) in crate::settings::FEED_LINES.iter().enumerate() {
                item(
                    lines_menu,
                    ID_MENU_FEED_LINES_FIRST + index,
                    crate::settings::feed_lines_label(*lines),
                );
            }
            let lines_label = wide("Строк в строке ленты");
            AppendMenuW(
                view_menu,
                MF_POPUP,
                lines_menu as usize,
                lines_label.as_ptr(),
            );
            AppendMenuW(view_menu, MF_SEPARATOR, 0, std::ptr::null());

            // The look of the whole program: day or night.
            let theme_menu = CreatePopupMenu();
            for (index, theme) in crate::settings::THEMES.iter().enumerate() {
                item(
                    theme_menu,
                    ID_MENU_THEME_FIRST + index,
                    crate::settings::theme_label(theme),
                );
            }
            let theme_label = wide("Тема оформления");
            AppendMenuW(view_menu, MF_POPUP, theme_menu as usize, theme_label.as_ptr());

            item(service_menu, ID_MENU_IMPORT, "Проверить сейчас\tF5");
            item(service_menu, ID_MENU_PAUSE, "Приостановить сбор");
            AppendMenuW(service_menu, MF_SEPARATOR, 0, std::ptr::null());
            item(
                service_menu,
                ID_MENU_RESET,
                "Прочитать базу уведомлений заново…",
            );
            AppendMenuW(service_menu, MF_SEPARATOR, 0, std::ptr::null());
            item(service_menu, ID_MENU_SETTINGS, "Настройки…");

            item(help, ID_MENU_ABOUT, "О программе");

            let file_label = wide("Файл");
            let view_label = wide("Вид");
            let service_label = wide("Сбор");
            let help_label = wide("Справка");
            AppendMenuW(bar, MF_POPUP, file as usize, file_label.as_ptr());
            AppendMenuW(bar, MF_POPUP, view_menu as usize, view_label.as_ptr());
            AppendMenuW(bar, MF_POPUP, service_menu as usize, service_label.as_ptr());
            AppendMenuW(bar, MF_POPUP, help as usize, help_label.as_ptr());
            SetMenu(self.hwnd, bar);
            self.refresh_menu_checks();
        }
    }

    /// Tick marks of the menu entries that mirror a setting.
    fn refresh_menu_checks(&self) {
        let check = |id: usize, checked: bool| unsafe {
            let menu = GetMenu(self.hwnd);
            CheckMenuItem(
                menu,
                id as UINT,
                if checked { MF_CHECKED } else { MF_UNCHECKED },
            );
        };
        check(ID_MENU_ICONS, self.settings.show_app_icons);
        check(ID_MENU_TECHNICAL, self.settings.show_technical_columns);
        check(ID_MENU_HIDE_DUPLICATES, self.settings.hide_duplicates);
        // The shape the window is in: the entry is a switch, and the tick says which
        // end of it is in use.
        check(ID_MENU_SIMPLE_VIEW, self.settings.simple_view);
        check(
            ID_MENU_PAUSE,
            self.collector
                .as_ref()
                .is_some_and(|control| control.is_paused()),
        );
        // Exactly one text size is in use.
        let current = crate::settings::FONT_SCALES
            .iter()
            .position(|scale| *scale == self.font_scale);
        for index in 0..crate::settings::FONT_SCALES.len() {
            unsafe {
                let menu = GetMenu(self.hwnd);
                CheckMenuRadioItem(
                    menu,
                    (ID_MENU_FONT_FIRST + index) as UINT,
                    (ID_MENU_FONT_FIRST + index) as UINT,
                    (ID_MENU_FONT_FIRST + current.unwrap_or(0)) as UINT,
                    MF_BYCOMMAND,
                );
            }
        }
        // The columns of the feed are switches: every one of them shows whether it
        // is on the strip right now.
        for (index, key) in view::FEED_COLUMN_KEYS.iter().enumerate() {
            check(
                ID_MENU_FEED_COLUMN_FIRST + index,
                self.settings.feed_columns.iter().any(|chosen| chosen == key),
            );
        }
        // And exactly one number of lines, exactly one look.
        self.check_radio(ID_MENU_FEED_LINES_FIRST, &crate::settings::FEED_LINES, |lines| {
            *lines == self.settings.feed_lines
        });
        self.check_radio(ID_MENU_THEME_FIRST, &crate::settings::THEMES, |theme| {
            *theme == self.settings.theme.as_str()
        });
    }

    /// Tick the one entry of a sub-menu that is in use.
    fn check_radio<T>(&self, first: usize, values: &[T], chosen: impl Fn(&T) -> bool) {
        if values.is_empty() {
            return;
        }
        let Some(index) = values.iter().position(chosen) else {
            return;
        };
        let menu = unsafe { GetMenu(self.hwnd) };
        if menu.is_null() {
            return;
        }
        let last = first + values.len() - 1;
        unsafe {
            CheckMenuRadioItem(
                menu,
                first as UINT,
                last as UINT,
                (first + index) as UINT,
                MF_BYCOMMAND,
            );
        }
    }
}

impl App {
    /// Toolbar, list view and status bar.
    fn create_children(&mut self) {
        let instance = module_handle();
        let parent = self.hwnd;
        let base = WS_CHILD | WS_VISIBLE;
        unsafe {
            self.search_label = create_child(
                parent,
                instance,
                "STATIC",
                "Поиск:",
                base | style::LABEL,
                0,
                ID_SEARCH_LABEL,
            );
            self.search = create_child(
                parent,
                instance,
                "EDIT",
                "",
                base | WS_TABSTOP | ES_AUTOHSCROLL,
                WS_EX_CLIENTEDGE,
                ID_SEARCH,
            );
            let buttons: [(usize, &str); 5] = [
                (ID_IMPORT, "Проверить сейчас"),
                (ID_MORE, "Ещё"),
                (ID_EXPORT, "Экспорт…"),
                (ID_SETTINGS, "Настройки"),
                (ID_FOLDER, "Папка"),
            ];
            for (id, text) in buttons {
                let control = create_child(
                    parent,
                    instance,
                    "BUTTON",
                    text,
                    base | WS_TABSTOP | BS_PUSHBUTTON,
                    0,
                    id,
                );
                match id {
                    ID_IMPORT => self.import_button = control,
                    ID_MORE => self.more_button = control,
                    ID_EXPORT => self.export_button = control,
                    ID_SETTINGS => self.settings_button = control,
                    _ => self.folder_button = control,
                }
            }
            // Filter row: the criteria a journal can answer without guessing.
            self.filter_app_label = create_child(
                parent,
                instance,
                "STATIC",
                "Фильтры:",
                base | style::LABEL,
                0,
                ID_FILTER_APP_LABEL,
            );
            self.filter_app = create_child(
                parent,
                instance,
                "COMBOBOX",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
                WS_EX_CLIENTEDGE,
                ID_FILTER_APP,
            );
            self.filter_kind = create_child(
                parent,
                instance,
                "COMBOBOX",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
                WS_EX_CLIENTEDGE,
                ID_FILTER_KIND,
            );
            self.filter_source = create_child(
                parent,
                instance,
                "COMBOBOX",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
                WS_EX_CLIENTEDGE,
                ID_FILTER_SOURCE,
            );
            self.filter_period = create_child(
                parent,
                instance,
                "COMBOBOX",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
                WS_EX_CLIENTEDGE,
                ID_FILTER_PERIOD,
            );
            self.filter_from_label = create_child(
                parent,
                instance,
                "STATIC",
                "с",
                base | style::LABEL,
                0,
                ID_FILTER_FROM_LABEL,
            );
            self.filter_from = create_child(
                parent,
                instance,
                "EDIT",
                "",
                base | WS_TABSTOP | ES_AUTOHSCROLL,
                WS_EX_CLIENTEDGE,
                ID_FILTER_FROM,
            );
            self.filter_to_label = create_child(
                parent,
                instance,
                "STATIC",
                "по",
                base | style::LABEL,
                0,
                ID_FILTER_TO_LABEL,
            );
            self.filter_to = create_child(
                parent,
                instance,
                "EDIT",
                "",
                base | WS_TABSTOP | ES_AUTOHSCROLL,
                WS_EX_CLIENTEDGE,
                ID_FILTER_TO,
            );
            self.filter_reset = create_child(
                parent,
                instance,
                "BUTTON",
                "Сбросить",
                base | WS_TABSTOP | BS_PUSHBUTTON,
                0,
                ID_FILTER_RESET,
            );
            self.create_list(parent, instance);
            // The status line is plain text on the window: three fields that say
            // what is shown, what the collector does and where the data lives.
            self.status = create_child(
                parent,
                instance,
                "STATIC",
                "",
                base | style::LABEL,
                0,
                ID_STATUS,
            );
            self.status_middle = create_child(
                parent,
                instance,
                "STATIC",
                "",
                base | style::LABEL,
                0,
                ID_STATUS_MIDDLE,
            );
            self.status_right = create_child(
                parent,
                instance,
                "STATIC",
                "",
                base | style::LABEL | SS_RIGHT | SS_ENDELLIPSIS,
                0,
                ID_STATUS_RIGHT,
            );
            self.status_filters = create_child(
                parent,
                instance,
                "STATIC",
                "",
                base | style::LABEL | SS_ENDELLIPSIS,
                0,
                ID_STATUS_FILTERS,
            );
            self.rule_filters = style::separator(parent, instance, ID_RULE_FILTERS);
            self.rule_status = style::separator(parent, instance, ID_RULE_STATUS);
            SendMessageW(self.search, EM_SETLIMITTEXT, 512, 0);
        }
        self.build_columns();
        self.refresh_icons();
        self.refresh_filter_lists();
        self.install_tooltips();
    }

    /// Create the report control that shows the journal.
    ///
    /// The rows are painted by the window itself (`LVS_OWNERDRAWFIXED`): only that
    /// way a row of the feed can be two or three lines tall and wrap its text, and
    /// only that way the night look reaches the list - a report view of `comctl32`
    /// draws itself with the system colours.
    fn create_list(&mut self, parent: HWND, instance: HINSTANCE) {
        let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | LVS_REPORT | LVS_SHOWSELALWAYS | LVS_OWNERDRAWFIXED;
        self.list = create_child(
            parent,
            instance,
            "SysListView32",
            "",
            style,
            WS_EX_CLIENTEDGE,
            ID_LIST,
        );
        unsafe {
            SendMessageW(
                self.list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                // `LVS_EX_LABELTIP` is deliberately not asked for: it makes the control
                // show the text of a clipped cell, and a control that does not draw the
                // cells cannot know which of them is clipped (see `draw_row`).
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as LPARAM,
            );
            // The area under the last row and the text colour of the control are
            // drawn by the list itself, even when the rows are ours.
            SendMessageW(self.list, LVM_SETBKCOLOR, 0, self.palette.list as LPARAM);
            SendMessageW(self.list, LVM_SETTEXTBKCOLOR, 0, self.palette.list as LPARAM);
            SendMessageW(self.list, LVM_SETTEXTCOLOR, 0, self.palette.text as LPARAM);
            if !self.font.is_null() {
                SendMessageW(self.list, WM_SETFONT, self.font as WPARAM, 1);
            }
        }
        // Explorer look for the list (no grid lines: the eye needs rest); the night
        // look asks the system for its dark variant of the same theme.
        if feature_enabled("WNL_NO_LIST_THEME") {
            if self.palette.dark {
                theme_control(self.list, true);
                theme_control(self.list_header(), true);
            } else {
                apply_theme(self.list, "Explorer");
            }
        }
    }

    /// The header control of the report (it draws the captions the rows need).
    fn list_header(&self) -> HWND {
        if self.list.is_null() {
            return std::ptr::null_mut();
        }
        unsafe { SendMessageW(self.list, LVM_GETHEADER, 0, 0) as HWND }
    }

    /// Build the list again.
    ///
    /// The height of a row is decided when the control is created: it asks its owner
    /// once, through `WM_MEASUREITEM`.  A feed that switches between one, two and
    /// three lines therefore needs a new control - and so does a change of the text
    /// size, which moves every line.
    fn recreate_list(&mut self) {
        let instance = module_handle();
        let parent = self.hwnd;
        unsafe {
            if !self.tooltip.is_null() {
                DestroyWindow(self.tooltip);
                self.tooltip = std::ptr::null_mut();
            }
            if !self.list.is_null() {
                DestroyWindow(self.list);
                self.list = std::ptr::null_mut();
            }
        }
        self.create_list(parent, instance);
        self.apply_fonts();
        self.build_columns();
        self.refresh_icons();
        self.install_tooltips();
        self.fill_list();
        self.layout();
        self.select_first();
    }

    /// The window font, enlarged by the *text size* setting.
    ///
    /// Everything the window draws (font, row heights, column widths, icons) is
    /// scaled by the same percentage, so the interface stays consistent instead
    /// of turning into big text with tiny buttons.
    fn apply_fonts(&mut self) {
        if self.font.is_null() {
            self.font = system_font_scaled(self.font_scale);
        }
        let font = self.font;
        for control in self.controls() {
            if !control.is_null() {
                unsafe {
                    SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
                }
            }
        }
    }

    /// Every control of the window (used for the font and for the layout).
    fn controls(&self) -> [HWND; 26] {
        [
            self.search_label,
            self.search,
            self.import_button,
            self.more_button,
            self.export_button,
            self.settings_button,
            self.folder_button,
            self.filter_app_label,
            self.filter_app,
            self.filter_kind,
            self.filter_source,
            self.filter_period,
            self.filter_from_label,
            self.filter_from,
            self.filter_to_label,
            self.filter_to,
            self.filter_reset,
            self.list,
            self.status,
            self.status_middle,
            self.status_right,
            self.status_filters,
            self.rule_filters,
            self.rule_status,
            self.hwnd,
            self.tooltip,
        ]
    }

    /// Change the text size: new font, new metrics, everything redrawn.
    fn set_font_scale(&mut self, scale: u32) {
        let scale = scale.clamp(
            crate::settings::MIN_FONT_SCALE,
            crate::settings::MAX_FONT_SCALE,
        );
        if scale == self.font_scale {
            return;
        }
        self.font_scale = scale;
        if !self.font.is_null() {
            unsafe {
                DeleteObject(self.font as HGDIOBJ);
            }
        }
        self.font = system_font_scaled(scale);
        self.settings.font_scale = scale;
        let _ = self.settings.save(&self.layout.settings_path);
        // The font moves every line, and the height of a row is decided when the list
        // is created: the list is built again, icons and columns with it.
        self.recreate_list();
        self.reload();
        self.refresh_menu_checks();
    }

    /// Effective DPI of the window: the monitor DPI times the text size.
    fn metric_scale(&self) -> u32 {
        effective_scale(self.dpi, self.font_scale)
    }

    /// Logical pixels to real pixels for this window.
    fn px(&self, value: i32) -> i32 {
        scale(value, self.metric_scale())
    }
}

impl App {
    /// Fill the filter lists from the journal, keeping the current choice.
    fn refresh_filter_lists(&mut self) {
        self.applications = self.journal.applications(200).unwrap_or_default();
        self.kinds = self.journal.kinds().unwrap_or_default();
        self.sources = self.journal.sources().unwrap_or_default();

        self.fill_combo(
            self.filter_app,
            "Все приложения",
            self.applications
                .iter()
                .map(|app| application_label(&app.label, app.count))
                .collect(),
        );
        self.fill_combo(
            self.filter_kind,
            "Все типы",
            self.kinds
                .iter()
                .map(|(kind, count)| format!("{kind}  ({count})"))
                .collect(),
        );
        self.fill_combo(
            self.filter_source,
            "Все источники",
            self.sources
                .iter()
                .map(|(source, count)| format!("{source}  ({count})"))
                .collect(),
        );
        self.fill_combo(
            self.filter_period,
            "",
            view::PERIODS
                .iter()
                .map(|(_, title)| (*title).to_string())
                .collect(),
        );
        self.select_filter_lists();
    }

    /// Replace the content of a combo box; the first entry means "no filter".
    fn fill_combo(&self, combo: HWND, first: &str, entries: Vec<String>) {
        unsafe {
            SendMessageW(combo, CB_RESETCONTENT, 0, 0);
            if !first.is_empty() {
                let text = wide(first);
                SendMessageW(combo, CB_ADDSTRING, 0, text.as_ptr() as LPARAM);
            }
            for entry in entries {
                let text = wide(&entry);
                SendMessageW(combo, CB_ADDSTRING, 0, text.as_ptr() as LPARAM);
            }
        }
    }

    /// Show the active filters in the lists.
    fn select_filter_lists(&self) {
        let app = self
            .application_filter
            .as_ref()
            .and_then(|aumid| self.applications.iter().position(|app| &app.aumid == aumid))
            .map(|index| index + 1)
            .unwrap_or(0);
        let kind = self
            .kind_filter
            .as_ref()
            .and_then(|kind| self.kinds.iter().position(|(value, _)| value == kind))
            .map(|index| index + 1)
            .unwrap_or(0);
        let source = self
            .source_filter
            .as_ref()
            .and_then(|source| self.sources.iter().position(|(value, _)| value == source))
            .map(|index| index + 1)
            .unwrap_or(0);
        unsafe {
            SendMessageW(self.filter_app, CB_SETCURSEL, app, 0);
            SendMessageW(self.filter_kind, CB_SETCURSEL, kind, 0);
            SendMessageW(self.filter_source, CB_SETCURSEL, source, 0);
            SendMessageW(
                self.filter_period,
                CB_SETCURSEL,
                view::period_index(self.period_filter),
                0,
            );
        }
    }

    /// Read the filter row into the fields the query is built from.
    ///
    /// Called whenever one of the controls changes; the list is then rebuilt so
    /// the user sees the effect immediately.
    fn read_filters(&mut self) {
        let selected = |control: HWND| unsafe {
            let index = SendMessageW(control, CB_GETCURSEL, 0, 0);
            (index > 0).then_some(index as usize - 1)
        };
        self.application_filter = selected(self.filter_app)
            .and_then(|index| self.applications.get(index))
            .map(|app| app.aumid.clone());
        self.kind_filter = selected(self.filter_kind)
            .and_then(|index| self.kinds.get(index))
            .map(|(kind, _)| kind.clone());
        self.source_filter = selected(self.filter_source)
            .and_then(|index| self.sources.get(index))
            .map(|(source, _)| source.clone());
        self.period_filter = selected(self.filter_period)
            .map(view::period_at)
            .unwrap_or(view::Period::All);
        self.update_filter_controls();
        self.reload();
    }

    /// Clear every filter and show the whole journal again.
    fn reset_filters(&mut self) {
        self.text.clear();
        set_window_text(self.search, "");
        self.application_filter = None;
        self.kind_filter = None;
        self.source_filter = None;
        self.period_filter = view::Period::All;
        set_window_text(self.filter_from, "");
        set_window_text(self.filter_to, "");
        unsafe {
            SendMessageW(self.filter_app, CB_SETCURSEL, 0, 0);
            SendMessageW(self.filter_kind, CB_SETCURSEL, 0, 0);
            SendMessageW(self.filter_source, CB_SETCURSEL, 0, 0);
            SendMessageW(self.filter_period, CB_SETCURSEL, 0, 0);
        }
        self.update_filter_controls();
        self.reload();
    }

    /// The date fields only matter for the custom range; grey them out otherwise.
    fn update_filter_controls(&self) {
        let custom = self.period_filter == view::Period::Custom;
        unsafe {
            EnableWindow(self.filter_from, if custom { 1 } else { 0 });
            EnableWindow(self.filter_to, if custom { 1 } else { 0 });
        }
    }

    /// Create every column once; technical ones are hidden by a zero width.
    fn build_columns(&mut self) {
        // A switch between the two shapes rebuilds the report: the feed and the full
        // list share nothing but the list itself.
        let columns = if self.settings.simple_view {
            view::feed_columns(&self.settings.feed_columns)
        } else {
            view::columns(self.settings.show_technical_columns)
        };
        // Columns of the shape that is being left behind are dropped first, so the
        // two sets can never end up side by side.
        for _ in 0..self.columns.len() {
            unsafe { SendMessageW(self.list, LVM_DELETECOLUMN, 0, 0) };
        }
        for (index, column) in columns.iter().enumerate() {
            let mut caption = wide(column.title);
            let mut info = LVCOLUMNW {
                mask: LVCF_FMT | LVCF_WIDTH | LVCF_TEXT | LVCF_SUBITEM,
                fmt: if column.right {
                    LVCFMT_RIGHT
                } else {
                    LVCFMT_LEFT
                },
                cx: scale(column.width, self.dpi),
                psz_text: caption.as_mut_ptr(),
                i_sub_item: index as i32,
                ..Default::default()
            };
            unsafe {
                SendMessageW(
                    self.list,
                    LVM_INSERTCOLUMNW,
                    index as WPARAM,
                    &mut info as *mut LVCOLUMNW as LPARAM,
                );
            }
        }
        self.columns = columns;
        self.apply_column_widths();
    }

    /// Widths follow the *технические столбцы* setting: zero means hidden.
    ///
    /// The feed is laid out for the window it is in: the source column takes about
    /// a third of the row, the message column the rest, and the columns the reader
    /// added share what is left.  That is what makes a narrow, tall strip readable -
    /// the same list is shown in a wide window as well.  The widths are remembered
    /// because the rows paint themselves and have to know where a column ends.
    fn apply_column_widths(&self) {
        let technical = self.settings.show_technical_columns;
        let feed = self.settings.simple_view;
        // The last column of the feed takes the room that is left: a table that ends
        // in the middle of the window looks broken.  What a vertical scroll bar and
        // the sunken edge of the list will take is subtracted right away (measured,
        // not guessed): a column that is a few pixels too wide makes the list open a
        // second, horizontal bar - which is exactly what a narrow window cannot
        // afford.
        let mut client = RECT::default();
        unsafe { GetClientRect(self.list, &mut client) };
        let reserved = unsafe { GetSystemMetrics(SM_CXVSCROLL) }.max(0) + self.px(4);
        let inner = (client.width() - reserved).max(0);
        let last_index = self.columns.len().saturating_sub(1);
        let widths: Vec<i32> = if feed {
            let fixed: Vec<i32> = self
                .columns
                .iter()
                .take(last_index)
                .map(|column| self.feed_fixed_width(column, inner))
                .collect();
            feed_column_widths(inner, &fixed, self.px(90))
        } else {
            self.columns
                .iter()
                .map(|column| {
                    if column.technical && !technical {
                        0
                    } else {
                        self.px(column.width)
                    }
                })
                .collect()
        };
        self.column_widths.borrow_mut().clear();
        self.column_widths.borrow_mut().extend(&widths);
        for (index, width) in widths.iter().enumerate() {
            unsafe {
                SendMessageW(
                    self.list,
                    LVM_SETCOLUMNWIDTH,
                    index as WPARAM,
                    *width as LPARAM,
                );
            }
        }
    }

    /// Width one of the fixed columns of the feed asks for in a row of `inner`
    /// pixels.
    fn feed_fixed_width(&self, column: &Column, inner: i32) -> i32 {
        match column.key {
            // The source gets a slice of the row: enough for a name, never more than
            // the window can spare.
            "app" => feed_source_width(inner, self.px(72), self.px(260)),
            // Every other column keeps its width, but never more than a third of the
            // row: a clock is not worth half of a narrow window.
            _ => self.px(column.width).min((inner / 3).max(self.px(40))),
        }
    }

    /// (Re)create the image list of application icons.
    fn refresh_icons(&mut self) {
        if !feature_enabled("WNL_NO_ICONS") {
            self.icons = None;
            return;
        }
        // The icons follow the text size: a list sizes its rows after them.  Both
        // shapes of the window show them at the same size, because the size of the
        // interface is a decision about the whole program (`font_scale`), not about
        // one view of it.
        let logical = appicon::ICON_SIZE;
        let size = self.px(logical).max(logical);
        match appicon::IconCache::new(size, self.settings.show_app_icons) {
            Some(cache) => {
                cache.attach(self.list);
                self.icons = Some(cache);
            }
            None => self.icons = None,
        }
    }

    /// Width a toolbar button needs for its caption.
    ///
    /// The caption is measured with the font the window really uses, so a longer
    /// word is not cut off and a short one does not leave a hole.  The number passed
    /// in is the width to fall back on when the measurement is not possible; both
    /// ends are clamped, because a button of two letters or of half the window would
    /// be worse than a fixed width.
    fn button_width(&self, control: HWND, fallback: i32) -> i32 {
        let measured = text_width(self.hwnd, self.font, &window_text(control));
        if measured <= 0 {
            return self.px(fallback);
        }
        (measured + self.px(BUTTON_PADDING))
            .clamp(self.px(BUTTON_MIN_WIDTH), self.px(BUTTON_MAX_WIDTH))
    }

    /// Height of one line of text in the font of the window.
    fn line_height(&self) -> i32 {
        let measured = text_size(self.hwnd, self.font, "Ай").cy;
        if measured <= 0 {
            self.px(LINE_HEIGHT)
        } else {
            measured
        }
    }

    /// Lines of text one row of the list may use.
    ///
    /// One in the full program: a report is scanned, so a line per record is what a
    /// reader expects there.  In the feed the reader decides (`Settings::feed_lines`),
    /// because two or three lines carry three times the message of one.
    fn row_lines(&self) -> i32 {
        if self.settings.simple_view {
            self.settings
                .feed_lines
                .clamp(
                    crate::settings::MIN_FEED_LINES,
                    crate::settings::MAX_FEED_LINES,
                ) as i32
        } else {
            1
        }
    }

    /// Height of one row of the list, in pixels.
    ///
    /// The control asks for this when it is created (`WM_MEASUREITEM`), so a change
    /// of the line count builds the list again (`recreate_list`).
    fn row_height(&self) -> i32 {
        let text = self.row_lines() * self.line_height();
        let icon = self.px(appicon::ICON_SIZE);
        text.max(icon) + 2 * self.px(ROW_PADDING)
    }

    /// Paint one row of the list (`LVS_OWNERDRAWFIXED`).
    ///
    /// The cells come from the journal rows the window already holds, so nothing is
    /// read back from the control.  In the feed a cell wraps into as many lines as the
    /// row is tall, and what still does not fit is cut off: a notification three
    /// screens long cannot become three screens tall.
    fn draw_row(&self, item: &DRAWITEMSTRUCT) {
        let index = item.item_id as usize;
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let selected = item.item_state & ODS_SELECTED != 0;
        let focused = unsafe { GetFocus() } == self.list;
        let (background, colour) = self.palette.row_colours(selected, focused);
        let padding = self.px(ROW_PADDING);
        let cell_padding = self.px(CELL_PADDING);
        let icon_size = self.px(appicon::ICON_SIZE);
        let line = self.line_height();
        let lines = self.row_lines();
        // The feed wraps *its last column*: that is the message (or the title), and a
        // row of two or three lines is meant for it.  The columns in front hold short
        // values - a time, a name - and a clock broken into two lines is a clock
        // nobody can read.
        let last = self.columns.len().saturating_sub(1);
        let cells = view::row_cells(row, &self.columns);
        let widths = self.column_widths.borrow();
        unsafe {
            // The whole row first: the free space to the right of the last column
            // belongs to the same line.
            FillRect(item.hdc, &item.rc_item, style::brush(background));
            SetBkMode(item.hdc, TRANSPARENT);
            SetTextColor(item.hdc, colour);
            let previous = SelectObject(item.hdc, self.font as HGDIOBJ);
            let mut left = item.rc_item.left;
            for (position, (_, text)) in self.columns.iter().zip(cells.iter()).enumerate() {
                let width = widths.get(position).copied().unwrap_or(0);
                // One line is centred in its row (as a report view does); several lines
                // start at the top, because that is where the eye starts reading.
                let wrapped = self.settings.simple_view && lines > 1 && position == last;
                if width > 0 {
                    let mut cell = RECT {
                        left: left + cell_padding,
                        top: item.rc_item.top + padding,
                        right: left + width - cell_padding,
                        bottom: item.rc_item.bottom - padding,
                    };
                    // The icon sits in front of the first column - the place a report
                    // view draws it, and the place the eye looks for it.
                    if position == 0 {
                        self.draw_row_icon(item, index, icon_size, &cell, wrapped);
                        cell.left += icon_size + cell_padding;
                    }
                    if wrapped {
                        // Exactly as many lines as the row is tall: half a line at the
                        // bottom would look like a defect.
                        cell.bottom = cell.top + lines * line;
                    }
                    if cell.right > cell.left && cell.bottom > cell.top {
                        let mut buffer = wide(text);
                        let flags = if wrapped {
                            DT_WORDBREAK | DT_NOPREFIX
                        } else {
                            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX
                        };
                        DrawTextW(item.hdc, buffer.as_mut_ptr(), -1, &mut cell, flags);
                    }
                }
                left += width;
            }
            if !previous.is_null() {
                SelectObject(item.hdc, previous);
            }
            if item.item_state & ODS_FOCUS != 0 {
                DrawFocusRect(item.hdc, &item.rc_item);
            }
        }
    }

    /// Draw the icon in front of the first column of a row.
    fn draw_row_icon(
        &self,
        item: &DRAWITEMSTRUCT,
        index: usize,
        icon_size: i32,
        cell: &RECT,
        wrapped: bool,
    ) {
        let Some(cache) = self.icons.as_ref() else {
            return;
        };
        if !cache.is_enabled() {
            return;
        }
        let icon = self.row_images.get(index).copied().unwrap_or(-1);
        if icon < 0 {
            return;
        }
        // A wrapped cell starts at the top (the icon sits on the first line); a cell
        // of one line takes the middle of the row, where its text sits.
        let top = if wrapped {
            cell.top
        } else {
            item.rc_item.top + (item.rc_item.height() - icon_size) / 2
        };
        unsafe {
            ImageList_Draw(
                cache.handle(),
                icon,
                item.hdc,
                cell.left,
                top,
                ILD_TRANSPARENT,
            );
        }
    }

    /// Paint a thin rule (`SS_OWNERDRAW`): the etched line a static control draws
    /// belongs to the day look, and the night look needs its own colour.
    fn draw_rule(&self, item: &DRAWITEMSTRUCT) {
        unsafe {
            FillRect(item.hdc, &item.rc_item, style::brush(self.palette.rule));
        }
    }
}

impl App {
    /// Place the controls inside the client area.
    ///
    /// The toolbar and the filter row wrap when the window is narrow, and the
    /// status line drops a field that would only show an ellipsis, so no control
    /// ever ends up on top of another one.
    ///
    /// Everything is placed for the width the window has *now*: the window may be
    /// dragged as narrow as its shape allows (see `minimum_window_size`) and the
    /// arrangement follows instead of overflowing.
    fn layout(&self) {
        // One window, two shapes: this function places the whole program,
        // `layout_feed` places the feed.  Everything the feed leaves out is hidden
        // with one call, so a switch cannot leave a stray button behind.
        if self.settings.simple_view {
            self.show_full_parts(false);
            self.layout_feed();
            return;
        }
        self.show_full_parts(true);
        let mut client = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut client) };
        let width = client.width();
        let height = client.height();
        let padding = self.px(PADDING);
        let gap = self.px(GAP);
        let row = self.px(ROW_HEIGHT);
        let row_gap = self.px(ROW_GAP);
        let line = self.px(LINE_HEIGHT);
        let rule = self.px(RULE_HEIGHT).max(1);
        let inner = (width - 2 * padding).max(0);

        // ---- toolbar: the search field first, the actions after it.  Every button
        // is as wide as its caption needs (measured with the font of the window) and
        // the ones that do not fit beside the field move to the next line.
        let top = padding;
        let label_width = self.px(SEARCH_LABEL_WIDTH);
        let toolbar: [(HWND, i32); 5] = [
            (
                self.import_button,
                self.button_width(self.import_button, BUTTON_IMPORT_WIDTH),
            ),
            (
                self.more_button,
                self.button_width(self.more_button, BUTTON_MORE_WIDTH),
            ),
            (
                self.export_button,
                self.button_width(self.export_button, BUTTON_EXPORT_WIDTH),
            ),
            (
                self.settings_button,
                self.button_width(self.settings_button, BUTTON_SETTINGS_WIDTH),
            ),
            (
                self.folder_button,
                self.button_width(self.folder_button, BUTTON_FOLDER_WIDTH),
            ),
        ];
        let widths: Vec<i32> = toolbar.iter().map(|(_, width)| *width).collect();
        let (search_width, rows) = toolbar_rows(
            inner,
            label_width,
            gap,
            self.px(SEARCH_MIN_WIDTH),
            &widths,
        );
        let used_rows = rows.iter().copied().max().unwrap_or(0) + 1;
        // The label gets the very box of the field and `SS_CENTERIMAGE` centres its
        // text in it: the two always share one baseline.
        let search_x = padding + label_width + gap;
        // Every line of buttons starts at the left edge; the first one continues
        // after the search field.
        let mut row_x = vec![padding; used_rows];
        row_x[0] = search_x + search_width + gap;
        unsafe {
            MoveWindow(self.search_label, padding, top, label_width, row, 1);
            MoveWindow(self.search, search_x, top, search_width, row, 1);
            for ((control, control_width), row_index) in toolbar.into_iter().zip(rows.iter()) {
                let y = top + (*row_index as i32) * (row + row_gap);
                MoveWindow(control, row_x[*row_index], y, control_width, row, 1);
                row_x[*row_index] += control_width + gap;
            }
        }

        // ---- filter row: label and controls, wrapped when there is no room.
        let mut x = padding;
        let mut y = top + used_rows as i32 * (row + row_gap);
        let mut first_on_row = true;
        // Every entry carries the gap that follows it: `с` and `по` belong to the
        // field beside them and are separated from it by less than the rest.
        let filter_controls: [(HWND, i32, i32); 9] = [
            (self.filter_app_label, FILTER_WORD_WIDTH, GAP),
            (self.filter_app, FILTER_APP_WIDTH, GAP),
            (self.filter_kind, FILTER_KIND_WIDTH, GAP),
            (self.filter_source, FILTER_SOURCE_WIDTH, GAP),
            (self.filter_period, FILTER_PERIOD_WIDTH, GAP),
            (self.filter_from_label, FILTER_DATE_LABEL_WIDTH, FILTER_PAIR_GAP),
            (self.filter_from, FILTER_DATE_WIDTH, GAP),
            (self.filter_to_label, FILTER_DATE_TO_LABEL_WIDTH, FILTER_PAIR_GAP),
            (self.filter_to, FILTER_DATE_WIDTH, 0),
        ];
        unsafe {
            for (control, control_width, next_gap) in filter_controls {
                let control_width = self.px(control_width);
                if !first_on_row && x + control_width > width - padding {
                    x = padding;
                    y += row + row_gap;
                }
                first_on_row = false;
                // A control wider than the window is cut down to what is there: its
                // left edge stays inside the window instead of hanging over the
                // neighbour, and it is still usable.
                let control_width = control_width.min((width - padding - x).max(self.px(60)));
                let control_height = if control == self.filter_period {
                    // A combo box needs room for its dropped down list.
                    row + self.px(160)
                } else {
                    row
                };
                MoveWindow(control, x, y, control_width, control_height, 1);
                x += control_width + self.px(next_gap);
            }
            // The reset button closes the last filter row.
            let reset_width = self.px(FILTER_RESET_WIDTH);
            if first_on_row {
                x = padding;
            }
            if x + reset_width > width - padding {
                x = padding;
                y += row + row_gap;
            }
            let reset_width = reset_width.min((width - padding - x).max(self.px(60)));
            MoveWindow(self.filter_reset, x, y, reset_width, row, 1);
        }

        // ---- the list fills the space between the two rules.
        let rule_filters_top = y + row + row_gap;
        let filter_line = if self.filters_active() {
            self.filter_summary()
        } else {
            String::new()
        };
        let extra_line = !filter_line.is_empty();
        // The status area is laid out from the bottom edge: one line of text, and
        // a second one only while a filter is active.
        let status_height = if extra_line {
            2 * line + self.px(4)
        } else {
            line
        };
        let rule_status_top = height - padding - status_height - row_gap - rule;
        let status_top = rule_status_top + rule + row_gap;
        let list_top = rule_filters_top + rule + row_gap;
        let list_height = (rule_status_top - row_gap - list_top).max(self.px(60));
        unsafe {
            MoveWindow(self.rule_filters, padding, rule_filters_top, inner, rule, 1);
            MoveWindow(self.rule_status, padding, rule_status_top, inner, rule, 1);
            MoveWindow(self.list, padding, list_top, inner, list_height, 1);
        }
        // Three fields: what is shown, what the collector does, where the data is.
        // They share the line proportionally and a field that would only show an
        // ellipsis is dropped, so the three never land on top of each other.
        let (left_width, middle_width, right_width) = status_widths(inner, gap, self.px(120));
        unsafe {
            MoveWindow(self.status, padding, status_top, left_width, line, 1);
            ShowWindow(
                self.status_middle,
                if middle_width > 0 { SW_SHOW } else { SW_HIDE },
            );
            MoveWindow(
                self.status_middle,
                padding + left_width + gap,
                status_top,
                middle_width,
                line,
                1,
            );
            ShowWindow(
                self.status_right,
                if right_width > 0 { SW_SHOW } else { SW_HIDE },
            );
            MoveWindow(
                self.status_right,
                padding + left_width + gap + middle_width + gap,
                status_top,
                right_width,
                line,
                1,
            );
            // The filter line spans the whole width and appears only when the
            // user is actually filtering something.
            ShowWindow(
                self.status_filters,
                if extra_line { SW_SHOW } else { SW_HIDE },
            );
            MoveWindow(
                self.status_filters,
                padding,
                status_top + line + self.px(4),
                inner,
                line,
                1,
            );
        }
    }

    /// Show or hide everything that belongs to the full program.
    ///
    /// The feed keeps the menu bar, the list and one button.  Both directions use
    /// one and the same list, because a control that is forgotten here would stay
    /// visible after a switch and look like a defect that belongs to nothing.
    fn show_full_parts(&self, show: bool) {
        let parts = [
            self.search_label,
            self.search,
            self.import_button,
            self.more_button,
            self.export_button,
            self.settings_button,
            self.folder_button,
            self.filter_app_label,
            self.filter_app,
            self.filter_kind,
            self.filter_source,
            self.filter_period,
            self.filter_from,
            self.filter_to,
            self.filter_from_label,
            self.filter_to_label,
            self.filter_reset,
            self.status,
            self.status_middle,
            self.status_right,
            self.status_filters,
            self.rule_filters,
            self.rule_status,
        ];
        let command = if show { SW_SHOW } else { SW_HIDE };
        unsafe {
            for control in parts {
                ShowWindow(control, command);
            }
        }
    }

    /// Place the feed: a list that fills the whole window.
    ///
    /// Nothing else exists in this shape - no search field, no filters, no buttons,
    /// no status lines, no rules - so the list gets every pixel there is.  The
    /// window may be a narrow, tall strip, the rows are allowed to be two or three
    /// lines tall and the columns follow the width (`apply_column_widths`).
    fn layout_feed(&self) {
        let mut client = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut client) };
        let width = client.width();
        let height = client.height();
        let padding = self.px(PADDING);
        let list_top = padding;
        // The list fills what is left; in a narrow window it is the window that
        // gives the measure, not a width invented for a wide one.
        let list_width = (width - 2 * padding).max(self.px(60));
        let list_height = (height - list_top - padding).max(self.px(60));
        unsafe {
            MoveWindow(self.list, padding, list_top, list_width, list_height, 1);
        }
        self.apply_column_widths();
    }

    /// Text of one status field: 0 left, 1 middle, 2 right, 3 filter summary.
    fn set_status_part(&self, part: usize, text: &str) {
        // Repainting a label costs a message and a paint per call; the text only
        // changes when something really changed.
        if let Some(previous) = self.status_texts.borrow().get(&part) {
            if previous.as_str() == text {
                return;
            }
        }
        self.status_texts
            .borrow_mut()
            .insert(part, text.to_string());
        let control = match part {
            0 => self.status,
            1 => self.status_middle,
            2 => self.status_right,
            _ => self.status_filters,
        };
        set_window_text(control, text);
    }

    /// Refresh the status area: three fields plus the filter summary.
    fn update_status(&mut self) {
        let matches = self.matches;
        let total = self.total;
        // The left field answers two questions: how much is filtered out and how
        // much of the result is on screen.
        let left = if self.filters_active() {
            tf!("Отобрано: {matches} из {total}", "Matched: {matches} of {total}")
        } else {
            tf!(
                "В журнале: {total}   •   показано: {}",
                "In the journal: {total}   •   shown: {}",
                self.rows.len()
            )
        };
        self.set_status_part(0, &left);
        let note = self.note.as_ref().map(|(text, at)| (text.as_str(), at.elapsed()));
        self.set_status_part(1, &middle_status_text(note, &self.collector_text));
        let folder = tf!("Данные: {}", "Data: {}", self.layout.data_dir.display());
        self.set_status_part(2, &folder);
        // The reason for the filtered list gets its own line: a long list of
        // criteria never fits into the left field of a narrow window.
        let summary = if self.filters_active() {
            self.filter_summary()
        } else {
            String::new()
        };
        self.set_status_part(3, &summary);
        // The status area grows and shrinks with that line, so the window needs a
        // new layout - but only when the line really appeared or disappeared.
        let extra_line = !summary.is_empty();
        if extra_line != self.status_extra_line {
            self.status_extra_line = extra_line;
            self.layout();
        }
    }

    /// One phrase naming what is currently filtered.
    fn filter_summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.text.trim().is_empty() {
            parts.push(tf!("поиск «{}»", "search \"{}\"", self.text.trim()));
        }
        if let Some(aumid) = &self.application_filter {
            let label = self
                .applications
                .iter()
                .find(|app| &app.aumid == aumid)
                .map(|app| app.label.clone())
                .unwrap_or_else(|| aumid.clone());
            parts.push(tf!("приложение: {label}", "application: {label}"));
        }
        if let Some(kind) = &self.kind_filter {
            parts.push(tf!("тип: {kind}", "type: {kind}"));
        }
        if let Some(source) = &self.source_filter {
            parts.push(tf!("источник: {source}", "source: {source}"));
        }
        match self.period_filter {
            view::Period::All => {}
            view::Period::Custom => parts.push(tr!("свой период").to_string()),
            other => parts.push(
                view::PERIODS
                    .iter()
                    .find(|(period, _)| *period == other)
                    .map(|(_, title)| (*title).to_string())
                    .unwrap_or_default(),
            ),
        }
        if self.settings.hide_duplicates {
            parts.push(tr!("без повторов").to_string());
        }
        filter_summary_text(&parts)
    }
}

impl App {
    /// Current listing query; `offset` walks through the pages.
    ///
    /// Filters live in three places and are combined here: the search box (full
    /// text), the filter row (application, kind, source, period) and the two
    /// date fields when the period is a custom range.
    fn search_query(&self, offset: i64, limit: i64) -> SearchQuery {
        let text = self.text.trim();
        let mut since = view::period_since(self.period_filter, time::now_micros());
        let mut until = None;
        if self.period_filter == view::Period::Custom {
            let (from, to) =
                view::custom_range(&window_text(self.filter_from), &window_text(self.filter_to));
            since = from;
            until = to;
        }
        SearchQuery {
            text: (!text.is_empty()).then(|| text.to_string()),
            app: self.application_filter.clone(),
            kind: self.kind_filter.clone(),
            source: self.source_filter.clone(),
            since,
            until,
            hide_duplicates: self.settings.hide_duplicates,
            sort: self.sort.clone(),
            descending: self.descending,
            limit,
            offset,
            ..Default::default()
        }
    }

    /// Are any filters active?  (Used by the status line and the reset button.)
    fn filters_active(&self) -> bool {
        !self.text.trim().is_empty()
            || self.application_filter.is_some()
            || self.kind_filter.is_some()
            || self.source_filter.is_some()
            || self.period_filter != view::Period::All
            || self.settings.hide_duplicates
    }

    /// Load the first page and rebuild the list view.
    fn reload(&mut self) {
        let query = self.search_query(0, PAGE_SIZE);
        let mut error_text = None;
        match self.journal.search(&query) {
            Ok(rows) => self.rows = rows,
            Err(error) => {
                self.rows.clear();
                error_text = Some(error.to_string());
            }
        }
        self.matches = self.journal.count(&query).unwrap_or(self.rows.len() as i64);
        self.total = self
            .journal
            .stats(true)
            .map(|stats| stats.total)
            .unwrap_or_default();
        self.seen_total = self.total;
        self.trace("filling list");
        self.fill_list();
        self.trace("list filled");
        if let Some(error) = error_text {
            self.collector_text = tf!("ошибка чтения журнала: {error}", "journal read error: {error}");
        } else if self.rows.is_empty() {
            self.selected = -1;
        } else {
            self.select_first();
        }
        self.refresh_filter_lists();
        self.refresh_more_button();
        self.update_status();
    }

    /// Append the next page (the *Ещё* button).
    fn load_more(&mut self) {
        if (self.rows.len() as i64) >= self.matches {
            return;
        }
        let offset = self.rows.len() as i64;
        match self.journal.search(&self.search_query(offset, PAGE_SIZE)) {
            Ok(mut rows) => {
                if rows.is_empty() {
                    return;
                }
                let first = self.rows.len();
                self.rows.append(&mut rows);
                let images = self.image_indexes();
                for index in first..self.rows.len() {
                    let image = images.get(index).copied().unwrap_or(-1);
                    let row = self.rows[index].clone();
                    self.insert_row(index, &row, image);
                }
            }
            Err(error) => {
                self.collector_text =
                    tf!("ошибка чтения журнала: {error}", "journal read error: {error}")
            }
        }
        self.refresh_more_button();
        self.update_status();
    }

    /// Rebuild every visible row.
    fn fill_list(&mut self) {
        unsafe { SendMessageW(self.list, LVM_DELETEALLITEMS, 0, 0) };
        let images = self.image_indexes();
        // The rows paint themselves, so the icon index of every row is kept here
        // instead of being handed to the control with the item.
        self.row_images = images.clone();
        for (index, row) in self.rows.iter().enumerate() {
            self.insert_row(index, row, images.get(index).copied().unwrap_or(-1));
        }
    }

    /// Icon index of every listed row (empty when icons are switched off).
    fn image_indexes(&mut self) -> Vec<i32> {
        if !self.settings.show_app_icons {
            return Vec::new();
        }
        let aumids: Vec<Option<String>> = self
            .rows
            .iter()
            .map(|row| row.optional_text("aumid"))
            .collect();
        // The picture the notification itself carried is the second hint: a browser
        // and a script post their logo that way and register no icon at all.
        let pictures: Vec<Option<String>> = self
            .rows
            .iter()
            .map(|row| view::images(row).into_iter().next())
            .collect();
        let mut indexes = Vec::with_capacity(aumids.len());
        for (position, aumid) in aumids.iter().enumerate() {
            let picture = pictures.get(position).and_then(|value| value.as_deref());
            let index = match self.icons.as_mut() {
                Some(cache) => cache.index_for(aumid.as_deref(), picture),
                None => -1,
            };
            indexes.push(index);
        }
        indexes
    }

    /// Insert one row: the cells of every column plus its application icon.
    fn insert_row(&self, index: usize, row: &Row, image: i32) {
        for (column, text) in view::row_cells(row, &self.columns).iter().enumerate() {
            let mut buffer = wide(text);
            let mut item = LVITEMW {
                mask: LVIF_TEXT | if column == 0 { LVIF_IMAGE } else { 0 },
                i_item: index as i32,
                i_sub_item: column as i32,
                i_image: if column == 0 { image } else { 0 },
                psz_text: buffer.as_mut_ptr(),
                ..Default::default()
            };
            let message = if column == 0 {
                LVM_INSERTITEMW
            } else {
                LVM_SETITEMW
            };
            unsafe {
                SendMessageW(self.list, message, 0, &mut item as *mut LVITEMW as LPARAM);
            }
        }
    }

    /// Select and reveal the first row so that the window never looks empty.
    fn select_first(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let mut item = LVITEMW {
            i_item: 0,
            state: LVIS_SELECTED | LVIS_FOCUSED,
            state_mask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                0,
                &mut item as *mut LVITEMW as LPARAM,
            );
            SendMessageW(self.list, LVM_ENSUREVISIBLE, 0, 0);
        }
        self.selected = 0;
    }

    /// Select exactly one row (used before a context menu acts on it).
    fn select_row(&mut self, index: i32) {
        if index < 0 || index as usize >= self.rows.len() {
            return;
        }
        // A click on another row replaces the selection; the rows the user picked
        // before are cleared in the same call (item -1 = "every item").
        let mut clear = LVITEMW {
            state: 0,
            state_mask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        let mut item = LVITEMW {
            i_item: index,
            state: LVIS_SELECTED | LVIS_FOCUSED,
            state_mask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                -1isize as WPARAM,
                &mut clear as *mut LVITEMW as LPARAM,
            );
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                index as WPARAM,
                &mut item as *mut LVITEMW as LPARAM,
            );
            SendMessageW(self.list, LVM_ENSUREVISIBLE, index as WPARAM, 0);
            SetFocus(self.list);
        }
        self.selected = index;
    }

    /// Make sure the row under the pointer takes part in the selection.
    ///
    /// A right click on a row that is already selected keeps the whole selection -
    /// that is how Explorer behaves, and it is what deleting several records at
    /// once needs; a right click anywhere else selects just that row.
    fn ensure_row_selected(&mut self, index: i32) {
        if index < 0 || index as usize >= self.rows.len() {
            return;
        }
        if self.selected_rows().contains(&index) {
            self.selected = index;
            unsafe {
                SetFocus(self.list);
            }
            return;
        }
        self.select_row(index);
    }

    /// Every selected row, in the order the list shows them.
    fn selected_rows(&self) -> Vec<i32> {
        let mut indexes: Vec<i32> = Vec::new();
        let mut position = -1isize;
        // `LVM_GETNEXTITEM` walks forward from a position and answers -1 at the end.
        loop {
            let next = unsafe {
                SendMessageW(self.list, LVM_GETNEXTITEM, position as WPARAM, LVNI_SELECTED) as isize
            };
            if next < 0 || next as usize >= self.rows.len() {
                break;
            }
            indexes.push(next as i32);
            position = next;
        }
        indexes
    }

    /// Row index selected in the list view (-1 when nothing is selected).
    fn selected_row(&self) -> i32 {
        unsafe { SendMessageW(self.list, LVM_GETNEXTITEM, -1isize as WPARAM, LVNI_SELECTED) as i32 }
    }

    /// Open the window of the selected notification.
    fn open_selected(&mut self) {
        let index = self.selected_row();
        if index < 0 {
            message_box(
                self.hwnd,
                "Сначала выберите уведомление в списке.",
                MB_ICONINFORMATION,
            );
            return;
        }
        let Some(row) = self.rows.get(index as usize).cloned() else {
            return;
        };
        if !detail::open(self.hwnd, row, self.metric_scale(), self.palette) {
            message_box(
                self.hwnd,
                "Не удалось открыть окно уведомления.",
                MB_ICONWARNING,
            );
        }
    }
}

impl App {
    /// Tooltips that explain every button and field in one sentence.
    ///
    /// The toolbar is intentionally small, so the explanation of what a button
    /// does belongs somewhere the user can find without leaving the window.
    fn install_tooltips(&mut self) {
        if !feature_enabled("WNL_NO_TOOLTIPS") {
            return;
        }
        if self.tooltip.is_null() {
            self.tooltip = create_tooltip(self.hwnd).unwrap_or(std::ptr::null_mut());
        }
        if self.tooltip.is_null() {
            return;
        }
        let tips: [(HWND, &str); 15] = [
            (
                self.import_button,
                "Прочитать новые уведомления из системы прямо сейчас.\n\
                 Обычно это происходит само: сбор идёт в фоне каждые несколько секунд \
                 (интервал в настройках). Кнопка нужна, когда ждать не хочется.",
            ),
            (
                self.search,
                "Поиск по всему, что есть в записях: заголовок, текст, приложение, \
                 содержимое уведомления. Работает вместе с фильтрами справа.",
            ),
            (
                self.filter_app,
                "Показать уведомления только одного приложения. В скобках — сколько \
                 записей от него в журнале.",
            ),
            (
                self.filter_kind,
                "Тип уведомления: toast (всплывающее), tile (плитка), badge (значок на плитке).",
            ),
            (
                self.filter_source,
                "Каким сборщиком запись попала в журнал: wpndb — база уведомлений Windows, \
                 uns — WinRT-сборщик.",
            ),
            (
                self.filter_period,
                "Ограничение по времени: сегодня, последние 7 или 30 дней, свой диапазон \
                 дат. «Всё время» ничего не ограничивает.",
            ),
            (
                self.filter_from,
                "Начало своего диапазона: 29.09.2026 или 2026-09-29.",
            ),
            (self.filter_to, "Конец своего диапазона (включительно)."),
            (
                self.filter_reset,
                "Убрать поиск и все фильтры — снова видно весь журнал.\n\
                 Сочетание клавиш: Ctrl+R.",
            ),
            (
                self.more_button,
                "Показать следующую часть списка. Кнопка гаснет, когда показано всё.",
            ),
            (
                self.export_button,
                "Сохранить то, что отобрано фильтрами, в файл: CSV, JSON, TXT или HTML.",
            ),
            (
                self.settings_button,
                "Настройки: сбор, работа в фоне, вид, размер текста, папка данных.",
            ),
            (
                self.folder_button,
                "Открыть папку, где лежит журнал, настройки и журнал работы.",
            ),
            (
                self.list,
                "Двойной щелчок открывает уведомление целиком, правая кнопка — меню \
                 действий (копировать, открыть ссылку, фильтр по приложению, экспорт, удалить).",
            ),
            (
                self.status_right,
                "Папка с данными. Её можно сменить в настройках — журнал при этом переносится.",
            ),
        ];
        for (control, text) in tips {
            tooltip_attach(self.tooltip, control, text);
        }
    }

    /// Update the tooltip of the *Ещё* button with the remaining count.
    fn refresh_more_button(&mut self) {
        let remaining = (self.matches - self.rows.len() as i64).max(0);
        let label = more_button_label(remaining);
        // A caption that grew ("Ещё 428") needs a wider button, and that can push it
        // out of its row: the toolbar is laid out again, but only when the word
        // really changed - this runs after every refresh.
        if window_text(self.more_button) != label {
            set_window_text(self.more_button, &label);
            self.layout();
        }
        unsafe {
            EnableWindow(self.more_button, if remaining > 0 { 1 } else { 0 });
        }
        if !self.tooltip.is_null() {
            let hint = if remaining > 0 {
                format!("Показать следующие {PAGE_SIZE} записей: осталось {remaining}.")
            } else {
                "Показаны все записи, отобранные фильтрами.".to_string()
            };
            tooltip_set_text(self.tooltip, self.more_button, &hint);
        }
    }
}

/// The picture a row can open: a local file when the program can reach one,
/// otherwise the web address of the picture.
///
/// Web pictures are never fetched by the program itself - a download would tell
/// the sender that the notification was opened - so the address is handed to the
/// browser on request instead.
fn openable_picture(row: &Row) -> Option<(String, String)> {
    let aumid = row.optional_text("aumid");
    for reference in view::images(row) {
        match imaging::availability(&reference, aumid.as_deref(), imaging::ANY_SIZE) {
            imaging::Availability::Local(path) => {
                return Some(("Открыть изображение".to_string(), path.display().to_string()))
            }
            imaging::Availability::Remote(address) => {
                return Some(("Открыть изображение в браузере".to_string(), address))
            }
            imaging::Availability::Missing => {}
        }
    }
    None
}

impl App {
    /// Context menu of the list: everything a user wants to do with a record.
    ///
    /// `index` is the row under the pointer (-1 for the empty area, where the
    /// menu offers the whole-list commands instead).
    fn list_context_menu(&mut self, index: i32, screen_x: i32, screen_y: i32) {
        let row = (index >= 0)
            .then(|| self.rows.get(index as usize).cloned())
            .flatten();
        let link = row.as_ref().and_then(view::first_link);
        let picture = row.as_ref().and_then(openable_picture);
        let selected = self.selected_rows().len().max(1) as i64;
        let filtered = self.filters_active();
        let mut items: Vec<MenuItem> = Vec::new();
        match &row {
            Some(_) => {
                items.push(MenuItem::new(ID_CTX_OPEN, "Открыть целиком\tEnter"));
                items.push(MenuItem::separator());
                items.push(MenuItem::new(
                    ID_CTX_COPY_TEXT,
                    "Копировать текст уведомления",
                ));
                items.push(MenuItem::new(ID_CTX_COPY_TITLE, "Копировать заголовок"));
                items.push(match &link {
                    Some(_) => MenuItem::new(ID_CTX_COPY_LINK, "Копировать ссылку"),
                    None => MenuItem::disabled(ID_CTX_COPY_LINK, "Копировать ссылку"),
                });
                items.push(match &link {
                    Some(_) => MenuItem::new(ID_CTX_OPEN_LINK, "Открыть ссылку в браузере"),
                    None => MenuItem::disabled(ID_CTX_OPEN_LINK, "Открыть ссылку в браузере"),
                });
                if row.as_ref().is_some_and(|row| !view::images(row).is_empty()) {
                    items.push(match &picture {
                        Some((label, _)) => MenuItem::new(ID_CTX_IMAGE, label),
                        None => MenuItem::disabled(ID_CTX_IMAGE, "Открыть изображение"),
                    });
                }
                items.push(MenuItem::separator());
                items.push(MenuItem::new(
                    ID_CTX_FILTER_APP,
                    "Показать только это приложение",
                ));
                items.push(MenuItem::new(ID_CTX_FILTER_KIND, "Показать только этот тип"));
                items.push(MenuItem::separator());
                items.push(MenuItem::new(
                    ID_CTX_DELETE_SELECTED,
                    &delete_selected_label(selected),
                ));
                if filtered {
                    items.push(MenuItem::new(
                        ID_CTX_DELETE_MATCHING,
                        &delete_matching_label(filtered, self.matches),
                    ));
                }
                items.push(MenuItem::new(ID_CTX_DELETE_PERIOD, "Удалить за период…"));
                items.push(MenuItem::separator());
                items.push(MenuItem::new(ID_CTX_EXPORT, "Экспорт отобранного…"));
            }
            None => {
                items.push(MenuItem::new(ID_CTX_REFRESH, "Обновить список\tF5"));
                if filtered {
                    items.push(MenuItem::new(ID_CTX_RESET_FILTERS, "Сбросить фильтры"));
                }
                items.push(MenuItem::separator());
                items.push(MenuItem::new(
                    ID_CTX_DELETE_MATCHING,
                    &delete_matching_label(filtered, self.matches),
                ));
                items.push(MenuItem::new(ID_CTX_DELETE_PERIOD, "Удалить за период…"));
                items.push(MenuItem::separator());
                items.push(MenuItem::new(ID_CTX_EXPORT, "Экспорт отобранного…"));
                items.push(MenuItem::new(ID_MENU_SETTINGS, "Настройки…"));
            }
        }
        // The two shapes offer each other: a right click in the feed brings the whole
        // program back, a right click in the program opens the feed.
        items.push(MenuItem::separator());
        items.push(MenuItem::new(
            ID_CTX_SIMPLE_VIEW,
            if self.settings.simple_view {
                "Полный вид программы"
            } else {
                "Простой вид (лента)"
            },
        ));
        let Some(command) = popup_menu(self.hwnd, &items, screen_x, screen_y, Some(ID_CTX_OPEN))
        else {
            return;
        };
        match command {
            ID_CTX_OPEN => self.open_selected(),
            ID_CTX_COPY_TEXT => self.copy_from_row(index, CopyPart::Everything),
            ID_CTX_COPY_TITLE => self.copy_from_row(index, CopyPart::Title),
            ID_CTX_COPY_LINK => self.copy_from_row(index, CopyPart::Link),
            ID_CTX_OPEN_LINK => {
                if let Some(link) = link {
                    if !shell_open(self.hwnd, &link) {
                        message_box(
                            self.hwnd,
                            &format!("Не удалось открыть ссылку:\n{link}"),
                            MB_ICONWARNING,
                        );
                    }
                }
            }
            ID_CTX_IMAGE => {
                if let Some((_, target)) = picture {
                    if !shell_open(self.hwnd, &target) {
                        message_box(
                            self.hwnd,
                            &format!("Не удалось открыть изображение:\n{target}"),
                            MB_ICONWARNING,
                        );
                    }
                }
            }
            ID_CTX_FILTER_APP => {
                let aumid = row.as_ref().and_then(|row| row.optional_text("aumid"));
                self.filter_by_application(aumid.as_deref());
            }
            ID_CTX_FILTER_KIND => {
                let kind = row
                    .as_ref()
                    .and_then(|row| row.optional_text("notification_type"));
                self.filter_by_kind(kind.as_deref());
            }
            ID_CTX_SIMPLE_VIEW => self.set_simple_view(!self.settings.simple_view),
            ID_CTX_EXPORT => self.export_rows(),
            ID_CTX_DELETE_SELECTED => self.delete_selected(),
            ID_CTX_DELETE_MATCHING => self.delete_matching(),
            ID_CTX_DELETE_PERIOD => self.purge_period(),
            ID_CTX_REFRESH => self.reload(),
            ID_CTX_RESET_FILTERS => self.reset_filters(),
            ID_MENU_SETTINGS => self.show_settings(),
            _ => {}
        }
    }

    /// Copy part of a record to the clipboard.
    fn copy_from_row(&self, index: i32, part: CopyPart) {
        let Some(row) = (index >= 0)
            .then(|| self.rows.get(index as usize))
            .flatten()
        else {
            return;
        };
        let text = match part {
            CopyPart::Everything => detail::clipboard_text(row),
            CopyPart::Title => detail::heading(row),
            CopyPart::Link => view::first_link(row).unwrap_or_default(),
        };
        if text.trim().is_empty() {
            message_box(
                self.hwnd,
                "В этой записи нечего копировать.",
                MB_ICONINFORMATION,
            );
            return;
        }
        if !copy_to_clipboard(self.hwnd, &text) {
            message_box(
                self.hwnd,
                "Не удалось скопировать в буфер обмена.",
                MB_ICONWARNING,
            );
        }
    }

    /// Select every row of the list (Ctrl+A in the list).
    ///
    /// Only the selection is set, never the focus: a list view gives the focus to
    /// one row alone ("although more than one item may be selected, only one item
    /// can have the focus"), and asking for the focus on every row makes the
    /// control keep a single row and drop the rest of the selection.
    fn select_all_rows(&self) {
        let mut item = LVITEMW {
            state: LVIS_SELECTED,
            state_mask: LVIS_SELECTED,
            ..Default::default()
        };
        unsafe {
            // `-1` as the item index means "every item" for a state change.
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                (-1i64) as WPARAM,
                &mut item as *mut LVITEMW as LPARAM,
            );
        }
    }

    /// Copy the selected records to the clipboard (Ctrl+C in the list).
    ///
    /// One record is copied the way the context menu does it; several are joined
    /// by an empty line, so the result stays readable when it is pasted.
    fn copy_selection(&mut self) {
        let blocks: Vec<String> = self
            .selected_rows()
            .iter()
            .filter_map(|index| self.rows.get(*index as usize))
            .map(detail::clipboard_text)
            .filter(|text| !text.trim().is_empty())
            .collect();
        if blocks.is_empty() {
            self.note = Some((
                "копировать нечего: ничего не выбрано".to_string(),
                Instant::now(),
            ));
            return;
        }
        let text = blocks.join("\n\n");
        if copy_to_clipboard(self.hwnd, &text) {
            let word = records_word(blocks.len() as i64);
            self.note = Some((format!("скопировано: {word}"), Instant::now()));
        } else {
            message_box(
                self.hwnd,
                "Не удалось скопировать в буфер обмена.",
                MB_ICONWARNING,
            );
        }
    }

    /// Apply the application filter of a record.
    fn filter_by_application(&mut self, aumid: Option<&str>) {
        let Some(aumid) = aumid.filter(|value| !value.trim().is_empty()) else {
            message_box(
                self.hwnd,
                "У записи нет идентификатора приложения, фильтровать не по чему.",
                MB_ICONINFORMATION,
            );
            return;
        };
        // The identifier may be new to the list (a notification arrived since the
        // list was filled), so the entry is added before it is selected.
        if !self.applications.iter().any(|app| app.aumid == aumid) {
            self.applications.insert(
                0,
                crate::store::ApplicationInfo {
                    aumid: aumid.to_string(),
                    label: aumid.to_string(),
                    count: 1,
                },
            );
            self.refresh_filter_lists();
        }
        self.application_filter = Some(aumid.to_string());
        self.select_filter_lists();
        self.reload();
    }

    /// Apply the kind filter of a record.
    fn filter_by_kind(&mut self, kind: Option<&str>) {
        let Some(kind) = kind.filter(|value| !value.trim().is_empty()) else {
            message_box(
                self.hwnd,
                "У записи нет типа уведомления, фильтровать не по чему.",
                MB_ICONINFORMATION,
            );
            return;
        };
        if !self.kinds.iter().any(|(value, _)| value == kind) {
            self.kinds.insert(0, (kind.to_string(), 1));
            self.refresh_filter_lists();
        }
        self.kind_filter = Some(kind.to_string());
        self.select_filter_lists();
        self.reload();
    }

    /// Ask before an irreversible deletion.
    ///
    /// The question itself has to name what goes away and how much of it:
    /// "вы уверены?" is not a question anybody can answer, and the next step
    /// cannot be undone.  The answer is asked with the words of the action and
    /// the focus sits on «Удалить», so Enter is the deliberate key and Escape
    /// the way out (see `confirm`).
    fn confirm_delete(&self, sentence: &str) -> bool {
        let question = format!(
            "{sentence}\n\n\
             Действие необратимо. Само уведомление останется системным — сборщик \
             пропустит его как уже виденное, потому что курсор базы уведомлений \
             общий для всех записей."
        );
        crate::confirm::ask(
            self.hwnd,
            &question,
            "Удалить",
            self.metric_scale(),
            self.palette,
        )
    }

    /// Report a finished deletion and reload the list.
    fn after_delete(&mut self, deleted: i64) {
        let text = if deleted == 0 {
            tr!("удалять было нечего").to_string()
        } else {
            tf!("удалено записей: {deleted}", "records deleted: {deleted}")
        };
        self.note = Some((text, Instant::now()));
        self.reload();
    }

    /// Delete every record of the current selection (Ctrl and Shift work in the list).
    fn delete_selected(&mut self) {
        let indexes = self.selected_rows();
        let ids: Vec<i64> = indexes
            .iter()
            .filter_map(|index| self.rows.get(*index as usize))
            .filter_map(|row| row.int("id"))
            .collect();
        if ids.is_empty() {
            message_box(
                self.hwnd,
                "Сначала выберите уведомления в списке: щелчок выбирает одно, \
                 Ctrl и Shift добавляют к нему соседние.",
                MB_ICONINFORMATION,
            );
            return;
        }
        let titles: Vec<String> = indexes
            .iter()
            .filter_map(|index| self.rows.get(*index as usize))
            .map(|row| view::flatten(&detail::heading(row), 40))
            .collect();
        let sentence = format!(
            "Delete {} from the journal: {}?",
            records_word(ids.len() as i64),
            sample_titles(&titles)
        );
        if !self.confirm_delete(&sentence) {
            return;
        }
        match self.journal.delete_records(&ids) {
            Ok(deleted) => self.after_delete(deleted),
            Err(error) => message_box(
                self.hwnd,
                &format!("Не удалось удалить записи:\n{error}"),
                MB_ICONWARNING,
            ),
        }
    }

    /// Delete everything the current filters select - the whole set at once.
    ///
    /// The filters are the same ones the list uses, so what the user sees is what
    /// disappears; with no filters at all that means the whole journal, and the
    /// question says so in those words.
    fn delete_matching(&mut self) {
        let query = self.search_query(0, 1);
        let count = match self.journal.count_matching(&query) {
            Ok(count) => count,
            Err(error) => {
                message_box(
                    self.hwnd,
                    &format!("Не удалось посчитать записи:\n{error}"),
                    MB_ICONWARNING,
                );
                return;
            }
        };
        if count == 0 {
            message_box(
                self.hwnd,
                "Под текущий отбор ничего не попало.",
                MB_ICONINFORMATION,
            );
            return;
        }
        let sentence = if self.filters_active() {
            format!(
                "Delete {} from the journal using the current filter?\nFilter: {}",
                records_word(count),
                self.filter_summary()
            )
        } else {
            tf!(
                "Удалить весь журнал — {}?",
                "Delete the whole journal — {}?",
                records_word(count)
            )
        };
        if !self.confirm_delete(&sentence) {
            return;
        }
        match self.journal.delete_matching(&query) {
            Ok(deleted) => self.after_delete(deleted),
            Err(error) => message_box(
                self.hwnd,
                &format!("Не удалось удалить записи:\n{error}"),
                MB_ICONWARNING,
            ),
        }
    }

    /// Open the window that deletes a period of the journal.
    fn purge_period(&mut self) {
        let from = window_text(self.filter_from);
        let to = window_text(self.filter_to);
        if !purge::open(self.hwnd, self.metric_scale(), &from, &to, self.palette) {
            message_box(
                self.hwnd,
                "Не удалось открыть окно удаления за период.",
                MB_ICONWARNING,
            );
        }
    }

    /// How many records a period holds (asked by the period window).
    fn count_period(&self, from: &str, to: &str) -> i64 {
        match self.period_query(from, to) {
            Some(query) => self.journal.count_matching(&query).unwrap_or(-1),
            None => -1,
        }
    }

    /// Delete a period of the journal after asking (asked by the period window).
    fn delete_period(&mut self, from: &str, to: &str) -> i64 {
        let Some(query) = self.period_query(from, to) else {
            return -1;
        };
        let count = self.journal.count_matching(&query).unwrap_or(-1);
        let sentence = format!(
            "{}. Delete these records?",
            purge::summary_text(from, to, count)
        );
        if !self.confirm_delete(&sentence) {
            return 0;
        }
        match self.journal.delete_matching(&query) {
            Ok(deleted) => {
                self.after_delete(deleted);
                deleted
            }
            Err(error) => {
                message_box(
                    self.hwnd,
                    &format!("Не удалось удалить записи:\n{error}"),
                    MB_ICONWARNING,
                );
                -1
            }
        }
    }

    /// Listing query for a period; `None` when neither date is readable.
    fn period_query(&self, from: &str, to: &str) -> Option<SearchQuery> {
        let (since, until) = view::custom_range(from, to);
        if since.is_none() && until.is_none() {
            return None;
        }
        Some(SearchQuery {
            since,
            until,
            ..Default::default()
        })
    }

    /// Start the background collector (or explain why it stays off).
    fn start_collector(&mut self) {
        if !self.settings.collect_on_start {
            self.collector_text = tr!("сбор выключен в настройках").to_string();
            return;
        }
        self.collector_params = service::Params::from_layout(&self.layout, &self.settings);
        self.collector = Some(service::spawn(self.collector_params.clone()));
        self.collector_text = tr!("сбор запущен").to_string();
    }

    /// Ask the collector to finish, waiting briefly for the thread.
    fn stop_collector(&mut self) {
        if let Some(control) = self.collector.take() {
            control.stop();
            let deadline = Instant::now() + Duration::from_secs(5);
            while control.is_running() && Instant::now() < deadline {
                unsafe {
                    Sleep(20);
                }
            }
        }
    }

    /// Restart the collector (used after the settings changed).
    fn restart_collector(&mut self) {
        self.stop_collector();
        self.start_collector();
    }

    /// Look at the collector and the journal; called once per second.
    fn poll_collector(&mut self) {
        // The collector runs in this process, so its state is right here in
        // memory and is always up to date; only a collector started elsewhere
        // (`wnl-cli.exe watch`) has to be read from the file - which is written
        // rarely, so it is deliberately the second choice.
        let status = self
            .collector
            .as_ref()
            .and_then(|control| control.state())
            .or_else(|| service::Status::read(&self.layout.status));
        self.trace("status read");
        match &status {
            Some(status) => self.collector_text = service::describe(status),
            None => {
                if self.settings.collect_on_start && self.collector.is_none() {
                    self.collector_text = tr!("сбор не выполняется").to_string();
                }
            }
        }
        // Pop-ups a rule asked for: the collector thread puts them into the status
        // file, and the balloon is shown here because the tray icon is this
        // thread's.  Numbers say which of them are new; the first look only takes
        // note of where the numbering stands, so pop-ups from before this start
        // are not repeated.
        if let Some(status) = &status {
            if let Some(previous) = self.shown_alert_seq {
                for alert in status.alerts.iter().filter(|alert| alert.seq > previous) {
                    self.balloon(&alert.heading(), &alert.summary());
                }
            }
            self.shown_alert_seq = Some(status.alert_seq.max(self.shown_alert_seq.unwrap_or(0)));
        }
        self.collector_status = status;
        let total = self
            .journal
            .stats(true)
            .map(|stats| stats.total)
            .unwrap_or(self.total);
        self.trace("journal counted");
        if total != self.seen_total {
            self.reload();
        } else {
            self.update_status();
        }
        self.trace("status painted");
    }

    /// Append one line to the log when tracing is switched on (`WNL_TRACE`).
    fn trace(&self, message: &str) {
        // Tracing is a diagnostic aid on top of the log: no log, no trace.
        if !self.settings.log_enabled || !trace_enabled() {
            return;
        }
        let stamp = time::format_local_iso(Some(&time::now_iso()));
        let line = format!("{stamp}  [trace] {message}\n");
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.layout.log)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// Collect right now: ask the thread, or do it here when it is off.
    fn import_now(&mut self) {
        if let Some(control) = &self.collector {
            control.kick();
            self.collector_text = tr!("запрошен сбор…").to_string();
            self.update_status();
            return;
        }
        let started = Instant::now();
        match service::pass(&self.journal, &self.collector_params, &self.resolver, false) {
            Ok(report) => {
                self.collector_text = format!(
                    "manual collection: +{} in {} ms",
                    report.emitted,
                    started.elapsed().as_millis()
                );
                self.reload();
            }
            Err(error) => {
                self.collector_text = tf!("ошибка сбора: {error}", "collection error: {error}");
                message_box(
                    self.hwnd,
                    &format!("Не удалось выполнить сбор:\n{error}"),
                    MB_ICONWARNING,
                );
                self.update_status();
            }
        }
    }

    /// Read the whole Windows database again (cursor reset).
    fn reset_scan(&mut self) {
        let question = wide(
            "Прочитать системную базу уведомлений заново?\n\n\
             Сохранённые записи не пострадают: повторно найденные\n\
             обновят свои счётчики и ревизию.",
        );
        let caption = wide(crate::appname::APPLICATION_TITLE);
        let answer = unsafe {
            MessageBoxW(
                self.hwnd,
                question.as_ptr(),
                caption.as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            )
        };
        if answer != IDYES {
            return;
        }
        match service::pass(&self.journal, &self.collector_params, &self.resolver, true) {
            Ok(report) => {
                self.collector_text = tf!(
                    "полное чтение: +{} записей",
                    "full read: +{} records",
                    report.emitted
                );
                self.reload();
                message_box(
                    self.hwnd,
                    &format!(
                        "База уведомлений прочитана заново.\n\nОбработано записей: {}\nПорций: {}",
                        report.emitted, report.batches
                    ),
                    MB_ICONINFORMATION,
                );
            }
            Err(error) => message_box(
                self.hwnd,
                &format!("Не удалось прочитать базу уведомлений:\n{error}"),
                MB_ICONWARNING,
            ),
        }
    }

    /// Pause or resume the collection.
    fn toggle_pause(&mut self) {
        let Some(control) = &self.collector else {
            message_box(self.hwnd, "Сбор выключен в настройках.", MB_ICONINFORMATION);
            return;
        };
        let paused = !control.is_paused();
        control.set_paused(paused);
        if !paused {
            control.kick();
        }
        self.collector_text = if paused {
            tr!("сбор приостановлен").to_string()
        } else {
            tr!("сбор продолжается").to_string()
        };
        self.refresh_menu_checks();
        self.update_status();
    }
}

impl App {
    /// Write what the list currently shows into a file the user picks.
    fn export_rows(&mut self) {
        let format = export::Format::from_name(&self.settings.export_format);
        let query = self.search_query(0, EXPORT_LIMIT);
        let rows = match self.journal.search(&query) {
            Ok(rows) => rows,
            Err(error) => {
                message_box(
                    self.hwnd,
                    &format!("Не удалось прочитать журнал:\n{error}"),
                    MB_ICONWARNING,
                );
                return;
            }
        };
        if rows.is_empty() {
            message_box(
                self.hwnd,
                "Нечего экспортировать: список пуст.",
                MB_ICONINFORMATION,
            );
            return;
        }
        let stamp = view::local_time(Some(&time::now_iso()));
        let suggested = export::suggested_name(format, &file_stamp(&stamp));
        match export::save(self.hwnd, &rows, format, &suggested, &stamp) {
            Ok(path) => message_box(
                self.hwnd,
                &format!(
                    "Records exported: {}\n\n{}",
                    rows.len(),
                    path.display()
                ),
                MB_ICONINFORMATION,
            ),
            Err(error) if error.contains("отменён") || error.contains("cancelled") => {}
            Err(error) => message_box(self.hwnd, &error, MB_ICONWARNING),
        }
    }

    /// Show the data folder in Explorer.
    fn open_data_folder(&self) {
        settings_ui::open_folder(self.hwnd, &self.layout.data_dir.display().to_string());
    }
}

impl App {
    /// The settings window, followed by applying what came back.
    fn show_settings(&mut self) {
        if let Some(updated) =
            settings_ui::show(self.hwnd, &self.settings, &self.layout, self.metric_scale())
        {
            self.apply_settings(updated);
        }
    }

    /// Store the new settings and make the running program follow them.
    fn apply_settings(&mut self, mut updated: Settings) {
        // The shape of the window may be part of this update; it is rebuilt below.
        let simple_before = self.settings.simple_view;
        let theme_before = self.settings.theme.clone();
        let language_before = self.settings.language.clone();
        let lines_before = self.settings.feed_lines;
        let columns_before = self.settings.feed_columns.clone();
        // A different data folder has to be prepared before it is used.
        let current_home = self.layout.data_dir.clone();
        match updated.data_dir.clone() {
            Some(requested) if requested != current_home => {
                if self.move_data_folder(&current_home, &requested) {
                    updated.data_dir = Some(requested);
                } else {
                    updated.data_dir = Some(current_home.clone());
                }
            }
            None => {
                // "Automatic" would move the journal behind the user's back;
                // the folder in use stays.
                updated.data_dir = Some(current_home.clone());
            }
            _ => {}
        }

        // Autostart follows the setting, and the registry is the truth.
        let exe = executable_path();
        if updated.start_with_windows != autostart::is_enabled(&exe) {
            if let Err(error) = autostart::set_enabled(&exe, updated.start_with_windows) {
                message_box(
                    self.hwnd,
                    &format!("Автозапуск не изменён: {error}"),
                    MB_ICONWARNING,
                );
            }
        }

        // The window on screen is newer than what the settings window read when it
        // opened: it is kept for the shape it belongs to, whichever shape that is.
        self.save_window_geometry();
        if self.settings.simple_view {
            updated.feed_window = self.settings.feed_window;
        } else {
            updated.window = self.settings.window;
        }
        if let Err(error) = self
            .settings_for_storage(updated.clone())
            .save(&self.layout.settings_path)
        {
            message_box(
                self.hwnd,
                &format!("Не удалось сохранить настройки:\n{error}"),
                MB_ICONWARNING,
            );
        }
        self.settings = updated;
        // The look may have been changed in the settings window, and so may the feed's
        // own columns and its row height.
        if self.settings.theme != theme_before {
            self.apply_look();
        }
        // The wording of every control is decided when the control is created, so
        // a change of the language is followed by everything that is built from
        // now on (dialogs, new windows) and the program says how to get the rest.
        if self.settings.language != language_before {
            crate::lang::set(crate::lang::Lang::from_code(&self.settings.language));
            let title = wide(&self.window_title());
            unsafe {
                SetWindowTextW(self.hwnd, title.as_ptr());
            }
            self.update_status();
            message_box(
                self.hwnd,
                tr!("Язык изменён. Надписи окна обновятся после перезапуска программы."),
                MB_ICONINFORMATION,
            );
        }
        // A switch between the feed and the program rebuilds the report and gives the
        // window the size of the shape that comes back; the menu bar is the same in
        // both shapes and only needs its ticks refreshed.
        if self.settings.simple_view != simple_before {
            self.resize_for_mode();
            self.build_columns();
            let title = wide(&self.window_title());
            unsafe {
                SetWindowTextW(self.hwnd, title.as_ptr());
            }
        } else if self.settings.feed_columns != columns_before {
            self.build_columns();
        }
        if self.settings.feed_lines != lines_before {
            // The height of a row is decided when the list is created.
            self.recreate_list();
        }
        // The picture setting takes effect at once; where the copies live is
        // fixed for the run (the folder is part of the layout).
        imaging::configure(self.layout.pictures.clone(), self.settings.keep_pictures);
        self.collector_params = service::Params::from_layout(&self.layout, &self.settings);
        self.apply_column_widths();
        self.refresh_icons();
        self.restart_collector();
        self.reload();
        self.layout();
        self.refresh_menu_checks();
        self.update_status();
    }

    /// Copy the journal into another folder; true when the move succeeded.
    fn move_data_folder(&mut self, from: &std::path::Path, to: &std::path::Path) -> bool {
        let question = wide(&format!(
            "Перенести журнал в новую папку?\n\nОткуда: {}\nКуда: {}\n\n\
             «Нет» оставит журнал на прежнем месте.",
            from.display(),
            to.display()
        ));
        let caption = wide(crate::appname::APPLICATION_TITLE);
        let answer = unsafe {
            MessageBoxW(
                self.hwnd,
                question.as_ptr(),
                caption.as_ptr(),
                MB_YESNO | MB_ICONQUESTION,
            )
        };
        if answer != IDYES {
            return false;
        }
        if let Err(error) = std::fs::create_dir_all(to) {
            message_box(
                self.hwnd,
                &format!("Не удалось создать папку:\n{error}"),
                MB_ICONWARNING,
            );
            return false;
        }
        let from_journal = from.join(storage::JOURNAL_FILE);
        let to_journal = to.join(storage::JOURNAL_FILE);
        if from_journal.exists() && !to_journal.exists() {
            if let Err(error) = std::fs::copy(&from_journal, &to_journal) {
                message_box(
                    self.hwnd,
                    &format!("Не удалось скопировать журнал:\n{error}"),
                    MB_ICONWARNING,
                );
                return false;
            }
        }
        // Reopen the journal in the new place; the old file stays as a backup.
        self.stop_collector();
        match Journal::open(&to_journal) {
            Ok(journal) => {
                self.journal = journal;
                self.layout =
                    storage::Layout::build(to, self.layout.settings_path.clone(), false, false);
                self.resolver = NameResolver::with_cache(
                    self.journal
                        .app_name_map()
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                );
                message_box(
                    self.hwnd,
                    &format!(
                        "Журнал перенесён в:\n{}\n\nСтарый файл оставлен как резервная копия.",
                        to.display()
                    ),
                    MB_ICONINFORMATION,
                );
                true
            }
            Err(error) => {
                message_box(
                    self.hwnd,
                    &format!("Не удалось открыть журнал в новой папке:\n{error}"),
                    MB_ICONWARNING,
                );
                false
            }
        }
    }
}

impl App {
    /// Add the notification area icon.
    fn add_tray(&mut self) {
        if self.tray_added {
            return;
        }
        let mut data = NOTIFYICONDATAW::default();
        data.hwnd = self.hwnd;
        data.u_id = TRAY_ID;
        data.u_flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.u_callback_message = WM_APP_TRAY;
        data.h_icon = appicon::application_icon();
        data.set_tip(&format!("{} {VERSION}", crate::appname::APPLICATION_TITLE));
        self.tray_added = unsafe { Shell_NotifyIconW(NIM_ADD, &mut data) } != 0;
    }

    /// Remove the notification area icon.
    fn remove_tray(&mut self) {
        if !self.tray_added {
            return;
        }
        let mut data = NOTIFYICONDATAW::default();
        data.hwnd = self.hwnd;
        data.u_id = TRAY_ID;
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &mut data);
        }
        self.tray_added = false;
    }

    /// Show a balloon at the tray icon.
    ///
    /// This is what a rule with the action `alert` asks for; the text arrives from
    /// the collector thread through `status.json` (see `poll_collector`).
    fn balloon(&self, title: &str, text: &str) {
        if !self.tray_added {
            return;
        }
        let mut data = NOTIFYICONDATAW::default();
        data.hwnd = self.hwnd;
        data.u_id = TRAY_ID;
        data.u_flags = NIF_INFO;
        fill_wide(&mut data.sz_info_title, title);
        fill_wide(&mut data.sz_info, text);
        data.dw_info_flags = NIIF_INFO;
        data.u_timeout_or_version = 10_000;
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &mut data);
        }
    }

    /// Tell the user (once) that the program continues in the background.
    fn balloon_once(&mut self) {
        if self.tray_balloon_shown || !self.tray_added {
            return;
        }
        self.tray_balloon_shown = true;
        let mut data = NOTIFYICONDATAW::default();
        data.hwnd = self.hwnd;
        data.u_id = TRAY_ID;
        data.u_flags = NIF_INFO;
        fill_wide(&mut data.sz_info_title, "Продолжаю работу");
        fill_wide(
            &mut data.sz_info,
            "Сбор уведомлений идёт в фоне. Открыть окно — двойной щелчок по значку.",
        );
        data.dw_info_flags = NIIF_INFO | NIIF_NOSOUND;
        data.u_timeout_or_version = 8000;
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &mut data);
        }
    }

    /// Hide the window, keeping the program (and the collection) alive.
    fn hide_to_tray(&mut self) {
        if !self.tray_added {
            // Without a tray icon hiding would strand the user: quit instead.
            self.about_to_quit = true;
            unsafe {
                DestroyWindow(self.hwnd);
            }
            return;
        }
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        self.balloon_once();
    }

    /// Bring the window back from the notification area.
    fn restore(&mut self) {
        unsafe {
            ShowWindow(self.hwnd, SW_RESTORE);
            SetForegroundWindow(self.hwnd);
        }
    }

    /// Tray context menu.
    fn show_tray_menu(&mut self) {
        let paused = self
            .collector
            .as_ref()
            .is_some_and(|control| control.is_paused());
        let items = vec![
            MenuItem::new(ID_TRAY_OPEN, "Открыть журнал"),
            MenuItem::new(ID_TRAY_IMPORT, "Проверить сейчас"),
            MenuItem::new(
                ID_TRAY_PAUSE,
                if paused {
                    "Продолжить сбор"
                } else {
                    "Приостановить сбор"
                },
            ),
            MenuItem::separator(),
            // The feed is one click away from the notification area as well.
            MenuItem::new(
                ID_TRAY_SIMPLE_VIEW,
                if self.settings.simple_view {
                    "Полный вид программы"
                } else {
                    "Простой вид (лента)"
                },
            ),
            MenuItem::separator(),
            MenuItem::new(ID_TRAY_SETTINGS, "Настройки…"),
            MenuItem::new(ID_TRAY_FOLDER, "Папка с данными"),
            MenuItem::separator(),
            MenuItem::new(ID_TRAY_EXIT, "Выход"),
        ];
        match popup_menu(self.hwnd, &items, -1, -1, None) {
            Some(ID_TRAY_SIMPLE_VIEW) => {
                self.restore();
                self.set_simple_view(!self.settings.simple_view);
            }
            Some(ID_TRAY_OPEN) => self.restore(),
            Some(ID_TRAY_IMPORT) => {
                self.restore();
                self.import_now();
            }
            Some(ID_TRAY_PAUSE) => self.toggle_pause(),
            Some(ID_TRAY_SETTINGS) => {
                self.restore();
                self.show_settings();
            }
            Some(ID_TRAY_FOLDER) => self.open_data_folder(),
            Some(ID_TRAY_EXIT) => {
                self.about_to_quit = true;
                unsafe {
                    DestroyWindow(self.hwnd);
                }
            }
            _ => {}
        }
    }

    /// Remember the geometry of the shape the window is in right now.
    ///
    /// A narrow, tall feed is exactly what the feed is for, so the rule that keeps
    /// the full window usable does not apply to it - only a window that could not
    /// show one line is left out.
    fn save_window_geometry(&mut self) {
        if unsafe { IsIconic(self.hwnd) } != 0 {
            return;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut rect) } == 0 {
            return;
        }
        let (least_width, least_height) = minimum_window_size(self.settings.simple_view);
        if rect.width() < self.px(least_width) || rect.height() < self.px(least_height) {
            return;
        }
        let geometry = [rect.left, rect.top, rect.width(), rect.height()];
        if self.settings.simple_view {
            self.settings.feed_window = Some(geometry);
        } else {
            self.settings.window = Some(geometry);
        }
    }

    /// Give the window the size the shape it is about to show asks for.
    ///
    /// Each shape keeps its own window: a narrow, tall feed survives a look at the
    /// full program, and the full window comes back in its own size.  A shape that
    /// was never arranged by the user gets at least the size it can work in - which
    /// is why a tiny feed does not stay tiny when the program comes back.
    fn resize_for_mode(&self) {
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut rect) } == 0 {
            return;
        }
        let (x, y, width, height) = if self.settings.simple_view {
            feed_window_geometry(&self.settings)
        } else {
            window_geometry(&self.settings)
        };
        let (least_width, least_height) = minimum_window_size(self.settings.simple_view);
        // `CW_USEDEFAULT` is not a position: the window keeps the place it has, it
        // only gets the size of the shape that is coming back.
        let place_kept = x == CW_USEDEFAULT || y == CW_USEDEFAULT;
        let flags = SWP_NOZORDER
            | SWP_NOACTIVATE
            | if place_kept { SWP_NOMOVE } else { 0 };
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                if place_kept { 0 } else { x },
                if place_kept { 0 } else { y },
                width.max(self.px(least_width)),
                height.max(self.px(least_height)),
                flags,
            );
        }
    }

    /// Settings as they belong in the file.
    ///
    /// A storage folder that only came from `--home` is dropped: the switch
    /// applies to one run, and a start without it must use the stored folder
    /// again.  A folder the user picked in the settings window is a decision and
    /// is kept - `data_dir` then differs from the temporary override.
    fn settings_for_storage(&self, current: Settings) -> Settings {
        let mut settings = current;
        if self.home_override && settings.data_dir == self.settings.data_dir {
            settings.data_dir = self.stored_data_dir.clone();
        }
        settings
    }

    /// Store the state and stop everything before the process ends.
    fn shutdown(&mut self) {
        self.save_window_geometry();
        let _ = self
            .settings_for_storage(self.settings.clone())
            .save(&self.layout.settings_path);
        self.stop_collector();
        self.remove_tray();
        if !self.font.is_null() {
            unsafe {
                DeleteObject(self.font as HGDIOBJ);
            }
            self.font = std::ptr::null_mut();
        }
    }

    /// Pump messages for a while and describe the state (used by `--selftest`).
    fn selftest(&mut self, duration: Duration) -> String {
        let deadline = Instant::now() + duration;
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while Instant::now() < deadline {
                if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if IsDialogMessageW(self.hwnd, &mut message) == 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                } else {
                    Sleep(15);
                }
            }
        }
        self.poll_collector();
        let items = unsafe { SendMessageW(self.list, LVM_GETITEMCOUNT, 0, 0) };
        let visible = self
            .columns
            .iter()
            .filter(|column| !column.technical || self.settings.show_technical_columns)
            .count();
        format!(
            "selftest: ok\n\
             window: {}\n\
             visible: {}\n\
             data dir: {}\n\
             journal: {}\n\
             records in journal: {}\n\
             matching rows: {}\n\
             rows loaded: {}\n\
             list items: {}\n\
             columns: {visible} of {}\n\
             technical columns: {}\n\
             text size: {} %\n\
             app icons: {}\n\
             tray icon: {}\n\
             collector running: {}\n\
             collector: {}\n\
             autostart: {}\n\
             filters: {}\n\
             status: {}\n",
            if self.hwnd.is_null() {
                "missing"
            } else {
                "created"
            },
            if unsafe { IsWindowVisible(self.hwnd) } != 0 {
                "shown"
            } else {
                "hidden (tray)"
            },
            self.layout.data_dir.display(),
            self.layout.journal.display(),
            self.total,
            self.matches,
            self.rows.len(),
            items,
            self.columns.len(),
            self.settings.show_technical_columns,
            self.font_scale,
            self.settings.show_app_icons,
            if self.tray_added { "added" } else { "missing" },
            self.collector
                .as_ref()
                .is_some_and(|control| control.is_running()),
            self.collector_text,
            autostart::is_enabled(&executable_path()),
            if self.filters_active() {
                self.filter_summary()
            } else {
                tr!("нет").to_string()
            },
            format!(
                "{} | {}",
                self.status_left_text(),
                self.status_middle_text()
            ),
        )
    }

    /// Text currently shown in the left status field (for the selftest report).
    fn status_left_text(&self) -> String {
        window_text(self.status)
    }

    /// Text of the middle field (the collector report).
    fn status_middle_text(&self) -> String {
        window_text(self.status_middle)
    }

    /// Text of the right field (the data folder).
    fn status_folder_text(&self) -> String {
        window_text(self.status_right)
    }
}

impl App {
    /// Switch between the feed and the whole program.
    ///
    /// The setting is written straight away, so the chosen shape survives a restart,
    /// and the window is rebuilt in it: geometry, columns, icons, layout and title.
    /// The menu bar stays where it is - it is the same in both shapes.
    fn set_simple_view(&mut self, enabled: bool) {
        if self.settings.simple_view == enabled {
            return;
        }
        // The shape that is being left keeps its window; the one that comes back
        // gets its own size (or the least size it can work in).
        self.save_window_geometry();
        self.settings.simple_view = enabled;
        let title = wide(&self.window_title());
        unsafe {
            SetWindowTextW(self.hwnd, title.as_ptr());
        }
        self.resize_for_mode();
        let _ = self.settings.save(&self.layout.settings_path);
        // The height of a row is decided when the list is created, and the feed and
        // the full list do not want the same height (see `row_height`).
        self.recreate_list();
        self.refresh_menu_checks();
        self.update_status();
    }

    /// Caption of the window; it names the shape the window is in.
    fn window_title(&self) -> String {
        if self.settings.simple_view {
            format!(
                "{} {} - лента уведомлений",
                crate::appname::APPLICATION_TITLE,
                crate::VERSION
            )
        } else {
            crate::appname::window_title()
        }
    }

    /// Switch the application icons on or off.
    fn toggle_icons(&mut self) {
        self.settings.show_app_icons = !self.settings.show_app_icons;
        let _ = self.settings.save(&self.layout.settings_path);
        self.refresh_icons();
        self.reload();
        self.refresh_menu_checks();
    }

    /// Hide (or show) notifications that only repeat an earlier one.
    fn toggle_hide_duplicates(&mut self) {
        self.settings.hide_duplicates = !self.settings.hide_duplicates;
        let _ = self.settings.save(&self.layout.settings_path);
        self.reload();
        self.refresh_menu_checks();
    }

    /// Show or hide the technical columns.
    fn toggle_technical_columns(&mut self) {
        self.settings.show_technical_columns = !self.settings.show_technical_columns;
        let _ = self.settings.save(&self.layout.settings_path);
        self.apply_column_widths();
        self.refresh_menu_checks();
    }

    /// Run the command one of the groups of menu entries stands for.
    ///
    /// The group says which table the id belongs to, the index says which entry of
    /// it was chosen (see `menu_groups`).
    fn run_menu_group(&mut self, group: MenuGroup, index: usize) {
        match group {
            MenuGroup::Font => {
                let scale = crate::settings::FONT_SCALES
                    .get(index)
                    .copied()
                    .unwrap_or(100);
                self.set_font_scale(scale);
            }
            MenuGroup::FeedColumn => {
                if let Some(key) = view::FEED_COLUMN_KEYS.get(index) {
                    self.toggle_feed_column(key);
                }
            }
            MenuGroup::FeedLines => {
                let lines = crate::settings::FEED_LINES
                    .get(index)
                    .copied()
                    .unwrap_or(crate::settings::MIN_FEED_LINES);
                self.set_feed_lines(lines);
            }
            MenuGroup::Theme => {
                let theme = crate::settings::THEMES
                    .get(index)
                    .copied()
                    .unwrap_or(crate::settings::THEMES[0]);
                self.set_theme(theme);
            }
        }
    }

    /// Show or hide one column of the feed.
    ///
    /// The last column is never switched off - a feed without columns would show
    /// nothing at all - and the order follows the menu, so the strip looks the same
    /// however the keys were added.
    fn toggle_feed_column(&mut self, key: &str) {
        let mut chosen = self.settings.feed_columns.clone();
        match chosen.iter().position(|existing| existing == key) {
            Some(position) => {
                if chosen.len() <= 1 {
                    self.note = Some((
                        tr!("последний столбец ленты выключать нельзя").to_string(),
                        Instant::now(),
                    ));
                    return;
                }
                chosen.remove(position);
            }
            None => chosen.push(key.to_string()),
        }
        chosen.sort_by_key(|existing| {
            view::FEED_COLUMN_KEYS
                .iter()
                .position(|offered| offered == existing)
                .unwrap_or(usize::MAX)
        });
        chosen.dedup();
        self.settings.feed_columns = chosen;
        let _ = self.settings.save(&self.layout.settings_path);
        self.build_columns();
        self.fill_list();
        self.layout();
        self.select_first();
        self.refresh_menu_checks();
        self.update_status();
    }

    /// How many lines of text one row of the feed may use.
    fn set_feed_lines(&mut self, lines: u32) {
        let lines = lines.clamp(
            crate::settings::MIN_FEED_LINES,
            crate::settings::MAX_FEED_LINES,
        );
        if self.settings.feed_lines == lines {
            return;
        }
        self.settings.feed_lines = lines;
        let _ = self.settings.save(&self.layout.settings_path);
        // The height of a row is decided when the list is created.
        self.recreate_list();
        self.refresh_menu_checks();
        self.update_status();
    }

    /// The look of the program: day or night.
    fn set_theme(&mut self, theme: &str) {
        if self.settings.theme == theme || !crate::settings::THEMES.contains(&theme) {
            return;
        }
        self.settings.theme = theme.to_string();
        let _ = self.settings.save(&self.layout.settings_path);
        self.apply_look();
        self.refresh_menu_checks();
    }

    /// Make the whole program follow the chosen look.
    ///
    /// The system is asked once for dark menus, dark scroll bars and dark tooltips;
    /// everything else is painted by the windows themselves, because Windows has no
    /// dark palette for the classic controls.
    fn apply_look(&mut self) {
        self.palette = style::Palette::from_setting(&self.settings.theme);
        set_preferred_app_mode(self.palette.dark);
        dark_title_bar(self.hwnd, self.palette.dark);
        for control in self.controls() {
            theme_control(control, self.palette.dark);
        }
        theme_control(self.list_header(), self.palette.dark);
        unsafe {
            SendMessageW(self.list, LVM_SETBKCOLOR, 0, self.palette.list as LPARAM);
            SendMessageW(self.list, LVM_SETTEXTBKCOLOR, 0, self.palette.list as LPARAM);
            SendMessageW(self.list, LVM_SETTEXTCOLOR, 0, self.palette.text as LPARAM);
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
            InvalidateRect(self.list, std::ptr::null(), 1);
            for control in self.controls() {
                InvalidateRect(control, std::ptr::null(), 1);
            }
        }
    }

    /// Sort by the column the user clicked.
    fn sort_by_column(&mut self, index: usize) {
        let key = self
            .columns
            .get(index)
            .map(|column| view::sort_key(column))
            .unwrap_or("time");
        if self.sort == key {
            self.descending = !self.descending;
        } else {
            self.sort = key.to_string();
            self.descending = true;
        }
        self.reload();
    }
}

/// The `GWLP_USERDATA` of the main window as the application state.
unsafe fn app_ref(hwnd: HWND) -> Option<&'static mut App> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    raw.as_mut()
}

/// Content of the *О программе* box: what the program does and where data lives.
fn about_text(layout: &Layout) -> String {
    tf!(
        "{} {VERSION}\n\n\
         Нативная сборка на Rust: один исполняемый файл без интерпретатора.\n\
         Уведомления читаются из системной базы Windows и сохраняются в журнал\n\
         SQLite с полнотекстовым поиском. Сбор идёт в фоне, окно можно закрыть.\n\n\
         Данные: {}\n\
         Журнал: {}\n\
         Настройки: {}\n\n\
         Двойной щелчок по уведомлению открывает его целиком; значки и\n\
         технические столбцы включаются в настройках и в меню «Вид».",
        "{} {VERSION}\n\n\
         Native build in Rust: a single executable, no interpreter.\n\
         Notifications are read from the Windows database and kept in a SQLite\n\
         journal with full-text search. Collection runs in the background; the\n\
         window may be closed.\n\n\
         Data: {}\n\
         Journal: {}\n\
         Settings: {}\n\n\
         A double click on a notification opens it in full; icons and the\n\
         technical columns are switched on in the settings and in the View menu.",
        crate::appname::APPLICATION_TITLE,
        layout.data_dir.display(),
        layout.journal.display(),
        layout.settings_path.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_geometry_uses_the_remembered_size() {
        let mut settings = Settings::default();
        assert_eq!(
            window_geometry(&settings),
            (CW_USEDEFAULT, CW_USEDEFAULT, 1280, 800)
        );
        settings.window = Some([10, 20, 1000, 700]);
        assert_eq!(window_geometry(&settings), (10, 20, 1000, 700));
        settings.window = Some([10, 20, 0, 0]);
        assert_eq!(
            window_geometry(&settings).2,
            1280,
            "a broken size falls back"
        );
    }

    /// Each shape of the window remembers its own geometry: the feed is a narrow,
    /// tall strip by default, and it has its own slot in the settings.
    #[test]
    fn the_two_shapes_remember_their_own_window() {
        let mut settings = Settings::default();
        let (_, _, width, height) = feed_window_geometry(&settings);
        assert!(
            width < height,
            "the feed starts as a strip ({width}×{height})"
        );
        settings.window = Some([10, 20, 1280, 800]);
        assert_eq!(
            feed_window_geometry(&settings).2,
            width,
            "the full window does not decide the shape of the feed"
        );
        settings.feed_window = Some([30, 40, 300, 900]);
        assert_eq!(feed_window_geometry(&settings), (30, 40, 300, 900));
    }

    /// The toolbar holds one line while the search field keeps its least width and
    /// wraps the buttons under it when the window is too narrow, so two controls
    /// never sit on top of each other.
    #[test]
    fn the_toolbar_wraps_instead_of_overlapping() {
        let buttons = [120, 60, 90, 80, 60];
        let gaps = 5 * 8;
        let total: i32 = buttons.iter().sum::<i32>() + gaps;

        // Room for everything: one line, the field takes what is left.
        let (search, rows) = toolbar_rows(1200, 50, 8, 140, &buttons);
        assert_eq!(rows, vec![0, 0, 0, 0, 0]);
        assert_eq!(search, 1200 - 50 - 8 - total - 8);

        // No room beside the field: the field keeps the first line and the buttons
        // flow underneath it.
        let (search, rows) = toolbar_rows(420, 50, 8, 140, &buttons);
        assert_eq!(search, 420 - 50 - 8);
        assert_eq!(rows[0], 1, "the first button starts the second line");
        assert!(
            rows.windows(2).all(|pair| pair[0] <= pair[1]),
            "a button never moves back up: {rows:?}"
        );
        assert!(rows.iter().all(|row| *row < 3), "two lines are enough: {rows:?}");

        // A window that is being created has no client area yet.
        let (search, rows) = toolbar_rows(0, 50, 8, 140, &buttons);
        assert_eq!(search, 0);
        assert_eq!(rows, vec![0, 0, 0, 0, 0]);
    }

    /// The three fields of the status line share it, and a field that would only
    /// show an ellipsis is dropped instead of being squeezed to nothing.
    #[test]
    fn the_status_line_gives_up_fields_instead_of_overlapping() {
        let (left, middle, right) = status_widths(1000, 8, 120);
        assert!(right >= 120);
        assert_eq!(left + middle + right + 16, 1000, "the three fill the line");

        let (left, middle, right) = status_widths(300, 8, 120);
        assert_eq!(right, 0, "the folder line goes first");
        assert!(middle >= 120);
        assert_eq!(left + middle + 8, 300);

        let (left, middle, right) = status_widths(100, 8, 120);
        assert_eq!((middle, right), (0, 0), "one field is better than three slits");
        assert_eq!(left, 100);
        assert_eq!(status_widths(0, 8, 120), (0, 0, 0));
    }

    /// The feed gives the source a readable slice of the row and leaves the rest to
    /// the message - in a strip and in a wide window alike.  The two columns always
    /// add up to the room the list has, so a narrow feed shows no horizontal bar.
    #[test]
    fn the_feed_columns_follow_the_window() {
        assert_eq!(feed_source_width(900, 64, 260), 260);
        assert_eq!(feed_source_width(600, 64, 260), 180);
        assert_eq!(feed_source_width(200, 64, 260), 64);
        assert_eq!(
            feed_source_width(600, 260, 64),
            180,
            "limits given the other way round are understood as well"
        );

        // A window that is too small for the minimum keeps the message a quarter of
        // the row instead of opening a scroll bar.
        for inner in [40, 100, 160] {
            let source = feed_source_width(inner, 100, 325);
            assert!(source >= 0, "{inner}");
            assert!(source <= inner * 3 / 4, "{inner}: {source} leaves no message");
        }
        assert_eq!(feed_source_width(0, 80, 260), 0);
        assert_eq!(feed_source_width(-40, 80, 260), 0);
    }

    /// Every id of every group is reachable, and no group reaches into the next one.
    ///
    /// This is the test that would have caught the font sizes swallowing the feed's
    /// own entries: a range that is one id too wide makes a menu entry do nothing.
    #[test]
    fn the_menu_group_ids_do_not_overlap() {
        let groups = menu_groups();
        let mut ids: Vec<usize> = Vec::new();
        for (group, first, count) in groups {
            assert!(count > 0, "{group:?} offers nothing");
            for index in 0..count {
                assert_eq!(
                    MenuGroup::of(first + index),
                    Some((group, index)),
                    "the entry {index} of {group:?} is not reachable"
                );
                ids.push(first + index);
            }
        }
        let offered = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), offered, "one id stands for two entries");

        // In front of the first group and behind the last one: nothing.
        let (_, first, _) = groups[0];
        let (_, last_first, last_count) = groups[groups.len() - 1];
        assert_eq!(MenuGroup::of(first - 1), None);
        assert_eq!(MenuGroup::of(last_first + last_count), None);
        // And the entries that stand alone are not caught by a group.
        assert!(ID_MENU_SIMPLE_VIEW < first, "the groups start above the single ids");
        assert_eq!(MenuGroup::of(ID_MENU_SIMPLE_VIEW), None);
    }

    /// The feed gives the message the room that is left, and in a window that is too
    /// narrow for the fixed columns together with the least room of the message it
    /// shrinks them all - the two columns of the default feed used to be a special
    /// case of exactly this rule.
    #[test]
    fn the_feed_columns_add_up_to_the_window() {
        // Room for everything: the fixed columns keep their width.
        let widths = feed_column_widths(1000, &[130, 200], 90);
        assert_eq!(widths, vec![130, 200, 670]);
        assert_eq!(widths.iter().sum::<i32>(), 1000, "no horizontal scroll bar");

        // A narrow window: the message keeps at least its least room and the fixed
        // columns give way together.
        let widths = feed_column_widths(300, &[130, 200], 90);
        assert!(
            widths[2] >= 90,
            "the message keeps its room: {widths:?}"
        );
        assert!(widths[0] < 130 && widths[1] < 200, "both gave way: {widths:?}");
        assert_eq!(widths.iter().sum::<i32>(), 300);

        // One column only: the whole row is the message.
        assert_eq!(feed_column_widths(400, &[], 90), vec![400]);
        // A window that is barely there still adds up.
        assert_eq!(feed_column_widths(0, &[100], 90), vec![0, 0]);
        assert_eq!(feed_column_widths(-40, &[100, 50], 90).iter().sum::<i32>(), 0);
    }

    /// The feed may be a strip; the full program keeps the room its toolbars need.
    #[test]
    fn the_feed_may_be_a_strip() {
        let (width, height) = minimum_window_size(true);
        assert_eq!(
            (width, height),
            (
                crate::settings::MIN_FEED_WIDTH,
                crate::settings::MIN_FEED_HEIGHT
            )
        );
        let (full_width, full_height) = minimum_window_size(false);
        assert!(width < full_width, "the feed is the narrow shape");
        assert!(height < full_height, "and the short one");
    }

    #[test]
    fn export_file_stamps_are_compact() {
        assert_eq!(file_stamp("29.09.2026 21:47:35"), "20260929-214735");
        assert_eq!(file_stamp("не дата"), "export");
    }

    #[test]
    fn the_text_size_scales_the_whole_window() {
        // 96 DPI at 100 %: the classic Windows look, nothing scaled.
        assert_eq!(effective_scale(96, 100), 96);
        // The monitor alone (125 %) and the reader's choice multiply.
        assert_eq!(effective_scale(120, 100), 120);
        assert_eq!(effective_scale(96, 150), 144);
        assert_eq!(effective_scale(120, 150), 180);
        // Values outside the offered range are pulled back in.
        assert_eq!(effective_scale(96, 5), 96 * 80 / 100);
        assert_eq!(effective_scale(96, 5000), 96 * 200 / 100);
        assert_eq!(effective_scale(0, 100), 96, "an unknown DPI means 96");
    }

    #[test]
    fn the_more_button_says_how_much_is_left() {
        assert_eq!(more_button_label(0), "Ещё");
        assert_eq!(more_button_label(-5), "Ещё", "never a negative count");
        assert_eq!(more_button_label(1), "Ещё 1");
        assert_eq!(more_button_label(1234), "Ещё 1234");
    }

    #[test]
    fn the_filter_summary_names_what_is_filtered() {
        assert_eq!(
            filter_summary_text(&[]),
            "",
            "no filters means no extra line in the status area"
        );
        assert_eq!(
            filter_summary_text(&["без повторов".to_string()]),
            "фильтры: без повторов"
        );
        assert_eq!(
            filter_summary_text(&[
                "поиск «отчёт»".to_string(),
                "приложение: Почта".to_string(),
                "Сегодня".to_string(),
            ]),
            "фильтры: поиск «отчёт», приложение: Почта, Сегодня"
        );
    }

    #[test]
    fn application_names_stay_readable() {
        assert_eq!(application_label("Почта", 5), "Почта  (5)");
        assert_eq!(
            application_label("Почта", 1),
            "Почта",
            "a single record needs no count"
        );
        let long = "Microsoft.Windows.PowerShell";
        assert_eq!(
            application_label(long, 20),
            long,
            "a long identifier keeps its whole name instead of an ellipsis"
        );
    }

    #[test]
    fn about_text_mentions_where_the_data_is() {
        let layout = Layout::build(
            std::path::Path::new(r"D:\журнал"),
            PathBuf::from(r"D:\журнал\settings.json"),
            true,
            false,
        );
        let text = about_text(&layout);
        assert!(text.contains(r"D:\журнал"));
        assert!(text.contains("history.db"));
        assert!(text.contains("settings.json"));
        assert!(text.contains(VERSION));
    }

    #[test]
    fn deletion_names_the_records_and_their_number() {
        // Deletion is irreversible, so the wording has to be exact ...
        assert_eq!(records_word(1), "1 запись");
        assert_eq!(records_word(3), "3 записи");
        assert_eq!(records_word(21), "21 запись");
        assert_eq!(records_word(11), "11 записей");
        assert_eq!(records_word(111), "111 записей");
        assert_eq!(records_word(0), "0 записей");

        // ... and the question has to say which records are meant.
        assert_eq!(
            sample_titles(&["Письмо".to_string()]),
            "«Письмо»"
        );
        assert_eq!(
            sample_titles(&["A".to_string(), "B".to_string(), "C".to_string()]),
            "«A», «B», «C»"
        );
        assert_eq!(
            sample_titles(&[
                "A".to_string(),
                "B".to_string(),
                "C".to_string(),
                "D".to_string(),
                "E".to_string()
            ]),
            "«A», «B», «C», и ещё 2"
        );

        // The menu offers the right command for one record and for many.
        assert_eq!(
            delete_selected_label(1),
            "Удалить запись из журнала…"
        );
        assert_eq!(delete_selected_label(4), "Удалить выбранные (4)…");
        assert_eq!(
            delete_matching_label(true, 12),
            "Удалить отобранное (12)…"
        );
        assert_eq!(
            delete_matching_label(false, 12),
            "Удалить все записи (12)…",
            "without filters the whole journal is meant, and the menu says so"
        );
        assert_eq!(
            delete_matching_label(true, crate::store::COUNT_CAP + 1),
            format!("Удалить отобранное ({}+)…", crate::store::COUNT_CAP)
        );
    }

    #[test]
    fn a_fresh_message_keeps_the_status_line_for_a_moment() {
        let collector = "сбор: 13:54:46  новых нет  •  записей 21";
        // Just happened: the answer to the user's click is what they see.
        assert_eq!(
            middle_status_text(Some(("удалено записей: 2", Duration::from_secs(1))), collector),
            "удалено записей: 2"
        );
        // Old enough: the collector takes the line back.
        assert_eq!(
            middle_status_text(Some(("удалено записей: 2", Duration::from_secs(30))), collector),
            collector
        );
        assert_eq!(middle_status_text(None, collector), collector);
    }
}

// ------------------------------------------------------------- window procedure
/// Secondary text of the main window: the status line and the word that opens
/// the filter row.  It is drawn in the system's grey, without a background of
/// its own, so it reads as a note rather than as a caption.
fn is_muted(id: i32) -> bool {
    matches!(
        id as usize,
        ID_STATUS
            | ID_STATUS_MIDDLE
            | ID_STATUS_RIGHT
            | ID_STATUS_FILTERS
            | ID_FILTER_APP_LABEL
            | ID_FILTER_FROM_LABEL
            | ID_FILTER_TO_LABEL
    )
}

/// Window procedure of the main window: all user input ends up here.
unsafe extern "system" fn window_proc(hwnd: HWND, message: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // The state travels with the window from its very first message: `lParam`
            // points at a `CREATESTRUCTW`, whose first member is what was passed as
            // `lpParam` to `CreateWindowExW` (the pointer to the state).  Every message
            // that follows - the next `WM_GETMINMAXINFO` among them - can then ask the
            // state about the program instead of guessing.
            let params = *(l as *const *mut std::ffi::c_void);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, params as isize);
            DefWindowProcW(hwnd, message, w, l)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(l as *mut MINMAXINFO);
            // A window that is dragged small has to stay usable, and the feed is
            // allowed to be a narrow, tall strip: the least size follows the shape
            // and the text size - a bigger font needs more room for the same
            // controls, so the window may not be dragged below what the font needs.
            //
            // The very first `WM_GETMINMAXINFO` of a window arrives *before*
            // `WM_NCCREATE`, so there is no state to ask and the shape is still
            // unknown: imposing the least size of the full program there would widen a
            // feed that is meant to be a narrow strip (the saved width is clamped to
            // it).  From the first message on that does know the state, the least size
            // follows the shape - and a stored size is always one the shape allows
            // (`Settings::sanitize`).
            if let Some(app) = app_ref(hwnd) {
                let (least_width, least_height) = minimum_window_size(app.settings.simple_view);
                info.min_track_size.x = app.px(least_width);
                info.min_track_size.y = app.px(least_height);
            }
            0
        }
        WM_SIZE => {
            if let Some(app) = app_ref(hwnd) {
                if w == SIZE_MINIMIZED && app.settings.minimize_to_tray {
                    app.hide_to_tray();
                    return 0;
                }
                app.layout();
            }
            0
        }
        WM_TIMER => {
            match w {
                TIMER_REFRESH => {
                    if let Some(app) = app_ref(hwnd) {
                        app.poll_collector();
                    }
                }
                TIMER_SEARCH => {
                    KillTimer(hwnd, TIMER_SEARCH);
                    if let Some(app) = app_ref(hwnd) {
                        app.text = window_text(app.search);
                        app.reload();
                    }
                }
                TIMER_FILTER => {
                    KillTimer(hwnd, TIMER_FILTER);
                    if let Some(app) = app_ref(hwnd) {
                        app.reload();
                    }
                }
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            let id = (w & 0xFFFF) as usize;
            let code = ((w >> 16) & 0xFFFF) as UINT;
            if id == ID_SEARCH && code == EN_CHANGE {
                // Debounce typing so the journal is not queried per keystroke.
                SetTimer(hwnd, TIMER_SEARCH, SEARCH_DELAY_MS, std::ptr::null_mut());
                return 0;
            }
            // The two date fields only matter for a custom range.
            if (id == ID_FILTER_FROM || id == ID_FILTER_TO) && code == EN_CHANGE {
                if let Some(app) = app_ref(hwnd) {
                    if app.period_filter == view::Period::Custom {
                        SetTimer(hwnd, TIMER_FILTER, SEARCH_DELAY_MS, std::ptr::null_mut());
                    }
                }
                return 0;
            }
            if let Some(app) = app_ref(hwnd) {
                match id {
                    ID_FILTER_APP | ID_FILTER_KIND | ID_FILTER_SOURCE | ID_FILTER_PERIOD
                        if code == CBN_SELCHANGE =>
                    {
                        app.read_filters();
                    }
                    ID_FILTER_RESET => app.reset_filters(),
                    ID_IMPORT | ID_MENU_IMPORT => app.import_now(),
                    ID_MENU_REFRESH => app.reload(),
                    ID_MORE => app.load_more(),
                    ID_EXPORT | ID_MENU_EXPORT => app.export_rows(),
                    ID_FOLDER | ID_MENU_FOLDER | ID_TRAY_FOLDER => app.open_data_folder(),
                    ID_SETTINGS | ID_MENU_SETTINGS | ID_TRAY_SETTINGS => app.show_settings(),
                    ID_MENU_OPEN => app.open_selected(),
                    ID_MENU_DELETE_SELECTED => app.delete_selected(),
                    ID_MENU_DELETE_MATCHING => app.delete_matching(),
                    ID_MENU_DELETE_PERIOD => app.purge_period(),
                    ID_MENU_RESET => app.reset_scan(),
                    ID_MENU_PAUSE => app.toggle_pause(),
                    ID_MENU_SIMPLE_VIEW | ID_CTX_SIMPLE_VIEW | ID_TRAY_SIMPLE_VIEW => {
                        app.set_simple_view(!app.settings.simple_view)
                    }
                    ID_MENU_ICONS => app.toggle_icons(),
                    ID_MENU_TECHNICAL => app.toggle_technical_columns(),
                    ID_MENU_HIDE_DUPLICATES => app.toggle_hide_duplicates(),
                    ID_MENU_ABOUT => {
                        message_box(hwnd, &about_text(&app.layout), MB_ICONINFORMATION)
                    }
                    ID_MENU_EXIT | ID_TRAY_EXIT => {
                        app.about_to_quit = true;
                        DestroyWindow(hwnd);
                    }
                    other => {
                        if let Some((group, index)) = MenuGroup::of(other) {
                            app.run_menu_group(group, index);
                        }
                    }
                }
            }
            0
        }
        purge::MSG_COUNT => {
            // Asked by the period window: how many records would this period take?
            if let Some(app) = app_ref(hwnd) {
                let (from, to) = purge::bounds(w as HWND);
                return app.count_period(&from, &to) as LRESULT;
            }
            0
        }
        purge::MSG_DELETE => {
            // Asked by the period window: ask the user, then delete.  The window
            // disables itself around this call, so no second question can start
            // while the first one is on screen.
            if let Some(app) = app_ref(hwnd) {
                let (from, to) = purge::bounds(w as HWND);
                return app.delete_period(&from, &to) as LRESULT;
            }
            -1
        }
        WM_CONTEXTMENU => {
            // Right click (or Shift+F10): the menu of the list.  The coordinates
            // of the keyboard invocation are (-1, -1).
            if let Some(app) = app_ref(hwnd) {
                let screen_x = (l & 0xFFFF) as i16 as i32;
                let screen_y = ((l >> 16) & 0xFFFF) as i16 as i32;
                let index = app.selected_row();
                app.list_context_menu(index, screen_x, screen_y);
            }
            0
        }
        WM_NOTIFY => {
            let header = notify_header(l);
            if let Some(app) = app_ref(hwnd) {
                match header.code {
                    LVN_ITEMCHANGED => {
                        let item = &*(l as *const NMLISTVIEW);
                        if item.u_new_state & LVIS_SELECTED != 0 {
                            app.selected = item.i_item;
                        }
                    }
                    LVN_COLUMNCLICK => {
                        let item = &*(l as *const NMLISTVIEW);
                        app.sort_by_column(item.i_sub_item.max(0) as usize);
                    }
                    LVN_ITEMACTIVATE => app.open_selected(),
                    LVN_KEYDOWN => {
                        // One implementation for every path a key can take: the
                        // list reports the key here as well (see `handle_key`).
                        let key = &*(l as *const NMLVKEYDOWN);
                        handle_key(hwnd, app.list, key.v_key as UINT);
                    }
                    NM_RCLICK | NM_RDBLCLK => {
                        // The row under the pointer takes part in the selection
                        // before the menu opens, so a command always acts on it.
                        let item = &*(l as *const NMLISTVIEW);
                        if item.i_item >= 0 {
                            app.ensure_row_selected(item.i_item);
                        }
                        let mut point = POINT::default();
                        GetCursorPos(&mut point);
                        app.list_context_menu(item.i_item, point.x, point.y);
                    }
                    _ => {}
                }
            }
            0
        }
        // The window itself only sees keys while it has the focus; the message
        // loop takes care of the rest (see `handle_key`).
        WM_KEYDOWN => {
            handle_key(hwnd, hwnd, w as UINT);
            0
        }
        WM_ERASEBKGND => {
            // The window paints its own background, so the look of the night theme
            // reaches the surface between the controls as well.
            if let Some(app) = app_ref(hwnd) {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                FillRect(w as HDC, &rect, style::window_brush(&app.palette));
                return 1;
            }
            0
        }
        WM_MEASUREITEM => {
            // The list asks once, when it is created, how tall one of its rows is;
            // a thin rule asks for its height as well (it is owner drawn).
            let item = &mut *(l as *mut MEASUREITEMSTRUCT);
            let Some(app) = app_ref(hwnd) else {
                return 0;
            };
            match item.ctl_id as usize {
                ID_LIST => item.item_height = app.row_height().max(1) as u32,
                ID_RULE_FILTERS | ID_RULE_STATUS => item.item_height = 1,
                _ => return 0,
            }
            1
        }
        WM_DRAWITEM => {
            // Every row of the list and both rules are painted here: the message is
            // what tells the control where.
            let item = &*(l as *const DRAWITEMSTRUCT);
            let Some(app) = app_ref(hwnd) else {
                return 0;
            };
            match item.ctl_id as usize {
                ID_LIST => app.draw_row(item),
                ID_RULE_FILTERS | ID_RULE_STATUS => app.draw_rule(item),
                _ => return 0,
            }
            1
        }
        WM_CTLCOLORSTATIC => {
            // A label has no background of its own: it answers with the brush of
            // the window, otherwise every caption carries a grey rectangle that
            // lines up with nothing.
            let control = l as HWND;
            let Some(app) = app_ref(hwnd) else {
                return DefWindowProcW(hwnd, message, w, l);
            };
            let muted = is_muted(GetDlgCtrlID(control));
            match style::paint_control(w as HDC, control, &app.palette, muted) {
                Some(background) => background as LRESULT,
                None => DefWindowProcW(hwnd, message, w, l),
            }
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
            // The fields, the dropped down lists and the buttons of the window: in
            // the night look a white field on a dark window would be the first thing
            // the eye sees, and the colour of the surface is the one thing a classic
            // control does ask its parent about.
            let Some(app) = app_ref(hwnd) else {
                return DefWindowProcW(hwnd, message, w, l);
            };
            let background = if message == WM_CTLCOLORBTN {
                style::paint_button(w as HDC, &app.palette)
            } else {
                style::paint_field(w as HDC, &app.palette)
            };
            background as LRESULT
        }
        WM_APP_TRAY => {
            match l as UINT {
                WM_LBUTTONDBLCLK => {
                    if let Some(app) = app_ref(hwnd) {
                        app.restore();
                    }
                }
                WM_RBUTTONUP => {
                    if let Some(app) = app_ref(hwnd) {
                        app.show_tray_menu();
                    }
                }
                _ => {}
            }
            0
        }
        WM_APP_SHOW => {
            // Another start of the program asked for the window.
            if let Some(app) = app_ref(hwnd) {
                app.restore();
            }
            0
        }
        WM_CLOSE => {
            if let Some(app) = app_ref(hwnd) {
                if app.settings.hide_to_tray_on_close && !app.about_to_quit {
                    app.hide_to_tray();
                } else {
                    app.about_to_quit = true;
                    DestroyWindow(hwnd);
                }
            }
            0
        }
        WM_DESTROY => {
            if let Some(app) = app_ref(hwnd) {
                app.remove_tray();
            }
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
            if !raw.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(raw));
            }
            DefWindowProcW(hwnd, message, w, l)
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}

/// `YYYYMMDD-HHMMSS` derived from a local timestamp (export file names).
fn file_stamp(local: &str) -> String {
    let digits: String = local.chars().filter(char::is_ascii_digit).collect();
    if digits.len() >= 14 {
        format!(
            "{}{}{}-{}",
            &digits[4..8],
            &digits[2..4],
            &digits[0..2],
            &digits[8..14]
        )
    } else {
        "export".to_string()
    }
}

/// Create one child control (font and placement follow later).
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
