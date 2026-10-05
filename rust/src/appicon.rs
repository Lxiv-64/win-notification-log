//! Icons of the applications that send notifications.
//!
//! Windows stores no icon in the notification database, so the icon is looked up
//! in this order:
//!
//! 1. `IconUri` of the application identifier in the registry (user, machine and
//!    the merged classes view; resolved through `SHLoadIndirectString`, which
//!    understands `@{dll?resource}`, together with the icon index behind the last
//!    comma, which is what actually picks the picture);
//! 2. the icon the manifest of an installed *package* declares
//!    (`packages::icon_asset`, a PNG that GDI+ decodes) - a packaged application
//!    registers nothing in the registry, so without this step every Store app
//!    ended up with the same default icon;
//! 3. the icon the shell shows for the application in the start menu
//!    (`shell:AppsFolder\<aumid>`) - the one place that answers for a packaged
//!    application *and* for a program that registered an identifier;
//! 4. the executable of a *running* process that reports the same application
//!    identifier - notifications almost always come from a running application;
//! 5. the picture the notification itself carried (`appLogoOverride`, see
//!    [`icon_for_notification`]) - a browser registers a random identifier and no
//!    icon at all, and a script that posts a toast has no icon to register, so for
//!    those the logo inside the notification is the only picture there is;
//! 6. the default application icon.
//!
//! Everything is cached: the list view asks for a small image index, not for a
//! handle, so scrolling never touches the registry or the process list.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::win32::{self, HICON, HIMAGELIST, LPARAM, LVSIL_SMALL, WPARAM};
use crate::{appname, imaging, packages};

/// How long the process table is reused before it is read again.
pub const PROCESS_CACHE_TTL: Duration = Duration::from_secs(30);

/// Size of the icons shown in the list (logical pixels at 96 DPI).
///
/// One size for both shapes of the window: the feed is the same list without the
/// technical surroundings, and a reader who wants bigger rows turns up the text
/// size for the whole program instead of for one view (`Settings::font_scale`).
pub const ICON_SIZE: i32 = 16;

/// Application identifier -> executable of a running process.
#[derive(Debug, Default, Clone)]
pub struct ProcessMap {
    entries: HashMap<String, String>,
    read_at: Option<Instant>,
}

impl ProcessMap {
    /// Rebuild from the running processes when the cache is stale.
    pub fn refresh(&mut self, now: Instant) {
        let stale = match self.read_at {
            Some(stamp) => now.duration_since(stamp) >= PROCESS_CACHE_TTL,
            None => true,
        };
        if !stale {
            return;
        }
        self.entries = read_process_map();
        self.read_at = Some(now);
    }

