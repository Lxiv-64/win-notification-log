//! The normalised notification record, shared by every native collector.
//!
//! Field names and semantics match the Python implementation, so both write and
//! read the same journal rows.  Two identity helpers matter for interoperability:
//!
//! * [`NotificationRecord::dedup_key`] has an *identical* format in both
//!   implementations (`aumid|type|windows id`, or `sha1:` + content hash when
//!   Windows exposes no id) - this is what makes the shared journal de-duplicate
//!   correctly no matter which implementation captured the notification;
//! * [`NotificationRecord::content_hash`] is deliberately treated as a local
//!   implementation detail: the hash input is a canonical JSON document and
//!   different JSON writers may choose different whitespace.  De-duplication
//!   therefore never relies on hash equality across implementations - both sides
//!   additionally compare the individual content fields (`store::content_matches`).

use std::collections::BTreeSet;

use serde_json::{json, Value as Json};
use sha1::Digest as _;

/// What happened to a record while it was stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    Inserted,
    /// A re-delivery of an observation whose content hash is identical.
    Duplicate,
    /// Every provided field already matched: the notification was seen again.
    Repeated,
    /// The same notification arrived with new content (same Windows id).
    Updated,
    /// Another collector observed the notification (the source list grew).
    Merged,
}

impl IngestOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            IngestOutcome::Inserted => "inserted",
            IngestOutcome::Duplicate => "duplicate",
            IngestOutcome::Repeated => "repeated",
            IngestOutcome::Updated => "updated",
            IngestOutcome::Merged => "merged",
        }
    }
}

/// A single notification observed by a collector.
#[derive(Debug, Clone)]
pub struct NotificationRecord {
    pub source: String,
    pub aumid: Option<String>,
    pub app_name: Option<String>,
    pub notification_type: String,
    pub windows_id: Option<i64>,
    pub tag: Option<String>,
    pub group: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub extra_text: Vec<String>,
    pub images: Vec<String>,
    pub actions: Vec<Json>,
    pub launch: Option<String>,
    pub activation_type: Option<String>,
    pub scenario: Option<String>,
    pub handler_id: Option<i64>,
    pub handler_type: Option<String>,
    pub activity_id: Option<String>,
    pub payload_xml: Option<String>,
    pub payload_type: Option<String>,
    pub created_utc: Option<String>,
    pub expires_utc: Option<String>,
    pub extra: Json,
    pub raw: Json,
}

impl Default for NotificationRecord {
    fn default() -> Self {
        NotificationRecord {
            source: String::new(),
            aumid: None,
            app_name: None,
            notification_type: "toast".to_string(),
            windows_id: None,
            tag: None,
            group: None,
            title: None,
            body: None,
            extra_text: Vec::new(),
            images: Vec::new(),
            actions: Vec::new(),
            launch: None,
            activation_type: None,
            scenario: None,
            handler_id: None,
            handler_type: None,
            activity_id: None,
            payload_xml: None,
            payload_type: None,
            created_utc: None,
            expires_utc: None,
            extra: Json::Null,
            raw: Json::Null,
        }
    }
}

impl NotificationRecord {
    pub fn new(source: impl Into<String>) -> Self {
        NotificationRecord {
            source: source.into(),
            ..Default::default()
        }
    }

