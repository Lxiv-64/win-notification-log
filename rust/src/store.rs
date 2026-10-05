//! The notification journal: schema, ingestion, search and statistics.
//!
//! The schema is byte-for-byte the same as the Python implementation produces, so
//! both programs can write to and read from the same `history.db` file - the
//! native version can be introduced gradually while the existing viewer, CLI and
//! already collected history keep working.
//!
//! Ingestion mirrors the Python semantics:
//!
//! * identity is `dedup_key` (unique index), which is why two independent
//!   collectors observing the same Windows notification produce one row;
//! * a repeat observation only bumps `seen_count`/`last_seen_utc`;
//! * new content for the same notification updates the row and bumps `revision`;
//! * a second collector contributing to a known row extends `sources`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value as Json;

use crate::model::{IngestOutcome, NotificationRecord};
use crate::sqlite::{self, SqliteError, Value};
use crate::time;
use crate::imaging;

pub const SOURCE_STATE_SOURCE: &str = "wpndb";

/// Counts returned by a batch ingestion.
#[derive(Debug, Default, Clone, Copy)]
pub struct IngestCounts {
    pub inserted: usize,
    pub duplicate: usize,
    pub repeated: usize,
    pub updated: usize,
    pub merged: usize,
    pub failed: usize,
}

impl IngestCounts {
    pub fn add(&mut self, outcome: IngestOutcome) {
        match outcome {
            IngestOutcome::Inserted => self.inserted += 1,
            IngestOutcome::Duplicate => self.duplicate += 1,
            IngestOutcome::Repeated => self.repeated += 1,
            IngestOutcome::Updated => self.updated += 1,
            IngestOutcome::Merged => self.merged += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.inserted + self.duplicate + self.repeated + self.updated + self.merged
    }
}

/// Journal level statistics.
#[derive(Debug, Default, Clone)]
pub struct JournalStats {
    pub total: i64,
    pub first_seen_utc: Option<String>,
    pub last_seen_utc: Option<String>,
    pub by_source: BTreeMap<String, i64>,
    pub by_type: BTreeMap<String, i64>,
    pub applications: i64,
    pub seen_total: i64,
    pub bytes: u64,
    pub fts: bool,
}

/// The journal database.
pub struct Journal {
    db: sqlite::Db,
    path: PathBuf,
}

impl Journal {
    /// Open (creating when needed) the journal and make sure the schema exists.
    pub fn open(path: &Path) -> Result<Journal, SqliteError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let db = sqlite::Db::open_journal(path)?;
        db.exec("PRAGMA journal_mode = WAL")?;
        db.exec("PRAGMA synchronous = NORMAL")?;
        db.exec("PRAGMA temp_store = MEMORY")?;
        let journal = Journal {
            db,
            path: path.to_path_buf(),
        };
        journal.ensure_schema()?;
        Ok(journal)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn connection(&self) -> &sqlite::Db {
        &self.db
    }

