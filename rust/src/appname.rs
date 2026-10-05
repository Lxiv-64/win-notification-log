//! Application display names for AUMIDs.
//!
//! Mirrors the Python lookup: Win32 applications register their label under
//! `HKCU\Software\Classes\AppUserModelId\<aumid>` (or the machine-wide
//! equivalent), packaged applications expose it through WinRT (added in a later
//! stage) and the notification database itself carries no name at all.
//!
//! The resolution is cached in memory and stored in the journal's `app_names`
//! table by the caller.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::os::windows::ffi::OsStrExt;

const HKEY_CURRENT_USER: usize = 0x8000_0001;
const HKEY_LOCAL_MACHINE: usize = 0x8000_0002;
/// The merged view the shell itself reads: it holds the classes of the machine
/// *and* the ones an installer registered for the user.
const HKEY_CLASSES_ROOT: usize = 0x8000_0000;
const KEY_READ: u32 = 0x0002_0019;

/// Title of the application, used in every caption and message.
pub const APPLICATION_TITLE: &str = "Win Notification Log";

/// Caption of the main window: the name, the version and what the program is.
///
/// The Python version carried the same words, so the window is recognisable
/// across the rewrite.
pub fn window_title() -> String {
    format!(
        "{APPLICATION_TITLE} {} - журнал уведомлений",
        crate::VERSION
    )
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        key: usize,
        sub_key: *const u16,
        options: u32,
        desired: u32,
        result: *mut usize,
    ) -> i32;
    fn RegQueryValueExW(
        key: usize,
        value_name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: usize) -> i32;
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Read `DisplayName` from one registry hive.
fn read_display_name(root: usize, path: &str) -> Option<String> {
    let wide_path = wide(path);
    let mut key: usize = 0;
    if unsafe { RegOpenKeyExW(root, wide_path.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
        return None;
    }
    let value = read_value(key, "DisplayName");
    unsafe { RegCloseKey(key) };
    value.and_then(|text| clean(&text))
}

/// `DisplayName` of an application identifier, in the three places an installer
/// can write it: the user's classes, the machine's classes and the merged view.
fn registry_display_name(aumid: &str) -> Option<String> {
    if aumid.trim().is_empty() {
        return None;
    }
    read_display_name(
        HKEY_CURRENT_USER,
        &format!("Software\\Classes\\AppUserModelId\\{aumid}"),
    )
    .or_else(|| {
        read_display_name(
            HKEY_LOCAL_MACHINE,
            &format!("SOFTWARE\\Classes\\AppUserModelId\\{aumid}"),
        )
    })
    .or_else(|| read_display_name(HKEY_CLASSES_ROOT, &format!("AppUserModelId\\{aumid}")))
}

/// Readable form of an application identifier.
///
/// Chromium based browsers (Chrome, Edge, Thorium, Brave) report
/// `Name.<hash of the user>` as their identifier, and that hash is all a reader
/// gets when the application registered no display name of its own: without this
/// the list shows `Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456` next to the plain
/// `Thorium` of the same browser.  The identifier itself is never changed - the
/// technical column still shows every letter of it.
pub fn readable_identifier(aumid: &str) -> &str {
    generated_suffix_head(aumid).unwrap_or(aumid)
}

/// The part in front of a machine generated suffix, when there is one.
///
/// `Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456` becomes `Thorium`, while
/// `Microsoft.Windows.PowerShell`, `Microsoft.Office.OUTLOOK.EXE.15` and
/// `Microsoft.WindowsCalculator_8wekyb3d8bbwe!App` keep their whole name.
fn generated_suffix_head(aumid: &str) -> Option<&str> {
    let (head, suffix) = aumid.rsplit_once('.')?;
    if head.is_empty() || !suffix_is_generated(suffix) {
        return None;
    }
    Some(head)
}

/// Does the text behind the last dot look like a hash instead of a word?
///
/// The rule stays narrow on purpose: at least eight characters, letters and
/// digits only, and at least two digits.  `PowerShell`, `15`,
/// `Notification.Manager` and `App_123!App` therefore stay as they are.
fn suffix_is_generated(suffix: &str) -> bool {
    const LEAST: usize = 8;
    suffix.chars().count() >= LEAST
        && suffix
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        && suffix.chars().filter(char::is_ascii_digit).count() >= 2
}

/// Read one string value of an already opened key.
fn read_value(key: usize, name: &str) -> Option<String> {
    let value_name = wide(name);
    let mut kind: u32 = 0;
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            buffer.as_mut_ptr() as *mut u8,
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let length = ((size as usize) / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..length])
        .trim_end_matches('\0')
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// Raw registry value of `...\AppUserModelId\<aumid>`, user first, machine second.
///
/// An installer may register the identifier for the machine, for the user, or in
/// the merged view of the classes; all three are read, because an application
/// that is only found in one of them is otherwise without a name and an icon.
pub fn registry_string(aumid: &str, value: &str) -> Option<String> {
    if aumid.trim().is_empty() {
        return None;
    }
    for (root, path) in [
        (
            HKEY_CURRENT_USER,
            format!("Software\\Classes\\AppUserModelId\\{aumid}"),
        ),
        (
            HKEY_LOCAL_MACHINE,
            format!("SOFTWARE\\Classes\\AppUserModelId\\{aumid}"),
        ),
        (HKEY_CLASSES_ROOT, format!("AppUserModelId\\{aumid}")),
    ] {
        let wide_path = wide(&path);
        let mut key: usize = 0;
        if unsafe { RegOpenKeyExW(root, wide_path.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
            continue;
        }
        let value = read_value(key, value);
        unsafe { RegCloseKey(key) };
        if value.is_some() {
            return value;
        }
    }
    None
}

/// Turn a registry icon value into a file path, when it is one.
///
/// `IconUri` is frequently an indirect string (`@{dll,-id}`) or a package
/// reference; only a real file (or executable) can be handed to the icon APIs.
pub fn icon_candidate(raw: &str) -> Option<String> {
    icon_target(raw).map(|(path, _)| path)
}

/// Icon file of an application together with the icon inside that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconReference {
    pub path: String,
    /// Position of the icon in the file, or, when negative, its *resource
    /// identifier* - `ExtractIconExW` understands both.
    pub index: i32,
}

/// Icon an application registers for its identifier (`IconUri`).
///
/// This is the picture Windows itself puts in front of a notification of that
/// application as long as the notification does not carry a logo of its own.
pub fn icon_reference(aumid: &str) -> Option<IconReference> {
    let raw = registry_string(aumid, "IconUri")?;
    let (path, index) = icon_target(&raw)?;
    std::path::Path::new(&path)
        .is_file()
        .then_some(IconReference { path, index })
}

/// Icon file registered for an application identifier, without the index.
pub fn icon_path(aumid: &str) -> Option<String> {
    icon_reference(aumid).map(|reference| reference.path)
}

/// File and icon index of a raw `IconUri` value.
///
/// Three things have to be understood here, because all three appear in the
/// registry of a normal machine: variables (`%SystemRoot%\...`), the indirect
/// strings a package or a resource identifier uses (`@{dll?resource}`), and the
/// index of the icon inside the file (`shell32.dll,-101`).
pub fn icon_target(raw: &str) -> Option<(String, i32)> {
    let expanded = crate::win32::expand_environment(raw.trim().trim_matches('"').trim());
    let value = expanded.trim().trim_matches('"').trim();
    let file = if value.starts_with('@') || value.starts_with("ms-resource:") {
        crate::win32::indirect_string(value)?
    } else {
        value.to_string()
    };
    split_icon_index(file.trim().trim_matches('"').trim())
}

/// `C:\Windows\System32\shell32.dll,-101` becomes the file and the icon index.
///
/// Only the part behind the last comma is split off, and only when it really is a
/// number: a file whose *name* contains a comma keeps its name.
fn split_icon_index(value: &str) -> Option<(String, i32)> {
    let path = path_only(value)?;
    let Some((file, index)) = path.rsplit_once(',') else {
        return Some((path, 0));
    };
    let index = index.trim();
    let numeric = !index.is_empty()
        && index
            .chars()
            .all(|character| character.is_ascii_digit() || character == '-');
    if !numeric {
        return Some((path, 0));
    }
    let index = index.parse::<i32>().ok()?;
    let file = file.trim();
    if file.is_empty() {
        return None;
    }
    Some((file.to_string(), index))
}

/// Accept plain paths only: resource references carry `?` or a scheme.
fn path_only(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim();
    if value.is_empty() || value.contains('?') || value.contains("://") {
        return None;
    }
    Some(value.to_string())
}

/// Reject indirection strings (`@{...}`, `ms-resource:`) that need a resource
/// loader to be resolved.
fn clean(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('@') || value.starts_with("ms-resource:") {
        return None;
    }
    Some(value.to_string())
}

/// Cache that combines registry names, names reported by collectors and the
/// journal cache.
pub struct NameResolver {
    cache: RefCell<HashMap<String, String>>,
    checked: RefCell<HashSet<String>>,
    /// Names learned since they were last stored in the journal.
    ///
    /// The cache is seeded from the journal at startup, so those names are
    /// already on disk; only what was resolved afterwards has to be written.
    /// Without this the collector would rewrite the whole cache on every pass.
    pending: RefCell<HashMap<String, String>>,
}

impl Default for NameResolver {
    fn default() -> Self {
        NameResolver::new()
    }
}

impl NameResolver {
    pub fn new() -> Self {
        NameResolver {
            cache: RefCell::new(HashMap::new()),
            checked: RefCell::new(HashSet::new()),
            pending: RefCell::new(HashMap::new()),
        }
    }

    /// Seed the cache (for example from the journal's `app_names` table).
    pub fn with_cache(names: HashMap<String, String>) -> Self {
        NameResolver {
            cache: RefCell::new(names),
            checked: RefCell::new(HashSet::new()),
            // The seeded names are exactly what the journal already holds.
            pending: RefCell::new(HashMap::new()),
        }
    }

    /// Store a name reported by a collector.
    pub fn remember(&self, aumid: &str, app_name: &str) {
        if aumid.is_empty() {
            return;
        }
        if let Some(cleaned) = clean(app_name) {
            if self.cache.borrow().get(aumid) == Some(&cleaned) {
                return; // already known: nothing new to store
            }
            self.cache
                .borrow_mut()
                .insert(aumid.to_string(), cleaned.clone());
            self.pending
                .borrow_mut()
                .insert(aumid.to_string(), cleaned);
        }
    }

    /// Resolve a name, consulting the registry once per AUMID.
    ///
    /// A browser of the Chromium family registers itself under an identifier that
    /// carries a hash of the user (`Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456`).  When
    /// that exact identifier has no name, the part in front of the hash is asked
    /// for as well - that is the plain `Thorium`, and it is the same application.
    pub fn resolve(&self, aumid: &str) -> Option<String> {
        if aumid.is_empty() {
            return None;
        }
        if let Some(name) = self.cache.borrow().get(aumid) {
            return Some(name.clone());
        }
        if self.checked.borrow().contains(aumid) {
            return None;
        }
        self.checked.borrow_mut().insert(aumid.to_string());

        let found = registry_display_name(aumid).or_else(|| {
            let head = generated_suffix_head(aumid)?;
            self.cache
                .borrow()
                .get(head)
                .cloned()
                .or_else(|| registry_display_name(head))
        });
        if let Some(name) = found {
            self.cache
                .borrow_mut()
                .insert(aumid.to_string(), name.clone());
            self.pending
                .borrow_mut()
                .insert(aumid.to_string(), name.clone());
            return Some(name);
        }
        None
    }

    /// Everything resolved so far (persisted into `app_names` by the caller).
    pub fn known(&self) -> HashMap<String, String> {
        self.cache.borrow().clone()
    }

    /// Names learned since the last [`NameResolver::forget_pending`].
    ///
    /// Only these have to be written: the rest of the cache came from the
    /// journal and is stored already.
    pub fn pending(&self) -> HashMap<String, String> {
        self.pending.borrow().clone()
    }

    /// Forget the names that were stored successfully.
    pub fn forget_pending(&self) {
        self.pending.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_indirection_strings() {
        assert_eq!(clean("Display"), Some("Display".to_string()));
        assert_eq!(clean("  padded  "), Some("padded".to_string()));
        assert_eq!(clean("@{Microsoft.Win32/Resource?x}"), None);
        assert_eq!(clean("ms-resource://App/Name"), None);
        assert_eq!(clean("   "), None);
    }

    #[test]
    fn resolver_uses_remembered_names() {
        let resolver = NameResolver::new();
        resolver.remember("Vendor.App!Id", "Vendor App");
        assert_eq!(
            resolver.resolve("Vendor.App!Id").as_deref(),
            Some("Vendor App")
        );
        assert_eq!(resolver.known().len(), 1);

        // A program that cannot be resolved is not an error: the identifier is then
        // shown instead of a name.
        assert_eq!(resolver.resolve("Definitely.Not.Installed_123!App"), None);
        assert_eq!(resolver.resolve(""), None);
    }

    /// Only names that are new or changed have to be written: the cache is seeded
    /// from the journal, so repeating a name must not produce a write.
    #[test]
    fn only_new_or_changed_names_are_pending() {
        let mut seeded = std::collections::HashMap::new();
        seeded.insert("Vendor.App!Id".to_string(), "Vendor App".to_string());
        let resolver = NameResolver::with_cache(seeded);

        // A seeded name is already stored: nothing to write.
        assert!(resolver.pending().is_empty());

        // Learning a name is worth a write.
        resolver.remember("Other.App!Id", "Other App");
        assert_eq!(
            resolver.pending().get("Other.App!Id").map(String::as_str),
            Some("Other App")
        );

        // Repeating it changes nothing.
        resolver.forget_pending();
        resolver.remember("Other.App!Id", "Other App");
        assert!(resolver.pending().is_empty());

        // A renamed application is written again.
        resolver.remember("Other.App!Id", "Other App (new)");
        assert_eq!(
            resolver.pending().get("Other.App!Id").map(String::as_str),
            Some("Other App (new)")
        );
    }

    /// A Chromium browser reports `Name.<hash of the user>`.  The reader sees the
    /// name; the whole identifier stays available for the technical column.
    #[test]
    fn a_hashed_identifier_reads_as_the_application() {
        assert_eq!(
            readable_identifier("Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"),
            "Thorium"
        );
        assert_eq!(
            readable_identifier("Chrome.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"),
            "Chrome"
        );
        // A word behind the last dot is a word, not a hash: every one of these
        // identifiers keeps its whole name.
        for aumid in [
            "Microsoft.Windows.PowerShell",
            "Microsoft.Office.OUTLOOK.EXE.15",
            "Acrobat.Reader.Notification.Manager",
            "org.meshcore.open.app",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            "Vendor.App_123!App",
            "BlueStacks_nxt",
            "Thorium",
            "",
        ] {
            assert_eq!(readable_identifier(aumid), aumid, "{aumid}");
        }
    }

    /// The application in front of the hash is the one that carries the name, and
    /// the resolver looks there as well; an unknown identifier stays nameless
    /// instead of inventing a name out of its parts.
    #[test]
    fn a_hashed_identifier_borrows_the_name_of_its_application() {
        let resolver = NameResolver::new();
        resolver.remember("Thorium", "Thorium");
        assert_eq!(
            resolver
                .resolve("Thorium.ABCDEFGHIJKLMNOPQRSTUVWXYZ123456")
                .as_deref(),
            Some("Thorium")
        );
        assert_eq!(resolver.resolve("Mystery.ABCDEFGH12345678"), None);
    }

    #[test]
    fn icon_candidates_keep_plain_paths_only() {
        assert_eq!(
            icon_candidate(r"C:\Program Files\Chat\chat.exe").as_deref(),
            Some(r"C:\Program Files\Chat\chat.exe")
        );
        assert_eq!(
            icon_candidate(r#"  "C:\tools\app.exe"  "#).as_deref(),
            Some(r"C:\tools\app.exe")
        );
        assert_eq!(icon_candidate("ms-appx:///Assets/Logo.png"), None);
        assert_eq!(icon_candidate("@{Microsoft.Win32/Resource?x}"), None);
        assert_eq!(icon_candidate("   "), None);
        assert_eq!(
            icon_candidate(r"C:\icons\a.ico"),
            Some(r"C:\icons\a.ico".to_string())
        );
    }

    /// An `IconUri` names a file *and* the icon inside it, and the index is what
    /// decides which picture the user sees: the first icon of `shell32.dll` is not
    /// the icon anybody registered.
    #[test]
    fn an_icon_reference_keeps_the_index_of_the_icon() {
        let (path, index) = icon_target(r"C:\Windows\System32\shell32.dll,-101").expect("path");
        assert_eq!(path, r"C:\Windows\System32\shell32.dll");
        assert_eq!(index, -101, "a negative value is a resource identifier");

        let (path, index) = icon_target(r"C:\apps\mail.exe, 3 ").expect("path");
        assert_eq!(path, r"C:\apps\mail.exe");
        assert_eq!(index, 3);

        // A comma inside a *name* is part of the name, not an index.
        let (path, index) = icon_target(r"C:\apps\mail, post.exe").expect("path");
        assert_eq!(path, r"C:\apps\mail, post.exe");
        assert_eq!(index, 0);

        assert_eq!(icon_target("@{Microsoft.Win32/Resource?x}"), None);
        assert_eq!(icon_target("   "), None);

        // A value written with a variable names the same file once the variable is
        // expanded (`%SystemRoot%\System32\shell32.dll`).
        if let Some(root) = std::env::var_os("SystemRoot") {
            let (path, index) = icon_target(r"%SystemRoot%\System32\shell32.dll,-3").expect("path");
            assert!(
                path.starts_with(&root.to_string_lossy().to_string()),
                "the variable was not expanded: {path}"
            );
            assert_eq!(index, -3);
        }
    }
}
