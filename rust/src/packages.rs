//! Files inside an installed application package.
//!
//! Notifications of Store-style applications rarely point at ordinary paths:
//! their pictures are `ms-appx:///Assets/…` (a file inside the installed
//! package) or `ms-appdata:///local/…` (a file in the application's own state
//! folder), and the *icon* of such an application lives in the package as well -
//! the registry carries nothing for it, which is why the list showed the same
//! default icon for every packaged sender.
//!
//! The package behind an identifier is found with the application model API
//! (`GetPackagesByPackageFamily` plus `GetPackagePathByFullName`): no COM, no
//! elevation and no manifest reader needed.  Windows stores pictures of a
//! package under qualifier names (`logo.scale-125.png`,
//! `logo.targetsize-20.png`, `logo.altform-unplated.png`), so the file that fits
//! a requested size is picked here as well.
//!
//! Every lookup is cached for the process: one listing asks for the same few
//! applications over and over, and a package root never changes while the
//! program runs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[link(name = "kernel32")]
extern "system" {
    fn GetPackagesByPackageFamily(
        family: *const u16,
        count: *mut u32,
        names: *mut *mut u16,
        length: *mut u32,
        buffer: *mut u16,
    ) -> u32;
    fn GetPackagePathByFullName(full_name: *const u16, length: *mut u32, path: *mut u16) -> u32;
}

/// Folder a package may write to (`ms-appdata`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Local,
    Roaming,
    Temp,
}

impl Area {
    fn folder(self) -> &'static str {
        match self {
            Area::Local => "LocalState",
            Area::Roaming => "RoamingState",
            Area::Temp => "TempState",
        }
    }
}

/// The two reference schemes a package uses for its own files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// `ms-appx:///…` - inside the installed package.
    Package,
    /// `ms-appdata:///local|roaming|temp/…` - inside the state folder.
    Data,
}

/// An `ms-appx`/`ms-appdata` reference, already split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageUri {
    pub scheme: Scheme,
    pub area: Area,
    /// Relative path with backslashes, without empty parts and without `..`.
    pub relative: String,
}

impl PackageUri {
    /// File name without its extension (`Assets\Logo.png` -> `Logo`).
    pub fn stem(&self) -> &str {
        let name = self
            .relative
            .rsplit_once(|character: char| character == '\\' || character == '/')
            .map(|(_, name)| name)
            .unwrap_or(&self.relative);
        name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
    }
}

