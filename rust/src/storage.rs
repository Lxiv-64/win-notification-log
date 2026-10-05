//! Where the application keeps its data.
//!
//! The rule is deliberately simple and predictable, because "где программа
//! хранит данные" must not be a mystery:
//!
//! 1. an explicitly chosen folder (settings window, `--home`, `WINNOTIFYLOG_HOME`)
//!    wins;
//! 2. otherwise `<folder of the executable>\data` - portable: the journal travels
//!    with the program and the folder is easy to find;
//! 3. otherwise `%LOCALAPPDATA%\WinNotificationLog` - the safe place for a copy
//!    installed under `C:\Program Files`, where the folder next to the exe is
//!    read-only for a normal user.
//!
//! `settings.json` follows the same logic but never rule 1: it must be readable
//! before the data folder is known, so it lives next to the executable (or in
//! `%LOCALAPPDATA%` when that folder is not writable).

use std::path::{Path, PathBuf};

use crate::settings::{Settings, FILE_NAME as SETTINGS_FILE};

/// Folder name used under `%LOCALAPPDATA%`.
pub const DIRECTORY_NAME: &str = "WinNotificationLog";
/// Data folder created next to the executable.
pub const PORTABLE_DIRECTORY: &str = "data";
/// Journal, export and log names inside the data directory.
pub const JOURNAL_FILE: &str = "history.db";
pub const EXPORT_DIRECTORY: &str = "exports";
/// Copies of the pictures notifications were sent with.
pub const PICTURES_DIRECTORY: &str = "pictures";
pub const LOG_FILE: &str = "wnl.log";
pub const STATUS_FILE: &str = "status.json";

/// The resolved directory layout of one run.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// `settings.json` (next to the executable when possible).
    pub settings_path: PathBuf,
    /// Data directory actually in use.
    pub data_dir: PathBuf,
    /// `history.db`.
    pub journal: PathBuf,
    /// Folder for exported files.
    pub exports: PathBuf,
    /// Folder with the pictures the notifications were sent with.
    pub pictures: PathBuf,
    /// Collector log.
    pub log: PathBuf,
    /// Machine readable state of the background collector.
    pub status: PathBuf,
    /// True when the data directory sits next to the executable.
    pub portable: bool,
    /// True when the requested directory had to be replaced by a fallback.
    pub fallback_used: bool,
}

impl Layout {
    /// Path of the settings file for a given executable folder.
    pub fn settings_path(exe_dir: &Path, local_app_data: Option<&Path>) -> PathBuf {
        if directory_is_writable(exe_dir) {
            return exe_dir.join(SETTINGS_FILE);
        }
        match local_app_data {
            Some(base) => base.join(DIRECTORY_NAME).join(SETTINGS_FILE),
            None => exe_dir.join(SETTINGS_FILE),
        }
    }

    /// Build the layout for a data directory.
    pub fn build(
        data_dir: &Path,
        settings_path: PathBuf,
        portable: bool,
        fallback: bool,
    ) -> Layout {
        Layout {
            settings_path,
            data_dir: data_dir.to_path_buf(),
            journal: data_dir.join(JOURNAL_FILE),
            exports: data_dir.join(EXPORT_DIRECTORY),
            pictures: data_dir.join(PICTURES_DIRECTORY),
            log: data_dir.join(LOG_FILE),
            status: data_dir.join(STATUS_FILE),
            portable,
            fallback_used: fallback,
        }
    }
}

