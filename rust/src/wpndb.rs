//! Collector for the Windows notification database (port of `sources/wpndb.py`).
//!
//! The Windows Push Notification service stores every delivered toast in
//! `%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db`.  The file is
//! readable by the owning user with ordinary permissions (`mode=ro`, while the
//! service holds its locks) and carries the richest metadata a third party can
//! get: the original payload XML, tags/groups, arrival and expiry times.
//!
//! New rows are followed through the `Notification.[Order]` column; that cursor
//! lives in the journal's `source_state` table - the same key the Python
//! implementation uses, so both can run alternately without re-importing.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::appname::NameResolver;
use crate::model::NotificationRecord;
use crate::payload;
use crate::sqlite::{self, SqliteError, Value};
use crate::store::Journal;
use crate::time;

pub const SOURCE: &str = "wpndb";

/// `%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db`.
pub fn database_path() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    local
        .join("Microsoft")
        .join("Windows")
        .join("Notifications")
        .join("wpndatabase.db")
}

/// Read-only access to the Windows notification database.
pub struct NotificationDatabase {
    db: sqlite::Db,
    path: PathBuf,
}

impl NotificationDatabase {
    pub fn open(path: &Path) -> Result<Self, SqliteError> {
        let db = sqlite::Db::open_readonly_uri(path)?;
        Ok(NotificationDatabase {
            db,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Highest `Order` value in the database.
    pub fn max_order(&self) -> Result<i64, SqliteError> {
        self.db.query_int(
            "SELECT IFNULL(MAX([Order]), 0) AS value FROM Notification",
            &[],
        )
    }

    /// Number of toast notifications currently present.
    pub fn toast_count(&self) -> Result<i64, SqliteError> {
        self.db.query_int(
            "SELECT COUNT(*) AS value FROM Notification WHERE [Type] = 'toast'",
            &[],
        )
    }

    /// The `Metadata` table (per-application limits such as `toast:maxCount`).
    pub fn metadata(&self) -> Result<BTreeMap<String, String>, SqliteError> {
        let rows = self.db.query(
            "SELECT [Key] AS key, [Value] AS value FROM Metadata",
            &[],
            |row| (row.required_text("key"), row.required_text("value")),
        )?;
        Ok(rows.into_iter().collect())
    }

    /// Fetch notifications newer than `after`.
    pub fn fetch(
        &self,
        after: i64,
        kinds: &[String],
        limit: i64,
    ) -> Result<Vec<sqlite::Row>, SqliteError> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<&str> = kinds.iter().map(|_| "?").collect();
        let sql = format!(
            "SELECT n.[Order] AS order_id, n.[Id] AS notification_id, n.[Type] AS notification_type, \
                    n.[Payload] AS payload, n.[PayloadType] AS payload_type, n.[Tag] AS tag, \
                    n.[Group] AS grp, n.[ActivityId] AS activity_id, n.[ArrivalTime] AS arrival_time, \
                    n.[ExpiryTime] AS expiry_time, n.[BootId] AS boot_id, \
                    n.[ExpiresOnReboot] AS expires_on_reboot, h.[RecordId] AS handler_id, \
                    h.[PrimaryId] AS primary_id, h.[HandlerType] AS handler_type \
             FROM Notification n LEFT JOIN NotificationHandler h ON h.RecordId = n.HandlerId \
             WHERE n.[Order] > ? AND n.[Type] IN ({}) ORDER BY n.[Order] ASC LIMIT ?",
            placeholders.join(", ")
        );
        let mut params = vec![Value::Int(after)];
        params.extend(kinds.iter().map(|kind| Value::Text(kind.clone())));
        params.push(Value::Int(limit));
        self.db.query(&sql, &params, |row| row.clone())
    }

    /// Extra key/value pairs attached to notifications (`NotificationData`).
    pub fn fetch_data(
        &self,
        ids: &[i64],
    ) -> Result<HashMap<i64, BTreeMap<String, String>>, SqliteError> {
        let mut result: HashMap<i64, BTreeMap<String, String>> = HashMap::new();
        if ids.is_empty() {
            return Ok(result);
        }
        let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
        let sql = format!(
            "SELECT NotificationId AS notification_id, [Key] AS key, Value AS value \
             FROM NotificationData WHERE NotificationId IN ({})",
            placeholders.join(", ")
        );
        let params: Vec<Value> = ids.iter().map(|id| Value::Int(*id)).collect();
        for row in self.db.query(&sql, &params, |row| row.clone())? {
            let id = row.int("notification_id").unwrap_or_default();
            let key = row.required_text("key");
            if !key.is_empty() {
                result
                    .entry(id)
                    .or_default()
                    .insert(key, row.required_text("value"));
            }
        }
        Ok(result)
    }

    /// Application user model id of one notification, by `Notification.[Id]`.
    ///
    /// The `uns` collector needs this when the platform does not report the
    /// application: `UserNotification.AppInfo` answers `E_NOTIMPL` on Windows 11,
    /// while the same notification is in this database with its handler.  The id
    /// is the same in both places, which is what lets the two observations merge
    /// into one journal row instead of producing an unknown application.
    pub fn aumid_for_id(&self, id: i64) -> Result<Option<String>, SqliteError> {
        let row = self.db.query_row(
            "SELECT h.[PrimaryId] AS primary_id FROM Notification n \
             LEFT JOIN NotificationHandler h ON h.RecordId = n.HandlerId \
             WHERE n.[Id] = ?",
            &[Value::Int(id)],
        )?;
        Ok(row
            .and_then(|row| row.optional_text("primary_id"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()))
    }

    /// Signature of the database files, used to wake up as soon as a toast lands.
    pub fn signature(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for suffix in ["", "-wal"] {
            let candidate = PathBuf::from(format!("{}{}", self.path.display(), suffix));
            match std::fs::metadata(&candidate) {
                Ok(metadata) => {
                    let modified = metadata
                        .modified()
                        .ok()
                        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|value| value.as_nanos())
                        .unwrap_or_default();
                    parts.push(format!("{suffix}:{modified}:{}", metadata.len()));
                }
                Err(_) => parts.push(format!("{suffix}:missing")),
            }
        }
        parts.join("|")
    }
}

/// Format a little-endian GUID blob the way `uuid.UUID(bytes_le=...)` does.
pub fn guid_from_le_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 16 || bytes.iter().all(|byte| *byte == 0) {
        return None;
    }
    Some(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[3],
        bytes[2],
        bytes[1],
        bytes[0],
        bytes[5],
        bytes[4],
        bytes[7],
        bytes[6],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

/// Convert one database row into a normalised record.
pub fn map_row(
    row: &sqlite::Row,
    data: Option<&BTreeMap<String, String>>,
    resolver: &NameResolver,
) -> NotificationRecord {
    let payload_text = row.get("payload").and_then(|value| value.as_text());
    let parsed = payload::parse(payload_text.as_deref());
    let aumid = row.optional_text("primary_id");
    let app_name = aumid.as_deref().and_then(|value| resolver.resolve(value));
    let notification_type = row
        .optional_text("notification_type")
        .unwrap_or_else(|| "toast".to_string())
        .to_lowercase();
    let notification_id = row.int("notification_id");
    let order = row.int("order_id");
    let data_map: BTreeMap<String, String> = data.cloned().unwrap_or_default();

    let record = NotificationRecord {
        source: SOURCE.to_string(),
        aumid: aumid.clone(),
        app_name: app_name.clone(),
        notification_type,
        windows_id: notification_id,
        tag: row.optional_text("tag"),
        group: row.optional_text("grp"),
        title: parsed.title.clone(),
        body: parsed.body.clone(),
        extra_text: parsed.extra_text.clone(),
        images: parsed.images.clone(),
        actions: parsed.actions.clone(),
        launch: parsed.launch.clone(),
        activation_type: parsed.activation_type.clone(),
        scenario: parsed.scenario.clone(),
        handler_id: row.int("handler_id"),
        handler_type: row.optional_text("handler_type"),
        activity_id: row.bytes("activity_id").and_then(guid_from_le_bytes),
        payload_xml: payload_text.clone(),
        payload_type: row.optional_text("payload_type"),
        created_utc: row.int("arrival_time").and_then(time::filetime_to_iso),
        expires_utc: row.int("expiry_time").and_then(time::filetime_to_iso),
        extra: json!({
            "order": order,
            "payload_parsed": parsed.parsed,
            "payload_attributes": parsed.attributes,
            "data": data_map,
            "boot_id": row.int("boot_id"),
            "expires_on_reboot": row.int("expires_on_reboot").unwrap_or(0) != 0,
        }),
        raw: json!({
            "order": order,
            "notification_id": notification_id,
            "primary_id": aumid,
            "handler_id": row.int("handler_id"),
            "handler_type": row.optional_text("handler_type"),
            "arrival_time": row.int("arrival_time"),
            "expiry_time": row.int("expiry_time"),
            "boot_id": row.int("boot_id"),
            "payload_type": row.optional_text("payload_type"),
            "payload_length": payload_text.as_deref().map(str::len).unwrap_or(0),
            "notification_data": data_map,
            "parser": "wpndb",
            "collected_utc": time::now_iso(),
        }),
    };
    if let (Some(aumid), Some(name)) = (&record.aumid, &record.app_name) {
        resolver.remember(aumid, name);
    }
    record
}

/// Add the label of a rule to the extra data of a record.
///
/// The extra data may already be an object (from the payload) or nothing at all;
/// the rule gets its own key, so nothing a payload carried is lost.
fn merge_extra(extra: &serde_json::Value, label: &str) -> serde_json::Value {
    let mut object = match extra {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    if let Ok(rule) = serde_json::from_str::<serde_json::Value>(label) {
        object.insert("rule".to_string(), rule);
    }
    serde_json::Value::Object(object)
}

/// Result of one scan of the notification database.
#[derive(Debug, Default, Clone)]
pub struct ScanOutcome {
    pub emitted: usize,
    pub batches: usize,
    pub cursor: Option<i64>,
    /// True when the scan stopped at the batch limit (more rows are pending).
    pub saturated: bool,
    /// How many notifications the rules kept out of the journal.
    pub skipped: i64,
    /// Pop-ups the rules asked for (the sequence number is given by the service,
    /// which knows which of them the window has already shown).
    pub alerts: Vec<crate::rules::Alert>,
}

/// Read new notifications and store them into the journal (one scan).
///
/// On the very first run the whole notification center content is imported
/// (`backfill_on_first_run`); afterwards only rows above the stored cursor are
/// read.  A recreated database (cursor above the maximum) restarts the scan -
/// duplicates cannot appear because of the unique de-duplication key.
pub fn scan_new(
    database: &NotificationDatabase,
    journal: &Journal,
    kinds: &[String],
    backfill_on_first_run: bool,
    max_backfill_records: i64,
    batch_limit: i64,
    duplicate_window_seconds: i64,
    resolver: &NameResolver,
    rules: &crate::rules::RuleSet,
) -> Result<ScanOutcome, SqliteError> {
    let stored_cursor = journal.source_cursor(SOURCE)?;
    let first_pass = stored_cursor
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty();
    let mut cursor = stored_cursor
        .as_deref()
        .and_then(|text| text.parse::<i64>().ok())
        .unwrap_or(0);

    let highest = database.max_order()?;
    if cursor > highest {
        eprintln!(
            "[wpndb] the notification database was reset (cursor {cursor} > max {highest}); rescanning"
        );
        cursor = 0;
        journal.touch_source(SOURCE, Some("0"), 0, false)?;
    }
    if first_pass && !backfill_on_first_run {
        journal.touch_source(SOURCE, Some(&highest.to_string()), 0, true)?;
        return Ok(ScanOutcome {
            emitted: 0,
            batches: 0,
            cursor: Some(highest),
            saturated: false,
            skipped: 0,
            alerts: Vec::new(),
        });
    }

    let limit = if first_pass {
        batch_limit.max(max_backfill_records)
    } else {
        batch_limit
    };
    let mut outcome = ScanOutcome {
        cursor: Some(cursor),
        ..Default::default()
    };
    let started_cursor = cursor;
    for _ in 0..20 {
        let rows = database.fetch(cursor, kinds, limit)?;
        if rows.is_empty() {
            break;
        }
        outcome.batches += 1;
        let ids: Vec<i64> = rows
            .iter()
            .filter_map(|row| row.int("notification_id"))
            .collect();
        let data = database.fetch_data(&ids)?;
        // The rules decide what happens with each notification before anything is
        // written: a skipped one must not reach the journal at all, and a rule
        // that asked for a pop-up is remembered here and shown by the window.
        let mapped: Vec<NotificationRecord> = rows
            .iter()
            .map(|row| {
                let id = row.int("notification_id").unwrap_or_default();
                map_row(row, data.get(&id), resolver)
            })
            .collect();
        let mut records: Vec<NotificationRecord> = Vec::with_capacity(mapped.len());
        for mut record in mapped {
            let decision = rules.evaluate(&record);
            if decision.is_skipped() {
                outcome.skipped += 1;
                continue;
            }
            if let Some(extra) = crate::rules::label_extra(&decision) {
                // Both places the Python implementation writes: the readable extra
                // data and the raw record, so a decision is visible to both.
                record.extra = merge_extra(&record.extra, &extra);
                record.raw = merge_extra(&record.raw, &extra);
            }
            if decision.alert {
                outcome.alerts.push(crate::rules::Alert {
                    seq: 0,
                    application: record.display_app(),
                    title: record.title.clone().unwrap_or_default(),
                    body: record.body.clone().unwrap_or_default(),
                    label: decision.label.clone(),
                });
            }
            records.push(record);
        }
        if records.is_empty() {
            if let Some(last) = rows.last().and_then(|row| row.int("order_id")) {
                cursor = last;
            }
            outcome.cursor = Some(cursor);
            continue;
        }
        let counts = journal.record_many(&records, duplicate_window_seconds)?;
        outcome.emitted += counts.total();
        if let Some(last) = rows.last().and_then(|row| row.int("order_id")) {
            cursor = last;
        }
        outcome.cursor = Some(cursor);
        journal.touch_source(
            SOURCE,
            Some(&cursor.to_string()),
            records.len() as i64,
            false,
        )?;
        if (rows.len() as i64) < limit {
            break;
        }
        outcome.saturated = true;
    }

    // A moved cursor has to be remembered (a pass whose rows the rules skipped
    // still advances it); an idle pass, on the other hand, must not touch the
    // journal at all - it used to rewrite the run counter on every pass.
    if cursor != started_cursor {
        journal.touch_source(SOURCE, Some(&cursor.to_string()), 0, true)?;
    }
    // Only names learned since the last write: the cache was seeded from the
    // journal, so everything else is already stored.
    let pending = resolver.pending();
    if !pending.is_empty() {
        for (aumid, name) in &pending {
            journal.remember_app_name(aumid, name, SOURCE)?;
        }
        resolver.forget_pending();
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::Value;

    #[test]
    fn guid_matches_the_python_uuid_format() {
        let bytes: Vec<u8> = (1u8..=16).collect();
        assert_eq!(
            guid_from_le_bytes(&bytes).as_deref(),
            Some("04030201-0605-0807-090a-0b0c0d0e0f10")
        );
        assert_eq!(guid_from_le_bytes(&[0u8; 16]), None);
        assert_eq!(guid_from_le_bytes(&[1u8, 2, 3]), None);
    }

    fn sample_row() -> sqlite::Row {
        sqlite::Row::from_pairs(
            vec![
                "order_id",
                "notification_id",
                "notification_type",
                "payload",
                "payload_type",
                "tag",
                "grp",
                "activity_id",
                "arrival_time",
                "expiry_time",
                "boot_id",
                "expires_on_reboot",
                "handler_id",
                "primary_id",
                "handler_type",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            vec![
                Value::Int(12055),
                Value::Int(18127),
                Value::Text("toast".to_string()),
                Value::Text(
                    "<toast activationType=\"foreground\" launch=\"app:test\"><visual>\
                     <binding template=\"ToastGeneric\"><text>Alice</text><text>Hello there</text>\
                     </binding></visual></toast>"
                        .to_string(),
                ),
                Value::Text("Xml".to_string()),
                Value::Text("my-tag".to_string()),
                Value::Text(String::new()),
                Value::Blob((1u8..=16).collect()),
                Value::Int(133_500_000_000_000_000),
                Value::Int(133_500_600_000_000_000),
                Value::Int(7),
                Value::Int(0),
                Value::Int(10),
                Value::Text("Vendor.Chat_123!App".to_string()),
                Value::Text("app:desktop".to_string()),
            ],
        )
    }

    #[test]
    fn mapping_produces_a_complete_record() {
        let resolver = NameResolver::new();
        resolver.remember("Vendor.Chat_123!App", "Vendor Chat");
        let record = map_row(&sample_row(), None, &resolver);

        assert_eq!(record.source, "wpndb");
        assert_eq!(record.aumid.as_deref(), Some("Vendor.Chat_123!App"));
        assert_eq!(record.app_name.as_deref(), Some("Vendor Chat"));
        assert_eq!(record.windows_id, Some(18127));
        assert_eq!(record.title.as_deref(), Some("Alice"));
        assert_eq!(record.body.as_deref(), Some("Hello there"));
        assert_eq!(record.tag.as_deref(), Some("my-tag"));
        assert_eq!(record.group, None, "an empty group must become None");
        assert_eq!(record.launch.as_deref(), Some("app:test"));
        assert_eq!(record.activation_type.as_deref(), Some("foreground"));
        assert_eq!(record.handler_type.as_deref(), Some("app:desktop"));
        assert_eq!(
            record.activity_id.as_deref(),
            Some("04030201-0605-0807-090a-0b0c0d0e0f10")
        );
        assert_eq!(
            record.created_utc.as_deref(),
            crate::time::filetime_to_iso(133_500_000_000_000_000).as_deref()
        );
        assert_eq!(record.dedup_key(), "Vendor.Chat_123!App|toast|18127");
        assert_eq!(record.extra["order"], serde_json::json!(12055));
        assert_eq!(record.extra["payload_parsed"], serde_json::json!(true));
        assert_eq!(record.raw["parser"], serde_json::json!("wpndb"));
    }

    #[test]
    fn mapping_survives_an_empty_payload_and_missing_handler() {
        let resolver = NameResolver::new();
        let row = sqlite::Row::from_pairs(
            vec!["order_id", "notification_id", "payload", "primary_id"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            vec![Value::Int(1), Value::Int(2), Value::Null, Value::Null],
        );
        let record = map_row(&row, None, &resolver);
        assert_eq!(record.title, None);
        assert_eq!(record.aumid, None);
        assert_eq!(record.windows_id, Some(2));
        assert!(record.dedup_key().starts_with("?|toast|2"));
        assert_eq!(record.extra["payload_parsed"], serde_json::json!(false));
    }

    #[test]
    fn database_path_points_into_the_user_profile() {
        let path = database_path();
        assert!(path.ends_with("wpndatabase.db"));
        assert!(path.to_string_lossy().contains("Notifications"));
    }
}
