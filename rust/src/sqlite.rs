//! Thin typed layer over the SQLite function table (`crate::sqlite_ffi`).
//!
//! Provides exactly what the journal needs: open (file or read-only URI),
//! `exec`/`execute`/`query` with positional parameters, typed values and an
//! explicit error type.  Everything else stays in the collector and store code.

use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;

use crate::sqlite_ffi as ffi;
use crate::sqlite_ffi::{Api, Sqlite3, Sqlite3Stmt};

pub use crate::sqlite_ffi::{
    OPEN_CREATE, OPEN_READONLY, OPEN_READWRITE, OPEN_URI, SQLITE_DONE, SQLITE_OK, SQLITE_ROW,
};

#[derive(Debug, Clone)]
pub struct SqliteError {
    pub code: c_int,
    pub message: String,
}

impl SqliteError {
    pub fn new(code: c_int, message: impl Into<String>) -> Self {
        SqliteError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SqliteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sqlite error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for SqliteError {}

/// A single column value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(number) => Some(*number),
            Value::Real(number) => Some(*number as i64),
            Value::Text(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        }
    }

    /// Textual form of the value; BLOB columns are decoded (UTF-8/UTF-16).
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Text(text) => Some(text.clone()),
            Value::Blob(bytes) => Some(decode_bytes(bytes)),
            Value::Int(number) => Some(number.to_string()),
            Value::Real(number) => Some(number.to_string()),
            Value::Null => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Text(value.to_string())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::Text(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Int(value)
    }
}

impl From<Option<String>> for Value {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(text) => Value::Text(text),
            None => Value::Null,
        }
    }
}

