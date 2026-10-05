//! Win Notification Log (native) - command line interface.
//!
//! Diagnostics and scripting companion of the graphical build.  Both binaries of
//! this crate share the same storage layer, configuration and collectors.
//!
//! Commands:
//!
//! ```text
//! wnl-cli doctor    environment diagnostics (SQLite, notification database, journal)
//! wnl-cli import    read new notifications and store them into the journal
//! wnl-cli delete    remove records from the journal (nothing happens without --yes)
//! wnl-cli search    list/filter the journal
//! wnl-cli stats     journal statistics
//! wnl-cli selftest  print timestamp/identity samples (used to cross-check Python)
//! ```

use std::path::PathBuf;

use crate::{appname, config, model, sqlite, store, time, view, wpndb};

use store::{Journal, SearchQuery};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parsed command line.
#[derive(Debug, Default, Clone)]
struct Args {
    command: String,
    home: Option<String>,
    database: Option<PathBuf>,
    json: bool,
    light: bool,
    reset: bool,
    /// `delete`: perform the deletion instead of only reporting what it would take.
    yes: bool,
    /// `delete`: allow deleting the whole journal without any filter.
    all: bool,
    limit: i64,
    query: Option<String>,
    app: Option<String>,
    kind: Option<String>,
    source: Option<String>,
    since: Option<String>,
    until: Option<String>,
    types: Option<String>,
    interval: Option<u32>,
}

fn usage() -> String {
    format!(
        "Win Notification Log (native) {VERSION}\n\n\
         usage: wnl <command> [options]\n\n\
         commands:\n  \
         doctor    environment diagnostics\n  \
         import    store new notifications from the Windows database\n  \
         delete    remove records from the journal (nothing without --yes)\n  \
         search    list the journal (optional full-text query)\n  \
         stats     journal statistics\n  \
         settings  print the effective settings and the data folder\n  \
         watch     collect in the foreground until Ctrl+C\n  \
         selftest  print samples used for cross-checking with the Python build\n\n\
         options:\n  \
         --home <dir>        data directory (default: next to the program, then %LOCALAPPDATA%)\n  \
         --database <file>   journal file (overrides the settings)\n  \
         --json              machine readable output\n  \
         --light             skip expensive aggregates (stats)\n  \
         --reset             ignore the stored cursor and rescan the database\n  \
         --yes               delete: really delete (without it only the count is printed)\n  \
         --all               delete: allow deleting the whole journal (no filter given)\n  \
         --types a,b         notification kinds to import (default: toast)\n  \
         --interval <n>      seconds between passes (watch)\n  \
         --limit <n>         maximum rows to list (search)\n  \
         --app <aumid>       filter by application\n  \
         --source <name>     filter by collector (wpndb, uns)\n  \
         --type <kind>       filter by kind\n  \
         --since <date>      lower bound, e.g. 2026-09-01 or 2026-09-01 10:00\n  \
         --until <date>      upper bound"
    )
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args {
        limit: 50,
        ..Default::default()
    };
    let mut index: usize = 0;
    while index < argv.len() {
        let token = argv[index].clone();
        let value_for = |name: &str, index: &mut usize| -> Result<String, String> {
            *index += 1;
            argv.get(*index)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match token.as_str() {
            "--home" => args.home = Some(value_for("--home", &mut index)?),
            "--database" | "--db" => {
                args.database = Some(PathBuf::from(value_for("--database", &mut index)?))
            }
            "--json" => args.json = true,
            "--light" => args.light = true,
            "--reset" => args.reset = true,
            "--yes" | "-y" => args.yes = true,
            "--all" => args.all = true,
            "--types" => args.types = Some(value_for("--types", &mut index)?),
            "--limit" => {
                let raw = value_for("--limit", &mut index)?;
                args.limit = raw
                    .parse::<i64>()
                    .map_err(|error| format!("--limit: {error}"))?;
            }
            "--app" => args.app = Some(value_for("--app", &mut index)?),
            "--source" => args.source = Some(value_for("--source", &mut index)?),
            "--type" | "--kind" => args.kind = Some(value_for("--type", &mut index)?),
            "--since" => args.since = Some(value_for("--since", &mut index)?),
            "--until" => args.until = Some(value_for("--until", &mut index)?),
            "--interval" => {
                let raw = value_for("--interval", &mut index)?;
                args.interval = Some(
                    raw.parse::<u32>()
                        .map_err(|error| format!("--interval: {error}"))?,
                );
            }
            "--help" | "-h" => return Err(usage()),
            other if other.starts_with("--") => return Err(format!("unknown option: {other}")),
            other => {
                if args.command.is_empty() {
                    args.command = other.to_string();
                } else {
                    let combined = match args.query.take() {
                        Some(existing) => format!("{existing} {other}"),
                        None => other.to_string(),
                    };
                    args.query = Some(combined);
                }
            }
        }
        index += 1;
    }
    if args.command.is_empty() {
        return Err(usage());
    }
    Ok(args)
}

/// Configuration, journal and name resolution shared by all commands.
struct Context {
    config: config::Config,
    journal: Journal,
    resolver: appname::NameResolver,
    settings: crate::settings::Settings,
    layout: crate::storage::Layout,
}

/// Data folder and settings for this invocation.
///
/// `--home` (and the environment variable) still win, so scripts keep working;
/// otherwise the same rules as the window apply: settings next to the executable,
/// data next to the executable when that folder is writable, `%LOCALAPPDATA%`
/// otherwise.
fn settings_and_layout(args: &Args) -> (crate::settings::Settings, crate::storage::Layout) {
    let exe_dir = crate::win32::executable_directory();
    let profile = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let settings_path = crate::storage::Layout::settings_path(&exe_dir, profile.as_deref());
    let mut settings = crate::settings::Settings::load(&settings_path);
    crate::lang::set(crate::lang::Lang::from_code(&settings.language));
    if let Some(home) = args
        .home
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        settings.data_dir = Some(PathBuf::from(home));
    }
    let layout = crate::storage::resolve(&settings, &exe_dir, profile.as_deref());
    // Pictures of notifications are kept next to the journal, exactly like the
    // window does it: the collector runs from here as well (`import`, `watch`).
    crate::imaging::configure(layout.pictures.clone(), settings.keep_pictures);
    (settings, layout)
}

