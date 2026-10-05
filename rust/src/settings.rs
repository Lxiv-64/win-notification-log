//! User settings of the native application.
//!
//! Settings are a small JSON file (`settings.json`) that belongs to the user:
//! every option the window offers is stored here, so reinstalling the
//! executable changes nothing about the journal or the behaviour.  Reading is
//! deliberately forgiving - a hand edited or truncated file falls back to the
//! defaults per field instead of refusing to start.

use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};

/// Name of the settings file.
pub const FILE_NAME: &str = "settings.json";

/// Interval limits of the collector (seconds).
pub const MIN_INTERVAL_SECONDS: u32 = 1;
pub const MAX_INTERVAL_SECONDS: u32 = 3600;
/// Upper bound of the first run backfill.
pub const MAX_BACKFILL: i64 = 200_000;
/// Longest retention period (days); `0` keeps everything.
pub const MAX_RETENTION_DAYS: i64 = 3650;

/// Export formats offered by the *Export* dialog.
pub const EXPORT_FORMATS: [&str; 4] = ["csv", "json", "txt", "html"];

/// Text sizes offered in the interface (`font_scale`, percent).
///
/// The names are what the menu and the settings window show; the numbers are
/// what the window scales by.  100 % is the system default, so a user who never
/// touches the option sees exactly the usual Windows look.
pub const FONT_SCALES: [u32; 5] = [100, 115, 130, 150, 175];
/// Limits of `font_scale`, so a hand edited file cannot make the UI unusable.
pub const MIN_FONT_SCALE: u32 = 80;
pub const MAX_FONT_SCALE: u32 = 200;

/// Least usable size of the full window (logical pixels at 96 DPI).
///
/// The window may be dragged narrower than it used to be: the toolbar and the
/// filter row wrap instead of running into each other, and this is the width at
/// which the wrapping still leaves every control usable.
pub const MIN_WINDOW_WIDTH: i32 = 560;
pub const MIN_WINDOW_HEIGHT: i32 = 480;
/// Least usable size of the *feed*.  The feed is meant to be a narrow, tall
/// strip, so the rule that keeps the full window usable does not apply to it.
pub const MIN_FEED_WIDTH: i32 = 180;
pub const MIN_FEED_HEIGHT: i32 = 240;

/// Lines of text one row of the feed may use.
///
/// A feed is read, not scanned: two or three lines carry three times the message of
/// one without making the window any wider.  A row that still does not fit ends in
/// an ellipsis.
pub const FEED_LINES: [u32; 3] = [1, 2, 3];
pub const MIN_FEED_LINES: u32 = 1;
pub const MAX_FEED_LINES: u32 = 3;

/// Looks the program offers (`Settings::theme`).
pub const THEMES: [&str; 2] = ["light", "dark"];

/// Interface languages offered (`Settings::language`).
pub const LANGUAGES: [&str; 2] = ["en", "ru"];

/// Name of a language, written the way its own speakers write it.
///
/// Deliberately never translated: the entry has to be readable to somebody who
/// cannot read the interface language that is in force.
pub fn language_label(code: &str) -> &'static str {
    match code {
        "ru" => "Русский",
        _ => "English",
    }
}

/// How many lines a row of the feed may use, as the menu calls it.
pub fn feed_lines_label(lines: u32) -> &'static str {
    match lines {
        1 => "Одна строка",
        2 => "Две строки",
        _ => "Три строки",
    }
}

/// How one look is called in the menu and in the settings window.
pub fn theme_label(theme: &str) -> &'static str {
    match theme {
        "dark" => "Ночная тема (тёмное оформление)",
        _ => "Дневная тема (светлое оформление)",
    }
}

/// How one text size is called in the menu and in the settings window.
///
/// The same words everywhere: a user who picked «Крупный (130 %)» in the settings
/// window has to find the same entry in the menu.
pub fn font_scale_label(scale: u32) -> &'static str {
    match scale {
        100 => "Обычный (100 %)",
        115 => "Крупнее (115 %)",
        130 => "Крупный (130 %)",
        150 => "Очень крупный (150 %)",
        175 => "Максимальный (175 %)",
        _ => "Свой размер",
    }
}