    /// Identity of the notification as exposed by Windows.
    ///
    /// The format is identical to the Python implementation, which is what lets
    /// both share one journal without producing duplicate rows.
    pub fn dedup_key(&self) -> String {
        if let Some(id) = self.windows_id {
            return format!(
                "{}|{}|{}",
                self.aumid.as_deref().unwrap_or("?"),
                self.notification_type,
                id
            );
        }
        let material = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.aumid.as_deref().unwrap_or("?"),
            self.notification_type,
            self.title.as_deref().unwrap_or(""),
            self.body.as_deref().unwrap_or(""),
            self.extra_text.join(" "),
            self.tag.as_deref().unwrap_or(""),
            self.group.as_deref().unwrap_or(""),
            self.created_utc.as_deref().unwrap_or(""),
        );
        let mut hasher = sha1::Sha1::new();
        hasher.update(material.as_bytes());
        format!("sha1:{}", hex(&hasher.finalize()))
    }

    /// Hash of the user visible content (used to notice in-place updates).
    pub fn content_hash(&self) -> String {
        let document = json!({
            "app": self.aumid.clone().or_else(|| self.app_name.clone()),
            "type": self.notification_type,
            "title": self.title,
            "body": self.body,
            "extra_text": self.extra_text,
            "images": self.images,
            "actions": self.actions,
            "launch": self.launch,
            "scenario": self.scenario,
            "tag": self.tag,
            "group": self.group,
        });
        let digest = sha2::Sha256::digest(document.to_string().as_bytes());
        hex(&digest)
    }

    /// The collectors that observed this record.
    pub fn sources(&self) -> BTreeSet<String> {
        self.source
            .split('+')
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Human readable application label for lists and filters.
    pub fn display_app(&self) -> String {
        match (&self.app_name, &self.aumid) {
            (Some(name), Some(aumid)) => {
                if name.to_lowercase().contains(&aumid.to_lowercase()) {
                    name.clone()
                } else {
                    format!("{name} ({aumid})")
                }
            }
            (Some(name), None) => name.clone(),
            (None, Some(aumid)) => aumid.clone(),
            (None, None) => "(unknown application)".to_string(),
        }
    }

    /// All user visible text of this notification (used by rules and search).
    pub fn searchable_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let candidates = [
            self.app_name.as_deref(),
            self.aumid.as_deref(),
            self.title.as_deref(),
            self.body.as_deref(),
            self.tag.as_deref(),
            self.group.as_deref(),
            self.launch.as_deref(),
        ];
        for value in candidates.into_iter().flatten() {
            parts.push(value.to_string());
        }
        if !self.extra_text.is_empty() {
            parts.push(self.extra_text.join(" "));
        }
        parts.join("\n")
    }

    /// Merge two observations of the same notification: newer non-empty fields
    /// win, the collector list is united.
    pub fn merged_with(&self, other: &NotificationRecord) -> NotificationRecord {
        let (older, newer) = match (&self.created_utc, &other.created_utc) {
            (Some(left), Some(right)) if right < left => (other, self),
            _ => (self, other),
        };
        let mut result = older.clone();
        let sources: BTreeSet<String> = older.sources().union(&newer.sources()).cloned().collect();
        result.source = sources.into_iter().collect::<Vec<_>>().join("+");

        if newer.aumid.is_some() {
            result.aumid = newer.aumid.clone();
        }
        if newer.app_name.is_some() {
            result.app_name = newer.app_name.clone();
        }
        if !newer.notification_type.is_empty() {
            result.notification_type = newer.notification_type.clone();
        }
        if newer.windows_id.is_some() {
            result.windows_id = newer.windows_id;
        }
        if newer.handler_id.is_some() {
            result.handler_id = newer.handler_id;
        }
        copy_if_some(&mut result.tag, &newer.tag);
        copy_if_some(&mut result.group, &newer.group);
        copy_if_some(&mut result.title, &newer.title);
        copy_if_some(&mut result.body, &newer.body);
        copy_if_some(&mut result.launch, &newer.launch);
        copy_if_some(&mut result.activation_type, &newer.activation_type);
        copy_if_some(&mut result.scenario, &newer.scenario);
        copy_if_some(&mut result.handler_type, &newer.handler_type);
        copy_if_some(&mut result.activity_id, &newer.activity_id);
        copy_if_some(&mut result.payload_xml, &newer.payload_xml);
        copy_if_some(&mut result.payload_type, &newer.payload_type);
        copy_if_some(&mut result.created_utc, &newer.created_utc);
        copy_if_some(&mut result.expires_utc, &newer.expires_utc);
        if !newer.extra_text.is_empty() {
            result.extra_text = newer.extra_text.clone();
        }
        if !newer.images.is_empty() {
            result.images = newer.images.clone();
        }
        if !newer.actions.is_empty() {
            result.actions = newer.actions.clone();
        }
        if !newer.extra.is_null() {
            result.extra = newer.extra.clone();
        }
        if !newer.raw.is_null() {
            result.raw = newer.raw.clone();
        }
        result
    }
}

fn copy_if_some(target: &mut Option<String>, source: &Option<String>) {
    if source.is_some() {
        *target = source.clone();
    }
}

