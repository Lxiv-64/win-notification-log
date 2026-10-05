//! Picture previews for the notifications that carry an image.
//!
//! Pictures are decoded with GDI+, which ships with Windows and understands PNG,
//! JPEG, GIF, BMP and TIFF - so no image library has to be compiled in.
//!
//! Three kinds of reference turn up in notifications, and all three are handled
//! here: an ordinary path (`C:\…`, `file:///C:/…`), a file inside an installed
//! application package (`ms-appx:///Assets/…`, `ms-appdata:///local/…`, see
//! `packages`) and a web address.  A web address is *not* fetched: downloading a
//! picture from a push would tell the sender that the notification has been
//! looked at, so the reference is handed to the user instead (the list offers
//! *Открыть изображение*, which opens it in the browser on request).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use sha1::Digest as _;

use crate::packages;
use crate::win32::{self, HBITMAP, HICON};

/// Largest preview shown in the details window (logical pixels at 96 DPI).
pub const MAX_PREVIEW_WIDTH: i32 = 320;
pub const MAX_PREVIEW_HEIGHT: i32 = 180;

/// Width a caller wants when it only needs a file, not a specific size.
pub const ANY_SIZE: u32 = 1024;

/// Signature of `GdiplusStartup`.
type StartupFunction = unsafe extern "system" fn(
    *mut usize,
    *const win32::GDIPLUS_STARTUP_INPUT,
    *mut std::ffi::c_void,
) -> i32;

/// GDI+ entry points, resolved once from `gdiplus.dll`.
struct GdiPlus {
    create_bitmap: unsafe extern "system" fn(*const u16, *mut *mut std::ffi::c_void) -> i32,
    thumbnail:
        unsafe extern "system" fn(*mut std::ffi::c_void, u32, u32, *mut *mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void) -> i32,
    to_bitmap: unsafe extern "system" fn(*mut std::ffi::c_void, *mut HBITMAP, u32) -> i32,
    to_icon: unsafe extern "system" fn(*mut std::ffi::c_void, *mut HICON) -> i32,
    dispose_image: unsafe extern "system" fn(*mut std::ffi::c_void) -> i32,
    image_width: unsafe extern "system" fn(*mut std::ffi::c_void, *mut u32) -> i32,
    image_height: unsafe extern "system" fn(*mut std::ffi::c_void, *mut u32) -> i32,
}

/// The library is loaded once and stays loaded: the GDI+ token lives as long as
/// the process, so freeing the module would be wrong.
fn gdiplus() -> Option<&'static GdiPlus> {
    static INSTANCE: OnceLock<Option<GdiPlus>> = OnceLock::new();
    INSTANCE
        .get_or_init(|| unsafe {
            let module = win32::Module::load("gdiplus.dll")?;
            let startup = module.symbol("GdiplusStartup");
            let create = module.symbol("GdipCreateBitmapFromFile");
            let thumbnail = module.symbol("GdipGetImageThumbnail");
            let to_bitmap = module.symbol("GdipCreateHBITMAPFromBitmap");
            let to_icon = module.symbol("GdipCreateHICONFromBitmap");
            let dispose = module.symbol("GdipDisposeImage");
            let width = module.symbol("GdipGetImageWidth");
            let height = module.symbol("GdipGetImageHeight");
            if startup.is_null()
                || create.is_null()
                || thumbnail.is_null()
                || to_bitmap.is_null()
                || to_icon.is_null()
                || dispose.is_null()
                || width.is_null()
                || height.is_null()
            {
                return None;
            }
            let startup: StartupFunction = std::mem::transmute(startup);
            let mut token: usize = 0;
            let input = win32::GDIPLUS_STARTUP_INPUT::default();
            if startup(&mut token, &input, std::ptr::null_mut()) != 0 {
                return None;
            }
            // Keep the DLL for the lifetime of the process on purpose.
            std::mem::forget(module);
            Some(GdiPlus {
                create_bitmap: std::mem::transmute(create),
                thumbnail: std::mem::transmute(thumbnail),
                to_bitmap: std::mem::transmute(to_bitmap),
                to_icon: std::mem::transmute(to_icon),
                dispose_image: std::mem::transmute(dispose),
                image_width: std::mem::transmute(width),
                image_height: std::mem::transmute(height),
            })
        })
        .as_ref()
}

