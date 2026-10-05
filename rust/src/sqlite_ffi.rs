//! Loader for the in-box SQLite library (`winsqlite3.dll`).
//!
//! The native build deliberately links no SQLite import library: the function
//! table is resolved at run time with `LoadLibraryW`/`GetProcAddress`.  Verified
//! properties of the system DLL (see `tools/probe_winsqlite3.py`):
//!
//! * SQLite 3.43.2 with `ENABLE_FTS5`, WAL, triggers and partial indexes;
//! * present in `C:\Windows\System32` and `SysWOW64` on Windows 10 and 11.
//!
//! A `sqlite3.dll` placed next to the executable takes precedence, so a specific
//! SQLite build can be shipped without recompiling the program.

use std::ffi::{c_char, c_int, c_void, CStr, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// SQLite open flags (see sqlite3.h).
pub const OPEN_READONLY: c_int = 0x0000_0001;
pub const OPEN_READWRITE: c_int = 0x0000_0002;
pub const OPEN_CREATE: c_int = 0x0000_0004;
pub const OPEN_FULLMUTEX: c_int = 0x0001_0000;
pub const OPEN_URI: c_int = 0x0000_0040;

pub const SQLITE_OK: c_int = 0;
pub const SQLITE_ROW: c_int = 100;
pub const SQLITE_DONE: c_int = 101;

pub const SQLITE_INTEGER: c_int = 1;
pub const SQLITE_FLOAT: c_int = 2;
pub const SQLITE_TEXT: c_int = 3;
pub const SQLITE_BLOB: c_int = 4;

pub type Sqlite3 = *mut c_void;
pub type Sqlite3Stmt = *mut c_void;

/// Loaded function table of the SQLite DLL.
///
/// Some entries are not used yet (they belong to later stages of the port), so
/// dead-code analysis is switched off for the table itself.
#[allow(dead_code)]
pub struct Api {
    pub open_v2: unsafe extern "C" fn(*const c_char, *mut Sqlite3, c_int, *const c_char) -> c_int,
    pub close_v2: unsafe extern "C" fn(Sqlite3) -> c_int,
    pub errmsg: unsafe extern "C" fn(Sqlite3) -> *const c_char,
    pub busy_timeout: unsafe extern "C" fn(Sqlite3, c_int) -> c_int,
    pub exec: unsafe extern "C" fn(
        Sqlite3,
        *const c_char,
        *mut c_void,
        *mut c_void,
        *mut *mut c_char,
    ) -> c_int,
    pub prepare_v2: unsafe extern "C" fn(
        Sqlite3,
        *const c_char,
        c_int,
        *mut Sqlite3Stmt,
        *mut *const c_char,
    ) -> c_int,
    pub step: unsafe extern "C" fn(Sqlite3Stmt) -> c_int,
    pub finalize: unsafe extern "C" fn(Sqlite3Stmt) -> c_int,
    pub reset: unsafe extern "C" fn(Sqlite3Stmt) -> c_int,
    pub clear_bindings: unsafe extern "C" fn(Sqlite3Stmt) -> c_int,
    pub column_count: unsafe extern "C" fn(Sqlite3Stmt) -> c_int,
    pub column_name: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> *const c_char,
    pub column_type: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> c_int,
    pub column_int64: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> i64,
    pub column_double: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> f64,
    pub column_text: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> *const u8,
    pub column_blob: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> *const c_void,
    pub column_bytes: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> c_int,
    pub bind_int64: unsafe extern "C" fn(Sqlite3Stmt, c_int, i64) -> c_int,
    pub bind_double: unsafe extern "C" fn(Sqlite3Stmt, c_int, f64) -> c_int,
    pub bind_text:
        unsafe extern "C" fn(Sqlite3Stmt, c_int, *const c_char, c_int, *mut c_void) -> c_int,
    pub bind_blob:
        unsafe extern "C" fn(Sqlite3Stmt, c_int, *const c_void, c_int, *mut c_void) -> c_int,
    pub bind_null: unsafe extern "C" fn(Sqlite3Stmt, c_int) -> c_int,
    pub last_insert_rowid: unsafe extern "C" fn(Sqlite3) -> i64,
    pub changes: unsafe extern "C" fn(Sqlite3) -> c_int,
    pub libversion: unsafe extern "C" fn() -> *const c_char,
    pub compileoption_get: unsafe extern "C" fn(c_int) -> *const c_char,
}

static API: OnceLock<Result<Api, String>> = OnceLock::new();

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Resolve the SQLite function table once per process.
pub fn api() -> Result<&'static Api, String> {
    let cached = API.get_or_init(|| {
        let mut module = std::ptr::null_mut();
        for candidate in ["sqlite3.dll", "winsqlite3.dll"] {
            module = unsafe { LoadLibraryW(wide(candidate).as_ptr()) };
            if !module.is_null() {
                break;
            }
        }
        if module.is_null() {
            return Err("cannot load sqlite3.dll or winsqlite3.dll".to_string());
        }

        macro_rules! symbol {
            ($name:literal, $type:ty) => {{
                let raw = unsafe {
                    GetProcAddress(module, concat!($name, "\0").as_ptr() as *const c_char)
                };
                if raw.is_null() {
                    return Err(format!("missing SQLite export: {}", $name));
                }
                unsafe { std::mem::transmute::<*mut c_void, $type>(raw) }
            }};
        }

        Ok(Api {
            open_v2: symbol!("sqlite3_open_v2", unsafe extern "C" fn(_, _, _, _) -> c_int),
            close_v2: symbol!("sqlite3_close_v2", unsafe extern "C" fn(_) -> c_int),
            errmsg: symbol!("sqlite3_errmsg", unsafe extern "C" fn(_) -> *const c_char),
            busy_timeout: symbol!("sqlite3_busy_timeout", unsafe extern "C" fn(_, _) -> c_int),
            exec: symbol!("sqlite3_exec", unsafe extern "C" fn(_, _, _, _, _) -> c_int),
            prepare_v2: symbol!(
                "sqlite3_prepare_v2",
                unsafe extern "C" fn(_, _, _, _, _) -> c_int
            ),
            step: symbol!("sqlite3_step", unsafe extern "C" fn(_) -> c_int),
            finalize: symbol!("sqlite3_finalize", unsafe extern "C" fn(_) -> c_int),
            reset: symbol!("sqlite3_reset", unsafe extern "C" fn(_) -> c_int),
            clear_bindings: symbol!("sqlite3_clear_bindings", unsafe extern "C" fn(_) -> c_int),
            column_count: symbol!("sqlite3_column_count", unsafe extern "C" fn(_) -> c_int),
            column_name: symbol!(
                "sqlite3_column_name",
                unsafe extern "C" fn(_, _) -> *const c_char
            ),
            column_type: symbol!("sqlite3_column_type", unsafe extern "C" fn(_, _) -> c_int),
            column_int64: symbol!("sqlite3_column_int64", unsafe extern "C" fn(_, _) -> i64),
            column_double: symbol!("sqlite3_column_double", unsafe extern "C" fn(_, _) -> f64),
            column_text: symbol!(
                "sqlite3_column_text",
                unsafe extern "C" fn(_, _) -> *const u8
            ),
            column_blob: symbol!(
                "sqlite3_column_blob",
                unsafe extern "C" fn(_, _) -> *const c_void
            ),
            column_bytes: symbol!("sqlite3_column_bytes", unsafe extern "C" fn(_, _) -> c_int),
            bind_int64: symbol!("sqlite3_bind_int64", unsafe extern "C" fn(_, _, _) -> c_int),
            bind_double: symbol!(
                "sqlite3_bind_double",
                unsafe extern "C" fn(_, _, _) -> c_int
            ),
            bind_text: symbol!(
                "sqlite3_bind_text",
                unsafe extern "C" fn(_, _, _, _, _) -> c_int
            ),
            bind_blob: symbol!(
                "sqlite3_bind_blob",
                unsafe extern "C" fn(_, _, _, _, _) -> c_int
            ),
            bind_null: symbol!("sqlite3_bind_null", unsafe extern "C" fn(_, _) -> c_int),
            last_insert_rowid: symbol!("sqlite3_last_insert_rowid", unsafe extern "C" fn(_) -> i64),
            changes: symbol!("sqlite3_changes", unsafe extern "C" fn(_) -> c_int),
            libversion: symbol!(
                "sqlite3_libversion",
                unsafe extern "C" fn() -> *const c_char
            ),
            compileoption_get: symbol!(
                "sqlite3_compileoption_get",
                unsafe extern "C" fn(_) -> *const c_char
            ),
        })
    });

    match cached {
        Ok(api) => Ok(api),
        Err(message) => Err(message.clone()),
    }
}

/// Convenience helper: read a NUL-terminated UTF-8 string from a C pointer.
pub fn cstr(raw: *const c_char) -> String {
    if raw.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned()
}