fn open_context(args: &Args) -> Result<Context, String> {
    let (settings, layout) = settings_and_layout(args);
    let config = config::Config::from_settings(&settings, &layout);
    let journal_path = config.journal_path(args.database.as_deref());
    let journal = Journal::open(&journal_path).map_err(|error| {
        format!(
            "cannot open the journal {}: {error}",
            journal_path.display()
        )
    })?;
    let cached = journal
        .app_name_map()
        .map_err(|error| format!("cannot read cached application names: {error}"))?;
    let resolver = appname::NameResolver::with_cache(cached.into_iter().collect());
    Ok(Context {
        config,
        journal,
        resolver,
        settings,
        layout,
    })
}

fn kinds_from(args: &Args, config: &config::Config) -> Vec<String> {
    match args
        .types
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        Some(list) => list
            .split(',')
            .map(|item| item.trim().to_lowercase())
            .filter(|item| !item.is_empty())
            .collect(),
        None => config.capture_types.clone(),
    }
}

/// Entry point of `wnl-cli.exe`.  Returns the process exit code.
pub fn run() -> i32 {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    match args.command.as_str() {
        "doctor" => cmd_doctor(&args),
        "import" => cmd_import(&args),
        "delete" | "purge" => cmd_delete(&args),
        "search" => cmd_search(&args),
        "stats" => cmd_stats(&args),
        "settings" => cmd_settings(&args),
        "watch" | "service" => cmd_watch(&args),
        "selftest" => cmd_selftest(&args),
        other => {
            eprintln!("unknown command: {other}\n\n{}", usage());
            2
        }
    }
}

/// Print the effective settings and where they live.
fn cmd_settings(args: &Args) -> i32 {
    let (settings, layout) = settings_and_layout(args);
    let document = serde_json::json!({
        "settings_file": layout.settings_path.display().to_string(),
        "data_dir": layout.data_dir.display().to_string(),
        "journal": layout.journal.display().to_string(),
        "feed_file": crate::feed::file_path(&layout, &settings).display().to_string(),
        "portable": layout.portable,
        "fallback_used": layout.fallback_used,
        "settings": settings.to_json(),
    });
    match serde_json::to_string_pretty(&document) {
        Ok(text) => println!("{text}"),
        Err(error) => {
            eprintln!("cannot serialise the settings: {error}");
            return 1;
        }
    }
    0
}

/// Stop flag for the console control handler.
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Console control handler: Ctrl+C, Ctrl+Break, window close.
unsafe extern "system" fn on_console_control(_kind: u32) -> i32 {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
    1
}

#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

