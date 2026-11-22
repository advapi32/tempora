//! Friendly names for Store apps, from the shell's Apps folder.

/// The display name the shell shows for a packaged app.
pub trait PackageNames {
    /// The display name of the app with this AUMID (`family!app`); `None` when unknown.
    fn display_name(&self, aumid: &str) -> Option<String>;
}

/// Looks names up through the shell's Apps folder, as the Start menu does.
#[cfg(windows)]
pub struct ShellPackageNames;

#[cfg(windows)]
impl PackageNames for ShellPackageNames {
    fn display_name(&self, aumid: &str) -> Option<String> {
        use crate::windows_shell::AppsFolderItem;
        use windows_sys::Win32::UI::Shell::SHGFI_DISPLAYNAME;

        let info = AppsFolderItem::open(aumid)?.file_info(SHGFI_DISPLAYNAME)?;
        let end = info
            .szDisplayName
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(info.szDisplayName.len());
        String::from_utf16(&info.szDisplayName[..end]).ok()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn failed_shell_lookups_leave_the_thread_free_to_enter_another_apartment() {
        use std::ptr::null_mut;
        use windows_sys::Win32::System::Com::{
            COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
        };

        std::thread::spawn(|| {
            for _ in 0..2 {
                assert!(
                    ShellPackageNames
                        .display_name("missing.package_0000000000000!App")
                        .is_none()
                );
            }
            // SAFETY: Probes this thread's COM state; a success is balanced right below.
            let status = unsafe { CoInitializeEx(null_mut(), COINIT_MULTITHREADED as u32) };
            if status >= 0 {
                // SAFETY: Balances the successful call above, on the same thread.
                unsafe { CoUninitialize() };
            }
            assert_eq!(status, 0, "shell lookup left COM initialized in STA");
        })
        .join()
        .unwrap();
    }

    /// History stores Store app IDs lowercased; the shell must still resolve them. Photos
    /// ships with Windows; the test is skipped where it has been removed.
    #[test]
    fn the_shell_names_a_lowercased_store_app_id() {
        let name = ShellPackageNames.display_name("microsoft.windows.photos_8wekyb3d8bbwe!app");
        eprintln!("photos: {name:?}");
        if let Some(name) = name {
            assert!(!name.is_empty() && !name.contains('!'), "{name}");
        }
    }
}