    /// Create the schema if it does not exist yet (identical to the Python DDL).
    fn ensure_schema(&self) -> Result<(), SqliteError> {
        self.db.exec(SCHEMA)?;
        // Columns added after the first release are created on demand.
        let existing: BTreeSet<String> = self
            .db
            .query("PRAGMA table_info(notifications)", &[], |row| {
                row.required_text("name")
            })?
            .into_iter()
            .collect();
        if !existing.contains("extra_json") {
            self.db
                .exec("ALTER TABLE notifications ADD COLUMN extra_json TEXT")?;
        }
        self.db.exec(FTS_SCHEMA)?;
        self.db.exec(
            "INSERT INTO meta(key, value) VALUES ('schema_version', '1') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?;
        self.db.exec(&format!(
            "INSERT INTO meta(key, value) VALUES ('created_utc', '{}') ON CONFLICT(key) DO NOTHING",
            time::now_iso()
        ))?;
        Ok(())
    }

    // ---------------------------------------------------------------- ingestion
    /// Store a batch of observations in one transaction.
    pub fn record_many(
        &self,
        records: &[NotificationRecord],
        duplicate_window_seconds: i64,
    ) -> Result<IngestCounts, SqliteError> {
        let mut counts = IngestCounts::default();
        if records.is_empty() {
            return Ok(counts);
        }
        let now = time::now_iso();
        self.db.exec("BEGIN IMMEDIATE")?;
        for record in records {
            match self.ingest(record, &now, duplicate_window_seconds) {
                Ok(outcome) => counts.add(outcome),
                Err(error) => {
                    counts.failed += 1;
                    eprintln!("failed to store {}: {error}", record.dedup_key());
                }
            }
        }
        self.db.exec("COMMIT")?;
        Ok(counts)
    }

    fn ingest(
        &self,
        record: &NotificationRecord,
        now_iso: &str,
        duplicate_window_seconds: i64,
    ) -> Result<IngestOutcome, SqliteError> {
        let dedup_key = record.dedup_key();
        let content_hash = record.content_hash();
        keep_pictures(record);
        let existing = self.db.query_row(
            "SELECT * FROM notifications WHERE dedup_key = ?",
            &[Value::Text(dedup_key)],
        )?;

        let Some(row) = existing else {
            let duplicate_of = if duplicate_window_seconds > 0 {
                let threshold =
                    time::micros_to_iso(time::now_micros() - duplicate_window_seconds * 1_000_000);
                self.db
                    .query_row(
                        "SELECT id FROM notifications WHERE content_hash = ? \
                         AND IFNULL(aumid, '') = ? AND first_seen_utc >= ? ORDER BY id DESC LIMIT 1",
                        &[
                            Value::Text(content_hash.clone()),
                            Value::Text(record.aumid.clone().unwrap_or_default()),
                            Value::Text(threshold),
                        ],
                    )?
                    .and_then(|row| row.int("id"))
            } else {
                None
            };
            let params = record_params(record, now_iso, now_iso, 1, 1, duplicate_of, &content_hash);
            self.db.execute(&insert_sql(), &params)?;
            return Ok(IngestOutcome::Inserted);
        };

        let stored_sources = row.required_text("sources");
        let mut merged_sources: Vec<String> = stored_sources
            .split('+')
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect();
        merged_sources.push(record.source.clone());
        merged_sources.sort();
        merged_sources.dedup();
        let merged_sources = merged_sources.join("+");
        let row_id = row.int("id").unwrap_or_default();

        if row.required_text("content_hash") == content_hash || content_matches(record, &row) {
            if merged_sources != stored_sources {
                self.db.execute(
                    "UPDATE notifications SET last_seen_utc = ?, seen_count = seen_count + 1, \
                     source = ?, sources = ? WHERE id = ?",
                    &[
                        Value::Text(now_iso.to_string()),
                        Value::Text(merged_sources.clone()),
                        Value::Text(merged_sources),
                        Value::Int(row_id),
                    ],
                )?;
                return Ok(IngestOutcome::Merged);
            }
            self.db.execute(
                "UPDATE notifications SET last_seen_utc = ?, seen_count = seen_count + 1 WHERE id = ?",
                &[Value::Text(now_iso.to_string()), Value::Int(row_id)],
            )?;
            return Ok(if row.required_text("content_hash") == content_hash {
                IngestOutcome::Duplicate
            } else {
                IngestOutcome::Repeated
            });
        }

        let stored = record_from_row(&row);
        let merged = stored.merged_with(record);
        let seen_count = row.int("seen_count").unwrap_or(1) + 1;
        let revision = row.int("revision").unwrap_or(1) + 1;
        let duplicate_of = row.int("duplicate_of");
        let first_seen = row.required_text("first_seen_utc");
        let merged_hash = merged.content_hash();
        let params = update_params(
            &merged,
            &first_seen,
            now_iso,
            seen_count,
            revision,
            duplicate_of,
            &merged_hash,
        );
        self.db.execute(&update_sql(), &params)?;
        Ok(IngestOutcome::Updated)
    }
}

/// Keep the pictures of a record next to the journal.
///
/// The copy has to be taken while the reference still leads somewhere: senders
/// point at a temporary file they delete again (or at a file inside their
/// package), and a journal that is read a week later has to show the picture that
/// came with the notification.
fn keep_pictures(record: &NotificationRecord) {
    if record.images.is_empty() {
        return;
    }
    let aumid = record.aumid.as_deref();
    for reference in &record.images {
        imaging::keep(reference, aumid);
    }
}

// ---------------------------------------------------------------------- schema
/// Column order shared by inserts, updates and row mapping.
const COLUMNS: [&str; 32] = [
    "first_seen_utc",
    "last_seen_utc",
    "created_utc",
    "expires_utc",
    "source",
    "sources",
    "aumid",
    "app_name",
    "handler_id",
    "handler_type",
    "notification_type",
    "windows_id",
    "tag",
    "grp",
    "title",
    "body",
    "extra_text",
    "launch",
    "activation_type",
    "scenario",
    "images",
    "actions",
    "payload_xml",
    "payload_type",
    "activity_id",
    "content_hash",
    "dedup_key",
    "duplicate_of",
    "seen_count",
    "revision",
    "extra_json",
    "raw",
];

/// Table and index definitions - identical to the Python implementation, so both
/// programs share one journal file.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS notifications (
    id                INTEGER PRIMARY KEY,
    first_seen_utc    TEXT NOT NULL,
    last_seen_utc     TEXT NOT NULL,
    created_utc       TEXT,
    expires_utc       TEXT,
    source            TEXT NOT NULL,
    sources           TEXT NOT NULL,
    aumid             TEXT,
    app_name          TEXT,
    handler_id        INTEGER,
    handler_type      TEXT,
    notification_type TEXT NOT NULL DEFAULT 'toast',
    windows_id        INTEGER,
    tag               TEXT,
    grp               TEXT,
    title             TEXT,
    body              TEXT,
    extra_text        TEXT,
    launch            TEXT,
    activation_type   TEXT,
    scenario          TEXT,
    images            TEXT,
    actions           TEXT,
    payload_xml       TEXT,
    payload_type      TEXT,
    activity_id       TEXT,
    content_hash      TEXT NOT NULL,
    dedup_key         TEXT NOT NULL,
    duplicate_of      INTEGER,
    seen_count        INTEGER NOT NULL DEFAULT 1,
    revision          INTEGER NOT NULL DEFAULT 1,
    extra_json        TEXT,
    raw               TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_notifications_dedup ON notifications(dedup_key);
CREATE INDEX IF NOT EXISTS ix_notifications_first_seen ON notifications(first_seen_utc DESC);
CREATE INDEX IF NOT EXISTS ix_notifications_app_time ON notifications(aumid, first_seen_utc DESC);
CREATE INDEX IF NOT EXISTS ix_notifications_hash ON notifications(aumid, content_hash, first_seen_utc DESC);
CREATE INDEX IF NOT EXISTS ix_notifications_type ON notifications(notification_type);

CREATE TABLE IF NOT EXISTS app_names (
    aumid        TEXT PRIMARY KEY,
    app_name     TEXT,
    source       TEXT,
    resolved_utc TEXT
);

CREATE TABLE IF NOT EXISTS source_state (
    source         TEXT PRIMARY KEY,
    last_ok_utc    TEXT,
    last_error     TEXT,
    last_error_utc TEXT,
    cursor         TEXT,
    seen_total     INTEGER NOT NULL DEFAULT 0,
    runs           INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);
"#;

/// Full-text index and triggers (same tokenizer as the Python version).
const FTS_SCHEMA: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS notifications_fts USING fts5(
    title, body, extra_text, app_name, aumid, tag, grp,
    content='notifications', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS notifications_fts_ai AFTER INSERT ON notifications BEGIN
    INSERT INTO notifications_fts(rowid, title, body, extra_text, app_name, aumid, tag, grp)
    VALUES (new.id, new.title, new.body, new.extra_text, new.app_name, new.aumid, new.tag, new.grp);
END;

CREATE TRIGGER IF NOT EXISTS notifications_fts_ad AFTER DELETE ON notifications BEGIN
    INSERT INTO notifications_fts(notifications_fts, rowid, title, body, extra_text, app_name, aumid, tag, grp)
    VALUES ('delete', old.id, old.title, old.body, old.extra_text, old.app_name, old.aumid, old.tag, old.grp);
END;

CREATE TRIGGER IF NOT EXISTS notifications_fts_au AFTER UPDATE ON notifications BEGIN
    INSERT INTO notifications_fts(notifications_fts, rowid, title, body, extra_text, app_name, aumid, tag, grp)
    VALUES ('delete', old.id, old.title, old.body, old.extra_text, old.app_name, old.aumid, old.tag, old.grp);
    INSERT INTO notifications_fts(rowid, title, body, extra_text, app_name, aumid, tag, grp)
    VALUES (new.id, new.title, new.body, new.extra_text, new.app_name, new.aumid, new.tag, new.grp);
END;
"#;

/// `INSERT` statement built from [`COLUMNS`].
fn insert_sql() -> String {
    let placeholders: Vec<&str> = COLUMNS.iter().map(|_| "?").collect();
    format!(
        "INSERT INTO notifications ({}) VALUES ({})",
        COLUMNS.join(", "),
        placeholders.join(", ")
    )
}

/// `UPDATE ... WHERE dedup_key = ?` statement built from [`COLUMNS`].
fn update_sql() -> String {
    let assignments: Vec<String> = COLUMNS
        .iter()
        .filter(|column| **column != "dedup_key")
        .map(|column| format!("{column} = ?"))
        .collect();
    format!(
        "UPDATE notifications SET {} WHERE dedup_key = ?",
        assignments.join(", ")
    )
}

/// Serialise a JSON value the way the Python implementation stores it: `NULL`
/// for empty containers, compact JSON otherwise.
fn json_text(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Array(items) if items.is_empty() => Value::Null,
        Json::Object(map) if map.is_empty() => Value::Null,
        other => Value::Text(other.to_string()),
    }
}

/// One JSON list stored in a text column (`images`, `extra_text`).
fn parse_list(value: Option<String>) -> Vec<String> {
    value
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
}

/// Parameter list in [`COLUMNS`] order.
fn record_params(
    record: &NotificationRecord,
    first_seen_iso: &str,
    last_seen_iso: &str,
    seen_count: i64,
    revision: i64,
    duplicate_of: Option<i64>,
    content_hash: &str,
) -> Vec<Value> {
    fn text(value: &Option<String>) -> Value {
        match value {
            Some(inner) => Value::Text(inner.clone()),
            None => Value::Null,
        }
    }
    fn integer(value: Option<i64>) -> Value {
        match value {
            Some(inner) => Value::Int(inner),
            None => Value::Null,
        }
    }

    let notification_type = if record.notification_type.is_empty() {
        "toast".to_string()
    } else {
        record.notification_type.clone()
    };
    vec![
        Value::Text(first_seen_iso.to_string()),
        Value::Text(last_seen_iso.to_string()),
        text(&record.created_utc),
        text(&record.expires_utc),
        Value::Text(record.source.clone()),
        Value::Text(record.source.clone()),
        text(&record.aumid),
        text(&record.app_name),
        integer(record.handler_id),
        text(&record.handler_type),
        Value::Text(notification_type),
        integer(record.windows_id),
        text(&record.tag),
        text(&record.group),
        text(&record.title),
        text(&record.body),
        json_text(&serde_json::to_value(&record.extra_text).unwrap_or(Json::Null)),
        text(&record.launch),
        text(&record.activation_type),
        text(&record.scenario),
        json_text(&serde_json::to_value(&record.images).unwrap_or(Json::Null)),
        json_text(&Json::Array(record.actions.clone())),
        text(&record.payload_xml),
        text(&record.payload_type),
        text(&record.activity_id),
        Value::Text(content_hash.to_string()),
        Value::Text(record.dedup_key()),
        integer(duplicate_of),
        Value::Int(seen_count),
        Value::Int(revision),
        json_text(&record.extra),
        json_text(&record.raw),
    ]
}

/// Same as [`record_params`], but with `dedup_key` moved to the end so that it
/// satisfies `UPDATE ... WHERE dedup_key = ?`.
fn update_params(
    record: &NotificationRecord,
    first_seen_iso: &str,
    last_seen_iso: &str,
    seen_count: i64,
    revision: i64,
    duplicate_of: Option<i64>,
    content_hash: &str,
) -> Vec<Value> {
    let mut params = record_params(
        record,
        first_seen_iso,
        last_seen_iso,
        seen_count,
        revision,
        duplicate_of,
        content_hash,
    );
    let key_index = COLUMNS
        .iter()
        .position(|column| *column == "dedup_key")
        .unwrap_or(0);
    let key = params.remove(key_index);
    params.push(key);
    params
}

/// True when everything the new observation provides is already stored.
///
/// The collectors expose different subsets of a notification (the documented API
/// returns texts, the database returns the payload and tags), so a plain hash
/// comparison would flag every re-observation of an unchanged notification as a
/// content update.  Comparing only the fields the record actually carries keeps
/// `revision` meaningful - and works across implementations, because it does not
/// depend on either side's hash algorithm.
fn content_matches(record: &NotificationRecord, row: &sqlite::Row) -> bool {
    let text_fields: [(&Option<String>, &str); 9] = [
        (&record.title, "title"),
        (&record.body, "body"),
        (&record.launch, "launch"),
        (&record.scenario, "scenario"),
        (&record.activation_type, "activation_type"),
        (&record.tag, "tag"),
        (&record.group, "grp"),
        (&record.aumid, "aumid"),
        (&record.handler_type, "handler_type"),
    ];
    for (value, column) in text_fields {
        if let Some(expected) = value {
            if row.text(column).as_deref() != Some(expected.as_str()) {
                return false;
            }
        }
    }
    if !record.notification_type.is_empty()
        && row.required_text("notification_type") != record.notification_type
    {
        return false;
    }
    if let Some(id) = record.windows_id {
        if row.int("windows_id") != Some(id) {
            return false;
        }
    }
    let list_fields: [(&Vec<String>, &str); 2] = [
        (&record.extra_text, "extra_text"),
        (&record.images, "images"),
    ];
    for (values, column) in list_fields {
        if values.is_empty() {
            continue;
        }
        let stored: Json = row
            .text(column)
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Json::Null);
        let expected = serde_json::to_value(values).unwrap_or(Json::Null);
        if stored != expected {
            return false;
        }
    }
    if !record.actions.is_empty() {
        let stored: Json = row
            .text("actions")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Json::Null);
        if stored != Json::Array(record.actions.clone()) {
            return false;
        }
    }
    true
}