/// Collect in the foreground: the console twin of the tray service.
///
/// Runs the very same collector as `wnl.exe`, so a machine without a desktop
/// session can still keep a journal - and `Ctrl+C` stops it cleanly, leaving a
/// truthful `status.json` behind.
fn cmd_watch(args: &Args) -> i32 {
    let (settings, layout) = settings_and_layout(args);
    let mut params = crate::service::Params::from_layout(&layout, &settings);
    if let Some(interval) = args.interval {
        params.interval_seconds = interval.max(1);
    }
    if let Some(types) = args
        .types
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        params.capture_types = types
            .split(',')
            .map(|item| item.trim().to_lowercase())
            .filter(|item| !item.is_empty())
            .collect();
    }
    unsafe {
        SetConsoleCtrlHandler(Some(on_console_control), 1);
    }
    println!(
        "{}",
        tf!("сбор каждые {} с", "collecting every {} s", params.interval_seconds)
    );
    println!("{}", tf!("данные: {}", "data: {}", layout.data_dir.display()));
    println!("{}", tr!("остановить: Ctrl+C"));
    let control = crate::service::spawn(params);
    while !STOP.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    control.stop();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while control.is_running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("{}", tr!("сбор остановлен"));
    0
}

// -------------------------------------------------------------------- commands
fn cmd_doctor(args: &Args) -> i32 {
    let mut problems = 0;
    println!("Win Notification Log (native) {VERSION}");

    match sqlite::library_version() {
        Ok(version) => println!(
            "[ok     ] sqlite : {version} (FTS5: {})",
            sqlite::has_fts5()
        ),
        Err(error) => {
            problems += 1;
            println!("[error  ] sqlite : {error}");
        }
    }

    let context = match open_context(args) {
        Ok(context) => {
            println!("[ok     ] home   : {}", context.config.home.display());
            match context.journal.stats(true) {
                Ok(stats) => println!(
                    "[ok     ] journal: {} record(s), FTS5 {}, {:.2} MB, {}",
                    stats.total,
                    stats.fts,
                    stats.bytes as f64 / 1_048_576.0,
                    context.journal.path().display()
                ),
                Err(error) => {
                    problems += 1;
                    println!("[error  ] journal: {error}");
                }
            }
            println!(
                "[ok     ] cursor : {:?} ({})",
                context.journal.source_cursor(wpndb::SOURCE).unwrap_or(None),
                wpndb::SOURCE
            );
            // Where the data lives and how the program is configured: the questions a
            // user actually asks first.
            println!(
                "[ok     ] layout : {}",
                crate::storage::describe(&context.layout)
            );
            println!(
                "[ok     ] settings: {}",
                context.layout.settings_path.display()
            );
            println!(
                "[ok     ] collect: every {} s, backfill {}, retention {}",
                context.settings.collect_interval_seconds,
                context.settings.backfill_limit,
                if context.settings.retention_days == 0 {
                    tr!("бессрочно").to_string()
                } else {
                    tf!("{} дн.", "{} days", context.settings.retention_days)
                }
            );
            let exe = crate::win32::executable_path();
            let autostart = crate::autostart::is_enabled(&exe);
            println!(
                "[ok     ] autostart: {}",
                if autostart {
                    tr!("включён")
                } else {
                    tr!("выключен")
                }
            );
            println!(
                "[ok     ] log     : {}",
                if context.settings.log_enabled {
                    format!("{}", context.layout.log.display())
                } else {
                    tr!("выключен в настройках").to_string()
                }
            );
            match crate::service::Status::read(&context.layout.status) {
                Some(status) => {
                    println!(
                        "{}",
                        tf!(
                            "[ok     ] service: {} (прогонов {}, ошибок {})",
                            "[ok     ] service: {} (runs {}, errors {})",
                            crate::service::describe(&status),
                            status.runs,
                            status.errors
                        )
                    );
                    match &status.uns {
                        Some(uns) => println!("[ok     ] uns    : {}", uns.describe()),
                        None => println!(
                            "{}",
                            tr!("[ok     ] uns    : состояние источника не записано")
                        ),
                    }
                }
                None => println!(
                    "{}",
                    tr!("[ok     ] service: состояние не публиковалось (status.json отсутствует)")
                ),
            }
            context
        }
        Err(message) => {
            println!("[error  ] journal: {message}");
            return 1;
        }
    };
    let _ = &context;

    let path = wpndb::database_path();
    match wpndb::NotificationDatabase::open(&path) {
        Ok(database) => {
            let toasts = database.toast_count().unwrap_or_default();
            let max_order = database.max_order().unwrap_or_default();
            let limits = database
                .metadata()
                .map(|metadata| metadata.get("toast:maxCount").cloned().unwrap_or_default())
                .unwrap_or_default();
            let kinds = kinds_from(args, &context.config);
            println!(
                "[ok     ] wpndb  : {} toast(s), max order {max_order}, limit {limits} per app, \
                 capturing {kinds:?}",
                toasts
            );
        }
        Err(error) => {
            problems += 1;
            println!("[error  ] wpndb  : {} ({error})", path.display());
        }
    }

    // The documented WinRT API is asked live: the answer is exactly what the
    // collector sees right now.  A source that cannot read is not counted as a
    // problem - the journal is kept either way - but the reason is shown.
    if !context.settings.uns_enabled {
        println!("{}", tr!("[ok     ] uns    : выключен в настройках"));
    } else {
        match crate::uns::probe() {
            Ok(state) => println!("[ok     ] uns    : {state}"),
            Err(reason) => println!("[note   ] uns    : {reason}"),
        }
    }

    // The text feed needs no live probe: its own state file says which file it
    // writes to and how much it already holds, and it is there when the window
    // is not running.
    let feed = crate::feed::status(&crate::feed::Config::from_layout(
        &context.layout,
        &context.settings,
    ));
    println!("[ok     ] feed   : {}", feed.describe());

    println!("\n{problems} problem(s)");
    if problems == 0 {
        0
    } else {
        1
    }
}

