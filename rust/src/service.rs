//! Background collection: the part of the application that works without a window.
//!
//! The collector runs in its own thread with its own SQLite connection, so
//! scanning a large notification database never freezes the user interface (the
//! window only reads the journal).  The same code runs in three situations:
//!
//! * inside `wnl.exe` while the window is open (a thread next to the window);
//! * in `wnl.exe --background`, started by the autostart entry - the tray icon
//!   stays, the window does not appear;
//! * in `wnl-cli.exe watch`, for people who prefer a console.
//!
//! State is published in `status.json` inside the data directory, which is what
//! the window, the diagnostics and external tools can read at any time.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};

use crate::appname::NameResolver;
use crate::store::Journal;
use crate::win32::{self, HANDLE};
use crate::{storage, time, wpndb, VERSION};

/// How much log text is kept before the file is rotated.
pub const MAX_LOG_BYTES: u64 = 512 * 1024;
/// Reset the write-ahead log and re-plan queries every this many passes.
pub const MAINTENANCE_EVERY: u64 = 60;
/// Precision of the sleep loop (keeps `stop` responsive).
const TICK: Duration = Duration::from_millis(100);
/// Shortest distance between two change-driven writes to the file (seconds).
pub const STATUS_FILE_MIN_SECONDS: u64 = 10;

/// State of one collector, as published in `status.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub running: bool,
    pub paused: bool,
    pub started_utc: String,
    pub last_run_utc: Option<String>,
    pub last_duration_ms: u64,
    pub last_emitted: i64,
    pub last_batches: i64,
    pub runs: u64,
    pub errors: u64,
    pub last_error: Option<String>,
    pub records: i64,
    pub journal_bytes: u64,
    pub journal: PathBuf,
    pub data_dir: PathBuf,
    pub interval_seconds: u32,
    pub version: String,
    /// Rules of notification processing that are in force (one line, for the log).
    pub rules: String,
    /// How many notifications the rules kept out of the journal.
    pub skipped_by_rules: i64,
    /// Pop-ups the rules asked for; the window shows the ones it has not seen.
    pub alerts: Vec<crate::rules::Alert>,
    /// Number of the last alert; it keeps counting across passes, so the window
    /// can tell a new pop-up from one it has already shown.
    pub alert_seq: u64,
    /// State of the documented WinRT source (`uns`), when it is switched on.
    pub uns: Option<crate::uns::Report>,
    /// State of the text feed of new notifications (`crate::feed`).
    pub feed: Option<crate::feed::Report>,
}

impl Status {
    /// Fresh snapshot for a collector that has not run yet.
    pub fn starting(params: &Params) -> Status {
        Status {
            running: true,
            paused: false,
            started_utc: time::now_iso(),
            last_run_utc: None,
            last_duration_ms: 0,
            last_emitted: 0,
            last_batches: 0,
            runs: 0,
            errors: 0,
            last_error: None,
            records: 0,
            journal_bytes: 0,
            journal: params.journal.clone(),
            data_dir: params.data_dir.clone(),
            interval_seconds: params.interval_seconds,
            version: VERSION.to_string(),
            rules: params.rules.describe(),
            skipped_by_rules: 0,
            alerts: Vec::new(),
            alert_seq: 0,
            uns: None,
            feed: None,
        }
    }

    /// JSON form written to disk.
    pub fn to_json(&self) -> Json {
        json!({
            "running": self.running,
            "paused": self.paused,
            "started_utc": self.started_utc,
            "last_run_utc": self.last_run_utc,
            "last_run_local": self
                .last_run_utc
                .as_deref()
                .map(|value| time::format_local_iso(Some(value))),
            "last_duration_ms": self.last_duration_ms,
            "last_emitted": self.last_emitted,
            "last_batches": self.last_batches,
            "runs": self.runs,
            "errors": self.errors,
            "last_error": self.last_error,
            "records": self.records,
            "journal_bytes": self.journal_bytes,
            "journal": self.journal.display().to_string(),
            "data_dir": self.data_dir.display().to_string(),
            "interval_seconds": self.interval_seconds,
            "version": self.version,
            "rules": self.rules,
            "skipped_by_rules": self.skipped_by_rules,
            "alerts": self
                .alerts
                .iter()
                .map(crate::rules::Alert::to_json)
                .collect::<Vec<_>>(),
            "alert_seq": self.alert_seq,
            "uns": self.uns.as_ref().map(crate::uns::Report::to_json),
            "feed": self.feed.as_ref().map(crate::feed::Report::to_json),
        })
    }

