//! The notification feed: a plain text file that grows while the collector runs.
//!
//! The journal answers "what was ever seen" and the window answers "what do I
//! want to look at"; this answers a third question - "what arrived just now" -
//! in the smallest possible form.  Every notification that appears *after the
//! option was switched on* is appended as one line:
//!
//! ```text
//! 30.09.2026 21:47:35<TAB>wpndb<TAB>Почта: Новое письмо — Привет
//! ```
//!
//! (time, source, content) and entries are separated by one empty line, so the
//! file stays usable in a chat window, in a `tail -f`, in a spreadsheet or as
//! an input for a script.  Nothing else is written - no header, no counters, no
//! re-formatting of what was written before: the file is only appended to.
//!
//! Two numbers make that work, and both live in `feed.json` inside the data
//! directory:
//!
//! * `since_utc` - the moment the option was switched on.  A notification that
//!   Windows created before that moment is never written, which is what makes
//!   the promise "new ones" true even when something old appears late (the very
//!   first import of a fresh journal, for example);
//! * `cursor` - the number of the last journal row that was written.  It is the
//!   same idea as the cursor of the collectors, and it makes a restart, a second
//!   collector or a hand edited settings file harmless: one notification, one
//!   entry.
//!
//! Switching the option off and on again starts a new period: the file keeps
//! what it has, and the entries after it are the notifications of that new
//! period ([`align`] drops the state of the period before).

use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};

use crate::settings::Settings;
use crate::sqlite::Row;
use crate::store::Journal;
use crate::{storage, time, view};

/// Name of the file the entries are appended to.
pub const FILE_NAME: &str = "notifications.txt";
/// Name of the state file in the data directory.
pub const STATE_FILE: &str = "feed.json";
/// Entries written by one pass.
///
/// A burst (hundreds of notifications at once after a long pause) is written in
/// portions: the pass stays short and the next one continues where this stopped.
pub const MAX_ENTRIES_PER_PASS: i64 = 500;

/// Where the feed is and whether it is switched on.
///
/// Part of `service::Params`, so the collector, the window and `wnl-cli watch`
/// all work with the same description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    /// The file entries are appended to.
    pub file: PathBuf,
    /// The state of the current period (cursor and boundary).
    pub state: PathBuf,
}

impl Config {
    /// Description for a resolved data directory and the user settings.
    ///
    /// The folder comes from the settings; without one the file goes to
    /// `exports` next to the journal, which is where the manual export puts its
    /// files as well.
    pub fn from_layout(layout: &storage::Layout, settings: &Settings) -> Config {
        Config {
            enabled: settings.feed_enabled,
            file: file_path(layout, settings),
            state: layout.data_dir.join(STATE_FILE),
        }
    }
}

/// The file the entries go to for a given folder setting.
pub fn file_path(layout: &storage::Layout, settings: &Settings) -> PathBuf {
    match settings
        .feed_dir
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    {
        Some(folder) => folder.join(FILE_NAME),
        None => layout.exports.join(FILE_NAME),
    }
}

/// One entry: time, source, content, and the empty line that separates it from
/// the next one.
///
/// Tabs and line breaks inside the values become spaces: a field of the line
/// must not be able to break the layout of the line.
pub fn entry(local_time: &str, sources: &str, content: &str) -> String {
    format!(
        "{}\t{}\t{}\r\n\r\n",
        one_line(local_time),
        one_line(sources),
        one_line(content)
    )
}

/// Collapse whitespace, so a value fits the single line it belongs to.
pub fn one_line(value: &str) -> String {
    view::single_line(value)
}

/// Content of one journal row: the application and what it said.
///
/// The application is part of the content on purpose - a file of bare texts
/// would not tell who sent them - and the name is used without the identifier
/// the listing shows in brackets: the file is meant to be read.
pub fn content(row: &Row) -> String {
    let application = row
        .optional_text("app_name")
        .or_else(|| row.optional_text("aumid"))
        .map(|name| one_line(&name))
        .filter(|name| !name.is_empty());
    let text = one_line(&view::row_text(row));
    match (application, text.is_empty()) {
        (Some(name), false) => format!("{name}: {text}"),
        (Some(name), true) => tf!("{name}: (без текста)", "{name}: (without text)"),
        (None, false) => text,
        (None, true) => tr!("(уведомление без текста)").to_string(),
    }
}

