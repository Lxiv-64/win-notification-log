//! Win Notification Log (native) - graphical build.
//!
//! Double-clicking the executable opens the journal window; the collector runs in
//! the same process while the window is open (pressing *Import now* or waiting for
//! the periodic refresh).  Console diagnostics live in `wnl-cli.exe`.

// No console window when started from Explorer.
#![windows_subsystem = "windows"]

fn main() {
    std::process::exit(win_notification_log_native::ui::run());
}