/// Decode a payload that may be stored as TEXT or BLOB (UTF-8 or UTF-16 with BOM).
pub fn decode_bytes(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && (bytes[0], bytes[1]) == (0xFF, 0xFE) {
        return decode_utf16(&bytes[2..], true);
    }
    if bytes.len() >= 2 && (bytes[0], bytes[1]) == (0xFE, 0xFF) {
        return decode_utf16(&bytes[2..], false);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

// ----------------------------------------------------------------------- rows
/// One result row with case-insensitive access by column name.
#[derive(Debug, Clone)]
pub struct Row {
    names: Vec<String>,
    values: Vec<Value>,
}

impl Row {
    /// Build a row from explicit column/value pairs (used by the test suite).
    pub fn from_pairs(names: Vec<String>, values: Vec<Value>) -> Row {
        Row { names, values }
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.names
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(name))
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.index_of(name).map(|index| &self.values[index])
    }

    pub fn text(&self, name: &str) -> Option<String> {
        self.get(name).and_then(|value| value.as_text())
    }

    pub fn required_text(&self, name: &str) -> String {
        self.text(name).unwrap_or_default()
    }

    /// Trimmed text, mapped to `None` when empty (mirrors the Python model).
    pub fn optional_text(&self, name: &str) -> Option<String> {
        self.text(name)
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(|value| value.as_i64())
    }

    pub fn bytes(&self, name: &str) -> Option<&[u8]> {
        match self.get(name) {
            Some(Value::Blob(bytes)) => Some(bytes),
            _ => None,
        }
    }

    pub fn column_names(&self) -> &[String] {
        &self.names
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

// ------------------------------------------------------------------ statements
/// Owned prepared statement: finalised when dropped.
struct Stmt {
    api: &'static Api,
    handle: Sqlite3Stmt,
}

impl Drop for Stmt {
    fn drop(&mut self) {
        unsafe { (self.api.finalize)(self.handle) };
    }
}

/// Percent-encode the characters that would break a `file:` URI.
fn to_uri(path: &Path, query: &str) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(raw.len() + 8);
    for character in raw.chars() {
        match character {
            ' ' => encoded.push_str("%20"),
            '%' => encoded.push_str("%25"),
            '?' => encoded.push_str("%3f"),
            '#' => encoded.push_str("%23"),
            other => encoded.push(other),
        }
    }
    format!("file:{encoded}?{query}")
}

// ------------------------------------------------------------------ connection
/// An open SQLite connection.  Closing happens on drop or via [`Db::close`].
pub struct Db {
    api: &'static Api,
    handle: Sqlite3,
    path: String,
}

impl Db {
    /// Open a database file; `flags` are SQLite open flags.
    pub fn open(path: &Path, flags: c_int) -> Result<Db, SqliteError> {
        let name = path.to_string_lossy().into_owned();
        Self::open_raw(&name, path, flags)
    }

    /// Open a database file read-write, creating it when missing.
    pub fn open_journal(path: &Path) -> Result<Db, SqliteError> {
        Db::open(path, OPEN_READWRITE | OPEN_CREATE)
    }

    /// Open the Windows notification database read-only through a `file:` URI -
    /// this is what allows access while the WPN service holds its locks.
    pub fn open_readonly_uri(path: &Path) -> Result<Db, SqliteError> {
        let uri = to_uri(path, "mode=ro");
        Self::open_raw(&uri, path, OPEN_READONLY | OPEN_URI)
    }

    fn open_raw(name: &str, display: &Path, flags: c_int) -> Result<Db, SqliteError> {
        let api = ffi::api().map_err(|message| SqliteError::new(-1, message))?;
        let c_name = CString::new(name.as_bytes())
            .map_err(|_| SqliteError::new(-1, "the database path contains a NUL byte"))?;
        let mut handle: Sqlite3 = std::ptr::null_mut();
        let code = unsafe {
            (api.open_v2)(
                c_name.as_ptr(),
                &mut handle,
                flags | ffi::OPEN_FULLMUTEX,
                std::ptr::null(),
            )
        };
        if code != SQLITE_OK || handle.is_null() {
            let message = if handle.is_null() {
                format!("cannot open {} (sqlite error {})", display.display(), code)
            } else {
                error_message(api, handle, code)
            };
            if !handle.is_null() {
                unsafe { (api.close_v2)(handle) };
            }
            return Err(SqliteError::new(code, message));
        }
        unsafe { (api.busy_timeout)(handle, 10_000) };
        Ok(Db {
            api,
            handle,
            path: display.to_string_lossy().into_owned(),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Execute one or more statements without parameters (DDL, PRAGMA).
    pub fn exec(&self, sql: &str) -> Result<(), SqliteError> {
        let c_sql = CString::new(sql)
            .map_err(|_| SqliteError::new(-1, "the SQL text contains a NUL byte"))?;
        let code = unsafe {
            (self.api.exec)(
                self.handle,
                c_sql.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if code == SQLITE_OK {
            Ok(())
        } else {
            Err(self.error(code))
        }
    }

    /// Execute a parameterised statement; returns the number of changed rows.
    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<usize, SqliteError> {
        let stmt = self.prepare(sql, params)?;
        let code = unsafe { (self.api.step)(stmt.handle) };
        if code != SQLITE_DONE && code != SQLITE_ROW {
            return Err(self.error(code));
        }
        Ok(unsafe { (self.api.changes)(self.handle) }.max(0) as usize)
    }

    /// Execute a query and map every row.
    pub fn query<T>(
        &self,
        sql: &str,
        params: &[Value],
        mut map: impl FnMut(&Row) -> T,
    ) -> Result<Vec<T>, SqliteError> {
        let stmt = self.prepare(sql, params)?;
        let names = self.column_names(&stmt);
        let count = names.len();
        let mut rows = Vec::new();
        loop {
            let code = unsafe { (self.api.step)(stmt.handle) };
            if code == SQLITE_ROW {
                let values = (0..count)
                    .map(|index| column_value(self.api, stmt.handle, index as c_int))
                    .collect();
                let row = Row {
                    names: names.clone(),
                    values,
                };
                rows.push(map(&row));
            } else if code == SQLITE_DONE {
                break;
            } else {
                return Err(self.error(code));
            }
        }
        Ok(rows)
    }

    /// Execute a query and return the first row (if any).
    pub fn query_row(&self, sql: &str, params: &[Value]) -> Result<Option<Row>, SqliteError> {
        let rows = self.query(sql, params, |row| Row {
            names: row.names.clone(),
            values: row.values.clone(),
        })?;
        Ok(rows.into_iter().next())
    }

    /// The integer in the first column of the first row (0 when no rows).
    pub fn query_int(&self, sql: &str, params: &[Value]) -> Result<i64, SqliteError> {
        Ok(self
            .query_row(sql, params)?
            .and_then(|row| row.values.first().and_then(|value| value.as_i64()))
            .unwrap_or(0))
    }

    /// The text in the first column of the first row (`None` when no rows).
    pub fn query_text(&self, sql: &str, params: &[Value]) -> Result<Option<String>, SqliteError> {
        Ok(self
            .query_row(sql, params)?
            .and_then(|row| row.values.first().and_then(|value| value.as_text())))
    }

    pub fn last_insert_rowid(&self) -> i64 {
        unsafe { (self.api.last_insert_rowid)(self.handle) }
    }

    /// Close the connection explicitly.
    pub fn close(self) {}
}

impl Drop for Db {
    fn drop(&mut self) {
        unsafe { (self.api.close_v2)(self.handle) };
    }
}

impl Db {
    /// Run `work` inside a transaction, rolling back on error.
    pub fn in_transaction(
        &self,
        work: impl FnOnce() -> Result<(), SqliteError>,
    ) -> Result<(), SqliteError> {
        self.exec("BEGIN IMMEDIATE")?;
        match work() {
            Ok(()) => {
                self.exec("COMMIT")?;
                Ok(())
            }
            Err(error) => {
                let _ = self.exec("ROLLBACK");
                Err(error)
            }
        }
    }

    fn column_names(&self, stmt: &Stmt) -> Vec<String> {
        let count = unsafe { (self.api.column_count)(stmt.handle) }.max(0) as usize;
        (0..count)
            .map(|index| {
                let raw = unsafe { (self.api.column_name)(stmt.handle, index as c_int) };
                ffi::cstr(raw)
            })
            .collect()
    }

    fn prepare(&self, sql: &str, params: &[Value]) -> Result<Stmt, SqliteError> {
        let c_sql = CString::new(sql)
            .map_err(|_| SqliteError::new(-1, "the SQL text contains a NUL byte"))?;
        let mut handle: Sqlite3Stmt = std::ptr::null_mut();
        let code = unsafe {
            (self.api.prepare_v2)(
                self.handle,
                c_sql.as_ptr(),
                -1,
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        if code != SQLITE_OK || handle.is_null() {
            return Err(self.error(code));
        }
        let stmt = Stmt {
            api: self.api,
            handle,
        };
        unsafe { (self.api.reset)(stmt.handle) };
        for (index, value) in params.iter().enumerate() {
            let position = (index + 1) as c_int;
            let code = unsafe {
                match value {
                    Value::Null => (self.api.bind_null)(stmt.handle, position),
                    Value::Int(number) => (self.api.bind_int64)(stmt.handle, position, *number),
                    Value::Real(number) => (self.api.bind_double)(stmt.handle, position, *number),
                    Value::Text(text) => {
                        let bytes = text.as_bytes();
                        (self.api.bind_text)(
                            stmt.handle,
                            position,
                            bytes.as_ptr() as *const c_char,
                            bytes.len() as c_int,
                            std::ptr::null_mut(),
                        )
                    }
                    Value::Blob(bytes) => (self.api.bind_blob)(
                        stmt.handle,
                        position,
                        bytes.as_ptr() as *const c_void,
                        bytes.len() as c_int,
                        std::ptr::null_mut(),
                    ),
                }
            };
            if code != SQLITE_OK {
                return Err(self.error(code));
            }
        }
        Ok(stmt)
    }

    fn error(&self, code: c_int) -> SqliteError {
        SqliteError::new(code, error_message(self.api, self.handle, code))
    }
}

fn error_message(api: &Api, handle: Sqlite3, code: c_int) -> String {
    if handle.is_null() {
        return format!("sqlite error {}", code);
    }
    ffi::cstr(unsafe { (api.errmsg)(handle) })
}

fn column_value(api: &Api, stmt: Sqlite3Stmt, index: c_int) -> Value {
    unsafe {
        match (api.column_type)(stmt, index) {
            ffi::SQLITE_TEXT => {
                let pointer = (api.column_text)(stmt, index);
                if pointer.is_null() {
                    Value::Null
                } else {
                    let length = (api.column_bytes)(stmt, index).max(0) as usize;
                    let slice = std::slice::from_raw_parts(pointer, length);
                    Value::Text(String::from_utf8_lossy(slice).into_owned())
                }
            }
            ffi::SQLITE_BLOB => {
                let pointer = (api.column_blob)(stmt, index) as *const u8;
                let length = (api.column_bytes)(stmt, index).max(0) as usize;
                if pointer.is_null() || length == 0 {
                    Value::Blob(Vec::new())
                } else {
                    Value::Blob(std::slice::from_raw_parts(pointer, length).to_vec())
                }
            }
            ffi::SQLITE_FLOAT => Value::Real((api.column_double)(stmt, index)),
            ffi::SQLITE_INTEGER => Value::Int((api.column_int64)(stmt, index)),
            _ => Value::Null,
        }
    }
}

// ----------------------------------------------------------------- diagnostics
/// Version of the loaded SQLite library.
pub fn library_version() -> Result<String, SqliteError> {
    let api = ffi::api().map_err(|message| SqliteError::new(-1, message))?;
    Ok(ffi::cstr(unsafe { (api.libversion)() }))
}

/// Compile-time options reported by the loaded SQLite library.
pub fn compile_options() -> Result<Vec<String>, SqliteError> {
    let api = ffi::api().map_err(|message| SqliteError::new(-1, message))?;
    let mut options = Vec::new();
    let mut index = 0;
    loop {
        let raw = unsafe { (api.compileoption_get)(index) };
        if raw.is_null() {
            break;
        }
        options.push(ffi::cstr(raw));
        index += 1;
    }
    Ok(options)
}

/// True when the loaded SQLite library has the FTS5 extension the journal needs.
pub fn has_fts5() -> bool {
    compile_options()
        .map(|options| options.iter().any(|option| option.contains("ENABLE_FTS5")))
        .unwrap_or(false)
}