/// Source of one journal row as the feed writes it (`uns`, `wpndb`, `uns+wpndb`).
pub fn sources(row: &Row) -> String {
    let sources = row
        .optional_text("sources")
        .unwrap_or_default()
        .trim()
        .to_string();
    if sources.is_empty() {
        row.optional_text("source").unwrap_or_default()
    } else {
        sources
    }
}

/// The period the feed is in: since when it counts and how far it got.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// `true` while entries are being appended.
    pub enabled: bool,
    /// Moment the period started (UTC, in the form the journal stores).
    pub since_utc: String,
    /// Number of the last row that was written.
    pub cursor: i64,
    /// Entries written in this period.
    pub written: u64,
    /// Local time of the last entry, for the status line.
    pub last_local: Option<String>,
    /// The file in use when the state was written.
    pub file: String,
}

impl State {
    /// A period that starts now.
    pub fn starting(file: &Path) -> State {
        State {
            enabled: true,
            since_utc: time::now_iso(),
            cursor: 0,
            written: 0,
            last_local: None,
            file: file.display().to_string(),
        }
    }

    pub fn to_json(&self) -> Json {
        json!({
            "enabled": self.enabled,
            "since_utc": self.since_utc,
            "cursor": self.cursor,
            "written": self.written,
            "last_local": self.last_local,
            "file": self.file,
        })
    }

    /// Tolerant mapping: a truncated or hand edited file cannot stop the feed.
    pub fn from_json(json: &Json) -> State {
        let text = |key: &str| {
            json.get(key)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string()
        };
        State {
            enabled: json
                .get("enabled")
                .and_then(Json::as_bool)
                .unwrap_or(false),
            since_utc: text("since_utc"),
            cursor: json.get("cursor").and_then(Json::as_i64).unwrap_or(0).max(0),
            written: json
                .get("written")
                .and_then(Json::as_i64)
                .unwrap_or(0)
                .max(0) as u64,
            last_local: json
                .get("last_local")
                .and_then(Json::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            file: text("file"),
        }
    }

    /// Read the state; `None` when there is none or it is unreadable.
    pub fn read(path: &Path) -> Option<State> {
        let text = std::fs::read_to_string(path).ok()?;
        let json: Json = serde_json::from_str(&text).ok()?;
        Some(State::from_json(&json))
    }

    /// Write the state (creating the parent directory when needed).
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_else(|_| "{}".into());
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, format!("{text}\n"))?;
        std::fs::rename(&temporary, path)
    }
}


/// What the feed did; part of the collector state (`status.json`, the log).
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// `true` while entries are being appended.
    pub enabled: bool,
    /// The file entries go to.
    pub file: String,
    /// Entries written since the option was switched on.
    pub written: u64,
    /// Local time of the last entry.
    pub last_local: Option<String>,
    /// Local time of the moment the option was switched on.
    pub since_local: Option<String>,
    /// Why the last attempt failed, if it did.
    pub error: Option<String>,
}

impl Report {
    /// The feed is switched off.
    pub fn disabled(file: &Path) -> Report {
        Report {
            enabled: false,
            file: file.display().to_string(),
            written: 0,
            last_local: None,
            since_local: None,
            error: None,
        }
    }

    fn from_state(state: &State) -> Report {
        Report {
            enabled: state.enabled,
            file: state.file.clone(),
            written: state.written,
            last_local: state.last_local.clone(),
            since_local: local(&state.since_utc),
            error: None,
        }
    }

    fn failed(file: &Path, state: Option<&State>, message: String) -> Report {
        let mut report = match state {
            Some(state) => Report::from_state(state),
            None => Report::disabled(file),
        };
        report.enabled = true;
        report.file = file.display().to_string();
        report.error = Some(message);
        report
    }

