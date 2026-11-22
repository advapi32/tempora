//! Product names from a file's version resource, read from the file on disk.

use std::path::Path;

/// The name fields of a file's version resource. Empty values are `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionInfo {
    pub company: Option<String>,
    pub product: Option<String>,
    pub description: Option<String>,
}

pub trait VersionSource {
    /// The version resource of the file at `path`; `None` when it has none or can't be read.
    fn read(&self, path: &Path) -> Option<VersionInfo>;
}

/// Reads version resources with `GetFileVersionInfoW`. Only the file on disk is read.
#[cfg(windows)]
pub struct FileVersionSource;

/// Larger than any real version resource; a guard against absurd sizes from a bad file.
#[cfg(windows)]
const MAX_BLOCK_BYTES: u32 = 16 * 1024 * 1024;

/// Language and code page used by most executables, tried when the translation table is absent.
#[cfg(windows)]
const FALLBACK_TRANSLATIONS: [(u16, u16); 2] = [(0x0409, 0x04b0), (0x0409, 0x04e4)];

#[cfg(windows)]
impl VersionSource for FileVersionSource {
    fn read(&self, path: &Path) -> Option<VersionInfo> {
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileVersionInfoSizeW, GetFileVersionInfoW,
        };

        crate::local_fs::metadata(path).ok()?;
        let file = wide(path.as_os_str());
        let mut handle = 0u32;
        // SAFETY: `file` is a NUL-terminated UTF-16 path that outlives the call, and `handle`
        // is a valid out pointer.
        let size = unsafe { GetFileVersionInfoSizeW(file.as_ptr(), &mut handle) };
        if size == 0 || size > MAX_BLOCK_BYTES {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        // SAFETY: `block` has exactly `size` writable bytes, and the API writes at most that many.
        let ok = unsafe { GetFileVersionInfoW(file.as_ptr(), 0, size, block.as_mut_ptr().cast()) };
        if ok == 0 {
            return None;
        }
        let translations = translations(&block);
        let field = |name: &str| {
            translations.iter().find_map(|(language, codepage)| {
                let sub_block = format!(r"\StringFileInfo\{language:04x}{codepage:04x}\{name}");
                query_string(&block, &sub_block)
            })
        };
        let info = VersionInfo {
            company: field("CompanyName"),
            product: field("ProductName"),
            description: field("FileDescription"),
        };
        (info != VersionInfo::default()).then_some(info)
    }
}

/// The translation table's first (language, code page) pair, followed by the fallbacks.
#[cfg(windows)]
fn translations(block: &[u8]) -> Vec<(u16, u16)> {
    let mut pairs = Vec::with_capacity(3);
    if let Some(bytes) = query(block, r"\VarFileInfo\Translation", 1)
        && bytes.len() >= 4
    {
        pairs.push((
            u16::from_le_bytes([bytes[0], bytes[1]]),
            u16::from_le_bytes([bytes[2], bytes[3]]),
        ));
    }
    pairs.extend(FALLBACK_TRANSLATIONS);
    pairs
}

/// A string value, trimmed. Empty values are `None`.
#[cfg(windows)]
fn query_string(block: &[u8], sub_block: &str) -> Option<String> {
    // A string value's length counts UTF-16 units, including the terminating NUL.
    let bytes = query(block, sub_block, 2)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let text = units.split(|unit| *unit == 0).next()?;
    let text = String::from_utf16(text).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The bytes of the value at `sub_block`, given the size of one counted unit in bytes. The
/// slice always lies inside `block`.
#[cfg(windows)]
fn query<'block>(block: &'block [u8], sub_block: &str, unit_bytes: usize) -> Option<&'block [u8]> {
    use windows_sys::Win32::Storage::FileSystem::VerQueryValueW;

    let sub_block = wide(std::ffi::OsStr::new(sub_block));
    let mut value: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut length = 0u32;
    // SAFETY: `block` is the buffer filled by GetFileVersionInfoW and outlives this call;
    // `sub_block` is NUL-terminated; both out pointers are valid.
    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            sub_block.as_ptr(),
            &mut value,
            &mut length,
        )
    };
    if ok == 0 || value.is_null() {
        return None;
    }
    // The API points into `block`. Check that before reading through the pointer.
    let start = (value as usize).checked_sub(block.as_ptr() as usize)?;
    let byte_length = usize::try_from(length).ok()?.checked_mul(unit_bytes)?;
    let end = start.checked_add(byte_length)?;
    block.get(start..end)
}

#[cfg(windows)]
fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::VersionSource;

    #[cfg(windows)]
    #[test]
    #[ignore = "reads the version resource of the running test binary"]
    fn reading_a_real_file_does_not_panic() {
        let exe = std::env::current_exe().unwrap();
        let _ = super::FileVersionSource.read(&exe);
    }
}