fn cmd_import(args: &Args) -> i32 {
    let context = match open_context(args) {
        Ok(context) => context,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let path = wpndb::database_path();
    let database = match wpndb::NotificationDatabase::open(&path) {
        Ok(database) => database,
        Err(error) => {
            eprintln!(
                "cannot open the notification database {}: {error}",
                path.display()
            );
            return 1;
        }
    };
    // The collector also owns the picture folder: copies whose record is gone are
    // removed (the window does the same when it starts).
    if let Ok(references) = context.journal.image_references() {
        crate::imaging::prune(&context.layout.pictures, &references);
    }

    if args.reset {
        if let Err(error) = context
            .journal
            .touch_source(wpndb::SOURCE, Some("0"), 0, false)
        {
            eprintln!("cannot reset the cursor: {error}");
            return 1;
        }
        println!("cursor reset: the whole notification database will be scanned");
    }

    let kinds = kinds_from(args, &context.config);
    let before = context
        .journal
        .stats(true)
        .map(|stats| stats.total)
        .unwrap_or_default();
    let outcome = match wpndb::scan_new(
        &database,
        &context.journal,
        &kinds,
        context.config.backfill_on_first_run,
        context.config.max_backfill_records,
        /* batch_limit */ 2000,
        context.config.duplicate_window_seconds,
        &context.resolver,
        // The command line applies the same rules as the window: an import that
        // quietly kept what the application would have skipped would make the two
        // disagree about the journal they share.
        &if context.settings.rules_enabled {
            context.settings.rules.clone()
        } else {
            crate::rules::RuleSet::default()
        },
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("scan failed: {error}");
            return 1;
        }
    };
    let after = context
        .journal
        .stats(true)
        .map(|stats| stats.total)
        .unwrap_or_default();

    println!(
        "scanned {} batch(es): {} observation(s) processed, journal {before} -> {after} record(s)",
        outcome.batches, outcome.emitted,
    );
    println!("cursor: {:?}", outcome.cursor);
    if outcome.saturated {
        println!("note: more rows are pending, run import again");
    }
    let _ = &context.resolver;
    0
}

/// Delete records from the journal - the scriptable twin of the window's deletion.
///
/// A script is worse than a person at noticing that it just removed the wrong
/// thing, so the command has two locks: nothing is deleted without `--yes`, and a
/// query without any filter is refused unless `--all` says that the whole journal
/// is meant.
fn cmd_delete(args: &Args) -> i32 {
    let context = match open_context(args) {
        Ok(context) => context,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let query = SearchQuery {
        text: args.query.clone(),
        app: args.app.clone(),
        kind: args.kind.clone(),
        source: args.source.clone(),
        since: normalize_bound(args.since.as_deref()),
        until: normalize_bound(args.until.as_deref()),
        ..Default::default()
    };
    let filtered = !deletes_everything(&query);
    if !filtered && !args.all {
        eprintln!(
            "refusing to delete the whole journal: narrow the selection with \
             --app/--type/--source/--since/--until/--query, or pass --all to mean it"
        );
        return 2;
    }
    let count = match context.journal.count_matching(&query) {
        Ok(count) => count,
        Err(error) => {
            eprintln!("cannot count the records: {error}");
            return 1;
        }
    };
    if !args.yes {
        if args.json {
            println!(
                "{}",
                serde_json::json!({
                    "would_delete": count,
                    "whole_journal": !filtered,
                    "journal": context.journal.path().display().to_string(),
                })
            );
        } else {
            println!(
                "would delete {count} record(s){}",
                if filtered { "" } else { " (the whole journal)" }
            );
            println!("run the same command with --yes to delete them");
        }
        return 0;
    }
    match context.journal.delete_matching(&query) {
        Ok(deleted) => {
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "deleted": deleted,
                        "whole_journal": !filtered,
                        "journal": context.journal.path().display().to_string(),
                    })
                );
            } else {
                println!("deleted {deleted} record(s)");
            }
            0
        }
        Err(error) => {
            eprintln!("delete failed: {error}");
            1
        }
    }
}