    /// Write the snapshot atomically (a reader never sees half a file).
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_else(|_| "{}".into());
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, format!("{text}\n"))?;
        std::fs::rename(&temporary, path)
    }

    /// Read a snapshot; `None` when there is none or it is unreadable.
    pub fn read(path: &Path) -> Option<Status> {
        let text = std::fs::read_to_string(path).ok()?;
        let json: Json = serde_json::from_str(&text).ok()?;
        Some(Status::from_json(&json))
    }

    /// Tolerant mapping of a snapshot (missing fields become defaults).
    pub fn from_json(json: &Json) -> Status {
        let flag = |key: &str| json.get(key).and_then(Json::as_bool).unwrap_or(false);
        let number = |key: &str| json.get(key).and_then(Json::as_i64).unwrap_or(0);
        let text = |key: &str| json.get(key).and_then(Json::as_str).map(str::to_string);
        Status {
            running: flag("running"),
            paused: flag("paused"),
            started_utc: text("started_utc").unwrap_or_default(),
            last_run_utc: text("last_run_utc"),
            last_duration_ms: number("last_duration_ms").max(0) as u64,
            last_emitted: number("last_emitted"),
            last_batches: number("last_batches"),
            runs: number("runs").max(0) as u64,
            errors: number("errors").max(0) as u64,
            last_error: text("last_error"),
            records: number("records"),
            journal_bytes: number("journal_bytes").max(0) as u64,
            journal: text("journal").map(PathBuf::from).unwrap_or_default(),
            data_dir: text("data_dir").map(PathBuf::from).unwrap_or_default(),
            interval_seconds: number("interval_seconds").max(0) as u32,
            version: text("version").unwrap_or_default(),
            rules: text("rules").unwrap_or_default(),
            skipped_by_rules: number("skipped_by_rules"),
            alerts: json
                .get("alerts")
                .and_then(Json::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(crate::rules::Alert::from_json)
                        .collect()
                })
                .unwrap_or_default(),
            alert_seq: number("alert_seq").max(0) as u64,
            // `null` (or a missing key) means "this version had no such source";
            // the collector writes an object once it has read the center.
            uns: json
                .get("uns")
                .filter(|value| !value.is_null())
                .map(crate::uns::Report::from_json),
            feed: json
                .get("feed")
                .filter(|value| !value.is_null())
                .map(crate::feed::Report::from_json),
        }
    }
}

/// The state of one collector: the truth lives in memory, the file is a copy.
///
/// The window and the collector share a process, so the window reads the snapshot
/// straight from memory.  `status.json` remains for everyone *outside*: external
/// tools (`wnl-cli doctor`, scripts), a collector started in another process
/// (`wnl-cli watch`) and the trace that survives a crash of the window.  It is
/// therefore written only when something a reader can act upon changed, plus
/// once at start and once at stop - never on a timer, so an idle collector
/// leaves the disk alone instead of rewriting the file to prove it is alive.
pub struct StatusStore {
    snapshot: Mutex<Option<Status>>,
    /// What the file holds right now, together with the time it was written.
    written: Mutex<Option<(Status, Instant)>>,
    path: Option<PathBuf>,
}

impl StatusStore {
    /// A store publishing into `path` (`None` keeps everything in memory).
    pub fn new(path: Option<PathBuf>) -> StatusStore {
        StatusStore {
            snapshot: Mutex::new(None),
            written: Mutex::new(None),
            path,
        }
    }

    /// The current state; `None` while no collector has published anything.
    pub fn snapshot(&self) -> Option<Status> {
        self.snapshot
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .clone()
    }

    /// Publish a state: always in memory, on disk only when it is worth it.
    pub fn publish(&self, status: Status, force: bool) {
        let now = Instant::now();
        {
            let mut written = self.written.lock().unwrap_or_else(|lock| lock.into_inner());
            if should_write(written.as_ref(), &status, now, force) {
                if let Some(path) = &self.path {
                    let _ = status.write(path);
                }
                *written = Some((status.clone(), now));
            }
        }
        let mut snapshot = self.snapshot.lock().unwrap_or_else(|lock| lock.into_inner());
        *snapshot = Some(status);
    }
}