/// Everything the user can adjust.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Directory holding `history.db`, `exports` and the log.  `None` means
    /// "decide automatically" (next to the executable, see `storage`).
    pub data_dir: Option<PathBuf>,
    /// Collect new notifications while the application runs.
    pub collect_on_start: bool,
    /// How often the Windows notification database is checked (seconds).
    pub collect_interval_seconds: u32,
    /// How many existing notifications the very first scan takes over.
    pub backfill_limit: i64,
    /// Delete journal entries older than this many days (`0` keeps everything).
    pub retention_days: i64,
    /// Poll interval of the Windows database collector.
    pub poll_seconds: f64,
    /// Closing the window keeps the application in the notification area.
    pub hide_to_tray_on_close: bool,
    /// Minimising the window moves it to the notification area as well.
    pub minimize_to_tray: bool,
    /// Start straight in the notification area without showing the window.
    pub start_minimized: bool,
    /// Register the application in `HKCU\...\Run`.
    pub start_with_windows: bool,
    /// Show the icon of the sending application in the list.
    pub show_app_icons: bool,
    /// Show the window as a bare feed instead of the whole program.
    ///
    /// The feed is the smallest useful form of the journal: one row per notification
    /// with the icon or picture it came with, the application it came from and its
    /// content - no toolbar, no filters, no status lines, no dialogs.  What stays is
    /// the menu bar (where the settings, the text size and the way back live), the
    /// list and the feed's own columns, so the switch is never a one way street.
    pub simple_view: bool,
    /// Columns of the feed, in the order of `crate::view::FEED_COLUMN_KEYS`.
    pub feed_columns: Vec<String>,
    /// Lines of text one row of the feed may use (`FEED_LINES`).
    pub feed_lines: u32,
    /// Look of the program: `"light"` or `"dark"` (`THEMES`).
    pub theme: String,
    /// Interface language: `"en"` (default) or `"ru"` (`lang::Lang`).
    pub language: String,
    /// Keep a copy of the pictures a notification was sent with.
    ///
    /// Notifications usually point at a temporary file that the sender deletes
    /// again, so without a copy the journal would only show a broken reference a
    /// day later.
    pub keep_pictures: bool,
    /// Show the technical columns (`id`, source, repeats, ...) in the list.
    pub show_technical_columns: bool,
    /// Text size of the whole interface in percent (`100` = system default).
    pub font_scale: u32,
    /// Hide notifications that only repeat an earlier one.
    pub hide_duplicates: bool,
    /// Write the collector log (`wnl.log`) in the data directory.
    ///
    /// Switching it off leaves no file at all behind: no log lines, no trace of
    /// a crash.  The journal itself is unaffected - everything the collector
    /// did stays visible in the window and in `status.json`.
    pub log_enabled: bool,
    /// Apply the rules of notification processing (see `rules`).
    pub rules_enabled: bool,
    /// Read the notification center through the documented WinRT API as a second
    /// source (`UserNotificationListener`).
    ///
    /// It brings the text lines and the times of every notification the center
    /// shows, and it is the only documented way to see notifications of other
    /// applications.  The journal row is the same one `wpndb` writes (the
    /// notification id is identical), so both observations merge; switching the
    /// source off changes nothing about what was already stored.
    pub uns_enabled: bool,
    /// How often that API is polled (seconds); `wpndb` is read on every pass.
    pub uns_poll_seconds: f64,
    /// Append every new notification to a plain text file (`notifications.txt`).
    ///
    /// The file holds only what appeared *after* the option was switched on and
    /// nothing else is written into it: it is the shortest possible answer to
    /// "what came in just now", meant for a chat window, a `tail -f` or a
    /// script.  Switching the option off leaves the file as it is, switching it
    /// on again starts a new period (see `crate::feed`).
    pub feed_enabled: bool,
    /// Folder the file is written to; `None` means `exports` next to the journal.
    pub feed_dir: Option<PathBuf>,
    /// The rules themselves, in the order they are evaluated.
    pub rules: crate::rules::RuleSet,
    /// Format preselected in the export dialog (`csv`, `json`, `txt`, `html`).
    pub export_format: String,
    /// Remembered window position and size (`x`, `y`, `width`, `height`).
    ///
    /// The *full* window only: the feed has its own slot, because it is meant to
    /// be a narrow strip and switching to the full program and back must not throw
    /// that shape away ([`Self::feed_window`]).
    pub window: Option<[i32; 4]>,
    /// Remembered geometry of the feed window (`x`, `y`, `width`, `height`).
    pub feed_window: Option<[i32; 4]>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            data_dir: None,
            collect_on_start: true,
            collect_interval_seconds: 3,
            backfill_limit: 5000,
            retention_days: 0,
            poll_seconds: 2.0,
            hide_to_tray_on_close: true,
            minimize_to_tray: false,
            start_minimized: false,
            start_with_windows: false,
            show_app_icons: true,
            simple_view: false,
            feed_columns: crate::view::default_feed_column_keys(),
            feed_lines: 2,
            theme: THEMES[0].to_string(),
            language: crate::lang::Lang::En.code().to_string(),
            keep_pictures: true,
            show_technical_columns: false,
            font_scale: 100,
            hide_duplicates: false,
            log_enabled: true,
            rules_enabled: true,
            uns_enabled: true,
            uns_poll_seconds: crate::uns::DEFAULT_POLL_SECONDS,
            feed_enabled: false,
            feed_dir: None,
            rules: crate::rules::RuleSet::default(),
            export_format: "csv".to_string(),
            window: None,
            feed_window: None,
        }
    }
}