    /// One line for the log and the status file.
    pub fn describe(&self) -> String {
        if let Some(error) = &self.error {
            return tf!("ошибка записи: {error}", "write error: {error}");
        }
        if !self.enabled {
            return tr!("выключен в настройках").to_string();
        }
        let mut text = tf!("файл {}  •  строк {}", "file {}  •  lines {}", self.file, self.written);
        if let (Some(last), Some(since)) = (&self.last_local, &self.since_local) {
            text.push_str(&tf!(
                "  •  последняя {last}  •  считаю с {since}",
                "  •  last {last}  •  counting from {since}"
            ));
        }
        text
    }

    /// What only changes when something happens: whether entries are being
    /// written, where they go and whether that works.
    ///
    /// The log follows this instead of [`Report::describe`], which carries the
    /// running count and would turn every entry into a line of its own.
    pub fn state_key(&self) -> String {
        match (&self.error, self.enabled) {
            (Some(error), _) => tf!("ошибка: {error}", "error: {error}"),
            (None, true) => tf!("пишу в {}", "writing to {}", self.file),
            (None, false) => tr!("выключена в настройках").to_string(),
        }
    }

    pub fn to_json(&self) -> Json {
        json!({
            "enabled": self.enabled,
            "file": self.file,
            "written": self.written,
            "last_local": self.last_local,
            "since_local": self.since_local,
            "error": self.error,
        })
    }

    /// Read the JSON form back (missing fields become defaults).
    pub fn from_json(value: &Json) -> Report {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Json::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Report {
            enabled: value
                .get("enabled")
                .and_then(Json::as_bool)
                .unwrap_or(false),
            file: text("file").unwrap_or_default(),
            written: value
                .get("written")
                .and_then(Json::as_i64)
                .unwrap_or(0)
                .max(0) as u64,
            last_local: text("last_local"),
            since_local: text("since_local"),
            error: text("error"),
        }
    }
}

/// State of the feed for everyone who does not run a collector (the window, the
/// diagnostics): what `feed.json` says, or "switched off".
pub fn status(config: &Config) -> Report {
    match State::read(&config.state) {
        Some(state) if state.enabled => Report::from_state(&state),
        _ => Report::disabled(&config.file),
    }
}

/// Drop the state of a period the user has left.
///
/// The setting and the stored period belong to each other, so a disagreement
/// means the feed was switched somewhere else - in the settings window, in
/// `settings.json` by hand, in a run of `wnl-cli watch`.  The period before is
/// over then and the next pass starts a new one (see the module note).
pub fn align(config: &Config) {
    let Some(state) = State::read(&config.state) else {
        return;
    };
    if state.enabled != config.enabled {
        let _ = std::fs::remove_file(&config.state);
    }
}

/// Append everything that is new; returns what happened.
///
/// A pass never fails because of the feed: writing a text file is a side
/// service and must not stop the collection (the same rule as for `uns`).
pub fn poll(journal: &Journal, config: &Config) -> Report {
    if !config.enabled {
        return Report::disabled(&config.file);
    }
    // The collector thread and the window ("Импорт сейчас") share the process
    // and both may run a pass at the same time: the cursor is read, used and
    // written under one lock.
    let _guard = lock();
    let mut state = match State::read(&config.state) {
        Some(state) if state.enabled => state,
        _ => {
            let state = State::starting(&config.file);
            // The moment of switching on has to survive a restart, so the new
            // period is stored *before* the first entry is looked for: a
            // boundary that is only written together with an entry would slide
            // forward with every pass, and a notification that arrived in
            // between (while the collector was off) would never be written.
            if let Err(error) = state.write(&config.state) {
                return Report::failed(
                    &config.file,
                    Some(&state),
                    tf!("состояние не сохранено: {error}", "the state was not saved: {error}"),
                );
            }
            state
        }
    };
    state.file = config.file.display().to_string();
    // A journal that was replaced (a restored backup, another file behind the
    // same name) can sit below the stored cursor.  Starting the count again is
    // safe: the boundary still keeps every old notification out.
    if let Ok(highest) = journal.highest_record_id() {
        if state.cursor > highest {
            state.cursor = 0;
        }
    }
    let rows = match journal.rows_after(state.cursor, &state.since_utc, MAX_ENTRIES_PER_PASS) {
        Ok(rows) => rows,
        Err(error) => return Report::failed(&config.file, Some(&state), error.to_string()),
    };
    if rows.is_empty() {
        // Nothing new: the file is not even opened.
        return Report::from_state(&state);
    }
    let mut text = String::new();
    for row in &rows {
        let stamp = view::local_time(row.text("first_seen_utc").as_deref());
        text.push_str(&entry(&stamp, &sources(row), &content(row)));
    }
    if let Err(error) = append(&config.file, &text) {
        return Report::failed(&config.file, Some(&state), error.to_string());
    }
    if let Some(last) = rows.last().and_then(|row| row.int("id")) {
        state.cursor = last;
    }
    state.written += rows.len() as u64;
    state.last_local = rows
        .last()
        .map(|row| view::local_time(row.text("first_seen_utc").as_deref()));
    let mut report = Report::from_state(&state);
    // A state that could not be stored would replay these entries on the next
    // pass; that is worth a word in the log.
    if let Err(error) = state.write(&config.state) {
        report.error = Some(tf!(
            "состояние не сохранено: {error}",
            "the state was not saved: {error}"
        ));
    }
    report
}