/// Decide whether a snapshot goes to the file.
///
/// A forced write always wins (start, stop, a change of the settings), otherwise
/// the file follows a change that waits long enough not to write on every single
/// pass.  There is deliberately no timer: when nothing a reader can act upon has
/// changed, the collector does not touch the disk at all.  Only what a reader can
/// see and act upon counts as a change.
fn should_write(
    previous: Option<&(Status, Instant)>,
    status: &Status,
    now: Instant,
    force: bool,
) -> bool {
    if force {
        return true;
    }
    let Some((written, at)) = previous else {
        return true;
    };
    if now.saturating_duration_since(*at) < Duration::from_secs(STATUS_FILE_MIN_SECONDS) {
        return false;
    }
    written.records != status.records
        || written.errors != status.errors
        || written.paused != status.paused
        || written.running != status.running
        || written.last_error != status.last_error
        || written.alert_seq != status.alert_seq
        || written.skipped_by_rules != status.skipped_by_rules
        // A change of the WinRT source (access granted, denied, an error) has to
        // reach the file as well: `wnl-cli doctor` reports it from there.
        || written.uns != status.uns
        // The same for the text feed: a reader has to see which file it writes
        // to and what went wrong when it cannot.
        || written.feed != status.feed
}

/// Everything one collector needs to know.
#[derive(Debug, Clone)]
pub struct Params {
    pub data_dir: PathBuf,
    pub journal: PathBuf,
    pub status: Option<PathBuf>,
    pub log: Option<PathBuf>,
    pub capture_types: Vec<String>,
    pub interval_seconds: u32,
    pub poll_seconds: f64,
    pub backfill_limit: i64,
    pub backfill_on_first_run: bool,
    pub duplicate_window_seconds: i64,
    pub retention_days: i64,
    /// Read the notification center through the documented WinRT API as well.
    pub uns_enabled: bool,
    /// How often the WinRT API is polled (seconds).
    pub uns_poll_seconds: f64,
    /// The text file new notifications are appended to (`crate::feed`).
    pub feed: crate::feed::Config,
    /// Rules of notification processing (see `crate::rules`).
    pub rules: crate::rules::RuleSet,
}

impl Params {
    /// Parameters for a resolved data directory and the user settings.
    pub fn from_layout(layout: &storage::Layout, settings: &crate::settings::Settings) -> Params {
        Params {
            data_dir: layout.data_dir.clone(),
            journal: layout.journal.clone(),
            status: Some(layout.status.clone()),
            // A switched-off log means no file at all: the log lines, the trace
            // and the note a crash would leave are all written through it.
            log: settings.log_enabled.then(|| layout.log.clone()),
            capture_types: crate::config::DEFAULT_CAPTURE_KINDS
                .iter()
                .map(|kind| kind.to_string())
                .collect(),
            interval_seconds: settings.collect_interval_seconds,
            poll_seconds: settings.poll_seconds,
            backfill_limit: settings.backfill_limit,
            backfill_on_first_run: true,
            duplicate_window_seconds: 0,
            retention_days: settings.retention_days,
            uns_enabled: settings.uns_enabled,
            uns_poll_seconds: settings.uns_poll_seconds,
            feed: crate::feed::Config::from_layout(layout, settings),
            // A switched-off rule list is the same as no rule list at all; the
            // switch in the settings window is what makes that difference.
            rules: if settings.rules_enabled {
                settings.rules.clone()
            } else {
                crate::rules::RuleSet::default()
            },
        }
    }
}

/// Result of one pass over the Windows notification database.
#[derive(Debug, Clone, Default)]
pub struct PassReport {
    pub emitted: usize,
    pub batches: usize,
    pub duration: Duration,
    pub records: i64,
    /// How many notifications the rules kept out of the journal.
    pub skipped: i64,
    /// Pop-ups the rules asked for during this pass.
    pub alerts: Vec<crate::rules::Alert>,
    /// What the documented WinRT source did during this pass.
    pub uns: Option<crate::uns::Report>,
    /// What the text feed did during this pass.
    pub feed: Option<crate::feed::Report>,
}

/// Switches shared between the owner of a collector and its thread.
#[derive(Clone)]
pub struct Control {
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    kick: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    /// Where the thread publishes its state (memory, and rarely the file).
    store: Arc<StatusStore>,
}

impl Default for Control {
    fn default() -> Control {
        Control::new(None)
    }
}

impl Control {
    /// A control whose collector publishes into `status.json` at `path`.
    pub fn new(path: Option<PathBuf>) -> Control {
        Control {
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            kick: Arc::new(AtomicBool::new(false)),
            alive: Arc::new(AtomicBool::new(false)),
            store: Arc::new(StatusStore::new(path)),
        }
    }

    /// The state of the collector, for the window in the same process.
    pub fn state(&self) -> Option<Status> {
        self.store.snapshot()
    }