/// Search parameters for a journal listing.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub app: Option<String>,
    pub kind: Option<String>,
    /// Collector that recorded the notification (`wpndb`, `uns`, `wpndb+uns`).
    pub source: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub hide_duplicates: bool,
    /// `time` (default), `created`, `app`, `title`, `kind`, `sources`, `seen` or `id`.
    pub sort: String,
    pub descending: bool,
    pub limit: i64,
    pub offset: i64,
}

impl Default for SearchQuery {
    fn default() -> Self {
        SearchQuery {
            text: None,
            app: None,
            kind: None,
            source: None,
            since: None,
            until: None,
            hide_duplicates: false,
            sort: "time".to_string(),
            descending: true,
            limit: 50,
            offset: 0,
        }
    }
}

/// One application present in the journal (used by the filter list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationInfo {
    /// Identifier stored in the journal (`aumid`).
    pub aumid: String,
    /// Name to show; the identifier when no name is known.
    pub label: String,
    /// How many records belong to it.
    pub count: i64,
}

/// Number of matching rows counted for interactive searches.
pub const COUNT_CAP: i64 = 5000;
/// Records deleted per statement when a selection is removed by id.
const DELETE_CHUNK: usize = 500;

impl Journal {
    /// Run a listing query.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<sqlite::Row>, SqliteError> {
        let (join, clause, params) = self.build_filter(query);
        let column = sort_column(&query.sort);
        let direction = if query.descending { "DESC" } else { "ASC" };
        let limit = query.limit.clamp(1, 5000);
        let offset = query.offset.max(0);
        let sql = format!(
            "SELECT n.* FROM notifications n{join}{clause} \
             ORDER BY n.{column} {direction}, n.id {direction} LIMIT {limit} OFFSET {offset}"
        );
        self.db.query(&sql, &params, |row| row.clone())
    }

    /// Number of the newest record (`0` for an empty journal).
    ///
    /// Used by the text feed: a cursor above this number means the journal was
    /// replaced, and the count starts again from the beginning.
    pub fn highest_record_id(&self) -> Result<i64, SqliteError> {
        self.db
            .query_int("SELECT IFNULL(MAX(id), 0) FROM notifications", &[])
    }

    /// Rows the text feed has not written yet, oldest first.
    ///
    /// `since_utc` is the moment the feed was switched on: a notification that
    /// Windows created before it is not new, even when it only reached the
    /// journal now (the first import of a fresh journal, for example).  A row
    /// without a creation time falls back on the moment it was stored.
    pub fn rows_after(
        &self,
        cursor: i64,
        since_utc: &str,
        limit: i64,
    ) -> Result<Vec<sqlite::Row>, SqliteError> {
        self.db.query(
            "SELECT n.* FROM notifications n \
             WHERE n.id > ? AND IFNULL(n.created_utc, n.first_seen_utc) >= ? \
             ORDER BY n.id LIMIT ?",
            &[
                Value::Int(cursor),
                Value::Text(since_utc.to_string()),
                Value::Int(limit.clamp(1, 5000)),
            ],
            |row| row.clone(),
        )
    }

    /// Count matching rows, capped so that interactive searches stay fast.
    pub fn count(&self, query: &SearchQuery) -> Result<i64, SqliteError> {
        let (join, clause, params) = self.build_filter(query);
        self.db.query_int(
            &format!(
                "SELECT COUNT(*) FROM (SELECT 1 FROM notifications n{join}{clause} LIMIT {COUNT_CAP})"
            ),
            &params,
        )
    }

    /// Journal statistics; `light` skips the expensive aggregates.
    pub fn stats(&self, light: bool) -> Result<JournalStats, SqliteError> {
        let mut stats = JournalStats {
            fts: sqlite::has_fts5(),
            ..Default::default()
        };
        stats.total = self
            .db
            .query_int("SELECT COUNT(*) FROM notifications", &[])?;
        stats.first_seen_utc = self.db.query_text(
            "SELECT first_seen_utc FROM notifications ORDER BY first_seen_utc ASC LIMIT 1",
            &[],
        )?;
        stats.last_seen_utc = self.db.query_text(
            "SELECT first_seen_utc FROM notifications ORDER BY first_seen_utc DESC LIMIT 1",
            &[],
        )?;
        stats.bytes = self.database_size();
        if light {
            return Ok(stats);
        }
        for (source, count) in self.db.query(
            "SELECT sources, COUNT(*) AS n FROM notifications GROUP BY sources ORDER BY n DESC",
            &[],
            |row| (row.required_text("sources"), row.int("n").unwrap_or(0)),
        )? {
            stats.by_source.insert(source, count);
        }
        for (kind, count) in self.db.query(
            "SELECT notification_type, COUNT(*) AS n FROM notifications \
             GROUP BY notification_type ORDER BY n DESC",
            &[],
            |row| {
                (
                    row.required_text("notification_type"),
                    row.int("n").unwrap_or(0),
                )
            },
        )? {
            stats.by_type.insert(kind, count);
        }
        stats.applications = self.db.query_int(
            "SELECT COUNT(DISTINCT aumid) FROM notifications WHERE aumid IS NOT NULL",
            &[],
        )?;
        stats.seen_total = self
            .db
            .query_int("SELECT IFNULL(SUM(seen_count), 0) FROM notifications", &[])?;
        Ok(stats)
    }
}

