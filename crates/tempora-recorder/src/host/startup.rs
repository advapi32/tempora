//! Start at sign-in: the recorder's value under the current user's `Run` key.

use super::{APP_NAME, tray::wide};
use anyhow::{Context, Result};
use std::{io, os::windows::ffi::OsStrExt};
use windows_sys::Win32::{
    Foundation::ERROR_FILE_NOT_FOUND,
    System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW},
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Adds or removes the `Run` value that starts this executable with `--sign-in`.
pub(super) fn configure(enabled: bool) -> Result<()> {
    let key = wide(RUN_KEY);
    let name = wide(APP_NAME);
    let code = if enabled {
        let command = sign_in_command(&std::env::current_exe()?);
        // SAFETY: Key and value names are NUL-terminated UTF-16. The data is a NUL-terminated
        // REG_SZ, and the byte count covers it including the terminator.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * size_of::<u16>()) as u32,
            )
        }
    } else {
        // SAFETY: Key and value names are NUL-terminated UTF-16.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) }
    };
    // Removing a value that isn't there is success.
    if code != 0 && (enabled || code != ERROR_FILE_NOT_FOUND) {
        return Err(io::Error::from_raw_os_error(code as i32))
            .context("Could not update start at sign-in");
    }
    Ok(())
}

/// `"<executable>" --sign-in`, NUL-terminated UTF-16.
fn sign_in_command(executable: &std::path::Path) -> Vec<u16> {
    let mut command: Vec<u16> = "\"".encode_utf16().collect();
    command.extend(executable.as_os_str().encode_wide());
    command.extend("\" --sign-in\0".encode_utf16());
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_quotes_the_path_and_ends_in_a_terminator() {
        let command = sign_in_command(std::path::Path::new(r"C:\Program Files\Tempora\x.exe"));
        let text = String::from_utf16(&command).unwrap();
        assert_eq!(text, "\"C:\\Program Files\\Tempora\\x.exe\" --sign-in\0");
    }
}
