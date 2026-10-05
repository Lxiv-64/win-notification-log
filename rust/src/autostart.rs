//! Autostart of the background collector.
//!
//! The entry lives in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, the
//! per user key every Windows application uses: no administrator rights, no
//! service registration, and the user can see (and remove) the entry in the
//! Task Manager's *Startup* tab.

use std::ffi::c_void;
use std::path::Path;

use crate::win32::wide;

const HKEY_CURRENT_USER: usize = 0x8000_0001;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_OPTION_NON_VOLATILE: u32 = 0;
const REG_SZ: u32 = 1;
const ERROR_SUCCESS: i32 = 0;
const ERROR_FILE_NOT_FOUND: i32 = 2;

/// Key holding the entries that run at sign-in.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Name of our entry.
pub const VALUE_NAME: &str = "WinNotificationLog";
/// Argument that starts the application in the notification area only.
pub const BACKGROUND_FLAG: &str = "--background";

#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        key: usize,
        sub_key: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        desired: u32,
        attributes: *mut c_void,
        result: *mut usize,
        disposition: *mut u32,
    ) -> i32;
    fn RegOpenKeyExW(
        key: usize,
        sub_key: *const u16,
        options: u32,
        desired: u32,
        result: *mut usize,
    ) -> i32;
    fn RegSetValueExW(
        key: usize,
        value_name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        size: u32,
    ) -> i32;
    fn RegQueryValueExW(
        key: usize,
        value_name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegDeleteValueW(key: usize, value_name: *const u16) -> i32;
    fn RegCloseKey(key: usize) -> i32;
}

/// Command line a Run entry has to contain (quoting included).
pub fn command_line(exe: &Path, flag: &str) -> String {
    format!("\"{}\" {flag}", exe.display())
}

/// True when the registered command starts exactly this executable.
///
/// Only the executable is compared: the user may have chosen a different flag,
/// and rewriting the entry on every start would be rude.
pub fn command_matches(registered: Option<&str>, exe: &Path) -> bool {
    let Some(registered) = registered else {
        return false;
    };
    let registered = registered.trim();
    let needle = format!("\"{}\"", exe.display());
    if registered.eq_ignore_ascii_case(&needle) || registered.starts_with(&format!("{needle} ")) {
        return true;
    }
    // Tolerate an unquoted path (written by hand or by an older version).
    registered.eq_ignore_ascii_case(&exe.display().to_string())
}

/// Read the registered command line, if any.
pub fn registered_command() -> Option<String> {
    let sub_key = wide(RUN_KEY);
    let mut key: usize = 0;
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            sub_key.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if opened != ERROR_SUCCESS {
        return None;
    }
    let name = wide(VALUE_NAME);
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut u8,
            &mut size,
        )
    };
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS {
        return None;
    }
    let length = ((size as usize) / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..length])
        .trim_end_matches('\0')
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// True when the application is registered to start at sign-in.
pub fn is_enabled(exe: &Path) -> bool {
    command_matches(registered_command().as_deref(), exe)
}

/// Add, update or remove the Run entry.
pub fn set_enabled(exe: &Path, enabled: bool) -> Result<(), String> {
    let sub_key = wide(RUN_KEY);
    let mut key: usize = 0;
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub_key.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            std::ptr::null_mut(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "could not open the autostart key (code {status})"
        ));
    }
    let name = wide(VALUE_NAME);
    let status = if enabled {
        let mut command: Vec<u16> = command_line(exe, BACKGROUND_FLAG).encode_utf16().collect();
        command.push(0);
        let size = (command.len() * 2) as u32;
        unsafe {
            #[cfg(test)]
            mod tests {
                use super::*;
                use std::path::PathBuf;

                #[test]
                fn command_line_is_quoted() {
                    let exe = PathBuf::from(r"C:\Program Files\Журнал уведомлений\wnl.exe");
                    assert_eq!(
                        command_line(&exe, BACKGROUND_FLAG),
                        r#""C:\Program Files\Журнал уведомлений\wnl.exe" --background"#
                    );
                }

                #[test]
                fn command_matching_tolerates_details() {
                    let exe = PathBuf::from(r"C:\tools\wnl.exe");
                    assert!(command_matches(
                        Some(r#""C:\tools\wnl.exe" --background"#),
                        &exe
                    ));
                    assert!(command_matches(
                        Some(r#""C:\TOOLS\WNL.EXE" --background"#),
                        &exe
                    ));
                    assert!(command_matches(Some(r#""C:\tools\wnl.exe""#), &exe));
                    assert!(command_matches(Some(r"C:\tools\wnl.exe"), &exe));
                    assert!(!command_matches(
                        Some(r#""C:\other\wnl.exe" --background"#),
                        &exe
                    ));
                    assert!(!command_matches(None, &exe));
                    assert!(!command_matches(Some(""), &exe));
                }
            }

            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr() as *const u8,
                size,
            )
        }
    } else {
        let result = unsafe { RegDeleteValueW(key, name.as_ptr()) };
        if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
            ERROR_SUCCESS
        } else {
            result
        }
    };
    unsafe { RegCloseKey(key) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("не удалось изменить автозапуск (код {status})"))
    }
}