/// Helper methods that complete the `Journal` API.
impl Journal {
    /// Size of the journal on disk (database plus its WAL files).
    pub fn database_size(&self) -> u64 {
        let mut total = 0;
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{}", self.path.display(), suffix));
            if let Ok(metadata) = std::fs::metadata(candidate) {
                total += metadata.len();
            }
        }
        total
    }

    /// Cursor stored for a collector (the highest processed notification order).
    pub fn source_cursor(&self, source: &str) -> Result<Option<String>, SqliteError> {
        Ok(self
            .db
            .query_row(
                "SELECT cursor FROM source_state WHERE source = ?",
                &[Value::Text(source.to_string())],
            )?
            .and_then(|row| row.optional_text("cursor")))
    }

    /// Update collector bookkeeping (cursor, counters, last success).
    pub fn touch_source(
        &self,
        source: &str,
        cursor: Option<&str>,
        seen_delta: i64,
        count_run: bool,
    ) -> Result<(), SqliteError> {
        self.db.execute(
            "INSERT INTO source_state(source, cursor, seen_total, runs, last_ok_utc) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(source) DO UPDATE SET \
               cursor = COALESCE(excluded.cursor, source_state.cursor), \
               seen_total = source_state.seen_total + excluded.seen_total, \
               runs = source_state.runs + excluded.runs, \
               last_ok_utc = COALESCE(excluded.last_ok_utc, source_state.last_ok_utc)",
            &[
                Value::Text(source.to_string()),
                Value::from(cursor.map(str::to_string)),
                Value::Int(seen_delta),
                Value::Int(if count_run { 1 } else { 0 }),
                Value::Text(time::now_iso()),
            ],
        )?;
        Ok(())
    }

    /// Cache an application display name resolved by a collector.
    ///
    /// The `WHERE` guard makes this a no-op when the stored row is already the
    /// same: without it every pass of the collector would rewrite the whole
    /// `app_names` cache, dirtying pages (and growing the write-ahead log) even
    /// when nothing a reader can see has changed.
    pub fn remember_app_name(
        &self,
        aumid: &str,
        app_name: &str,
        source: &str,
    ) -> Result<(), SqliteError> {
        self.db.execute(
            "INSERT INTO app_names(aumid, app_name, source, resolved_utc) VALUES (?, ?, ?, ?) \
             ON CONFLICT(aumid) DO UPDATE SET app_name = excluded.app_name, \
             source = excluded.source, resolved_utc = excluded.resolved_utc \
             WHERE app_names.app_name IS NOT excluded.app_name \
                OR app_names.source IS NOT excluded.source",
            &[
                Value::Text(aumid.to_string()),
                Value::Text(app_name.to_string()),
                Value::Text(source.to_string()),
                Value::Text(time::now_iso()),
            ],
        )?;
        Ok(())
    }

    /// All cached application names.
    pub fn app_name_map(&self) -> Result<BTreeMap<String, String>, SqliteError> {
        let rows = self.db.query(
            "SELECT aumid, app_name FROM app_names WHERE app_name IS NOT NULL",
            &[],
            |row| (row.required_text("aumid"), row.required_text("app_name")),
        )?;
        Ok(rows.into_iter().collect())
    }

    /// Every picture reference the journal still holds.
    ///
    /// Used to keep the picture folder in step with the records: copies that no
    /// record refers to any more are deleted (see `imaging::prune`).
    pub fn image_references(&self) -> Result<Vec<String>, SqliteError> {
        let rows = self.db.query(
            "SELECT images FROM notifications WHERE images IS NOT NULL",
            &[],
            |row| parse_list(row.text("images")),
        )?;
        Ok(rows.into_iter().flatten().collect())
    }

    /// Number of entries recorded before `iso` (used by the retention preview).
    pub fn count_older_than(&self, iso: &str) -> Result<i64, SqliteError> {
        self.db.query_int(
            "SELECT COUNT(*) FROM notifications WHERE first_seen_utc < ?",
            &[Value::Text(iso.to_string())],
        )
    }

    /// Delete entries recorded before `iso`; returns how many rows disappeared.
    ///
    /// The full-text index follows automatically: `notifications_fts_ad` is an
    /// `AFTER DELETE` trigger, so no separate bookkeeping is needed.
    pub fn delete_older_than(&self, iso: &str) -> Result<i64, SqliteError> {
        let removed = self.count_older_than(iso)?;
        if removed == 0 {
            return Ok(0);
        }
        self.db.execute(
            "DELETE FROM notifications WHERE first_seen_utc < ?",
            &[Value::Text(iso.to_string())],
        )?;
        Ok(removed)
    }

    /// Housekeeping: let SQLite re-plan queries and shrink the write-ahead log.
    ///
    /// `optimize` runs first on purpose: it may write query statistics of its
    /// own, and the checkpoint that follows leaves the write-ahead log empty
    /// instead of holding those pages.
    pub fn maintain(&self) -> Result<(), SqliteError> {
        self.db.exec("PRAGMA optimize")?;
        self.db.exec("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    /// Applications present in the journal, most frequent first.
    ///
    /// This is what fills the application filter: the user recognises "Почта" or
    /// "Telegram", not `Vendor.Mail_1!App`, so the cached display name wins over
    /// the identifier.
    pub fn applications(&self, limit: i64) -> Result<Vec<ApplicationInfo>, SqliteError> {
        let limit = limit.clamp(1, 500);
        self.db.query(
            &format!(
                "SELECT n.aumid AS aumid, IFNULL(a.app_name, n.app_name) AS label, COUNT(*) AS n \
                 FROM notifications n LEFT JOIN app_names a ON a.aumid = n.aumid \
                 WHERE n.aumid IS NOT NULL AND n.aumid <> '' \
                 GROUP BY n.aumid ORDER BY n DESC, label ASC LIMIT {limit}"
            ),
            &[],
            |row| {
                let aumid = row.required_text("aumid");
                let label = row
                    .text("label")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| crate::appname::readable_identifier(&aumid).to_string());
                ApplicationInfo {
                    aumid,
                    label,
                    count: row.int("n").unwrap_or(0),
                }
            },
        )
    }

    /// Notification kinds present in the journal (`toast`, `tile`, `badge`).
    pub fn kinds(&self) -> Result<Vec<(String, i64)>, SqliteError> {
        self.db.query(
            "SELECT notification_type AS value, COUNT(*) AS n FROM notifications \
             WHERE notification_type IS NOT NULL AND notification_type <> '' \
             GROUP BY notification_type ORDER BY n DESC",
            &[],
            |row| (row.required_text("value"), row.int("n").unwrap_or(0)),
        )
    }

    /// Collectors that contributed to the journal, most frequent first.
    pub fn sources(&self) -> Result<Vec<(String, i64)>, SqliteError> {
        self.db.query(
            "SELECT sources AS value, COUNT(*) AS n FROM notifications \
             WHERE sources IS NOT NULL AND sources <> '' \
             GROUP BY sources ORDER BY n DESC",
            &[],
            |row| (row.required_text("value"), row.int("n").unwrap_or(0)),
        )
    }

    /// Delete one record by its journal id; `false` when it was already gone.
    ///
    /// The full-text index follows through its `AFTER DELETE` trigger.  Records
    /// that were merged into this one are left alone: they are separate
    /// notifications, and losing them silently would be worse than a leftover.
    pub fn delete_record(&self, id: i64) -> Result<bool, SqliteError> {
        let affected = self
            .db
            .execute("DELETE FROM notifications WHERE id = ?", &[Value::Int(id)])?;
        Ok(affected > 0)
    }

    /// Count matching records without the cap that keeps the listing fast.
    ///
    /// Deleting asks the user for confirmation first, and "5000+" is not an answer
    /// to "сколько записей будет удалено".
    pub fn count_matching(&self, query: &SearchQuery) -> Result<i64, SqliteError> {
        let (join, clause, params) = self.build_filter(query);
        self.db.query_int(
            &format!("SELECT COUNT(*) FROM notifications n{join}{clause}"),
            &params,
        )
    }

    /// Delete every record the query selects; returns how many rows went away.
    ///
    /// `limit`, `offset` and `sort` belong to a listing and play no part here: the
    /// query is used as a filter and nothing else.  An empty query therefore means
    /// the *whole journal* - which is why every caller asks the user first and
    /// says how many records are about to go.  The full-text index follows through
    /// the same `AFTER DELETE` trigger as a single deletion.
    pub fn delete_matching(&self, query: &SearchQuery) -> Result<i64, SqliteError> {
        let (join, clause, params) = self.build_filter(query);
        let deleted = self.db.execute(
            &format!(
                "DELETE FROM notifications WHERE id IN \
                 (SELECT n.id FROM notifications n{join}{clause})"
            ),
            &params,
        )?;
        Ok(deleted as i64)
    }

    /// Delete a set of records by id - a multi-row selection in the list.
    ///
    /// Long selections are deleted in chunks, because one statement can only bind
    /// a limited number of parameters; the transaction keeps the set together, so
    /// a failure leaves the journal as it was.
    pub fn delete_records(&self, ids: &[i64]) -> Result<i64, SqliteError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut deleted = 0_i64;
        self.db.in_transaction(|| {
            for chunk in ids.chunks(DELETE_CHUNK) {
                let placeholders = vec!["?"; chunk.len()].join(", ");
                let params: Vec<Value> = chunk.iter().copied().map(Value::Int).collect();
                deleted += self.db.execute(
                    &format!("DELETE FROM notifications WHERE id IN ({placeholders})"),
                    &params,
                )? as i64;
            }
            Ok(())
        })?;
        Ok(deleted)
    }

    fn build_filter(&self, query: &SearchQuery) -> (String, String, Vec<Value>) {
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let mut join = String::new();

        if let Some(text) = query.text.as_deref() {
            let fts = fts_query(text);
            if !fts.is_empty() {
                join = " JOIN notifications_fts ON notifications_fts.rowid = n.id".to_string();
                conditions.push("notifications_fts MATCH ?".to_string());
                params.push(Value::Text(fts));
            }
        }
        if let Some(app) = query.app.as_deref().filter(|value| !value.is_empty()) {
            conditions.push("n.aumid = ?".to_string());
            params.push(Value::Text(app.to_string()));
        }
        if let Some(kind) = query.kind.as_deref().filter(|value| !value.is_empty()) {
            conditions.push("n.notification_type = ?".to_string());
            params.push(Value::Text(kind.to_string()));
        }
        if let Some(source) = query.source.as_deref().filter(|value| !value.is_empty()) {
            // `sources` lists what contributed to a row (`wpndb+uns`), so a part
            // of the list is what the user means by "source".
            conditions.push("n.sources LIKE ?".to_string());
            params.push(Value::Text(format!("%{source}%")));
        }
        if let Some(since) = query.since.as_deref().filter(|value| !value.is_empty()) {
            conditions.push("n.first_seen_utc >= ?".to_string());
            params.push(Value::Text(since.to_string()));
        }
        if let Some(until) = query.until.as_deref().filter(|value| !value.is_empty()) {
            conditions.push("n.first_seen_utc <= ?".to_string());
            params.push(Value::Text(until.to_string()));
        }
        if query.hide_duplicates {
            conditions.push("n.duplicate_of IS NULL".to_string());
        }
        let clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        (join, clause, params)
    }
}

