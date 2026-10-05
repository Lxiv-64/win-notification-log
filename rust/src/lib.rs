//! Win Notification Log (native) - shared library.
//!
//! The crate is split into a library plus two binaries:
//!
//! * `wnl.exe`     - graphical build (`src/bin/wnl.rs`), the application itself
//! * `wnl-cli.exe` - console diagnostics/scripting (`src/bin/wnl-cli.rs`)
//!
//! Layout of the modules mirrors the flow of a notification: it is read from a
//! Windows source (`wpndb`, `uns`), normalised into `model::NotificationRecord`,
//! which is then written to the SQLite journal in `store`.
//!
//! Dependencies are intentionally minimal and pure Rust: SQLite is taken from the
//! operating system (`winsqlite3.dll`, see `sqlite_ffi`), the user interface is
//! built directly on Win32 (`win32`), so no C toolchain is required to build.

#![allow(dead_code)] // stages of the port land one by one

#[macro_use]
pub mod lang;
pub mod appicon;
pub mod appname;
pub mod autostart;
pub mod cli;
pub mod config;
pub mod confirm;
pub mod detail;
pub mod export;
pub mod feed;
pub mod imaging;
pub mod model;
pub mod packages;
pub mod payload;
pub mod purge;
pub mod rules;
pub mod service;
pub mod settings;
pub mod settings_ui;
pub mod sqlite;
pub mod sqlite_ffi;
pub mod storage;
pub mod store;
pub mod style;
pub mod theme;
pub mod time;
pub mod ui;
pub mod uns;
pub mod view;
pub mod win32;
pub mod wpndb;

/// Version reported by `--version`, the window title and the diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