/// True when a file can be created in `directory` (creating it when needed).
pub fn directory_is_writable(directory: &Path) -> bool {
    if std::fs::create_dir_all(directory).is_err() {
        return false;
    }
    let probe = directory.join(".wnl-write-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Decide the directories for a run.
///
/// `local_app_data` is passed in instead of being read from the environment so
/// that the decision is testable without touching the real profile.
pub fn resolve(settings: &Settings, exe_dir: &Path, local_app_data: Option<&Path>) -> Layout {
    let settings_path = Layout::settings_path(exe_dir, local_app_data);
    let portable_dir = exe_dir.join(PORTABLE_DIRECTORY);
    let fallback_dir = match local_app_data {
        Some(base) => base.join(DIRECTORY_NAME),
        None => portable_dir.clone(),
    };

    if let Some(requested) = &settings.data_dir {
        if directory_is_writable(requested) {
            let inside_exe_dir = requested.starts_with(exe_dir);
            return Layout::build(requested, settings_path, inside_exe_dir, false);
        }
    }
    if directory_is_writable(&portable_dir) {
        return Layout::build(
            &portable_dir,
            settings_path,
            true,
            settings.data_dir.is_some(),
        );
    }
    if directory_is_writable(&fallback_dir) {
        return Layout::build(
            &fallback_dir,
            settings_path,
            false,
            settings.data_dir.is_some(),
        );
    }
    // Nothing is writable: keep the requested location so the message points at
    // what the user asked for instead of a silent substitute.
    match &settings.data_dir {
        Some(requested) => Layout::build(requested, settings_path, false, false),
        None => Layout::build(&fallback_dir, settings_path, false, false),
    }
}

/// Human readable one line description used by the window and the diagnostics.
pub fn describe(layout: &Layout) -> String {
    let kind = if layout.portable {
        tr!("рядом с программой")
    } else {
        tr!("в профиле пользователя")
    };
    if layout.fallback_used {
        tf!(
            "{} — выбранная папка недоступна, данные сохраняются {kind}",
            "{} — the chosen folder is unavailable, data is kept {kind}",
            layout.data_dir.display()
        )
    } else {
        tf!("{} ({kind})", "{} ({kind})", layout.data_dir.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("wnl-storage-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch directory");
        directory
    }

    #[test]
    fn portable_folder_wins_by_default() {
        let exe = scratch("portable");
        let profile = exe.join("profile");
        let layout = resolve(&Settings::default(), &exe, Some(&profile));
        assert_eq!(layout.data_dir, exe.join(PORTABLE_DIRECTORY));
        assert!(layout.portable);
        assert!(!layout.fallback_used);
        assert_eq!(
            layout.journal,
            exe.join(PORTABLE_DIRECTORY).join(JOURNAL_FILE)
        );
        assert_eq!(layout.settings_path, exe.join(SETTINGS_FILE));
    }

    #[test]
    fn chosen_folder_is_used_verbatim() {
        let exe = scratch("chosen-exe");
        let chosen = scratch("chosen-data");
        let mut settings = Settings::default();
        settings.data_dir = Some(chosen.clone());
        let layout = resolve(&settings, &exe, Some(&exe.join("profile")));
        assert_eq!(layout.data_dir, chosen);
        assert!(!layout.portable);
        assert!(!layout.fallback_used);
    }

    #[test]
    fn unwritable_folder_falls_back_to_the_default() {
        let exe = scratch("blocked");
        // A file where a directory is expected: every write attempt fails.
        let blocker = exe.join("blocker");
        std::fs::write(&blocker, b"not a directory").expect("written");
        let mut settings = Settings::default();
        settings.data_dir = Some(blocker.join("under-a-file"));
        let layout = resolve(&settings, &exe, Some(&exe.join("profile")));
        // The chosen folder is unusable, so the portable default next to the
        // executable takes over - and the window says so.
        assert_eq!(layout.data_dir, exe.join(PORTABLE_DIRECTORY));
        assert!(layout.portable);
        assert!(layout.fallback_used);
        assert!(describe(&layout).contains("недоступна"));
        // The description names the folder that is really in use.
        assert!(describe(&layout).contains(&layout.data_dir.display().to_string()));
    }

    #[test]
    fn read_only_installation_uses_the_profile() {
        let parent = scratch("readonly");
        let blocker = parent.join("installation");
        std::fs::write(&blocker, b"not a directory").expect("written");
        let profile = parent.join("profile");
        // The "executable folder" is not a directory at all, so nothing can be
        // created next to it.
        let layout = resolve(&Settings::default(), &blocker, Some(&profile));
        assert_eq!(layout.data_dir, profile.join(DIRECTORY_NAME));
        assert!(!layout.portable);
        assert_eq!(
            layout.settings_path,
            profile.join(DIRECTORY_NAME).join(SETTINGS_FILE)
        );
    }

    #[test]
    fn probe_reports_writability_honestly() {
        let directory = scratch("probe");
        assert!(directory_is_writable(&directory));
        let blocker = directory.join("file");
        std::fs::write(&blocker, b"x").expect("written");
        assert!(!directory_is_writable(&blocker));
    }
}