/// True when a query holds no filter at all, so it would take the whole journal.
fn deletes_everything(query: &SearchQuery) -> bool {
    query.text.is_none()
        && query.app.is_none()
        && query.kind.is_none()
        && query.source.is_none()
        && query.since.is_none()
        && query.until.is_none()
}

fn cmd_search(args: &Args) -> i32 {
    let context = match open_context(args) {
        Ok(context) => context,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let query = SearchQuery {
        text: args.query.clone(),
        app: args.app.clone(),
        kind: args.kind.clone(),
        source: args.source.clone(),
        since: normalize_bound(args.since.as_deref()),
        until: normalize_bound(args.until.as_deref()),
        hide_duplicates: false,
        sort: "time".to_string(),
        descending: true,
        limit: args.limit,
        offset: 0,
    };
    let rows = match context.journal.search(&query) {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!("search failed: {error}");
            return 1;
        }
    };
    let total = context.journal.count(&query).unwrap_or(rows.len() as i64);

    if args.json {
        for row in &rows {
            println!("{}", row_as_json(row));
        }
        return 0;
    }
    if rows.is_empty() {
        println!("no records match the query");
        return 0;
    }
    println!("showing {} of {total} matching record(s)\n", rows.len());
    for row in &rows {
        println!(
            "{}  {:<32}  {}",
            time::format_local_iso(row.text("first_seen_utc").as_deref()),
            truncate(&display_app(row), 32),
            truncate(&row_text(row), 80)
        );
    }
    0
}

fn cmd_stats(args: &Args) -> i32 {
    let context = match open_context(args) {
        Ok(context) => context,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let stats = match context.journal.stats(args.light) {
        Ok(stats) => stats,
        Err(error) => {
            eprintln!("statistics failed: {error}");
            return 1;
        }
    };
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "journal": context.journal.path().display().to_string(),
                "total": stats.total,
                "first_seen_utc": stats.first_seen_utc,
                "last_seen_utc": stats.last_seen_utc,
                "by_source": stats.by_source,
                "by_type": stats.by_type,
                "applications": stats.applications,
                "seen_total": stats.seen_total,
                "bytes": stats.bytes,
                "fts": stats.fts,
            })
        );
        return 0;
    }
    println!("journal        : {}", context.journal.path().display());
    println!("records        : {}", stats.total);
    println!(
        "first record   : {}",
        time::format_local_iso(stats.first_seen_utc.as_deref())
    );
    println!(
        "latest record  : {}",
        time::format_local_iso(stats.last_seen_utc.as_deref())
    );
    println!(
        "size on disk   : {:.2} MB",
        stats.bytes as f64 / 1_048_576.0
    );
    println!(
        "full-text index: {}",
        if stats.fts { "FTS5" } else { "unavailable" }
    );
    if !args.light {
        println!("by source      : {:?}", stats.by_source);
        println!("by kind        : {:?}", stats.by_type);
        println!("applications   : {}", stats.applications);
        println!("observations   : {}", stats.seen_total);
    }
    0
}