/// Package family of an application identifier (`App_x!App` -> `App_x`).
pub fn family_name(aumid: &str) -> Option<String> {
    let value = aumid.trim();
    if value.is_empty() {
        return None;
    }
    let family = value.split('!').next().unwrap_or(value).trim();
    (!family.is_empty()).then(|| family.to_string())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Read a NUL terminated string the system API pointed at.
fn pointer_text(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0usize;
    unsafe {
        while length < 512 && *pointer.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
    }
}

/// Split an `ms-appx`/`ms-appdata` reference into scheme, area and path.
///
/// Anything else - plain paths, `file:///`, `https://` - is not a package
/// reference and is refused, as are references that try to walk out of the
/// package with `..`.
pub fn parse_uri(raw: &str) -> Option<PackageUri> {
    let value = raw.trim().trim_matches('"').trim();
    let (head, rest) = value.split_once("://")?;
    let scheme = match head.trim().to_ascii_lowercase().as_str() {
        "ms-appx" => Scheme::Package,
        "ms-appdata" => Scheme::Data,
        _ => return None,
    };
    let mut parts = rest
        .split(|character: char| character == '/' || character == '\\')
        .filter(|part| !part.is_empty() && *part != ".");
    let area = match scheme {
        Scheme::Package => Area::Local,
        Scheme::Data => match parts.next()?.to_ascii_lowercase().as_str() {
            "local" | "localstate" => Area::Local,
            "roaming" | "roamingstate" => Area::Roaming,
            "temp" | "tempstate" => Area::Temp,
            _ => return None,
        },
    };
    let mut relative: Vec<&str> = Vec::new();
    for part in parts {
        if part == ".." {
            return None;
        }
        relative.push(part);
    }
    if relative.is_empty() {
        return None;
    }
    Some(PackageUri {
        scheme,
        area,
        relative: relative.join("\\"),
    })
}

/// Full names of the installed packages of one family (usually exactly one).
fn packages_of_family(family: &str) -> Vec<String> {
    let family_w = wide(family);
    let mut count: u32 = 0;
    let mut length: u32 = 0;
    // The first call only reports how large the two buffers have to be.
    unsafe {
        GetPackagesByPackageFamily(
            family_w.as_ptr(),
            &mut count,
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
        );
    }
    if count == 0 || length == 0 {
        return Vec::new();
    }
    let mut names: Vec<*mut u16> = vec![std::ptr::null_mut(); count as usize];
    let mut buffer: Vec<u16> = vec![0; length as usize];
    let status = unsafe {
        GetPackagesByPackageFamily(
            family_w.as_ptr(),
            &mut count,
            names.as_mut_ptr(),
            &mut length,
            buffer.as_mut_ptr(),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    names
        .iter()
        .take(count as usize)
        .map(|name| pointer_text(*name))
        .filter(|name| !name.trim().is_empty())
        .collect()
}

/// Installation folder of one package.
fn path_of_package(full_name: &str) -> Option<PathBuf> {
    let full_w = wide(full_name);
    let mut length: u32 = 0;
    unsafe {
        GetPackagePathByFullName(full_w.as_ptr(), &mut length, std::ptr::null_mut());
    }
    if length == 0 {
        return None;
    }
    let mut buffer: Vec<u16> = vec![0; length as usize];
    let status =
        unsafe { GetPackagePathByFullName(full_w.as_ptr(), &mut length, buffer.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    let text = pointer_text(buffer.as_ptr());
    (!text.trim().is_empty()).then(|| PathBuf::from(text))
}

/// Everything that is looked up once per process.
#[derive(Default)]
struct Cache {
    packages: HashMap<String, Vec<String>>,
    roots: HashMap<String, Option<PathBuf>>,
    logos: HashMap<String, Vec<Logo>>,
    icons: HashMap<(String, u32), Option<PathBuf>>,
    listings: HashMap<PathBuf, Vec<String>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Run `action` with the cache of this thread.
///
/// A lookup that happens *inside* another lookup (the icon of an application
/// asks for its package root) uses a fresh cache instead of borrowing twice -
/// the answer is remembered by the outer call, so nothing is lost.
fn with_cache<T>(action: impl FnOnce(&mut Cache) -> T) -> T {
    CACHE.with(|cache| match cache.try_borrow_mut() {
        Ok(mut borrowed) => action(&mut borrowed),
        Err(_) => action(&mut Cache::default()),
    })
}

/// Installation folder of an application (its first installed package).
pub fn package_root(aumid: &str) -> Option<PathBuf> {
    let family = family_name(aumid)?;
    with_cache(|cache| {
        if let Some(known) = cache.roots.get(&family) {
            return known.clone();
        }
        if !cache.packages.contains_key(&family) {
            let packages = packages_of_family(&family);
            cache.packages.insert(family.clone(), packages);
        }
        let root = cache
            .packages
            .get(&family)
            .into_iter()
            .flatten()
            .find_map(|full_name| {
                let path = path_of_package(full_name)?;
                path.is_dir().then_some(path)
            });
        cache.roots.insert(family, root.clone());
        root
    })
}

/// State folder of an application: `%LOCALAPPDATA%\Packages\<family>\…`.
pub fn state_root(aumid: &str, area: Area) -> Option<PathBuf> {
    let family = family_name(aumid)?;
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let root = local.join("Packages").join(family).join(area.folder());
    root.is_dir().then_some(root)
}

/// Folder a reference points into (the package root or a state folder).
fn root_for(uri: &PackageUri, aumid: &str) -> Option<PathBuf> {
    match uri.scheme {
        Scheme::Package => package_root(aumid),
        Scheme::Data => state_root(aumid, uri.area),
    }
}

/// Names inside a folder (cached; empty when the folder cannot be read).
fn listing(folder: &Path) -> Vec<String> {
    with_cache(|cache| {
        if let Some(known) = cache.listings.get(folder) {
            return known.clone();
        }
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        cache.listings.insert(folder.to_path_buf(), names.clone());
        names
    })
}

/// Local file behind a reference of an application.
///
/// `target` is the width the caller wants in pixels: Windows often installs
/// only the sized variants of a picture, so the closest one is taken.
pub fn resolve(uri: &PackageUri, aumid: &str, target: u32) -> Option<PathBuf> {
    let root = root_for(uri, aumid)?;
    let wanted = root.join(&uri.relative);
    if let Some(folder) = wanted.parent() {
        if let Some(name) = pick_variant(&listing(folder), uri.stem(), None, target) {
            let file = folder.join(name);
            if file.is_file() {
                return Some(file);
            }
        }
    }
    wanted.is_file().then_some(wanted)
}

/// Resolve a raw reference (`ms-appx:///Assets/Logo.png`) of an application.
pub fn resolve_uri(raw: &str, aumid: &str, target: u32) -> Option<PathBuf> {
    resolve(&parse_uri(raw)?, aumid, target)
}

/// One `…Logo` attribute of an application manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logo {
    /// Nominal size in pixels (`Square44x44Logo` -> 44).
    pub base: u32,
    /// Path inside the package, as the manifest writes it.
    pub path: String,
}

/// Logos an application manifest declares, best first.
///
/// The 44x44 icon is the one the shell uses for lists; the tile pictures come
/// after it, so the first entry that exists on disk wins.
pub fn manifest_logos(xml: &str) -> Vec<Logo> {
    let mut ranked: Vec<(u8, Logo)> = Vec::new();
    let mut search = 0usize;
    while let Some(found) = xml[search..].find("Logo=\"") {
        let at = search + found;
        let value_start = at + "Logo=\"".len();
        // The scan stops at the attribute name *before* the word "Logo", which
        // the pattern itself consumed.
        let name_start = xml[..at]
            .char_indices()
            .rev()
            .find(|(_, character)| !is_name_character(*character))
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(0);
        let name = format!("{}Logo", &xml[name_start..at]);
        let Some(end) = xml[value_start..].find('"') else {
            break;
        };
        let value = xml[value_start..value_start + end].trim();
        search = value_start + end;
        if value.is_empty() || value.contains("ms-resource:") {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let rank = match lower.as_str() {
            "square44x44logo" => 0,
            "logo" | "square150x150logo" => 1,
            _ => 2,
        };
        let logo = Logo {
            base: logo_base(&name),
            path: value.to_string(),
        };
        if !ranked.iter().any(|(_, known)| known.path == logo.path) {
            ranked.push((rank, logo));
        }
    }
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, logo)| logo).collect()
}

/// Character an XML attribute name may consist of (`Square150x150Logo`).
fn is_name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
}

/// Nominal pixel size an attribute name implies.
fn logo_base(name: &str) -> u32 {
    let lower = name.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("square") {
        if let Some(number) = rest.split('x').next() {
            if let Ok(value) = number.parse::<u32>() {
                return value;
            }
        }
    }
    match lower.as_str() {
        "smalllogo" => 24,
        _ => 150,
    }
}

/// The part of a file name that follows the asset name (`Logo.scale-125`).
fn qualifier_tail(entry: &str, stem: &str) -> Option<String> {
    let extension = entry.rsplit_once('.').map(|(_, extension)| extension)?;
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "bmp" | "ico"
    ) {
        return None;
    }
    let without_extension = &entry[..entry.len() - extension.len() - 1];
    let rest = without_extension.strip_prefix(stem)?;
    if !rest.is_empty() && !rest.starts_with('.') {
        // Another asset that merely starts with the same letters.
        return None;
    }
    Some(rest.to_ascii_lowercase())
}

/// Number behind a qualifier (`scale-125_altform-unplated` + `scale-` -> 125).
fn qualifier_number(tail: &str, prefix: &str) -> Option<u32> {
    let start = tail.find(prefix)? + prefix.len();
    let digits: String = tail[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Pixels a variant declares.
///
/// `Some(None)` is the file itself when the nominal size of the asset is
/// unknown; `None` means the variant is not the plain flavour (high contrast,
/// unplated, dark/light, another language) and is skipped.
fn qualifier_pixels(tail: &str, base: Option<u32>) -> Option<Option<u32>> {
    for skipped in [
        "contrast-",
        "altform-",
        "theme-",
        "lang-",
        "homeregion-",
        "configuration-",
        "dxfeaturelevel-",
    ] {
        if tail.contains(skipped) {
            return None;
        }
    }
    if tail.is_empty() {
        return Some(base);
    }
    if let Some(pixels) = qualifier_number(tail, "targetsize-") {
        return Some(Some(pixels));
    }
    if let Some(scale) = qualifier_number(tail, "scale-") {
        // The scale is relative to the nominal size; when that size is unknown
        // the numbers still order the variants among themselves.
        return Some(Some(match base {
            Some(base) => base.saturating_mul(scale) / 100,
            None => scale,
        }));
    }
    None
}

/// Best installed variant of one asset for `target` pixels.
///
/// A variant that is at least as large as requested wins, the smallest of those
/// first; when nothing is large enough, the largest installed one is taken
/// instead.  `base` is the nominal size of the asset (`Some(44)` for an
/// application icon), `None` when only the order of the variants is known -
/// then the file itself is the fallback.
pub fn pick_variant(
    entries: &[String],
    stem: &str,
    base: Option<u32>,
    target: u32,
) -> Option<String> {
    // A candidate is described by whether it is large enough and by the key
    // that orders the candidates of the same kind.
    let mut best: Option<(bool, (i64, u8), String)> = None;
    for entry in entries {
        let Some(tail) = qualifier_tail(entry, stem) else {
            continue;
        };
        let Some(size) = qualifier_pixels(&tail, base) else {
            continue;
        };
        let rank = if tail.contains("targetsize-") {
            0
        } else if tail.contains("scale-") {
            1
        } else {
            2
        };
        let (fits, key) = match size {
            // The file itself: its size is unknown, so it counts as usable but
            // loses against every variant that is known to be large enough.
            None => (true, (i64::from(u32::MAX), rank)),
            Some(pixels) if pixels >= target => (true, (i64::from(pixels), rank)),
            Some(pixels) => (false, (-i64::from(pixels), rank)),
        };
        let better = match &best {
            None => true,
            Some((best_fits, best_key, _)) => match (fits, *best_fits) {
                (true, false) => true,
                (false, true) => false,
                _ => key < *best_key,
            },
        };
        if better {
            best = Some((fits, key, entry.clone()));
        }
    }
    best.map(|(_, _, entry)| entry)
}

/// Logos of an application, read from its manifest once per process.
fn logos_of(aumid: &str) -> Vec<Logo> {
    let family = family_name(aumid).unwrap_or_default();
    with_cache(|cache| {
        if let Some(known) = cache.logos.get(&family) {
            return known.clone();
        }
        let logos = package_root(aumid)
            .map(|root| root.join("AppxManifest.xml"))
            .and_then(|manifest| std::fs::read_to_string(manifest).ok())
            .map(|xml| manifest_logos(&xml))
            .unwrap_or_default();
        cache.logos.insert(family, logos.clone());
        logos
    })
}

/// Icon file the manifest of an application declares, sized for `target` pixels.
pub fn icon_asset(aumid: &str, target: u32) -> Option<PathBuf> {
    let family = family_name(aumid)?;
    with_cache(|cache| {
        if let Some(known) = cache.icons.get(&(family.clone(), target)) {
            return known.clone();
        }
        let found = icon_asset_uncached(aumid, target);
        cache.icons.insert((family, target), found.clone());
        found
    })
}

/// Look for the icon of an application inside its package.
fn icon_asset_uncached(aumid: &str, target: u32) -> Option<PathBuf> {
    let root = package_root(aumid)?;
    for logo in logos_of(aumid) {
        let declared = root.join(&logo.path);
        if let Some(folder) = declared.parent() {
            let name = declared
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name);
            if let Some(variant) = pick_variant(&listing(folder), stem, Some(logo.base), target) {
                let file = folder.join(variant);
                if file.is_file() {
                    return Some(file);
                }
            }
        }
        if declared.is_file() {
            return Some(declared);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_are_read_for_the_icon_of_the_application() {
        let logos = manifest_logos(
            r#"<Package xmlns:uap="x"><Applications><Application>
            <uap:VisualElements DisplayName="ms-resource:Name"
            Square150x150Logo="Assets\Retail\PhotosMedTile.png"
            Square71x71Logo="Assets\Retail\PhotosSmallTile.png"
            Square44x44Logo="Assets\Retail\PhotosAppList.png"
            Square44x44Logo="Assets\Retail\PhotosAppList.png"
            Description="ms-resource:Description"/></Application></Applications></Package>"#,
        );
        assert_eq!(
            logos,
            vec![
                Logo {
                    base: 44,
                    path: r"Assets\Retail\PhotosAppList.png".to_string(),
                },
                Logo {
                    base: 150,
                    path: r"Assets\Retail\PhotosMedTile.png".to_string(),
                },
                Logo {
                    base: 71,
                    path: r"Assets\Retail\PhotosSmallTile.png".to_string(),
                },
            ],
            "the 44x44 icon comes first and duplicates are dropped"
        );
        assert!(manifest_logos("<Package/>").is_empty());
        assert!(manifest_logos(r#"<Logo></Logo><Logo>ms-resource:x</Logo>"#).is_empty());
        assert!(manifest_logos(r#"<VisualElements Square44x44Logo=""/>"#).is_empty());
    }

    /// References to package files have to be read exactly, because a wrong
    /// path would look at somebody else's file.
    #[test]
    fn package_references_are_split_into_scheme_area_and_path() {
        let package = parse_uri("ms-appx:///Assets/LiveTiles/avatar150x150.png").expect("package");
        assert_eq!(package.scheme, Scheme::Package);
        assert_eq!(package.area, Area::Local);
        assert_eq!(package.relative, r"Assets\LiveTiles\avatar150x150.png");
        assert_eq!(package.stem(), "avatar150x150");

        for (raw, area) in [
            ("ms-appdata:///local/Tile.jpg", Area::Local),
            ("ms-appdata:///roaming/state.dat", Area::Roaming),
            ("ms-appdata:///TempState/t.jpg", Area::Temp),
        ] {
            let uri = parse_uri(raw).expect(raw);
            assert_eq!(uri.scheme, Scheme::Data);
            assert_eq!(uri.area, area);
        }

        // Normalisation: the scheme ignores case, redundant parts disappear.
        assert_eq!(
            parse_uri("MS-APPX:///./Assets//Logo.png")
                .expect("normalised")
                .relative,
            r"Assets\Logo.png"
        );

        // Nothing else is a package reference.
        for raw in [
            r"C:\Windows\logo.png",
            "file:///C:/Windows/logo.png",
            "https://example.org/logo.png",
            "ms-appx:///",
            "ms-appdata:///other/x.png",
            "ms-appx:///../secret.txt",
            "",
        ] {
            assert_eq!(parse_uri(raw), None, "{raw} is not a package file");
        }
    }

    #[test]
    fn identifiers_are_split_into_family_and_name() {
        assert_eq!(
            family_name("Vendor.Chat_123!App").as_deref(),
            Some("Vendor.Chat_123")
        );
        assert_eq!(
            family_name("windows.immersivecontrolpanel_cw5n1h2txyewy").as_deref(),
            Some("windows.immersivecontrolpanel_cw5n1h2txyewy")
        );
        assert_eq!(family_name("  "), None);
        assert_eq!(family_name(""), None);
        assert_eq!(family_name("!App"), None);
    }

    /// Windows ships one picture under many names; the variant that fits the
    /// requested size has to win, and the flavoured ones must not.
    #[test]
    fn variants_are_chosen_by_size_without_the_flavoured_ones() {
        let settings = [
            "logo.png",
            "logo.altform-unplated.png",
            "logo.contrast-black.png",
            "logo.contrast-black_scale-100.png",
            "logo.scale-100.png",
            "logo.scale-100_altform-lightunplated.png",
            "logo.scale-125.png",
            "logo.scale-150.png",
            "logo.scale-200.png",
            "logo.scale-400.png",
            "logo.targetsize-16.png",
            "logo.targetsize-20_altform-unplated.png",
            "logo.targetsize-24.png",
            "logo.contrast-white.png",
            "TileSmall.png",
            "logo.txt",
        ]
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<String>>();

        assert_eq!(
            pick_variant(&settings, "logo", Some(44), 16).as_deref(),
            Some("logo.targetsize-16.png"),
            "an exact size wins over the scaled variants"
        );
        assert_eq!(
            pick_variant(&settings, "logo", Some(44), 20).as_deref(),
            Some("logo.targetsize-24.png"),
            "the next bigger size is taken when the exact one is unplated"
        );
        assert_eq!(
            pick_variant(&settings, "logo", Some(44), 48).as_deref(),
            Some("logo.scale-125.png"),
            "scale-125 of a 44 pixel icon is the first one big enough for 48"
        );
        assert_eq!(
            pick_variant(&settings, "logo", Some(44), 900).as_deref(),
            Some("logo.scale-400.png"),
            "when nothing is big enough the largest variant is used"
        );
        assert_eq!(
            pick_variant(&settings, "TileSmall", Some(150), 16).as_deref(),
            Some("TileSmall.png"),
            "only files of this asset count"
        );

        // A picture of unknown nominal size: the scale numbers order themselves.
        let pictures = [
            "avatar150x150.png",
            "avatar310x150.scale-200.png",
            "avatar310x310.jpg",
        ]
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<String>>();
        assert_eq!(
            pick_variant(&pictures, "avatar310x310", None, 320).as_deref(),
            Some("avatar310x310.jpg"),
            "without a nominal size the file itself is taken"
        );
        assert_eq!(
            pick_variant(&pictures, "avatar150x150", None, 320).as_deref(),
            Some("avatar150x150.png"),
            "a file without qualifiers is found by its own name"
        );
        assert_eq!(
            pick_variant(&pictures, "avatar310x150", None, 320).as_deref(),
            Some("avatar310x150.scale-200.png"),
            "a scaled variant is used when the plain file is missing"
        );
        assert!(pick_variant(&pictures, "avatar99x99", None, 320).is_none());
        assert!(pick_variant(&[], "logo", Some(44), 16).is_none());
    }

    /// The system is asked for the same few applications again and again, and a
    /// program that is not a package has to answer "nothing".
    #[test]
    fn only_installed_packages_are_found() {
        assert_eq!(package_root("Definitely.Not.Installed_0123456789abc!App"), None);
        assert_eq!(
            icon_asset("Definitely.Not.Installed_0123456789abc!App", 20),
            None
        );
        assert_eq!(
            resolve_uri(
                "ms-appx:///Assets/Logo.png",
                "Definitely.Not.Installed_x!App",
                20
            ),
            None
        );
        assert_eq!(state_root("Definitely.Not.Installed_x!App", Area::Local), None);

        // Settings is part of every Windows 10 and 11 installation; where it is
        // there, its manifest has to name a real icon file.
        let settings =
            "windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel";
        if let Some(root) = package_root(settings) {
            assert!(root.is_dir(), "the package root is a folder: {root:?}");
            let icon = icon_asset(settings, 20).expect("the manifest names an icon");
            assert!(icon.is_file(), "the icon file exists: {icon:?}");
            assert!(
                icon.to_string_lossy().to_lowercase().contains("logo"),
                "the icon is one of the logo files: {icon:?}"
            );
        }
    }
}