/// Append text to the feed file, creating the folder and the file when needed.
fn append(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(text.as_bytes())
}

/// One writer at a time inside this process.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Local form of a stored UTC stamp (`None` when there is none).
fn local(utc: &str) -> Option<String> {
    (!utc.trim().is_empty()).then(|| time::format_local_iso(Some(utc)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NotificationRecord;
    use crate::sqlite::Value;

    const HOUR_MICROS: i64 = 3_600_000_000;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("wnl-feed-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    fn config(directory: &Path, enabled: bool) -> Config {
        Config {
            enabled,
            file: directory.join(FILE_NAME),
            state: directory.join(STATE_FILE),
        }
    }

    /// A row with the columns the feed reads.
    fn row(pairs: &[(&str, Value)]) -> Row {
        let names: Vec<String> = pairs.iter().map(|(key, _)| key.to_string()).collect();
        let values: Vec<Value> = pairs.iter().map(|(_, value)| value.clone()).collect();
        Row::from_pairs(names, values)
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_string())
    }

    /// Store one notification, the way a collector does it.
    fn observe(journal: &Journal, title: &str, created_utc: String) {
        static NEXT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
        let windows_id = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let record = NotificationRecord {
            source: "wpndb".to_string(),
            aumid: Some(format!("Vendor.App_{windows_id}!App")),
            app_name: Some("Тестовое приложение".to_string()),
            windows_id: Some(windows_id),
            title: Some(title.to_string()),
            body: Some("текст".to_string()),
            created_utc: Some(created_utc),
            ..Default::default()
        };
        let counts = journal.record_many(&[record], 0).expect("запись в журнал");
        assert_eq!(counts.inserted, 1, "уведомление должно быть новым");
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("файл ленты")
    }

    #[test]
    fn an_entry_is_one_line_and_ends_with_an_empty_one() {
        let line = entry(
            "01.10.2026 09:15:00",
            "uns+wpndb",
            "Почта: строка\nвторая\tещё",
        );
        assert_eq!(
            line, "01.10.2026 09:15:00\tuns+wpndb\tПочта: строка вторая ещё\r\n\r\n",
            "табуляция и перевод строки внутри значений не ломают строку"
        );
        assert_eq!(line.matches('\t').count(), 2, "время, источник, содержание");
        assert_eq!(line.matches("\r\n\r\n").count(), 1, "одна пустая строка");
    }

    #[test]
    fn content_names_the_application() {
        let named = row(&[
            ("app_name", text("Почта")),
            ("aumid", text("Vendor.Mail_1!App")),
            ("title", text("Новое письмо")),
            ("body", text("Привет")),
        ]);
        assert_eq!(content(&named), "Почта: Новое письмо — Привет");

        let only_id = row(&[("aumid", text("Vendor.Mail_1!App"))]);
        assert_eq!(content(&only_id), "Vendor.Mail_1!App: (без текста)");

        let bare = row(&[("body", text("только текст"))]);
        assert_eq!(content(&bare), "только текст");
        assert_eq!(content(&row(&[])), "(уведомление без текста)");

        let sourced = row(&[
            ("sources", text("uns+wpndb")),
            ("source", text("uns")),
            ("title", text("Т")),
        ]);
        assert_eq!(sources(&sourced), "uns+wpndb", "список источников важнее");
        let one = row(&[("sources", text("  ")), ("source", text("wpndb"))]);
        assert_eq!(sources(&one), "wpndb", "пустой список читается как один");
    }

    #[test]
    fn the_report_names_the_file_and_the_count() {
        let report = Report {
            enabled: true,
            file: r"D:\лог\notifications.txt".to_string(),
            written: 12,
            last_local: Some("01.10.2026 09:15:00".to_string()),
            since_local: Some("01.10.2026 09:00:00".to_string()),
            error: None,
        };
        let text = report.describe();
        assert!(text.contains("notifications.txt"), "{text}");
        assert!(text.contains("12"), "{text}");
        assert!(text.contains("09:15:00"), "{text}");

        assert_eq!(
            Report::disabled(Path::new("x.txt")).describe(),
            "выключен в настройках"
        );
        let broken = Report::failed(Path::new("x.txt"), None, "нет доступа".to_string());
        assert!(broken.describe().starts_with("ошибка записи"), "ошибка видна");

        let restored = Report::from_json(&report.to_json());
        assert_eq!(restored, report);
        let empty = Report::from_json(&json!({}));
        assert!(!empty.enabled);
        assert_eq!(empty.written, 0);

        // The log follows the key, not the count: two reports of the same period
        // differ in `written` and must not produce two log lines.
        let later = Report {
            written: 34,
            ..report.clone()
        };
        assert_eq!(later.state_key(), report.state_key());
        assert!(report.state_key().contains("notifications.txt"));
        assert!(Report::disabled(Path::new("x.txt"))
            .state_key()
            .contains("выключена"));
    }

    #[test]
    fn the_state_survives_a_round_trip() {
        let directory = scratch("state");
        let path = directory.join("feed.json");
        let state = State {
            enabled: true,
            since_utc: "2026-10-01T06:00:00.000000Z".to_string(),
            cursor: 41,
            written: 7,
            last_local: Some("01.10.2026 09:15:00".to_string()),
            file: directory.join(FILE_NAME).display().to_string(),
        };
        state.write(&path).expect("состояние записано");
        assert_eq!(State::read(&path), Some(state));

        std::fs::write(&path, "{ это не json").expect("written");
        assert_eq!(State::read(&path), None, "сломанный файл не пугает");
        std::fs::write(&path, r#"{"cursor": -5}"#).expect("written");
        let damaged = State::read(&path).expect("читается");
        assert_eq!(damaged.cursor, 0, "отрицательный курсор невозможен");
        assert!(!damaged.enabled, "по умолчанию лента выключена");
    }

    #[test]
    fn only_notifications_of_the_current_period_are_written() {
        let directory = scratch("period");
        let journal = Journal::open(&directory.join("history.db")).expect("журнал");
        let config = config(&directory, true);
        let mut state = State::starting(&config.file);
        state.since_utc = time::micros_to_iso(time::now_micros() - HOUR_MICROS);
        state.write(&config.state).expect("состояние");

        // Что-то старое, попавшее в журнал уже после включения (первый импорт).
        observe(
            &journal,
            "Старое",
            time::micros_to_iso(time::now_micros() - 7_200_000_000),
        );
        let report = poll(&journal, &config);
        assert_eq!(report.written, 0, "старое уведомление не пишется");
        assert!(!config.file.exists(), "пустой файл не создаётся");
        assert!(report.enabled, "лента включена");

        // Новое уведомление попадает в файл один раз.
        observe(&journal, "Новое", time::now_iso());
        let report = poll(&journal, &config);
        assert_eq!(report.written, 1);
        assert!(report.last_local.is_some(), "время последней записи");
        let written = read(&config.file);
        assert!(written.contains("Новое"), "{written}");
        assert!(!written.contains("Старое"), "{written}");
        assert_eq!(written.matches('\t').count(), 2, "одна строка: два таба");
        assert_eq!(written.matches("\r\n\r\n").count(), 1, "одна пустая строка");

        // Повторный проход ничего не добавляет: курсор держит счёт.
        let report = poll(&journal, &config);
        assert_eq!(report.written, 1, "счётчик периода не растёт");
        assert_eq!(read(&config.file).matches("Новое").count(), 1);

        // Следующее уведомление дописывается, порядок сохраняется.
        observe(&journal, "Третье", time::now_iso());
        let report = poll(&journal, &config);
        assert_eq!(report.written, 2);
        let written = read(&config.file);
        assert_eq!(written.matches("\r\n\r\n").count(), 2, "две записи");
        let new_at = written.find("Новое").expect("Новое");
        let third_at = written.find("Третье").expect("Третье");
        assert!(new_at < third_at, "файл только дописывается");

        // Состояние помнит период и курсор.
        let state = State::read(&config.state).expect("состояние");
        assert_eq!(state.written, 2);
        assert!(state.cursor > 0);
    }

    #[test]
    fn switching_the_option_starts_a_new_period() {
        let directory = scratch("switch");
        let journal = Journal::open(&directory.join("history.db")).expect("журнал");
        observe(&journal, "До включения", time::now_iso());

        // Выключенная лента не делает ничего и не создаёт файл.
        let off = config(&directory, false);
        let report = poll(&journal, &off);
        assert!(!report.enabled);
        assert!(!off.file.exists());

        // Включение: уже существующее уведомление новым не считается.
        let on = config(&directory, true);
        let report = poll(&journal, &on);
        assert_eq!(report.written, 0, "уведомление появилось до включения");
        assert!(!on.file.exists());

        observe(&journal, "После включения", time::now_iso());
        assert_eq!(poll(&journal, &on).written, 1);
        assert!(read(&on.file).contains("После включения"));

        // Выключение закрывает период: состояние убирается, и следующее
        // включение начинает считать заново.
        align(&on);
        assert!(State::read(&on.state).is_some(), "период ещё идёт");
        align(&off);
        assert!(
            State::read(&off.state).is_none(),
            "выключение закрывает период"
        );
        observe(&journal, "Во время паузы", time::now_iso());
        assert_eq!(poll(&journal, &off).written, 0, "выключенная лента молчит");

        let report = poll(&journal, &on);
        assert_eq!(report.written, 0, "пауза не попадает в новую ленту");
        observe(&journal, "После второго включения", time::now_iso());
        assert_eq!(poll(&journal, &on).written, 1);
        let written = read(&on.file);
        assert!(written.contains("После второго включения"), "{written}");
        assert!(!written.contains("Во время паузы"), "{written}");
        assert!(
            written.contains("После включения"),
            "прежние записи остаются: файл только дописывается"
        );
    }

    #[test]
    fn the_file_path_follows_the_setting() {
        let layout = storage::Layout::build(
            Path::new(r"D:\данные"),
            PathBuf::from(r"D:\данные\settings.json"),
            false,
            false,
        );
        let automatic = Settings::default();
        assert_eq!(
            file_path(&layout, &automatic),
            layout.exports.join(FILE_NAME),
            "по умолчанию файл лежит рядом с журналом"
        );

        let chosen = Settings {
            feed_dir: Some(PathBuf::from(r"D:\на рабочий стол")),
            ..Settings::default()
        };
        assert_eq!(
            file_path(&layout, &chosen),
            PathBuf::from(r"D:\на рабочий стол").join(FILE_NAME)
        );

        let empty = Settings {
            feed_dir: Some(PathBuf::new()),
            ..Settings::default()
        };
        assert_eq!(
            file_path(&layout, &empty),
            layout.exports.join(FILE_NAME),
            "пустая папка значит «как решено по умолчанию»"
        );

        let config = Config::from_layout(&layout, &chosen);
        assert!(!config.enabled, "по умолчанию лента выключена");
        assert_eq!(config.state, layout.data_dir.join(STATE_FILE));
    }
}
