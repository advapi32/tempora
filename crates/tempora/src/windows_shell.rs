use std::{
    marker::PhantomData,
    ptr::{NonNull, null_mut},
};
use windows_sys::Win32::{
    System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
    UI::Shell::{
        Common::ITEMIDLIST, SHFILEINFOW, SHGFI_FLAGS, SHGFI_PIDL, SHGetFileInfoW,
        SHParseDisplayName,
    },
};

/// COM initialized on this thread for as long as the value lives. Not `Send`: COM apartments
/// are per thread.
struct Apartment(PhantomData<*mut ()>);

impl Apartment {
    fn enter() -> Option<Self> {
        // SAFETY: The reserved argument is null. Each success is balanced by CoUninitialize in
        // Drop, on this same thread (the type is not Send).
        let status = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED as u32) };
        (status >= 0).then(|| Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Balances the successful CoInitializeEx in `enter`, on the same thread.
        unsafe { CoUninitialize() };
    }
}

/// An AppsFolder item, with COM initialized for the lifetime of its shell lookups.
pub struct AppsFolderItem {
    id: NonNull<ITEMIDLIST>,
    _apartment: Apartment,
}

impl AppsFolderItem {
    pub fn open(aumid: &str) -> Option<Self> {
        let apartment = Apartment::enter()?;
        let name: Vec<u16> = format!(r"shell:AppsFolder\{aumid}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut id = null_mut();
        // SAFETY: `name` is NUL-terminated UTF-16 and `id` a valid out pointer; COM is
        // initialized by `apartment`. The returned ID list is freed in Drop.
        let status =
            unsafe { SHParseDisplayName(name.as_ptr(), null_mut(), &mut id, 0, null_mut()) };
        let item = Self {
            id: NonNull::new(id)?,
            _apartment: apartment,
        };
        (status >= 0).then_some(item)
    }

    pub fn file_info(&self, flags: SHGFI_FLAGS) -> Option<SHFILEINFOW> {
        let mut info = SHFILEINFOW::default();
        // SAFETY: With SHGFI_PIDL the path argument is our live ID list, and `info` is sized as
        // passed.
        let found = unsafe {
            SHGetFileInfoW(
                self.id.as_ptr().cast(),
                0,
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                flags | SHGFI_PIDL,
            )
        };
        (found != 0).then_some(info)
    }
}

impl Drop for AppsFolderItem {
    fn drop(&mut self) {
        // SAFETY: SHParseDisplayName allocated the list with the COM allocator; freed once.
        unsafe { CoTaskMemFree(self.id.as_ptr().cast()) };
    }
}
