//! Read-only registry access for recognition. Every failure means "absent".

use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, FILETIME};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_ROUTINE_FLAGS, RRF_RT_REG_DWORD,
    RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    RegQueryInfoKeyW,
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Hive {
    CurrentUser,
    LocalMachine,
}

/// An open key, opened read-only and closed on drop.
pub(super) struct Key(HKEY);

impl Key {
    /// Opens a key under `HKEY_CURRENT_USER`.
    pub(super) fn open(subkey: &str) -> Option<Self> {
        Self::open_in(Hive::CurrentUser, subkey)
    }

    pub(super) fn open_in(hive: Hive, subkey: &str) -> Option<Self> {
        let root = match hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        };
        open_under(root, subkey)
    }

    pub(super) fn subkey(&self, name: &str) -> Option<Self> {
        open_under(self.0, name)
    }

    /// The names of this key's direct subkeys. Stops at the first error.
    pub(super) fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        // Key names are at most 255 characters.
        let mut buffer = [0u16; 256];
        for index in 0.. {
            let mut length = buffer.len() as u32;
            // SAFETY: `buffer` has `length` writable UTF-16 units, `length` is a valid in/out
            // pointer, and the optional class and time outputs are null.
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            if let Ok(name) = String::from_utf16(&buffer[..length as usize]) {
                names.push(name);
            }
        }
        names
    }

    /// When the key last changed, as a raw FILETIME. Adding or removing a subkey changes it.
    pub(super) fn last_write(&self) -> Option<u64> {
        let mut time = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        // SAFETY: every output but the last-write time is null, which the API allows, and
        // `time` is a valid out pointer.
        let status = unsafe {
            RegQueryInfoKeyW(
                self.0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut time,
            )
        };
        (status == ERROR_SUCCESS)
            .then(|| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime))
    }

    /// A string value, with `REG_EXPAND_SZ` expanded. Blank strings are `None`.
    pub(super) fn string(&self, name: &str) -> Option<String> {
        let data = self.value(name, RRF_RT_REG_SZ)?;
        let units = u16_units(&data);
        let text = units.split(|unit| *unit == 0).next()?;
        let text = String::from_utf16(text).ok()?;
        present_owned(text)
    }

    pub(super) fn multi_sz(&self, name: &str) -> Option<Vec<String>> {
        let data = self.value(name, RRF_RT_REG_MULTI_SZ)?;
        u16_units(&data)
            .split(|unit| *unit == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16)
            .collect::<Result<_, _>>()
            .ok()
    }

    pub(super) fn dword(&self, name: &str) -> Option<u32> {
        let data = self.value(name, RRF_RT_REG_DWORD)?;
        Some(u32::from_le_bytes(data.as_slice().try_into().ok()?))
    }

    /// Reads a value of the requested type. The first call asks for the size, the second
    /// fetches the data. Any failure, including a type mismatch, returns `None`.
    fn value(&self, name: &str, value_type: REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
        let name = wide(name);
        let mut size = 0u32;
        // SAFETY: a null data pointer asks only for the size, written to `size`.
        let status = unsafe {
            RegGetValueW(
                self.0,
                ptr::null(),
                name.as_ptr(),
                value_type,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        // SAFETY: `data` has `size` bytes and `size` holds that length, so RegGetValueW
        // writes within the buffer.
        let status = unsafe {
            RegGetValueW(
                self.0,
                ptr::null(),
                name.as_ptr(),
                value_type,
                ptr::null_mut(),
                data.as_mut_ptr().cast::<c_void>(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        data.truncate(size as usize);
        Some(data)
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful RegOpenKeyExW and is closed only here.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn open_under(parent: HKEY, subkey: &str) -> Option<Key> {
    let subkey = wide(subkey);
    let mut key = ptr::null_mut();
    // SAFETY: `parent` is a predefined root or a key this module opened and still holds,
    // `subkey` is a NUL-terminated UTF-16 string that outlives the call, and `key` is a valid
    // out pointer.
    let status = unsafe { RegOpenKeyExW(parent, subkey.as_ptr(), 0, KEY_READ, &mut key) };
    (status == ERROR_SUCCESS && !key.is_null()).then_some(Key(key))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Registry data is bytes; decode them without assuming alignment.
fn u16_units(data: &[u8]) -> Vec<u16> {
    data.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect()
}

fn present_owned(text: String) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
