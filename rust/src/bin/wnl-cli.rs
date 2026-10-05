//! Console companion of the graphical build (diagnostics and scripting).
//!
//! ```text
//! wnl-cli doctor | import | search | stats | selftest
//! ```

fn main() {
    std::process::exit(win_notification_log_native::cli::run());
}
