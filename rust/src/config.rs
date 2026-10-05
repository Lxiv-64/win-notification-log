//! Configuration and directory layout (same files as the Python version).
//!
//! Only the options the native collector currently needs are read; everything
//! else stays untouched in `config.json`, so both implementations can run
//! against the same data directory without clobbering each other's settings.

use std::path::{Path, PathBuf};

use serde_json::Value as Json;

pub const DEFAULT_CAPTURE_KINDS: [&str; 1] = ["toast"];

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub database: PathBuf,
    pub capture_types: Vec<String>,
    pub backfill_on_first_run: bool,
    pub max_backfill_records: i64,
    pub duplicate_window_seconds: i64,
    pub poll_seconds: f64,
}

impl Config {
    /// Data directory: `--home`, then `WINNOTIFYLOG_HOME`, then `%LOCALAPPDATA%`.
    pub fn resolve_home(home_override: Option<&str>) -> PathBuf {
        if let Some(value) = home_override.filter(|value| !value.trim().is_empty()) {
            return PathBuf::from(value);
        }
        if let Some(value) = std::env::var_os("WINNOTIFYLOG_HOME") {
            return PathBuf::from(value);
        }
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        local.join("WinNotificationLog")
    }

    /// Load the configuration, falling back to defaults for anything missing.
    pub fn load(home_override: Option<&str>) -> Config {
        let home = Self::resolve_home(home_override);
        let mut config = Config {
            home: home.clone(),
            database: home.join("history.db"),
            capture_types: DEFAULT_CAPTURE_KINDS
                .iter()
                .map(|kind| kind.to_string())
                .collect(),
            backfill_on_first_run: true,
            max_backfill_records: 5000,
            duplicate_window_seconds: 0,
            poll_seconds: 2.0,
        };

        let file = home.join("config.json");
        let Ok(text) = std::fs::read_to_string(&file) else {
            return config;
        };
        let Ok(json) = serde_json::from_str::<Json>(&text) else {
            return config;
        };

        if let Some(path) = json
            .get("storage")
            .and_then(|storage| storage.get("database"))
            .and_then(|value| value.as_str())
        {
            config.database = PathBuf::from(path);
        }
        if let Some(kinds) = json
            .get("wpndb")
            .and_then(|section| section.get("capture_types"))
            .and_then(|value| value.as_array())
        {
            let types: Vec<String> = kinds
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::to_string)
                .collect();
            if !types.is_empty() {
                config.capture_types = types;
            }
        }
        if let Some(value) = json
            .get("wpndb")
            .and_then(|section| section.get("backfill_on_first_run"))
            .and_then(|value| value.as_bool())
        {
            config.backfill_on_first_run = value;
        }
        if let Some(value) = json
            .get("wpndb")
            .and_then(|section| section.get("max_backfill_records"))
            .and_then(|value| value.as_i64())
        {
            config.max_backfill_records = value;
        }
        if let Some(value) = json
            .get("wpndb")
            .and_then(|section| section.get("poll_seconds"))
            .and_then(|value| value.as_f64())
        {
            config.poll_seconds = value;
        }
        if let Some(value) = json
            .get("dedupe")
            .and_then(|section| section.get("identical_content_window_seconds"))
            .and_then(|value| value.as_i64())
        {
            config.duplicate_window_seconds = value;
        }
        config
    }

    /// Journal path used by the CLI (`--database` overrides the configuration).
    pub fn journal_path(&self, override_path: Option<&Path>) -> PathBuf {
        match override_path {
            Some(path) => path.to_path_buf(),
            None => self.database.clone(),
        }
    }

    /// Effective collection parameters for a resolved data directory.
    ///
    /// This is what both binaries use now: the folder comes from `storage`, the
    /// numbers come from `settings.json`, and nothing depends on the legacy
    /// `config.json` any more (it is only read by `load` for compatibility).
    pub fn from_settings(
        settings: &crate::settings::Settings,
        layout: &crate::storage::Layout,
    ) -> Config {
        Config {
            home: layout.data_dir.clone(),
            database: layout.journal.clone(),
            capture_types: DEFAULT_CAPTURE_KINDS
                .iter()
                .map(|kind| kind.to_string())
                .collect(),
            backfill_on_first_run: true,
            max_backfill_records: settings.backfill_limit,
            duplicate_window_seconds: 0,
            poll_seconds: settings.poll_seconds,
        }
    }
}