/// Print the samples used to verify that the Rust and Python builds agree on the
/// journal formats (timestamps, de-duplication keys and file times).
fn cmd_selftest(_args: &Args) -> i32 {
    println!("iso samples (microseconds since the Unix epoch):");
    for micros in [
        0_i64,
        1_000_000,
        1_700_000_000_000_000,
        1_774_934_846_676_804,
    ] {
        println!("  {micros} -> {}", time::micros_to_iso(micros));
    }
    println!("filetime samples (100 ns ticks since 1601):");
    for ticks in [0_i64, 116_444_736_000_000_000, 133_500_000_000_000_000] {
        println!("  {ticks} -> {:?}", time::filetime_to_iso(ticks));
    }
    let record = model::NotificationRecord {
        source: "wpndb".to_string(),
        aumid: Some("WinNotificationLog.TestSender".to_string()),
        notification_type: "toast".to_string(),
        windows_id: Some(42),
        title: Some("Title".to_string()),
        body: Some("Body".to_string()),
        ..Default::default()
    };
    println!("dedup key      : {}", record.dedup_key());
    println!("content hash   : {}", record.content_hash());
    let fallback = model::NotificationRecord {
        source: "wpndb".to_string(),
        aumid: Some("App".to_string()),
        title: Some("hello".to_string()),
        body: Some("world".to_string()),
        ..Default::default()
    };
    println!("fallback key   : {}", fallback.dedup_key());
    0
}

// --------------------------------------------------------------------- helpers
/// Turn a `YYYY-MM-DD[ HH:MM[:SS]]` bound into a journal timestamp (UTC).
fn normalize_bound(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains('T') {
        return Some(value.to_string());
    }
    let (date, rest) = match value.split_once(' ') {
        Some((date, rest)) => (date, rest.trim()),
        None => (value, ""),
    };
    let mut parts = date.split('-');
    let year = parts.next().unwrap_or_default().to_string();
    let month = parts.next().unwrap_or("01").to_string();
    let day = parts.next().unwrap_or("01").to_string();
    let mut time_parts = rest.split(':');
    let hour = time_parts.next().unwrap_or("00").to_string();
    let minute = time_parts.next().unwrap_or("00").to_string();
    let second = time_parts.next().unwrap_or("00").to_string();
    if year.len() != 4 {
        return None;
    }
    Some(format!(
        "{year}-{month:0>2}-{day:0>2}T{hour:0>2}:{minute:0>2}:{second:0>2}.000000+00:00"
    ))
}

/// Application of a row: the name a collector reported, otherwise the identifier
/// itself - in the readable form the window uses as well, so a Chromium browser
/// reads `Thorium` here and not `Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456`.
fn display_app(row: &sqlite::Row) -> String {
    let aumid = row.optional_text("aumid");
    row.optional_text("app_name")
        .or_else(|| {
            aumid
                .as_deref()
                .map(crate::appname::readable_identifier)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "(unknown application)".to_string())
}

fn row_text(row: &sqlite::Row) -> String {
    crate::view::row_text(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_without_any_filter_is_the_whole_journal() {
        assert!(deletes_everything(&SearchQuery::default()));
        // Every single criterion is enough to make the deletion specific.
        for query in [
            SearchQuery {
                text: Some("счёт".to_string()),
                ..Default::default()
            },
            SearchQuery {
                app: Some("Vendor.Mail_1!App".to_string()),
                ..Default::default()
            },
            SearchQuery {
                kind: Some("toast".to_string()),
                ..Default::default()
            },
            SearchQuery {
                source: Some("wpndb".to_string()),
                ..Default::default()
            },
            SearchQuery {
                since: Some("2026-09-01T00:00:00.000000+00:00".to_string()),
                ..Default::default()
            },
            SearchQuery {
                until: Some("2026-09-01T00:00:00.000000+00:00".to_string()),
                ..Default::default()
            },
        ] {
            assert!(
                !deletes_everything(&query),
                "a filter must be recognised: {query:?}"
            );
        }
    }
}

fn truncate(text: &str, limit: usize) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= limit {
        return single_line;
    }
    let mut shortened: String = single_line.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

/// One journal row as a JSON object (JSON Lines output).
fn row_as_json(row: &sqlite::Row) -> String {
    let document = serde_json::json!({
        "id": row.int("id"),
        "first_seen_utc": row.text("first_seen_utc"),
        "first_seen_local": time::format_local_iso(row.text("first_seen_utc").as_deref()),
        "created_utc": row.text("created_utc"),
        "aumid": row.text("aumid"),
        "app_name": row.text("app_name"),
        "notification_type": row.text("notification_type"),
        "windows_id": row.int("windows_id"),
        "tag": row.text("tag"),
        "grp": row.text("grp"),
        "title": row.text("title"),
        "body": row.text("body"),
        "images": view::images(row),
        "sources": row.text("sources"),
        "seen_count": row.int("seen_count"),
        "revision": row.int("revision"),
        "dedup_key": row.text("dedup_key"),
    });
    document.to_string()
}