impl Settings {
    /// Read the file; a missing or unreadable file yields the defaults.
    pub fn load(path: &Path) -> Settings {
        let mut settings = Settings::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return settings;
        };
        // A byte order mark is not JSON, and a settings file written by hand with
        // a Windows editor or by `Set-Content -Encoding UTF8` of PowerShell 5
        // starts with exactly that mark.  Ignoring the whole file because of three
        // invisible bytes would silently undo every setting the user changed.
        let text = text.trim_start_matches('\u{feff}');
        let Ok(json) = serde_json::from_str::<Json>(text.trim_start()) else {
            return settings;
        };
        settings.apply_json(&json);
        settings.sanitize();
        settings
    }

    /// Overlay the values present in `json` (unknown keys stay untouched).
    pub fn apply_json(&mut self, json: &Json) {
        let text = |key: &str| json.get(key).and_then(Json::as_str).map(str::to_string);
        let flag = |key: &str| json.get(key).and_then(Json::as_bool);
        let number = |key: &str| json.get(key).and_then(Json::as_i64);

        if let Some(value) = text("data_dir").filter(|value| !value.trim().is_empty()) {
            self.data_dir = Some(PathBuf::from(value));
        }
        if let Some(value) = flag("collect_on_start") {
            self.collect_on_start = value;
        }
        if let Some(value) = number("collect_interval_seconds") {
            self.collect_interval_seconds = value.clamp(0, i64::from(MAX_INTERVAL_SECONDS)) as u32;
        }
        if let Some(value) = number("backfill_limit") {
            self.backfill_limit = value;
        }
        if let Some(value) = number("retention_days") {
            self.retention_days = value;
        }
        if let Some(value) = json.get("poll_seconds").and_then(Json::as_f64) {
            self.poll_seconds = value;
        }
        if let Some(value) = flag("hide_to_tray_on_close") {
            self.hide_to_tray_on_close = value;
        }
        if let Some(value) = flag("minimize_to_tray") {
            self.minimize_to_tray = value;
        }
        if let Some(value) = flag("start_minimized") {
            self.start_minimized = value;
        }
        if let Some(value) = flag("start_with_windows") {
            self.start_with_windows = value;
        }
        if let Some(value) = flag("show_app_icons") {
            self.show_app_icons = value;
        }
        if let Some(value) = flag("simple_view") {
            self.simple_view = value;
        }
        if let Some(values) = json.get("feed_columns").and_then(Json::as_array) {
            // Unknown keys are dropped by `sanitize`; the reader's order is kept.
            self.feed_columns = values
                .iter()
                .filter_map(Json::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
        }
        if let Some(value) = json.get("feed_lines").and_then(Json::as_u64) {
            self.feed_lines = value.min(u64::from(u32::MAX)) as u32;
        }
        if let Some(value) = text("theme") {
            self.theme = value;
        }
        if let Some(value) = text("language") {
            self.language = value;
        }
        if let Some(value) = flag("keep_pictures") {
            self.keep_pictures = value;
        }
        if let Some(value) = flag("show_technical_columns") {
            self.show_technical_columns = value;
        }
        if let Some(value) = number("font_scale") {
            self.font_scale = value.clamp(0, i64::from(MAX_FONT_SCALE)) as u32;
        }
        if let Some(value) = flag("hide_duplicates") {
            self.hide_duplicates = value;
        }
        if let Some(value) = flag("log_enabled") {
            self.log_enabled = value;
        }
        if let Some(value) = flag("rules_enabled") {
            self.rules_enabled = value;
        }
        if let Some(value) = flag("uns_enabled") {
            self.uns_enabled = value;
        }
        if let Some(value) = json.get("uns_poll_seconds").and_then(Json::as_f64) {
            self.uns_poll_seconds = value;
        }
        if let Some(value) = flag("feed_enabled") {
            self.feed_enabled = value;
        }
        // An empty folder in the file means "decide automatically", exactly like
        // an empty data folder: the option is then stored as absent.
        if let Some(value) = text("feed_dir") {
            let trimmed = value.trim();
            self.feed_dir = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
        }
        if let Some(value) = json.get("rules") {
            self.rules = crate::rules::RuleSet::from_json(value);
        }
        if let Some(value) = text("export_format") {
            self.export_format = value;
        }
        // Both shapes carry their own geometry; the reader is the same and the
        // judge is `sanitize`, which knows that a feed may be narrow.
        let geometry = |key: &str| -> Option<[i32; 4]> {
            let values = json.get(key).and_then(Json::as_array)?;
            let numbers: Vec<i32> = values
                .iter()
                .filter_map(Json::as_i64)
                .map(|value| value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
                .collect();
            (numbers.len() == 4).then(|| [numbers[0], numbers[1], numbers[2], numbers[3]])
        };
        if let Some(window) = geometry("window") {
            self.window = Some(window);
        }
        if let Some(window) = geometry("feed_window") {
            self.feed_window = Some(window);
        }
    }

    /// Keep the values inside the ranges the application can work with.
    pub fn sanitize(&mut self) {
        self.collect_interval_seconds = self
            .collect_interval_seconds
            .clamp(MIN_INTERVAL_SECONDS, MAX_INTERVAL_SECONDS);
        if !self.poll_seconds.is_finite() {
            self.poll_seconds = 0.5;
        }
        self.poll_seconds = self
            .poll_seconds
            .clamp(0.5, f64::from(MAX_INTERVAL_SECONDS));
        self.backfill_limit = self.backfill_limit.clamp(0, MAX_BACKFILL);
        self.retention_days = self.retention_days.clamp(0, MAX_RETENTION_DAYS);
        if !self.uns_poll_seconds.is_finite() {
            self.uns_poll_seconds = crate::uns::DEFAULT_POLL_SECONDS;
        }
        // The API is cheap but not free: a poll more often than twice a second
        // would only burn processor time, and a minute is the slowest that still
        // feels alive.
        self.uns_poll_seconds = self.uns_poll_seconds.clamp(0.5, 60.0);
        self.font_scale = self.font_scale.clamp(MIN_FONT_SCALE, MAX_FONT_SCALE);
        // A column key the program does not know is dropped, and a choice of nothing
        // falls back on the pair a feed starts with: a table without columns would
        // show no records at all.
        self.feed_columns
            .retain(|key| crate::view::FEED_COLUMN_KEYS.contains(&key.as_str()));
        // A key that appears twice is still one column (the columns come from the
        // table), so the file is tidied up here.
        let mut seen: Vec<String> = Vec::with_capacity(self.feed_columns.len());
        for key in self.feed_columns.drain(..) {
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
        self.feed_columns = seen;
        if self.feed_columns.is_empty() {
            self.feed_columns = crate::view::default_feed_column_keys();
        }
        self.feed_lines = self.feed_lines.clamp(MIN_FEED_LINES, MAX_FEED_LINES);
        if !THEMES.contains(&self.theme.as_str()) {
            self.theme = THEMES[0].to_string();
        }
        self.language = crate::lang::Lang::from_code(&self.language).code().to_string();
        if !EXPORT_FORMATS.contains(&self.export_format.as_str()) {
            self.export_format = EXPORT_FORMATS[0].to_string();
        }
        if let Some(window) = self.window {
            if window[2] < MIN_WINDOW_WIDTH || window[3] < MIN_WINDOW_HEIGHT {
                self.window = None;
            }
        }
        // The feed is allowed to be a narrow, tall strip: only a window that could
        // not show one line of a message is thrown away.
        if let Some(window) = self.feed_window {
            if window[2] < MIN_FEED_WIDTH || window[3] < MIN_FEED_HEIGHT {
                self.feed_window = None;
            }
        }
        if self
            .data_dir
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            self.data_dir = None;
        }
        if self
            .feed_dir
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            self.feed_dir = None;
        }
    }

    /// Serialised form written to disk.
    pub fn to_json(&self) -> Json {
        json!({
            "data_dir": self.data_dir.as_ref().map(|path| path.display().to_string()),
            "collect_on_start": self.collect_on_start,
            "collect_interval_seconds": self.collect_interval_seconds,
            "backfill_limit": self.backfill_limit,
            "retention_days": self.retention_days,
            "poll_seconds": self.poll_seconds,
            "hide_to_tray_on_close": self.hide_to_tray_on_close,
            "minimize_to_tray": self.minimize_to_tray,
            "start_minimized": self.start_minimized,
            "start_with_windows": self.start_with_windows,
            "show_app_icons": self.show_app_icons,
            "simple_view": self.simple_view,
            "feed_columns": self.feed_columns,
            "feed_lines": self.feed_lines,
            "theme": self.theme,
            "language": self.language,
            "keep_pictures": self.keep_pictures,
            "show_technical_columns": self.show_technical_columns,
            "font_scale": self.font_scale,
            "hide_duplicates": self.hide_duplicates,
            "log_enabled": self.log_enabled,
            "rules_enabled": self.rules_enabled,
            "uns_enabled": self.uns_enabled,
            "uns_poll_seconds": self.uns_poll_seconds,
            "feed_enabled": self.feed_enabled,
            "feed_dir": self
                .feed_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            "rules": self.rules.to_json(),
            "export_format": self.export_format,
            "window": self.window.map(|values| values.to_vec()),
            "feed_window": self.feed_window.map(|values| values.to_vec()),
        })
    }

    /// Write the file (creating the parent directory when needed).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_else(|_| "{}".into());
        // No byte order mark: the file has to stay readable for every reader, and
        // `Settings::load` should not have to repair what this wrote.
        std::fs::write(path, format!("{text}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_path(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("wnl-settings-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp directory");
        directory.join(FILE_NAME)
    }

    #[test]
    fn defaults_are_friendly() {
        let settings = Settings::default();
        assert_eq!(settings.collect_interval_seconds, 3);
        assert!(settings.collect_on_start);
        assert!(settings.hide_to_tray_on_close);
        assert!(settings.show_app_icons);
        assert!(
            settings.keep_pictures,
            "pictures are kept by default: the sender deletes them right away"
        );
        assert!(
            !settings.show_technical_columns,
            "technical columns stay optional"
        );
        assert_eq!(settings.retention_days, 0);
        assert!(
            settings.log_enabled,
            "the log is written unless the user switches it off"
        );
        assert_eq!(settings.export_format, "csv");
        assert_eq!(settings.data_dir, None);
        assert_eq!(settings.feed_dir, None);
        assert!(
            !settings.feed_enabled,
            "the text feed writes files, so it is switched on by hand"
        );
    }

    #[test]
    fn an_empty_feed_folder_means_automatic() {
        let mut settings = Settings::default();
        settings.apply_json(&json!({"feed_enabled": true, "feed_dir": "  "}));
        assert!(settings.feed_enabled);
        assert_eq!(settings.feed_dir, None, "пустая папка значит «по умолчанию»");

        settings.apply_json(&json!({"feed_dir": "  D:\\лента  "}));
        assert_eq!(settings.feed_dir, Some(PathBuf::from(r"D:\лента")));

        settings.feed_dir = Some(PathBuf::new());
        settings.sanitize();
        assert_eq!(settings.feed_dir, None, "пустой путь не становится папкой");
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_settings() {
        // A settings file edited by hand (or written by PowerShell 5's
        // `Set-Content -Encoding UTF8`) starts with a byte order mark.  It is not
        // JSON, and reading it as broken would silently drop every setting.
        let path = temporary_path("bom");
        let text = format!(
            "\u{feff}{{\n  \"start_minimized\": true,\n  \"keep_pictures\": false,\n  \"rules\": [{{\"name\": \"q\", \"action\": \"skip\"}}]\n}}\n"
        );
        std::fs::write(&path, text).expect("written");

        let settings = Settings::load(&path);
        assert!(settings.start_minimized, "значение из файла прочитано");
        assert!(!settings.keep_pictures);
        assert_eq!(settings.rules.rules().len(), 1);

        // What the program writes itself stays free of the mark.
        settings.save(&path).expect("saved");
        let bytes = std::fs::read(&path).expect("read");
        assert!(!bytes.starts_with(&[0xef, 0xbb, 0xbf]), "BOM не пишется");
        assert_eq!(Settings::load(&path).rules.rules().len(), 1);
    }

    #[test]
    fn round_trip_through_the_file() {
        let path = temporary_path("roundtrip");
        let mut settings = Settings::default();
        settings.data_dir = Some(PathBuf::from(r"D:\журнал"));
        settings.collect_interval_seconds = 12;
        settings.minimize_to_tray = true;
        settings.start_with_windows = true;
        settings.show_technical_columns = true;
        settings.simple_view = true;
        settings.feed_columns = vec!["time".to_string(), "app".to_string()];
        settings.feed_lines = 3;
        settings.theme = "dark".to_string();
        settings.font_scale = 130;
        settings.hide_duplicates = true;
        settings.log_enabled = false;
        settings.retention_days = 30;
        settings.export_format = "json".to_string();
        settings.feed_enabled = true;
        settings.feed_dir = Some(PathBuf::from(r"D:\лента"));
        settings.window = Some([100, 200, 1240, 780]);
        settings.feed_window = Some([-1180, 40, 340, 900]);
        settings.save(&path).expect("saved");

        assert_eq!(Settings::load(&path), settings);
    }

    #[test]
    fn missing_and_broken_files_fall_back_to_defaults() {
        let path = temporary_path("broken");
        assert_eq!(Settings::load(&path), Settings::default());

        std::fs::write(&path, "{ this is not json").expect("written");
        assert_eq!(Settings::load(&path), Settings::default());

        std::fs::write(&path, r#"{"collect_interval_seconds": 7}"#).expect("written");
        let loaded = Settings::load(&path);
        assert_eq!(loaded.collect_interval_seconds, 7);
        assert_eq!(loaded.retention_days, Settings::default().retention_days);
    }

    #[test]
    fn values_are_clamped_to_workable_ranges() {
        let mut settings = Settings::default();
        settings.apply_json(&json!({
            "collect_interval_seconds": 0,
            "backfill_limit": 10_000_000,
            "retention_days": -5,
            "poll_seconds": 0.0,
            "export_format": "pdf",
            "font_scale": 5000,
            "window": [0, 0, 10, 10],
        }));
        settings.sanitize();
        assert_eq!(settings.collect_interval_seconds, MIN_INTERVAL_SECONDS);
        assert_eq!(settings.backfill_limit, MAX_BACKFILL);
        assert_eq!(settings.retention_days, 0);
        assert_eq!(settings.poll_seconds, 0.5);
        assert_eq!(settings.export_format, "csv");
        assert_eq!(settings.font_scale, MAX_FONT_SCALE);
        assert_eq!(settings.window, None, "a window that small is unusable");
    }

    /// The two shapes remember their own window: the feed is meant to be a narrow,
    /// tall strip, and the rule that keeps the full window usable must not throw
    /// that shape away.
    #[test]
    fn the_feed_remembers_its_own_narrow_window() {
        let mut settings = Settings::default();
        settings.apply_json(&json!({
            "window": [10, 20, 1280, 800],
            "feed_window": [30, 40, 320, 860],
        }));
        settings.sanitize();
        assert_eq!(settings.window, Some([10, 20, 1280, 800]));
        assert_eq!(settings.feed_window, Some([30, 40, 320, 860]));
        assert_eq!(
            settings.to_json()["feed_window"],
            json!([30, 40, 320, 860]),
            "the narrow shape has to survive a save"
        );

        // A strip that cannot show one line of a message helps nobody.
        settings.apply_json(&json!({"feed_window": [30, 40, 120, 200]}));
        settings.sanitize();
        assert_eq!(settings.feed_window, None);

        // The narrow shape is only allowed in the feed: the full window keeps the
        // rule it always had.
        settings.apply_json(&json!({"window": [0, 0, 200, 900]}));
        settings.sanitize();
        assert_eq!(settings.window, None);
    }

    /// The feed's own look: which columns it shows, how many lines a row may use and
    /// the theme.  A file written by hand may carry anything, so this is where the
    /// nonsense is filtered out.
    #[test]
    fn the_feed_keeps_workable_columns_and_lines() {
        let mut settings = Settings::default();
        settings.apply_json(&json!({
            "feed_columns": ["time", "app", "time", "nonsense"],
            "feed_lines": 9,
            "theme": "midnight",
        }));
        settings.sanitize();
        assert_eq!(
            settings.feed_columns,
            vec!["time".to_string(), "app".to_string()],
            "unknown keys are dropped"
        );
        assert_eq!(settings.feed_lines, MAX_FEED_LINES);
        assert_eq!(settings.theme, THEMES[0], "an unknown look falls back");

        // Nothing chosen means the default pair, and one line is still a feed.
        settings.apply_json(&json!({"feed_columns": [], "feed_lines": 0}));
        settings.sanitize();
        assert_eq!(settings.feed_columns, crate::view::default_feed_column_keys());
        assert_eq!(settings.feed_lines, MIN_FEED_LINES);

        // What was chosen survives a save, in the order the reader picked.
        settings.theme = "dark".to_string();
        settings.feed_lines = 3;
        let path = temporary_path("feed-look");
        settings.save(&path).expect("saved");
        assert_eq!(Settings::load(&path), settings);
    }

    #[test]
    fn every_line_count_and_look_has_a_name() {
        let labels: Vec<&str> = FEED_LINES
            .iter()
            .map(|lines| feed_lines_label(*lines))
            .collect();
        assert_eq!(labels.len(), 3);
        assert!(
            labels.iter().all(|label| !label.is_empty()),
            "every offered count needs words"
        );
        assert_eq!(labels[0], "Одна строка");
        assert_eq!(labels[1], "Две строки");
        assert_eq!(labels[2], "Три строки");
        assert_eq!(
            FEED_LINES.len() as u32,
            MAX_FEED_LINES - MIN_FEED_LINES + 1,
            "the menu offers exactly the range the settings accept"
        );

        for theme in THEMES {
            assert!(!theme_label(theme).is_empty(), "{theme} has no name");
        }
        assert_ne!(theme_label("dark"), theme_label("light"));
    }

    #[test]
    fn every_text_size_has_a_name() {
        for scale in FONT_SCALES {
            let label = font_scale_label(scale);
            assert!(
                label.contains(&scale.to_string()),
                "«{label}» does not name its own size"
            );
            assert_ne!(label, "Свой размер", "an offered size has a real name");
        }
        assert_eq!(font_scale_label(12345), "Свой размер");
    }

    #[test]
    fn the_text_size_never_becomes_unreadable() {
        let mut settings = Settings::default();
        settings.apply_json(&json!({"font_scale": 10}));
        settings.sanitize();
        assert_eq!(settings.font_scale, MIN_FONT_SCALE, "too small to read");
        assert!(
            FONT_SCALES.iter().all(|scale| *scale >= MIN_FONT_SCALE),
            "every offered size has to survive sanitizing"
        );
        assert!(FONT_SCALES.contains(&Settings::default().font_scale));
    }

    #[test]
    fn unknown_keys_do_not_disturb_known_ones() {
        let mut settings = Settings::default();
        settings.apply_json(&json!({"something_else": true, "collect_on_start": false}));
        assert!(!settings.collect_on_start);
        assert_eq!(settings.to_json()["collect_on_start"], json!(false));
    }
}