/// Lowercase hexadecimal representation of a digest.
fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> NotificationRecord {
        NotificationRecord {
            source: "wpndb".to_string(),
            aumid: Some("Vendor.Chat_123!App".to_string()),
            windows_id: Some(42),
            title: Some("Alice".to_string()),
            body: Some("Hello".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn dedup_key_matches_the_python_format() {
        assert_eq!(record().dedup_key(), "Vendor.Chat_123!App|toast|42");
    }

    #[test]
    fn dedup_key_is_source_independent() {
        let database = record();
        let api = NotificationRecord {
            source: "uns".to_string(),
            ..record()
        };
        assert_eq!(database.dedup_key(), api.dedup_key());
    }

    #[test]
    fn dedup_key_falls_back_to_sha1() {
        let subject = NotificationRecord {
            source: "wpndb".to_string(),
            aumid: Some("App".to_string()),
            title: Some("hello".to_string()),
            body: Some("world".to_string()),
            ..Default::default()
        };
        let key = subject.dedup_key();
        assert!(key.starts_with("sha1:"), "unexpected key: {key}");
        assert_eq!(key.len(), 5 + 40);
        let other_source = NotificationRecord {
            source: "uns".to_string(),
            ..subject.clone()
        };
        assert_eq!(other_source.dedup_key(), key);
    }

    #[test]
    fn content_hash_changes_with_content() {
        let first = record();
        let second = NotificationRecord {
            title: Some("Bob".to_string()),
            ..record()
        };
        assert_ne!(first.content_hash(), second.content_hash());
        assert_eq!(first.content_hash(), record().content_hash());
    }

    #[test]
    fn merge_unites_sources_and_fills_gaps() {
        let database = NotificationRecord {
            source: "wpndb".to_string(),
            aumid: Some("App".to_string()),
            windows_id: Some(7),
            title: Some("Title".to_string()),
            payload_xml: Some("<toast/>".to_string()),
            tag: Some("tag".to_string()),
            ..Default::default()
        };
        let api = NotificationRecord {
            source: "uns".to_string(),
            aumid: Some("App".to_string()),
            windows_id: Some(7),
            app_name: Some("App name".to_string()),
            ..Default::default()
        };
        let merged = database.merged_with(&api);
        assert_eq!(merged.source, "uns+wpndb");
        assert_eq!(merged.title.as_deref(), Some("Title"));
        assert_eq!(merged.payload_xml.as_deref(), Some("<toast/>"));
        assert_eq!(merged.app_name.as_deref(), Some("App name"));
    }

    #[test]
    fn merge_is_idempotent_for_sources() {
        let merged = NotificationRecord {
            source: "uns+wpndb".to_string(),
            aumid: Some("App".to_string()),
            windows_id: Some(1),
            title: Some("T".to_string()),
            ..Default::default()
        };
        let api = NotificationRecord {
            source: "uns".to_string(),
            aumid: Some("App".to_string()),
            windows_id: Some(1),
            title: Some("T".to_string()),
            ..Default::default()
        };
        assert_eq!(merged.merged_with(&api).source, "uns+wpndb");
    }

    #[test]
    fn display_app_combines_name_and_aumid() {
        let record = NotificationRecord {
            source: "uns".to_string(),
            aumid: Some("Vendor.App_123!App".to_string()),
            app_name: Some("Vendor App".to_string()),
            ..Default::default()
        };
        assert_eq!(record.display_app(), "Vendor App (Vendor.App_123!App)");
        assert_eq!(
            NotificationRecord::default().display_app(),
            "(unknown application)"
        );
    }

    #[test]
    fn searchable_text_contains_every_visible_field() {
        let subject = NotificationRecord {
            extra_text: vec!["extra".to_string()],
            ..record()
        };
        let text = subject.searchable_text();
        for expected in ["Vendor.Chat_123!App", "Alice", "Hello", "extra"] {
            assert!(text.contains(expected), "{text} lacks {expected}");
        }
    }

    #[test]
    fn outcomes_have_stable_names() {
        assert_eq!(IngestOutcome::Repeated.as_str(), "repeated");
        assert_eq!(IngestOutcome::Merged.as_str(), "merged");
    }
}