    /// Publish a state (see `StatusStore::publish`).
    pub fn publish(&self, status: &Status, force: bool) {
        self.store.publish(status.clone(), force);
    }

    /// Ask the collector to finish and return.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Pause or resume collection (the thread keeps running).
    pub fn set_paused(&self, paused: bool) {
        self.pause.store(paused, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.pause.load(Ordering::SeqCst)
    }

    /// Collect right now instead of waiting for the next interval.
    pub fn kick(&self) {
        self.kick.store(true, Ordering::SeqCst);
    }

    /// True while the collector thread is inside its loop.
    pub fn is_running(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn take_kick(&self) -> bool {
        self.kick.swap(false, Ordering::SeqCst)
    }
}

/// One pass over the notification database with an existing journal connection.
///
/// The window calls this for *Импорт сейчас* (its own connection), the collector
/// thread calls it in a loop (its own connection).
pub fn pass(
    journal: &Journal,
    params: &Params,
    resolver: &NameResolver,
    reset: bool,
) -> Result<PassReport, String> {
    let started = Instant::now();
    let path = wpndb::database_path();
    let database = wpndb::NotificationDatabase::open(&path)
        .map_err(|error| format!("база уведомлений недоступна ({}): {error}", path.display()))?;
    if reset {
        journal
            .touch_source(wpndb::SOURCE, Some("0"), 0, false)
            .map_err(|error| error.to_string())?;
    }
    let outcome = wpndb::scan_new(
        &database,
        journal,
        &params.capture_types,
        params.backfill_on_first_run,
        params.backfill_limit,
        2000,
        params.duplicate_window_seconds,
        resolver,
        &params.rules,
    )
    .map_err(|error| error.to_string())?;
    let records = journal
        .stats(true)
        .map(|stats| stats.total)
        .unwrap_or_default();
    // The same pass also reads the notification center through the documented
    // WinRT API (`uns`).  It needs the database only for the application name
    // (Windows 11 does not report `AppInfo`), and it never fails the pass: a
    // source that cannot read reports that in its own state.
    let uns = if params.uns_enabled {
        Some(crate::uns::poll(
            Some(&database),
            journal,
            resolver,
            params.uns_poll_seconds,
            params.duplicate_window_seconds,
            &params.rules,
        ))
    } else {
        Some(crate::uns::Report::disabled("выключен в настройках"))
    };
    // The notifications of this pass go into the text file the user chose.  Like
    // the WinRT source, the feed never fails the pass: a file that cannot be
    // written reports that in its own state.
    let feed = crate::feed::poll(journal, &params.feed);
    Ok(PassReport {
        emitted: outcome.emitted,
        batches: outcome.batches,
        duration: started.elapsed(),
        records,
        skipped: outcome.skipped,
        alerts: outcome.alerts,
        uns,
        feed: Some(feed),
    })
}

/// Start the collector in a background thread.
pub fn spawn(params: Params) -> Control {
    let control = Control::new(params.status.clone());
    let thread_control = control.clone();
    std::thread::Builder::new()
        .name("wnl-collector".to_string())
        .spawn(move || run(params, thread_control))
        .expect("collector thread");
    control
}

/// The collector loop; returns when `control.stop()` was called.
pub fn run(params: Params, control: Control) {
    control.alive.store(true, Ordering::SeqCst);
    let mut status = Status::starting(&params);
    // The number of the last pop-up continues where the previous run stopped:
    // otherwise the numbers would start from one again and the window, which
    // remembers what it has shown, would treat new pop-ups as old ones.
    if let Some(path) = &params.status {
        if let Some(previous) = Status::read(path) {
            status.alert_seq = previous.alert_seq;
        }
    }
    control.publish(&status, true);
    write_log(&params, "коллектор запущен");
    write_log(
        &params,
        if params.uns_enabled {
            "источник uns: читаю центр уведомлений через WinRT"
        } else {
            "источник uns: выключен в настройках"
        },
    );
    if !params.rules.is_empty() {
        write_log(
            &params,
            &tf!(
                "правила обработки: {}",
                "processing rules: {}",
                params.rules.describe()
            ),
        );
    }
    // The text feed: a setting that changed outside a running collector (in the
    // settings window, in the file by hand) opens a new period, so the state of
    // the previous one is dropped first.
    crate::feed::align(&params.feed);
    let feed_line = if params.feed.enabled {
        tf!(
            "лента новых уведомлений: дописываю в {}",
            "feed of new notifications: appending to {}",
            params.feed.file.display()
        )
    } else {
        tr!("лента новых уведомлений: выключена в настройках").to_string()
    };
    write_log(&params, &feed_line);

    let journal = match Journal::open(&params.journal) {
        Ok(journal) => journal,
        Err(error) => {
            status.running = false;
            status.errors += 1;
            status.last_error = Some(format!("не удалось открыть журнал: {error}"));
            control.publish(&status, true);
            write_log(
                &params,
                &tf!("журнал недоступен: {error}", "journal is unavailable: {error}"),
            );
            control.alive.store(false, Ordering::SeqCst);
            return;
        }
    };
    let cached = journal
        .app_name_map()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let resolver = NameResolver::with_cache(cached);
    let interval = Duration::from_secs(u64::from(params.interval_seconds.max(1)));
    let mut next_maintenance = MAINTENANCE_EVERY;
    // True once a pass has written to the journal: housekeeping (a checkpoint,
    // query statistics) is held back while nothing changes, so an idle collector
    // never touches `history.db` or its write-ahead log.
    let mut journal_dirty = false;
    // The state of the WinRT source is only written to the log when it changes:
    // an access answer or an error is worth a line, "everything is fine" is not.
    let mut uns_state: Option<String> = None;
    // The same for the feed, plus one line per pass that appended entries.
    let mut feed_state: Option<String> = None;
    let mut feed_written: u64 = 0;

    while !control.is_stopped() {
        let paused = control.is_paused();
        if paused != status.paused {
            status.paused = paused;
            // A pause or a resume is a user action: it reaches the file at once,
            // so a reader outside this process does not have to wait for the next
            // change (there is no periodic write any more).
            control.publish(&status, true);
        }
        if !status.paused || control.take_kick() {
            match pass(&journal, &params, &resolver, false) {
                Ok(report) => {
                    status.runs += 1;
                    status.last_run_utc = Some(time::now_iso());
                    status.last_duration_ms = report.duration.as_millis() as u64;
                    status.last_emitted = report.emitted as i64;
                    status.last_batches = report.batches as i64;
                    status.records = report.records;
                    status.last_error = None;
                    status.uns = report.uns.clone();
                    if let Some(uns) = &status.uns {
                        let line = uns.describe();
                        if uns_state.as_deref() != Some(line.as_str()) {
                            write_log(
                                &params,
                                &tf!("источник uns: {line}", "uns source: {line}"),
                            );
                            uns_state = Some(line);
                        }
                    }
                    status.feed = report.feed.clone();
                    if let Some(feed) = &status.feed {
                        let key = feed.state_key();
                        if feed_state.as_deref() != Some(key.as_str()) {
                            write_log(&params, &tf!("лента: {key}", "feed: {key}"));
                            feed_state = Some(key);
                        }
                        if feed.written > feed_written {
                            write_log(
                                &params,
                                &tf!(
                                    "в ленту добавлено строк: {} (всего {})",
                                    "lines added to the feed: {} ({} in total)",
                                    feed.written - feed_written,
                                    feed.written
                                ),
                            );
                            feed_written = feed.written;
                        }
                    }
                    status.skipped_by_rules += report.skipped;
                    if report.emitted > 0 || report.skipped > 0 {
                        journal_dirty = true;
                    }
                    // Every pop-up gets its own number: the window shows the ones
                    // it has not seen yet, and the numbers do not repeat after a
                    // restart of the collector.
                    for mut alert in report.alerts {
                        status.alert_seq += 1;
                        alert.seq = status.alert_seq;
                        status.alerts.push(alert);
                    }
                    // The list of pop-ups is only a hand-over to the window; keep
                    // the last few and let the numbers do the remembering.
                    if status.alerts.len() > 20 {
                        let drop = status.alerts.len() - 20;
                        status.alerts.drain(..drop);
                    }
                    if report.skipped > 0 {
                        write_log(
                            &params,
                            &tf!(
                                "правила пропустили уведомлений: {}",
                                "the rules skipped {} notifications",
                                report.skipped
                            ),
                        );
                    }
                    if report.emitted > 0 {
                        write_log(
                            &params,
                            &tf!(
                                "+{} новых уведомлений за {} мс",
                                "+{} new notifications in {} ms",
                                report.emitted,
                                report.duration.as_millis()
                            ),
                        );
                    }
                }
                Err(error) => {
                    status.errors += 1;
                    status.last_run_utc = Some(time::now_iso());
                    status.last_error = Some(error.clone());
                    write_log(
                        &params,
                        &tf!("ошибка сбора: {error}", "collection error: {error}"),
                    );
                }
            }
            if params.retention_days > 0
                && prune(&journal, params.retention_days, &mut status, &params) > 0
            {
                journal_dirty = true;
            }
            if status.runs >= next_maintenance {
                next_maintenance += MAINTENANCE_EVERY;
                if journal_dirty {
                    let _ = journal.maintain();
                    journal_dirty = false;
                }
            }
            status.journal_bytes = journal.database_size();
            // The window sees this at once (memory); the file follows when the
            // numbers changed or when the reader has waited long enough.
            control.publish(&status, false);
        }
        // Sleep in small steps so that stop, pause and kick react at once.
        let deadline = Instant::now() + interval;
        while !control.is_stopped() && Instant::now() < deadline {
            if control.take_kick() {
                break;
            }
            std::thread::sleep(TICK);
        }
    }

    status.running = false;
    // The last state has to reach the file whatever the throttling says.
    control.publish(&status, true);
    write_log(&params, "коллектор остановлен");
    control.alive.store(false, Ordering::SeqCst);
}

/// Delete entries older than the configured retention period.
///
/// Returns how many records disappeared; the collector uses that to tell a pass
/// that wrote to the journal from one that did not (see the maintenance gate).
fn prune(journal: &Journal, days: i64, status: &mut Status, params: &Params) -> i64 {
    let cutoff = time::micros_to_iso(time::now_micros() - days * 86_400 * 1_000_000);
    match journal.delete_older_than(&cutoff) {
        Ok(removed) if removed > 0 => {
            status.records = journal
                .stats(true)
                .map(|stats| stats.total)
                .unwrap_or(status.records);
            write_log(
                params,
                &tf!(
                    "удалено старых записей: {removed}",
                    "old records deleted: {removed}"
                ),
            );
            removed
        }
        Ok(_) => 0,
        Err(error) => {
            write_log(
                params,
                &tf!(
                    "не удалось применить срок хранения: {error}",
                    "could not apply the retention policy: {error}"
                ),
            );
            0
        }
    }
}

/// Append one line to the collector log, rotating it when it grows too large.
pub fn write_log(params: &Params, line: &str) {
    let Some(path) = &params.log else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0) > MAX_LOG_BYTES {
        let rotated = path.with_extension("log.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(path, &rotated);
    }
    let stamp = time::format_local_iso(Some(&time::now_iso()));
    // A line that is not composed on the spot (a plain sentence) is translated
    // here; composed lines carry their own two templates and arrive ready.
    let text = format!("{stamp}  {}\n", crate::lang::tr(line));
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(text.as_bytes());
    }
}

/// A named mutex that tells whether this user already runs the application.
pub struct Instance {
    handle: HANDLE,
}

impl Drop for Instance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                win32::CloseHandle(self.handle);
            }
        }
    }
}