/// True when previews are possible at all (GDI+ present).
pub fn is_available() -> bool {
    gdiplus().is_some()
}

/// A decoded picture plus the bitmap handle the window can display.
pub struct Preview {
    pub bitmap: HBITMAP,
    pub width: i32,
    pub height: i32,
}

impl Drop for Preview {
    fn drop(&mut self) {
        if !self.bitmap.is_null() {
            unsafe {
                win32::DeleteObject(self.bitmap);
            }
        }
    }
}

/// A decoded picture: the GDI+ image plus the size it is shown at.
struct Decoded {
    image: *mut std::ffi::c_void,
    width: i32,
    height: i32,
}

impl Drop for Decoded {
    fn drop(&mut self) {
        unsafe {
            if let Some(library) = gdiplus() {
                (library.dispose_image)(self.image);
            }
        }
    }
}

/// Decode a picture file scaled into the given box (never enlarged).
fn decode(path: &Path, max_width: i32, max_height: i32) -> Option<Decoded> {
    let library = gdiplus()?;
    let source = win32::wide(&path.display().to_string());
    let mut image: *mut std::ffi::c_void = std::ptr::null_mut();
    let status = unsafe { (library.create_bitmap)(source.as_ptr(), &mut image) };
    if status != 0 || image.is_null() {
        return None;
    }
    let mut width = 0u32;
    let mut height = 0u32;
    unsafe {
        (library.image_width)(image, &mut width);
        (library.image_height)(image, &mut height);
    }
    let (target_width, target_height) = fit(width, height, max_width, max_height);
    if target_width <= 0 || target_height <= 0 {
        unsafe {
            (library.dispose_image)(image);
        }
        return None;
    }
    if target_width as u32 == width && target_height as u32 == height {
        return Some(Decoded {
            image,
            width: target_width,
            height: target_height,
        });
    }
    // A picture that is too large is scaled by GDI+ itself; the original is then
    // no longer needed.
    let mut scaled: *mut std::ffi::c_void = std::ptr::null_mut();
    let status = unsafe {
        (library.thumbnail)(
            image,
            target_width as u32,
            target_height as u32,
            &mut scaled,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    unsafe {
        (library.dispose_image)(image);
    }
    if status != 0 || scaled.is_null() {
        return None;
    }
    Some(Decoded {
        image: scaled,
        width: target_width,
        height: target_height,
    })
}

/// Decode a picture file; `None` when the file is unreadable or not an image.
///
/// The decoded bitmap has the size reported in `Preview`, because an oversized
/// picture is scaled down: a static control does not scale anything by itself.
pub fn load(path: &Path, max_width: i32, max_height: i32) -> Option<Preview> {
    let library = gdiplus()?;
    let decoded = decode(path, max_width, max_height)?;
    let mut bitmap: HBITMAP = std::ptr::null_mut();
    // 0xFFFFFFFF asks GDI+ for a transparent background.
    let status = unsafe { (library.to_bitmap)(decoded.image, &mut bitmap, 0xFFFF_FFFF) };
    if status != 0 || bitmap.is_null() {
        return None;
    }
    Some(Preview {
        bitmap,
        width: decoded.width,
        height: decoded.height,
    })
}

/// Fit a picture into a box without distorting it (pure, therefore testable).
pub fn fit(width: u32, height: u32, max_width: i32, max_height: i32) -> (i32, i32) {
    if width == 0 || height == 0 || max_width <= 0 || max_height <= 0 {
        return (0, 0);
    }
    let scale_w = f64::from(max_width) / f64::from(width);
    let scale_h = f64::from(max_height) / f64::from(height);
    let scale = scale_w.min(scale_h).min(1.0);
    let target_w = (f64::from(width) * scale).round().max(1.0) as i32;
    let target_h = (f64::from(height) * scale).round().max(1.0) as i32;
    (target_w, target_h)
}

/// What can be done with one picture reference of a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// A file the program can decode and show.
    Local(PathBuf),
    /// A web address; the program never fetches such a picture by itself.
    Remote(String),
    /// A reference that leads nowhere on this machine.
    Missing,
}

/// Local file behind a picture reference of an application.
///
/// `target` is the width the caller wants in pixels; a picture that Windows
/// installed only in sized variants is picked for that width.  `aumid` is the
/// application the notification came from, which is what package references are
/// resolved against.
pub fn resolve(raw: &str, aumid: Option<&str>, target: u32) -> Option<PathBuf> {
    // The journal's own copy comes first: it is the only version that is still
    // there when the sender deleted its temporary file.
    if let Some(known) = cached(raw) {
        return Some(known);
    }
    let value = raw.trim().trim_matches('"').trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with("ms-appx://") || value.starts_with("ms-appdata://") {
        return packages::resolve_uri(value, aumid?, target);
    }
    let path = if let Some(rest) = value.strip_prefix("file:///") {
        PathBuf::from(rest.replace('/', "\\"))
    } else if let Some(rest) = value.strip_prefix("file://") {
        PathBuf::from(rest.replace('/', "\\"))
    } else if value.contains("://") {
        return None;
    } else {
        PathBuf::from(value)
    };
    path.is_file().then_some(path)
}

/// Classify a reference without decoding it.
///
/// This is what tells the details window whether it can show a picture, has to
/// explain a web address, or has to say that a file is missing.
pub fn availability(raw: &str, aumid: Option<&str>, target: u32) -> Availability {
    if let Some(path) = resolve(raw, aumid, target) {
        return Availability::Local(path);
    }
    let value = raw.trim().trim_matches('"').trim();
    let scheme = value
        .split("://")
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    if value.contains("://") && !matches!(scheme.as_str(), "ms-appx" | "ms-appdata" | "file") {
        return Availability::Remote(value.to_string());
    }
    Availability::Missing
}

/// Decode a picture into an icon, for pictures that are shown as a small image.
///
/// The icon is built by GDI+ itself: the bitmap it hands out is a *DIB section*,
/// and `CreateIconIndirect` refuses those (it fails with
/// `ERROR_INVALID_PARAMETER`), so the detour through `ICONINFO` cannot be used
/// here.  The icon belongs to the caller and has to be freed with
/// [`win32::destroy_icon`].
pub fn load_icon(path: &Path, size: i32) -> Option<HICON> {
    let library = gdiplus()?;
    let decoded = decode(path, size, size)?;
    let mut icon: HICON = std::ptr::null_mut();
    let status = unsafe { (library.to_icon)(decoded.image, &mut icon) };
    (status == 0 && !icon.is_null()).then_some(icon)
}

/// Folder the journal keeps copies of pictures in (set once at start-up).
static PICTURE_FOLDER: OnceLock<PathBuf> = OnceLock::new();
/// Whether copies are taken at all (the setting of the same name).
static KEEP_PICTURES: AtomicBool = AtomicBool::new(true);

/// Largest picture that becomes part of the journal (4 MiB).
///
/// A journal has to stay small and portable; a bigger picture is not copied, the
/// reference stays readable in the record.
pub const MAX_PICTURE_BYTES: u64 = 4 * 1024 * 1024;

/// Extensions a kept picture may have.  The list exists because the name of the
/// copy is derived from the reference: looking one up means probing these.
pub const PICTURE_EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "tif", "tiff", "webp", "img", "tmp",
];