    /// Executable reported by a running process for `aumid`.
    pub fn executable(&self, aumid: &str) -> Option<&str> {
        self.entries.get(&normalise(aumid)).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Identifiers are compared case-insensitively.
fn normalise(aumid: &str) -> String {
    aumid.trim().to_lowercase()
}

/// Read identifier -> executable for every accessible process.
fn read_process_map() -> HashMap<String, String> {
    let mut map = HashMap::new();
    unsafe {
        let snapshot = win32::CreateToolhelp32Snapshot(win32::TH32CS_SNAPPROCESS, 0);
        if snapshot == win32::INVALID_HANDLE_VALUE || snapshot.is_null() {
            return map;
        }
        let mut entry = win32::PROCESSENTRY32W::default();
        if win32::Process32FirstW(snapshot, &mut entry) == 0 {
            win32::CloseHandle(snapshot);
            return map;
        }
        loop {
            if let Some((aumid, path)) = identifier_of(entry.th32_process_id) {
                map.entry(normalise(&aumid)).or_insert(path);
            }
            if win32::Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }
        win32::CloseHandle(snapshot);
    }
    map
}

/// Application identifier and image path of one process.
fn identifier_of(process_id: u32) -> Option<(String, String)> {
    unsafe {
        let process = win32::OpenProcess(win32::PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 512];
        let mut length = buffer.len() as u32;
        let status = win32::GetApplicationUserModelId(process, &mut length, buffer.as_mut_ptr());
        let aumid = if status == 0 {
            win32::read_wide(&buffer[..(length as usize).min(buffer.len())])
        } else {
            String::new()
        };
        let mut path_buffer = vec![0u16; 1024];
        let mut path_size = path_buffer.len() as u32;
        let ok =
            win32::QueryFullProcessImageNameW(process, 0, path_buffer.as_mut_ptr(), &mut path_size);
        win32::CloseHandle(process);
        if ok == 0 || path_size == 0 {
            return None;
        }
        let path = win32::read_wide(&path_buffer[..path_size as usize]);
        if aumid.trim().is_empty() || path.trim().is_empty() {
            return None;
        }
        Some((aumid, path))
    }
}

/// An icon together with the information whether we own it.
///
/// Icons from `LoadIconW` are *shared* by the system and must never be passed to
/// `DestroyIcon`; icons from `ExtractIconExW`/`SHGetFileInfoW` are ours and have
/// to be freed.  Mixing the two up corrupts the system icon table, which is one
/// of the ways to crash another program later.
#[derive(Debug, Clone, Copy)]
pub struct LoadedIcon {
    pub handle: HICON,
    pub owned: bool,
}

impl LoadedIcon {
    /// The shared icon of a program without its own icon.
    pub fn shared_default() -> LoadedIcon {
        let handle = unsafe { win32::LoadIconW(std::ptr::null_mut(), win32::IDI_APPLICATION) };
        LoadedIcon {
            handle,
            owned: false,
        }
    }

    /// Release the icon when it belongs to us.
    pub fn release(self) {
        if self.owned {
            win32::destroy_icon(self.handle);
        }
    }
}

/// Identifier of the icon of this program inside the executable (`build.rs`).
pub const APPLICATION_ICON_ID: u32 = 1;

/// The icon of this program: task bar, window corner, tray, file properties.
///
/// It comes from the resource section the build script writes (`WNL.ico`), so
/// the handle belongs to the module and must not be destroyed - which is why a
/// plain `HICON` is returned instead of `LoadedIcon`.  Executables built without
/// the icon simply fall back to the shared system icon.
pub fn application_icon() -> HICON {
    let icon = unsafe {
        win32::LoadIconW(
            win32::module_handle(),
            win32::int_resource(APPLICATION_ICON_ID),
        )
    };
    if icon.is_null() {
        unsafe { win32::LoadIconW(std::ptr::null_mut(), win32::IDI_APPLICATION) }
    } else {
        icon
    }
}

/// Icon of an application identifier, sized `size` device pixels.
///
/// `LoadedIcon::owned` is the answer to "was a real icon found": the shared default
/// icon is never owned, which is how [`icon_for_notification`] knows that the
/// picture of the notification is a better answer after all.
pub fn icon_for(aumid: Option<&str>, processes: &ProcessMap, size: i32) -> LoadedIcon {
    if let Some(aumid) = aumid.filter(|value| !value.trim().is_empty()) {
        if let Some(reference) = appname::icon_reference(aumid) {
            if let Some(icon) = win32::extract_icon_at(&reference.path, reference.index, size <= 16)
            {
                return owned(icon);
            }
            // An icon file the icon APIs cannot read (`.png` without an index, a
            // file that is a picture only) still has a shell icon.
            if let Some(icon) = win32::shell_icon(&reference.path, size <= 16) {
                return owned(icon);
            }
        }
        if let Some(icon) = package_icon(aumid, size) {
            return icon;
        }
        if let Some(icon) = win32::apps_folder_icon(aumid, size <= 16) {
            return owned(icon);
        }
        if let Some(executable) = processes.executable(aumid) {
            if let Some(icon) = win32::extract_icon(executable, size <= 16) {
                return owned(icon);
            }
            if let Some(icon) = win32::shell_icon(executable, size <= 16) {
                return owned(icon);
            }
        }
    }
    LoadedIcon::shared_default()
}

/// Icon of one notification: the icon of its sender, or the picture it came with.
///
/// The picture of the notification is the second choice on purpose.  When the
/// system knows the application, its own icon is the honest answer; when it does
/// not - and that is the normal case for a browser with a random identifier, for a
/// script that posts a toast, or for an application that registers nothing but a
/// name - the logo the notification carries is exactly the picture the user saw in
/// the notification centre.
pub fn icon_for_notification(
    aumid: Option<&str>,
    picture: Option<&str>,
    processes: &ProcessMap,
    size: i32,
) -> LoadedIcon {
    let icon = icon_for(aumid, processes, size);
    if icon.owned {
        return icon;
    }
    match picture.filter(|value| !value.trim().is_empty()) {
        Some(reference) => picture_icon(reference, aumid, size).unwrap_or(icon),
        None => icon,
    }
}

/// Icon built from the picture a notification carried.
///
/// The picture is a PNG or a JPEG, which no icon API can read, so GDI+ decodes it
/// and hands back an icon of exactly `size` pixels (see [`imaging::load_icon`]).
fn picture_icon(reference: &str, aumid: Option<&str>, size: i32) -> Option<LoadedIcon> {
    let target = u32::try_from(size).ok()?;
    let path = imaging::resolve(reference, aumid, target)?;
    imaging::load_icon(&path, size).map(owned)
}

/// An icon that has to be destroyed when it is no longer used.
fn owned(handle: HICON) -> LoadedIcon {
    LoadedIcon {
        handle,
        owned: true,
    }
}

/// Icon a packaged application declares in its manifest.
///
/// The file is a PNG (or the tile art of the application), which no icon API can
/// read - it is decoded by GDI+ and turned into an icon.
fn package_icon(aumid: &str, size: i32) -> Option<LoadedIcon> {
    let target = u32::try_from(size).ok()?;
    let path = packages::icon_asset(aumid, target)?;
    let handle = imaging::load_icon(&path, size)?;
    Some(LoadedIcon {
        handle,
        owned: true,
    })
}

/// Image list plus the mapping identifier -> image index.
pub struct IconCache {
    list: HIMAGELIST,
    indexes: HashMap<String, i32>,
    processes: ProcessMap,
    /// Pixel size of the icons in this list (the list view asks for that size,
    /// and so do the variants inside an application package).
    size: i32,
    enabled: bool,
}

impl IconCache {
    /// Create the image list (`None` when the common controls refuse).
    pub fn new(size: i32, enabled: bool) -> Option<IconCache> {
        let list = unsafe { win32::ImageList_Create(size, size, win32::ILC_COLOR32, 0, 32) };
        if list.is_null() {
            return None;
        }
        Some(IconCache {
            list,
            indexes: HashMap::new(),
            processes: ProcessMap::default(),
            size,
            enabled,
        })
    }

    pub fn handle(&self) -> HIMAGELIST {
        self.list
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Attach the image list to a list view.
    pub fn attach(&self, list_view: win32::HWND) {
        unsafe {
            win32::SendMessageW(
                list_view,
                win32::LVM_SETIMAGELIST,
                LVSIL_SMALL as WPARAM,
                self.list as LPARAM,
            );
        }
    }

    /// Image index of one row: the icon of its application or the picture the
    /// notification came with, added to the list on first use.
    ///
    /// Returns `-1` when icons are switched off or no picture can be produced,
    /// which the list view reads as "no image".  The key holds both hints, because
    /// two notifications of one application regularly carry two different logos.
    pub fn index_for(&mut self, aumid: Option<&str>, picture: Option<&str>) -> i32 {
        if !self.enabled {
            return -1;
        }
        let key = format!(
            "{}\u{1}{}",
            aumid.unwrap_or_default().trim(),
            picture.unwrap_or_default().trim()
        );
        if let Some(index) = self.indexes.get(&key) {
            return *index;
        }
        self.processes.refresh(Instant::now());
        let size = self.size;
        let loaded = icon_for_notification(aumid, picture, &self.processes, size);
        let index = if loaded.handle.is_null() {
            -1
        } else {
            let index = unsafe { win32::ImageList_ReplaceIcon(self.list, -1, loaded.handle) };
            // The image list keeps its own copy; only icons we created are freed.
            loaded.release();
            index.max(0)
        };
        self.indexes.insert(key, index);
        index
    }

    /// Forget everything (used when the setting is switched off and on again).
    pub fn clear(&mut self) {
        self.indexes.clear();
        self.processes = ProcessMap::default();
        unsafe {
            win32::ImageList_Remove(self.list, -1);
        }
    }
}

impl Drop for IconCache {
    fn drop(&mut self) {
        if !self.list.is_null() {
            unsafe {
                win32::ImageList_Destroy(self.list);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lookup table is the only place where a notification is connected to a
    /// program, so both the comparison rule and the keys of a refreshed table are
    /// checked here.
    #[test]
    fn identifiers_are_looked_up_by_normalised_key() {
        let mut map = ProcessMap::default();
        map.entries.insert(
            normalise("Vendor.Chat_123!App"),
            r"C:\apps\chat.exe".to_string(),
        );
        assert_eq!(
            map.executable("Vendor.Chat_123!App"),
            Some(r"C:\apps\chat.exe")
        );
        assert_eq!(
            map.executable("vendor.chat_123!app"),
            Some(r"C:\apps\chat.exe"),
            "the comparison ignores case"
        );
        assert_eq!(map.executable("Other.App"), None);
        assert_eq!(map.len(), 1);

        // An empty identifier belongs to no program and matches nothing.
        let empty = ProcessMap::default();
        assert!(empty.executable("").is_none());
        assert!(empty.executable("   ").is_none());
        assert!(empty.is_empty());

        // Reading the process table is best effort (a sandbox may deny it), but the
        // keys have to look like normalised application identifiers.
        let mut live = ProcessMap::default();
        live.refresh(Instant::now());
        for aumid in live.entries.keys() {
            assert!(!aumid.is_empty());
            assert_eq!(aumid, &aumid.to_lowercase());
        }
    }

    /// A packaged application registers no icon in the registry; its icon comes
    /// from the package itself, and the PNG it sits in has to become a real icon
    /// (this is the step that used to fail silently and left every Store app with
    /// the same default icon).
    #[test]
    fn packaged_applications_get_their_icon_from_the_package() {
        let settings =
            "windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel";
        if packages::icon_asset(settings, 16).is_none() {
            return; // an installation without that package
        }
        assert!(
            appname::icon_path(settings).is_none(),
            "the point of the test: the registry knows nothing about this app"
        );
        let icon = icon_for(Some(settings), &ProcessMap::default(), 16);
        assert!(
            icon.owned,
            "the icon has to come from the package, not from the fallback"
        );
        assert!(!icon.handle.is_null());
        icon.release();
    }

    /// A two by two pixel picture, written by the test itself: GDI+ decodes it, so
    /// the whole path from a notification's picture to an icon runs for real.
    fn tiny_bitmap_file(directory: &std::path::Path) -> std::path::PathBuf {
        let mut bytes: Vec<u8> = Vec::new();
        let pixels: [u8; 16] = [
            0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0x00, 0x00,
            0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00,
        ];
        let size = 14 + 40 + pixels.len();
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(size as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&24u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        for _ in 0..4 {
            bytes.extend_from_slice(&0i32.to_le_bytes());
        }
        bytes.extend_from_slice(&pixels);
        let file = directory.join("notification-logo.bmp");
        std::fs::write(&file, bytes).expect("written");
        file
    }

    /// The notification's own logo is what has to be shown when the system knows no
    /// icon for the sender: a browser registers a random identifier, a script that
    /// posts a toast registers nothing at all, and both used to end up with the same
    /// default icon.
    #[test]
    fn a_sender_without_a_known_icon_is_shown_through_its_own_picture() {
        let root = std::env::temp_dir().join("wnl-appicon-picture");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("folder");
        let picture = tiny_bitmap_file(&root);
        let reference = picture.display().to_string();

        let unknown = "Vendor.Definitely.Not.Installed_123!App";
        assert!(
            appname::icon_reference(unknown).is_none(),
            "the point of the test: nothing is registered for this identifier"
        );
        let processes = ProcessMap::default();
        let from_picture = icon_for_notification(Some(unknown), Some(&reference), &processes, 16);
        assert!(
            from_picture.owned,
            "the picture of the notification has to be turned into an icon"
        );
        assert!(!from_picture.handle.is_null());
        from_picture.release();

        // Without a picture the same notification falls back on the shared default
        // icon, which belongs to the system and must never be destroyed.
        let plain = icon_for_notification(Some(unknown), None, &processes, 16);
        assert!(!plain.owned);
        assert!(!plain.handle.is_null());

        // A reference that leads nowhere is not an error either.
        let broken = icon_for_notification(
            Some(unknown),
            Some(r"C:\definitely\missing-logo.png"),
            &processes,
            16,
        );
        assert!(!broken.owned);
        assert!(!broken.handle.is_null());

        let _ = std::fs::remove_dir_all(&root);
    }
}