/// Name of the mutex; session-local, so every signed-in user can run a copy.
pub const INSTANCE_NAME: &str = r"Local\WinNotificationLog";

/// One line describing what the collector is doing (the status bar uses it).
///
/// Kept free of formatting surprises (no floating point, no timezone maths) so
/// that the window and the diagnostics show the same sentence.
pub fn describe(status: &Status) -> String {
    if let Some(error) = &status.last_error {
        return tf!("ошибка сбора: {error}", "collection error: {error}");
    }
    if !status.running {
        return tr!("сбор остановлен").to_string();
    }
    if status.paused {
        return tr!("сбор приостановлен").to_string();
    }
    let run = status.last_run_utc.as_deref();
    let Some(run) = run else {
        return tr!("сбор запущен, идёт первое чтение…").to_string();
    };
    let local = time::format_local_iso(Some(run));
    let clock = local.rsplit(' ').next().unwrap_or(&local).to_string();
    // Rules that kept notifications out of the journal are worth mentioning: an
    // empty list of new records looks wrong until one knows why it is empty.
    let filtered = if status.skipped_by_rules > 0 {
        tf!(
            "  •  правила пропустили {}",
            "  •  rules skipped {}",
            status.skipped_by_rules
        )
    } else {
        String::new()
    };
    if status.last_emitted > 0 {
        tf!(
            "сбор: {clock}  +{} за {} мс  •  записей {}{filtered}",
            "collect: {clock}  +{} in {} ms  •  records {}{filtered}",
            status.last_emitted,
            status.last_duration_ms,
            status.records
        )
    } else {
        tf!(
            "сбор: {clock}  новых нет  •  записей {}{filtered}",
            "collect: {clock}  nothing new  •  records {}{filtered}",
            status.records
        )
    }
}

