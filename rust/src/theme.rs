//! Modern look: common controls v6 and DPI awareness through a manifest.
//!
//! Windows decides how buttons, lists and scroll bars look *before* the first
//! window exists, and the decision is driven by an application manifest.  The
//! manifest now travels **inside** the executable: `build.rs` embeds
//! `wnl.manifest` as the resource `RT_MANIFEST`, so a normal build needs nothing
//! else - and no file appears next to the journal.
//!
//! The file-based path is kept as the fallback for an executable without that
//! resource (an older build, or one made without `WNL.ico`/`wnl.manifest`): the
//! manifest is then written next to the journal and activated at start-up with
//! `CreateActCtxW`, which is what this module did before.

use std::path::{Path, PathBuf};

use crate::win32;

/// File name of the generated manifest.
pub const MANIFEST_FILE: &str = "wnl.exe.manifest";
/// Identifier of the embedded manifest resource (`build.rs`).
const MANIFEST_RESOURCE_ID: u32 = 1;
/// `RT_MANIFEST`.
const RT_MANIFEST: u32 = 24;

/// The manifest itself: common controls 6.0, per monitor DPI, long paths.
///
/// It lives in `wnl.manifest` because both the build script (which embeds it)
/// and this module (which writes it as the fallback) need the very same text.
pub const MANIFEST: &str = include_str!("../wnl.manifest");

/// True when the executable carries the manifest itself.
///
/// Windows has then already applied it before the program started; writing or
/// activating a copy would only leave a file behind.
pub fn is_embedded() -> bool {
    let resource = unsafe {
        win32::FindResourceW(
            win32::module_handle(),
            win32::int_resource(MANIFEST_RESOURCE_ID),
            win32::int_resource(RT_MANIFEST),
        )
    };
    !resource.is_null()
}

/// Where the manifest is kept for a given data directory.
pub fn manifest_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MANIFEST_FILE)
}

/// Write the manifest when it is missing or outdated.
pub fn ensure(path: &Path) -> std::io::Result<()> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    if current == MANIFEST {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, MANIFEST)
}

/// Activate the visual style; returns false when the manifest is unavailable.
///
/// With the manifest inside the executable this is a no-op that reports success;
/// otherwise the file next to the journal is (re)written and activated.
pub fn activate(data_dir: &Path) -> bool {
    if is_embedded() {
        return true;
    }
    let path = manifest_path(data_dir);
    if ensure(&path).is_err() {
        return false;
    }
    win32::activate_manifest(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_the_modern_controls() {
        assert!(MANIFEST.contains("Microsoft.Windows.Common-Controls"));
        assert!(MANIFEST.contains("version=\"6.0.0.0\""));
        assert!(MANIFEST.contains("permonitorv2"));

        let directory = std::env::temp_dir().join("wnl-theme-test");
        let _ = std::fs::remove_dir_all(&directory);
        let path = manifest_path(&directory);
        ensure(&path).expect("written");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), MANIFEST);
        let stamp = std::fs::metadata(&path).expect("metadata").len();
        ensure(&path).expect("second call");
        assert_eq!(std::fs::metadata(&path).expect("metadata").len(), stamp);
    }
}
