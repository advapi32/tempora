//! File access for automatic recognition. Only direct local volumes and ordinary paths are used.

use std::{
    fs::{self, Metadata, ReadDir},
    io,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, QueryDosDeviceW};

pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let denied = || io::Error::new(io::ErrorKind::PermissionDenied, "Not a direct local path");
    let mut components = path.as_ref().components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(denied());
    };
    let letter = match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
        _ => return Err(denied()),
    };
    if components.next() != Some(Component::RootDir) || !is_local_drive(letter) {
        return Err(denied());
    }

    let mut current = PathBuf::from(prefix.as_os_str());
    current.push(r"\");
    let mut metadata = fs::symlink_metadata(&current)?;
    for component in components {
        let Component::Normal(name) = component else {
            return Err(denied());
        };
        if name.encode_wide().any(|unit| matches!(unit, 0 | 47 | 58)) {
            return Err(denied());
        }
        current.push(name);
        // Inspect the component itself before descending into it, including junctions and
        // mount points whose directory entries may look ordinary to higher-level APIs.
        metadata = fs::symlink_metadata(&current)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(denied());
        }
    }
    Ok(metadata)
}

fn is_local_drive(letter: u8) -> bool {
    let drive = [u16::from(letter), u16::from(b':'), 0];
    let mut target = [0u16; 1024];
    // SAFETY: `drive` is NUL-terminated, and the buffer and its length describe `target`.
    let count =
        unsafe { QueryDosDeviceW(drive.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
    count != 0 && is_local_device(&target)
}

fn is_local_device(target: &[u16]) -> bool {
    let device = target.split(|&unit| unit == 0).next().unwrap_or_default();
    let device = String::from_utf16_lossy(device);
    [
        r"\Device\HarddiskVolume",
        r"\Device\CdRom",
        r"\Device\Floppy",
    ]
    .iter()
    .any(|prefix| {
        device.strip_prefix(prefix).is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
    })
}

pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    metadata(path.as_ref())?;
    fs::read_dir(path)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    metadata(path.as_ref())?;
    fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_aliases_and_remote_devices_are_rejected_without_opening_them() {
        let is_local = |device: &str| is_local_device(&device.encode_utf16().collect::<Vec<_>>());
        assert!(is_local(r"\Device\HarddiskVolume12"));
        assert!(!is_local(r"\Device\Mup"));
        assert!(!is_local(r"\??\C:\redirected"));
        for path in [
            r"\\server\share\a.exe",
            r"\\?\UNC\server\share\a.exe",
            r"C:a.exe",
        ] {
            assert_eq!(
                metadata(path).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        assert!(metadata(std::env::current_exe().unwrap()).is_ok());
    }
}