/// Take the single instance lock; `None` when another copy is already running.
pub fn acquire_instance(name: &str) -> Option<Instance> {
    let name_w = win32::wide(name);
    let handle = unsafe { win32::CreateMutexW(std::ptr::null_mut(), 0, name_w.as_ptr()) };
    if handle.is_null() {
        // No mutex at all: never block the user because of a system limit.
        return Some(Instance { handle });
    }
    if std::io::Error::last_os_error().raw_os_error() == Some(183) {
        unsafe {
            win32::CloseHandle(handle);
        }
        return None;
    }
    Some(Instance { handle })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("wnl-service-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    fn params(directory: &Path) -> Params {
        Params {
            data_dir: directory.to_path_buf(),
            journal: directory.join("history.db"),
            status: Some(directory.join("status.json")),
            log: Some(directory.join("wnl.log")),
            capture_types: vec!["toast".to_string()],
            interval_seconds: 30,
            poll_seconds: 2.0,
            backfill_limit: 10,
            backfill_on_first_run: false,
            duplicate_window_seconds: 0,
            retention_days: 0,
            uns_enabled: false,
            uns_poll_seconds: crate::uns::DEFAULT_POLL_SECONDS,
            feed: crate::feed::Config {
                enabled: false,
                file: directory.join(crate::feed::FILE_NAME),
                state: directory.join(crate::feed::STATE_FILE),
            },
            rules: crate::rules::RuleSet::default(),
        }
    }

    #[test]
    fn the_file_follows_changes_but_not_every_pass() {
        let directory = scratch("status-policy");
        let params = params(&directory);
        let mut status = Status::starting(&params);
        let start = Instant::now();

        // The first write always happens: a reader has to see something.
        assert!(should_write(None, &status, start, false));

        let written = (status.clone(), start);

        // A change right after a write waits: a loop that runs every three
        // seconds must not become a file writer that runs every three seconds.
        status.records += 1;
        assert!(!should_write(
            Some(&written),
            &status,
            start + Duration::from_secs(1),
            false
        ));

        // The same change, a little later, is worth a write.
        assert!(should_write(
            Some(&written),
            &status,
            start + Duration::from_secs(STATUS_FILE_MIN_SECONDS),
            false
        ));

        // Nothing changed: no write, however long the collector runs.  There is
        // no timer that rewrites the file just to prove the collector is alive.
        assert!(!should_write(
            Some(&written),
            &written.0,
            start + Duration::from_secs(3600),
            false
        ));

        // Starting, stopping and a change of the settings go through at once.
        assert!(should_write(Some(&written), &status, start, true));
    }

    #[test]
    fn the_snapshot_is_read_from_memory_and_the_file_stays_behind() {
        let directory = scratch("status-memory");
        let path = directory.join("status.json");
        let store = StatusStore::new(Some(path.clone()));
        let mut status = Status::starting(&params(&directory));

        store.publish(status.clone(), true);
        assert_eq!(store.snapshot().expect("snapshot"), status);
        assert!(path.exists(), "первая запись создаёт файл");

        // Memory follows every pass, the file waits for its moment.
        status.records += 7;
        status.last_duration_ms = 12;
        store.publish(status.clone(), false);
        assert_eq!(store.snapshot().expect("snapshot").records, status.records);
        assert_eq!(
            Status::read(&path).expect("file").records,
            0,
            "файл обновляется не на каждом проходе"
        );
    }

    #[test]
    fn status_survives_a_round_trip() {
        let directory = scratch("status");
        let mut status = Status::starting(&params(&directory));
        status.runs = 4;
        status.last_emitted = 3;
        status.records = 42;
        status.last_run_utc = Some(time::now_iso());
        status.last_error = Some("пример".to_string());
        let path = directory.join("status.json");
        status.write(&path).expect("written");

        let loaded = Status::read(&path).expect("readable");
        assert_eq!(loaded, status);
        assert!(loaded.to_json()["last_run_local"].is_string());
        assert_eq!(loaded.to_json()["version"], json!(VERSION));

        // A journal that was never collected, or a file that is not a status at
        // all, is simply "no status" - the window has to start anyway.
        let directory = scratch("no-status");
        assert_eq!(Status::read(&directory.join("status.json")), None);
        std::fs::write(directory.join("status.json"), "not json").expect("written");
        assert_eq!(Status::read(&directory.join("status.json")), None);
    }

    #[test]
    fn log_lines_accumulate_and_rotate() {
        let directory = scratch("log");
        let params = params(&directory);
        write_log(&params, "первая строка");
        write_log(&params, "вторая строка");
        let text = std::fs::read_to_string(directory.join("wnl.log")).expect("log");
        assert!(text.contains("первая строка"));
        assert!(text.contains("вторая строка"));

        // A log larger than the limit is moved aside before the next line.
        let big = vec![b'x'; (MAX_LOG_BYTES + 10) as usize];
        std::fs::write(directory.join("wnl.log"), &big).expect("written");
        write_log(&params, "после ротации");
        assert!(directory.join("wnl.log.1").exists());
        let fresh = std::fs::read_to_string(directory.join("wnl.log")).expect("log");
        assert!(fresh.contains("после ротации"));
        assert!(fresh.len() < 200, "the fresh file starts empty: {fresh:?}");
    }

    #[test]
    fn a_switched_off_log_writes_nothing() {
        let directory = scratch("log-off");
        let layout = storage::Layout::build(
            &directory,
            directory.join("settings.json"),
            true,
            false,
        );
        let mut settings = crate::settings::Settings::default();
        settings.data_dir = Some(directory.clone());
        let log = directory.join(storage::LOG_FILE);

        // On: the line lands in the file.
        let on = Params::from_layout(&layout, &settings);
        assert!(on.log.is_some());
        write_log(&on, "строка журнала");
        assert!(log.exists());

        // Off: no line is written - and no file is created either.
        settings.log_enabled = false;
        let off = Params::from_layout(&layout, &settings);
        assert!(off.log.is_none(), "выключенный лог не имеет пути");
        write_log(&off, "этой строки быть не должно");
        assert!(
            !std::fs::read_to_string(&log)
                .expect("log")
                .contains("не должно"),
            "выключенный лог не пишется"
        );
        std::fs::remove_file(&log).expect("removed");
        write_log(&off, "и этой строки тоже");
        assert!(!log.exists(), "выключенный лог не создаёт файл");
    }

    #[test]
    fn single_instance_lock_rejects_the_second_taker() {
        let name = format!(r"Local\WinNotificationLog-test-{}", std::process::id());
        let first = acquire_instance(&name);
        assert!(first.is_some(), "the first acquirer gets the lock");
        assert!(
            acquire_instance(&name).is_none(),
            "a second copy is turned away"
        );
        drop(first);
        assert!(
            acquire_instance(&name).is_some(),
            "after the owner exits the name is free"
        );
    }

    #[test]
    fn control_switches_are_visible_to_both_sides() {
        let control = Control::default();
        assert!(!control.is_paused());
        assert!(!control.is_stopped());
        control.set_paused(true);
        assert!(control.is_paused());
        control.kick();
        assert!(control.take_kick(), "a kick is consumed exactly once");
        assert!(!control.take_kick());
        let clone = control.clone();
        control.stop();
        assert!(clone.is_stopped(), "clones share the same switches");
    }

    #[test]
    fn describe_explains_every_state() {
        let directory = scratch("describe");
        let mut status = Status::starting(&params(&directory));
        assert!(describe(&status).contains("первое чтение"));

        status.last_run_utc = Some(time::now_iso());
        status.last_emitted = 3;
        status.last_duration_ms = 42;
        status.records = 12;
        let text = describe(&status);
        assert!(text.contains("+3"));
        assert!(text.contains("42 мс"));
        assert!(text.contains("записей 12"));

        status.last_emitted = 0;
        assert!(describe(&status).contains("новых нет"));

        status.paused = true;
        assert_eq!(describe(&status), "сбор приостановлен");

        status.paused = false;
        status.running = false;
        assert_eq!(describe(&status), "сбор остановлен");

        status.running = true;
        status.last_error = Some("нет доступа".to_string());
        assert!(describe(&status).starts_with("ошибка сбора"));
    }

    #[test]
    fn a_collector_can_be_started_and_stopped() {
        let directory = scratch("loop");
        let control = spawn(params(&directory));
        control.stop();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status_path = directory.join("status.json");
        while (control.is_running() || Status::read(&status_path).is_none())
            && Instant::now() < deadline
        {
            std::thread::sleep(TICK);
        }
        assert!(!control.is_running(), "the thread finishes after stop");
        let status = Status::read(&status_path).expect("status written");
        assert!(
            !status.running,
            "the last snapshot reports a stopped collector"
        );
        assert_eq!(status.journal, directory.join("history.db"));
    }
}