/// Column whitelist for `ORDER BY` (same list as the Python implementation).
fn sort_column(sort: &str) -> &'static str {
    match sort {
        "created" => "created_utc",
        "app" => "app_name",
        "aumid" => "aumid",
        "title" => "title",
        "kind" => "notification_type",
        "sources" => "sources",
        "seen" => "seen_count",
        "id" => "id",
        _ => "first_seen_utc",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NotificationRecord;

    fn temporary_journal(name: &str) -> (Journal, PathBuf) {
        // The directory name is deliberately stable (not per process): repeated test
        // runs reuse and re-clean the same handful of folders instead of littering
        // the temporary directory with one set per run.
        let directory = std::env::temp_dir().join(format!("wnl-store-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp directory");
        let path = directory.join("history.db");
        (Journal::open(&path).expect("journal"), path)
    }

    /// Size of the write-ahead log that belongs to a journal file.
    fn wal_bytes(path: &Path) -> u64 {
        std::fs::metadata(format!("{}-wal", path.display()))
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    }

    fn record(windows_id: i64) -> NotificationRecord {
        NotificationRecord {
            source: SOURCE_STATE_SOURCE.to_string(),
            aumid: Some("Vendor.Chat_123!App".to_string()),
            app_name: Some("Vendor Chat".to_string()),
            notification_type: "toast".to_string(),
            windows_id: Some(windows_id),
            title: Some("Alice".to_string()),
            body: Some("Hello there".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn insert_then_repeat_is_a_duplicate() {
        let (journal, _path) = temporary_journal("insert");
        let first = journal.record_many(&[record(1)], 0).expect("stored");
        assert_eq!(first.inserted, 1);
        let second = journal.record_many(&[record(1)], 0).expect("stored");
        assert_eq!(second.duplicate, 1);
        assert_eq!(journal.stats(true).expect("stats").total, 1);
        let row = journal
            .search(&SearchQuery::default())
            .expect("search")
            .into_iter()
            .next()
            .expect("row");
        assert_eq!(row.int("seen_count"), Some(2));
        assert_eq!(row.int("revision"), Some(1));
    }

    #[test]
    fn a_second_collector_merges_the_sources() {
        let (journal, _path) = temporary_journal("merge");
        journal
            .record_many(
                &[NotificationRecord {
                    source: "wpndb".to_string(),
                    ..record(7)
                }],
                0,
            )
            .expect("stored");
        let merged = journal
            .record_many(
                &[NotificationRecord {
                    source: "uns".to_string(),
                    ..record(7)
                }],
                0,
            )
            .expect("stored");
        assert_eq!(merged.merged, 1);
        let row = journal
            .search(&SearchQuery::default())
            .expect("search")
            .into_iter()
            .next()
            .expect("row");
        assert_eq!(row.required_text("sources"), "uns+wpndb");
        assert_eq!(row.int("revision"), Some(1), "content did not change");
    }

    #[test]
    fn changed_content_updates_the_row_and_bumps_the_revision() {
        let (journal, _path) = temporary_journal("update");
        journal.record_many(&[record(3)], 0).expect("stored");
        let updated = journal
            .record_many(
                &[NotificationRecord {
                    title: Some("Bob".to_string()),
                    ..record(3)
                }],
                0,
            )
            .expect("stored");
        assert_eq!(updated.updated, 1);
        let row = journal
            .search(&SearchQuery::default())
            .expect("search")
            .into_iter()
            .next()
            .expect("row");
        assert_eq!(row.required_text("title"), "Bob");
        assert_eq!(row.int("revision"), Some(2));
        assert_eq!(row.int("seen_count"), Some(2));
    }

    #[test]
    fn different_ids_are_separate_rows() {
        let (journal, _path) = temporary_journal("ids");
        journal
            .record_many(&[record(1), record(2)], 0)
            .expect("stored");
        assert_eq!(journal.stats(true).expect("stats").total, 2);
    }

    #[test]
    fn full_text_search_finds_cyrillic_and_ascii() {
        let (journal, _path) = temporary_journal("search");
        journal
            .record_many(
                &[
                    NotificationRecord {
                        title: Some("Счёт".to_string()),
                        body: Some("Оплатите счёт".to_string()),
                        aumid: Some("Mail!App".to_string()),
                        ..record(11)
                    },
                    NotificationRecord {
                        title: Some("Invoice".to_string()),
                        body: Some("pay now".to_string()),
                        aumid: Some("Chat!App".to_string()),
                        ..record(12)
                    },
                ],
                0,
            )
            .expect("stored");

        let cyrillic = journal
            .search(&SearchQuery {
                text: Some("счёт".to_string()),
                ..SearchQuery::default()
            })
            .expect("search");
        assert_eq!(cyrillic.len(), 1);
        assert_eq!(cyrillic[0].required_text("title"), "Счёт");

        let prefix = journal
            .search(&SearchQuery {
                text: Some("invo".to_string()),
                ..SearchQuery::default()
            })
            .expect("search");
        assert_eq!(prefix.len(), 1);

        let by_app = journal
            .search(&SearchQuery {
                app: Some("Chat!App".to_string()),
                ..SearchQuery::default()
            })
            .expect("search");
        assert_eq!(by_app.len(), 1);
    }

    #[test]
    fn cursor_round_trip_and_counters() {
        let (journal, _path) = temporary_journal("cursor");
        assert_eq!(
            journal.source_cursor(SOURCE_STATE_SOURCE).expect("cursor"),
            None
        );
        journal
            .touch_source(SOURCE_STATE_SOURCE, Some("12055"), 3, true)
            .expect("touch");
        assert_eq!(
            journal
                .source_cursor(SOURCE_STATE_SOURCE)
                .expect("cursor")
                .as_deref(),
            Some("12055")
        );
        journal
            .touch_source(SOURCE_STATE_SOURCE, None, 0, false)
            .expect("touch");
        assert_eq!(
            journal
                .source_cursor(SOURCE_STATE_SOURCE)
                .expect("cursor")
                .as_deref(),
            Some("12055"),
            "a missing cursor must not clear the stored one"
        );
    }

    #[test]
    fn application_names_are_cached() {
        let (journal, _path) = temporary_journal("names");
        journal
            .remember_app_name("Vendor.Chat_123!App", "Vendor Chat", "wpndb")
            .expect("stored");
        journal
            .remember_app_name("", "ignored", "wpndb")
            .expect("stored");
        let names = journal.app_name_map().expect("map");
        assert_eq!(
            names.get("Vendor.Chat_123!App").map(String::as_str),
            Some("Vendor Chat")
        );
    }

    /// An unchanged application name must not be written again: the collector
    /// calls `remember_app_name` for every cached name when it looks at its
    /// sources, and a stray write would grow the write-ahead log on every pass -
    /// even when no notification arrived.
    #[test]
    fn storing_the_same_application_name_twice_does_not_write() {
        let (journal, path) = temporary_journal("names-idempotent");
        journal
            .remember_app_name("Vendor.Chat_123!App", "Vendor Chat", "wpndb")
            .expect("stored");
        // Settle the log so the sizes below compare the storing, not the schema.
        journal
            .connection()
            .exec("PRAGMA wal_checkpoint(TRUNCATE)")
            .expect("checkpoint");
        let settled = wal_bytes(&path);

        for _ in 0..10 {
            journal
                .remember_app_name("Vendor.Chat_123!App", "Vendor Chat", "wpndb")
                .expect("stored");
        }
        assert_eq!(
            wal_bytes(&path),
            settled,
            "an unchanged name must not be written again"
        );

        // A name that really changed is written, and the change is readable.
        journal
            .remember_app_name("Vendor.Chat_123!App", "Vendor Chat 2", "wpndb")
            .expect("stored");
        assert!(wal_bytes(&path) > settled, "a changed name is written");
        assert_eq!(
            journal
                .app_name_map()
                .expect("map")
                .get("Vendor.Chat_123!App")
                .map(String::as_str),
            Some("Vendor Chat 2")
        );
    }

    /// A journal with two notifications of two applications and two kinds.
    fn filter_journal(name: &str) -> Journal {
        let (journal, _path) = temporary_journal(name);
        let mut chat = record(1);
        chat.aumid = Some("Vendor.Chat_123!App".to_string());
        chat.app_name = Some("Vendor Chat".to_string());
        chat.title = Some("Alice".to_string());
        chat.body = Some("Hello there".to_string());

        let mut mail = record(2);
        mail.aumid = Some("Vendor.Mail_1!App".to_string());
        mail.app_name = Some("Vendor Mail".to_string());
        mail.notification_type = "tile".to_string();
        mail.title = Some("Новое письмо".to_string());
        mail.body = Some("Отчёт за сентябрь".to_string());
        mail.created_utc = Some("2026-09-20T10:00:00.000000+00:00".to_string());

        // The same toast again: it merges into the first row and raises its
        // repeat counter, which is what the *hide repeats* switch looks at.
        let repeated = record(1);
        journal.record_many(&[chat, mail], 0).expect("stored");
        journal.record_many(&[repeated], 0).expect("stored");
        journal
    }

    #[test]
    fn filters_narrow_the_listing() {
        let journal = filter_journal("filters");
        let all = journal.search(&SearchQuery::default()).expect("search");
        assert_eq!(all.len(), 2, "one record is a repeat of the first");

        let by_app = journal
            .search(&SearchQuery {
                app: Some("Vendor.Mail_1!App".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(by_app.len(), 1);
        assert_eq!(by_app[0].text("app_name").as_deref(), Some("Vendor Mail"));

        let by_kind = journal
            .search(&SearchQuery {
                kind: Some("tile".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(by_kind.len(), 1);

        let by_source = journal
            .search(&SearchQuery {
                source: Some("wpndb".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(by_source.len(), 2, "the collector wrote both records");

        let unknown_source = journal
            .search(&SearchQuery {
                source: Some("uns".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert!(unknown_source.is_empty(), "no WinRT collector has run yet");

        // `first_seen_utc` is stamped when a record is stored, so the range is
        // checked against "now" (an hour ago, an hour into the future).
        let now = crate::time::now_micros();
        let hour = 3_600_000_000i64;
        let since = journal
            .search(&SearchQuery {
                since: Some(crate::time::micros_to_iso(now - hour)),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(since.len(), 2, "both records were stored just now");

        let before = journal
            .search(&SearchQuery {
                until: Some(crate::time::micros_to_iso(now - hour)),
                ..Default::default()
            })
            .expect("search");
        assert!(before.is_empty(), "nothing was recorded an hour ago");

        let future = journal
            .search(&SearchQuery {
                until: Some(crate::time::micros_to_iso(now + hour)),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(future.len(), 2);

        // A combination of criteria narrows further, just like the filter row.
        let combined = journal
            .search(&SearchQuery {
                text: Some("Отчёт".to_string()),
                kind: Some("tile".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(combined.len(), 1);
        assert_eq!(journal.count(&SearchQuery::default()).expect("count"), 2);
    }

    #[test]
    fn sorting_accepts_the_technical_columns() {
        let journal = filter_journal("sorting");
        for key in ["kind", "sources", "seen", "aumid", "title"] {
            let rows = journal
                .search(&SearchQuery {
                    sort: key.to_string(),
                    ..Default::default()
                })
                .expect("sorted");
            assert_eq!(rows.len(), 2, "sorting by {key} must not lose rows");
        }
        let ascending = journal
            .search(&SearchQuery {
                sort: "kind".to_string(),
                descending: false,
                ..Default::default()
            })
            .expect("sorted");
        assert_eq!(
            ascending[0].text("notification_type").as_deref(),
            Some("tile")
        );
    }

    #[test]
    fn the_filter_lists_name_what_the_journal_holds() {
        let journal = filter_journal("filterlists");
        let applications = journal.applications(50).expect("applications");
        assert_eq!(applications.len(), 2);
        let labels: Vec<&str> = applications.iter().map(|app| app.label.as_str()).collect();
        assert!(labels.contains(&"Vendor Chat"));
        assert!(labels.contains(&"Vendor Mail"));
        assert!(applications.iter().all(|app| app.count == 1));
        let kinds = journal.kinds().expect("kinds");
        assert_eq!(kinds.len(), 2);
        assert!(kinds
            .iter()
            .any(|(kind, count)| kind == "toast" && *count == 1));
        assert!(kinds
            .iter()
            .any(|(kind, count)| kind == "tile" && *count == 1));

        assert_eq!(
            journal.sources().expect("sources"),
            vec![("wpndb".to_string(), 2)]
        );
    }

    #[test]
    fn a_hashed_identifier_appears_under_its_name() {
        let (journal, _path) = temporary_journal("hashed");
        let mut browser = record(1);
        browser.aumid = Some("Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456".to_string());
        browser.app_name = None;
        journal.record_many(&[browser], 0).expect("stored");

        let applications = journal.applications(50).expect("applications");
        assert_eq!(applications.len(), 1);
        assert_eq!(
            applications[0].label, "Thorium",
            "the filter offers the browser by the name a reader knows"
        );
        assert_eq!(
            applications[0].aumid, "Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456",
            "the filter itself still asks for the whole identifier"
        );
    }

    #[test]
    fn a_single_record_can_be_deleted() {
        let journal = filter_journal("delete");
        let before = journal.stats(true).expect("stats").total;
        let found = journal
            .search(&SearchQuery {
                text: Some("Alice".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(found.len(), 1, "one record mentions Alice");
        let id = found[0].int("id").expect("id");

        assert!(journal.delete_record(id).expect("delete"));
        assert!(
            !journal.delete_record(id).expect("second delete"),
            "already gone"
        );
        assert_eq!(journal.stats(true).expect("stats").total, before - 1);

        // The full-text index followed the row out of the table.
        let gone = journal
            .search(&SearchQuery {
                text: Some("Alice".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert!(gone.is_empty());

        // The other record stayed where it was, together with its text.
        let kept = journal
            .search(&SearchQuery {
                text: Some("Отчёт".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn a_whole_filter_can_be_deleted_at_once() {
        let journal = filter_journal("delete-filter");
        let query = SearchQuery {
            app: Some("Vendor.Mail_1!App".to_string()),
            ..SearchQuery::default()
        };
        assert_eq!(journal.count_matching(&query).expect("count"), 1);
        assert_eq!(journal.delete_matching(&query).expect("delete"), 1);
        assert_eq!(journal.count_matching(&query).expect("count"), 0);
        assert_eq!(
            journal.delete_matching(&query).expect("second delete"),
            0,
            "nothing is left to delete"
        );

        // Only what the filter named is gone, and the index followed along.
        assert_eq!(journal.stats(true).expect("stats").total, 1);
        assert!(journal
            .search(&SearchQuery {
                text: Some("Отчёт".to_string()),
                ..SearchQuery::default()
            })
            .expect("search")
            .is_empty());
    }

    #[test]
    fn a_period_can_be_deleted_without_touching_older_records() {
        let journal = filter_journal("delete-range");
        let now = time::now_micros();
        let hour = 3_600_000_000i64;

        // Nothing is old enough for the past window ...
        let past = SearchQuery {
            until: Some(time::micros_to_iso(now - hour)),
            ..SearchQuery::default()
        };
        assert_eq!(journal.count_matching(&past).expect("count"), 0);
        assert_eq!(journal.delete_matching(&past).expect("delete"), 0);
        assert_eq!(journal.stats(true).expect("stats").total, 2);

        // ... while a window that starts an hour ago holds both records, so the
        // journal ends up empty - which is exactly what the dialog warns about.
        let window = SearchQuery {
            since: Some(time::micros_to_iso(now - hour)),
            ..SearchQuery::default()
        };
        assert_eq!(journal.count_matching(&window).expect("count"), 2);
        assert_eq!(journal.delete_matching(&window).expect("delete"), 2);
        assert_eq!(journal.stats(true).expect("stats").total, 0);
    }

    #[test]
    fn a_selection_is_deleted_in_one_go() {
        let journal = filter_journal("delete-selection");
        let ids: Vec<i64> = journal
            .search(&SearchQuery::default())
            .expect("search")
            .iter()
            .filter_map(|row| row.int("id"))
            .collect();
        assert_eq!(ids.len(), 2);

        assert_eq!(journal.delete_records(&[]).expect("nothing"), 0);
        assert_eq!(
            journal.delete_records(&[ids[0], 4_242_424]).expect("delete"),
            1,
            "a record that is already gone is not an error"
        );
        assert_eq!(journal.stats(true).expect("stats").total, 1);
        assert_eq!(
            journal.delete_records(&ids).expect("delete again"),
            1,
            "the second id was still there"
        );
        assert_eq!(journal.stats(true).expect("stats").total, 0);
    }

    #[test]
    fn helper_functions_match_the_python_behaviour() {
        assert_eq!(sort_column("app"), "app_name");
        assert_eq!(sort_column("unknown"), "first_seen_utc");
        assert_eq!(fts_query("hello world"), "\"hello\"* AND \"world\"*");
        assert_eq!(fts_query("\"quoted\""), "\"quoted\"*");
        assert_eq!(fts_query("   "), "");
        assert_eq!(json_text(&Json::Array(Vec::new())), Value::Null);
        assert_eq!(
            json_text(&Json::Bool(true)),
            Value::Text("true".to_string())
        );
    }
}

/// Build an FTS5 prefix query from user input: every word becomes `"word"*`
/// joined with `AND`; characters that would break the query are removed.
pub fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|character| *character != '"' && !character.is_control())
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\"*"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// Rebuild a record from a stored row (used when merging two observations).
fn record_from_row(row: &sqlite::Row) -> NotificationRecord {
    NotificationRecord {
        source: row.required_text("source"),
        aumid: row.optional_text("aumid"),
        app_name: row.optional_text("app_name"),
        notification_type: row.required_text("notification_type"),
        windows_id: row.int("windows_id"),
        tag: row.optional_text("tag"),
        group: row.optional_text("grp"),
        title: row.optional_text("title"),
        body: row.optional_text("body"),
        extra_text: parse_list(row.text("extra_text")),
        images: parse_list(row.text("images")),
        actions: row
            .text("actions")
            .and_then(|text| serde_json::from_str::<Vec<Json>>(&text).ok())
            .unwrap_or_default(),
        launch: row.optional_text("launch"),
        activation_type: row.optional_text("activation_type"),
        scenario: row.optional_text("scenario"),
        handler_id: row.int("handler_id"),
        handler_type: row.optional_text("handler_type"),
        activity_id: row.optional_text("activity_id"),
        payload_xml: row.text("payload_xml"),
        payload_type: row.optional_text("payload_type"),
        created_utc: row.optional_text("created_utc"),
        expires_utc: row.optional_text("expires_utc"),
        extra: row
            .text("extra_json")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Json::Null),
        raw: row
            .text("raw")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Json::Null),
    }
}