/// Tell the module where the journal keeps its pictures and whether new ones are
/// kept; every entry point calls this once after the layout is known.
pub fn configure(folder: PathBuf, keep: bool) {
    let _ = PICTURE_FOLDER.set(folder);
    KEEP_PICTURES.store(keep, Ordering::Relaxed);
}

/// Name (without extension) a reference is kept under.
///
/// It is derived from the *reference*, not from the picture: the reference is the
/// only thing the journal still knows when the temporary file of the sender is
/// long gone.
pub fn cache_stem(reference: &str) -> String {
    let mut hasher = sha1::Sha1::new();
    hasher.update(reference.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Copy of a picture the journal made for a reference, when there is one.
pub fn cached(reference: &str) -> Option<PathBuf> {
    let folder = PICTURE_FOLDER.get()?;
    let stem = cache_stem(reference);
    PICTURE_EXTENSIONS
        .iter()
        .map(|extension| folder.join(format!("{stem}.{extension}")))
        .find(|path| path.is_file())
}

/// Copy a picture into the journal while the reference still leads somewhere.
///
/// Does nothing when keeping is switched off, when the reference is not readable
/// (anymore), or when the picture is too large; the record keeps its reference in
/// any case.
pub fn keep(reference: &str, aumid: Option<&str>) -> Option<PathBuf> {
    let folder = PICTURE_FOLDER.get()?;
    if !KEEP_PICTURES.load(Ordering::Relaxed) {
        return None;
    }
    if let Some(known) = cached(reference) {
        return Some(known);
    }
    let source = resolve(reference, aumid, ANY_SIZE)?;
    if source.parent() == Some(folder.as_path()) {
        return None;
    }
    let size = std::fs::metadata(&source).ok()?.len();
    if size == 0 || size > MAX_PICTURE_BYTES {
        return None;
    }
    let target = folder.join(format!("{}.{}", cache_stem(reference), picture_extension(&source)));
    std::fs::create_dir_all(folder).ok()?;
    std::fs::copy(&source, &target).ok()?;
    Some(target)
}

/// Delete the copies that no record refers to any more; returns how many went.
///
/// The picture folder belongs to the journal, so a record that is deleted (by
/// hand, by a filter, or by the retention rule) must not leave its picture
/// behind forever.  The names are derived from the references, so the caller
/// hands in the references the journal still holds.
pub fn prune(folder: &Path, references: &[String]) -> usize {
    let mut expected: HashSet<String> = HashSet::new();
    for reference in references {
        let stem = cache_stem(reference);
        for extension in PICTURE_EXTENSIONS {
            expected.insert(format!("{stem}.{extension}"));
        }
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if expected.contains(name) {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Extension of the copy: the known one, otherwise `img`.
///
/// Senders do put a PNG into a file called `….tmp`; the format is decided by the
/// content (GDI+ reads it by itself), so the name only has to be honest about
/// what it is.
fn picture_extension(source: &Path) -> String {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if PICTURE_EXTENSIONS.contains(&extension.as_str()) {
        extension
    } else {
        "img".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_keeps_the_aspect_ratio() {
        assert_eq!(
            fit(100, 100, 320, 180),
            (100, 100),
            "small pictures stay as they are"
        );
        assert_eq!(fit(1000, 500, 320, 180), (320, 160));
        assert_eq!(fit(200, 1000, 320, 180), (36, 180));
        assert_eq!(fit(0, 100, 320, 180), (0, 0));
        assert_eq!(fit(100, 100, 0, 180), (0, 0));
    }

    #[test]
    fn only_files_on_this_machine_are_previewable() {
        // A package reference needs the application it belongs to, and one that
        // is not installed has no files at all.
        assert_eq!(resolve("ms-appx:///Assets/Logo.png", None, ANY_SIZE), None);
        assert_eq!(
            resolve(
                "ms-appx:///Assets/Logo.png",
                Some("Definitely.Not.Installed_x!App"),
                ANY_SIZE
            ),
            None
        );
        assert_eq!(
            resolve("ms-appdata:///local/x.png", None, ANY_SIZE),
            None
        );
        assert_eq!(resolve("https://example.org/a.png", None, ANY_SIZE), None);
        assert_eq!(resolve("   ", None, ANY_SIZE), None);
        // A file that does not exist is not an error, just nothing to show.
        assert_eq!(resolve(r"C:\definitely\missing\image.png", None, ANY_SIZE), None);

        // The classification says *why* nothing can be shown.
        assert_eq!(
            availability("https://example.org/a.png", None, ANY_SIZE),
            Availability::Remote("https://example.org/a.png".to_string())
        );
        assert_eq!(
            availability("ms-appx:///Assets/Logo.png", None, ANY_SIZE),
            Availability::Missing,
            "a package reference is not a web address"
        );
        assert_eq!(availability("file:///C:/nope.png", None, ANY_SIZE), Availability::Missing);
        assert_eq!(availability("", None, ANY_SIZE), Availability::Missing);
    }

    /// A tiny uncompressed picture written by the test: two by two pixels, so
    /// that decoding, scaling and the icon conversion all run for real.
    fn tiny_bitmap_file(directory: &Path, name: &str) -> PathBuf {
        let mut bytes: Vec<u8> = Vec::new();
        let pixels: [u8; 16] = [
            0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0x00, 0x00, // first row, bottom up
            0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, // second row
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
            bytes.extend_from_slice(&0i32.to_le_bytes()); // resolution and palette
        }
        bytes.extend_from_slice(&pixels);
        let file = directory.join(name);
        std::fs::write(&file, bytes).expect("written");
        file
    }

    /// A picture whose sender deletes the file right away has to survive in the
    /// journal - that is what makes "картинка из пуша" visible a week later.
    #[test]
    fn pictures_are_kept_next_to_the_journal() {
        // The two folders are unique to this test, so the process wide
        // configuration cannot disturb the other tests.
        let root = std::env::temp_dir().join("wnl-imaging-keep");
        let _ = std::fs::remove_dir_all(&root);
        let sender = root.join("sender");
        let folder = root.join("pictures");
        std::fs::create_dir_all(&sender).expect("sender folder");
        let source = tiny_bitmap_file(&sender, "seen-once.bmp");
        let reference = source.display().to_string();

        configure(folder.clone(), true);
        let kept = keep(&reference, None).expect("the picture is kept");
        assert_eq!(kept.parent(), Some(folder.as_path()), "kept: {kept:?}");
        assert_eq!(
            kept.extension().and_then(|value| value.to_str()),
            Some("bmp"),
            "the copy keeps the format of the file"
        );
        assert_eq!(cached(&reference).as_deref(), Some(kept.as_path()));

        // The sender deletes its file: the journal still shows the picture.
        std::fs::remove_file(&source).expect("deleted");
        assert_eq!(resolve(&reference, None, ANY_SIZE), Some(kept.clone()));
        assert!(load(&kept, 64, 64).is_some(), "the copy is a real picture");

        // Keeping the same reference twice copies nothing twice.
        assert_eq!(keep(&reference, None), Some(kept.clone()));

        // A reference that leads nowhere, and a picture that is too large to be
        // part of a journal, are left alone.
        assert_eq!(keep(r"C:\definitely\missing\picture.png", None), None);
        assert_eq!(cached(r"C:\definitely\missing\picture.png"), None);

        // Switched off means nothing is copied, even for a readable file.
        let second = tiny_bitmap_file(&sender, "second.bmp");
        configure(folder.clone(), false);
        assert_eq!(keep(&second.display().to_string(), None), None);
        configure(folder.clone(), true);

        // A copy whose record is gone is removed again; the ones the journal
        // still holds stay.
        let other = tiny_bitmap_file(&sender, "other.bmp");
        let other_reference = other.display().to_string();
        let kept_other = keep(&other_reference, None).expect("second picture kept");
        assert_eq!(prune(&folder, std::slice::from_ref(&reference)), 1);
        assert!(!kept_other.exists(), "the unkept copy is gone");
        assert!(cached(&reference).is_some(), "the picture of a record stays");
        assert_eq!(prune(&folder, &[]), 1, "an empty journal keeps nothing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_addresses_become_paths() {
        let directory = std::env::temp_dir().join("wnl-imaging-test");
        std::fs::create_dir_all(&directory).expect("directory");
        let file = tiny_bitmap_file(&directory, "picture.bmp");
        std::fs::write(directory.join("garbage.png"), b"not really a picture").expect("written");

        let direct = resolve(&file.display().to_string(), None, ANY_SIZE).expect("plain path");
        assert_eq!(direct, file);

        let address = format!("file:///{}", file.display().to_string().replace('\\', "/"));
        let from_address = resolve(&address, None, ANY_SIZE).expect("file address");
        assert_eq!(from_address, file);
        assert_eq!(
            availability(&address, None, ANY_SIZE),
            Availability::Local(file.clone())
        );

        // The picture is decoded, and the decoded bitmap becomes an icon, which
        // is what the list and the details window show.
        let preview = load(&file, 64, 64).expect("decoded");
        assert_eq!((preview.width, preview.height), (2, 2));
        let scaled = load(&file, 1, 1).expect("scaled down");
        assert_eq!(
            (scaled.width, scaled.height),
            (1, 1),
            "an oversized picture is scaled into the box it is shown in"
        );
        let icon = load_icon(&file, 16).expect("icon");
        assert!(!icon.is_null(), "the icon is a real handle");
        win32::destroy_icon(icon);
        assert!(load(&directory.join("garbage.png"), 64, 64).is_none());
        assert!(load_icon(&directory.join("garbage.png"), 16).is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
