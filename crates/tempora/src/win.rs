//! Safe wrappers over the Windows APIs the recorder observes with, plus the few window actions
//! it takes on the dashboard it launched.
//!
//! Each function owns its `unsafe` call and states why it is sound; callers stay in safe Rust.
//! Handles that need closing are `OwnedHandle`s, so they close on every path.

use std::{
    ffi::OsString,
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_FILES, FILETIME, HWND, INVALID_HANDLE_VALUE, RECT,
    },
    Graphics::{
        Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    },
    Storage::Packaging::Appx::{GetApplicationUserModelId, GetPackageFamilyName},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        RemoteDesktop::{
            ProcessIdToSessionId, WTS_SESSIONSTATE_UNLOCK, WTSActive, WTSFreeMemory, WTSINFOEXW,
            WTSQuerySessionInformationW, WTSSessionInfoEx,
        },
        SystemInformation::GetTickCount64,
        Threading::{
            GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
        WindowsAndMessaging::{
            AllowSetForegroundWindow, EnumChildWindows, EnumWindows, GW_OWNER, GWL_EXSTYLE,
            GWL_STYLE, GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow,
            GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
            IsWindowVisible, PostMessageW, SW_RESTORE, SetForegroundWindow, ShowWindowAsync,
            WM_CLOSE,
        },
    },
};

/// A top-level or child window. Windows validates the handle on every call, so a window that
/// closed in the meantime makes the queries fail rather than misbehave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window(HWND);

impl Window {
    pub fn raw(self) -> HWND {
        self.0
    }

    /// The window in the foreground, if any.
    pub fn foreground() -> Option<Self> {
        // SAFETY: No arguments; returns a possibly null window handle.
        Self::new(unsafe { GetForegroundWindow() })
    }

    pub fn desktop() -> Option<Self> {
        // SAFETY: No arguments; returns the desktop window handle.
        Self::new(unsafe { GetDesktopWindow() })
    }

    /// The shell's desktop window (Explorer's), if a shell is running.
    pub fn shell() -> Option<Self> {
        // SAFETY: No arguments; returns a possibly null window handle.
        Self::new(unsafe { GetShellWindow() })
    }

    fn new(handle: HWND) -> Option<Self> {
        (!handle.is_null()).then_some(Self(handle))
    }

    /// All top-level windows, in Z order.
    pub fn top_level() -> Vec<Self> {
        let mut windows = Vec::new();
        // SAFETY: `collect` only runs during this call and receives the pointer to `windows`,
        // which outlives the enumeration.
        unsafe { EnumWindows(Some(collect), (&raw mut windows) as isize) };
        windows
    }

    /// The window's descendants.
    pub fn children(self) -> Vec<Self> {
        let mut windows = Vec::new();
        // SAFETY: As in `top_level`; the vector outlives the enumeration.
        unsafe { EnumChildWindows(self.0, Some(collect), (&raw mut windows) as isize) };
        windows
    }

    /// The ID of the process that created the window.
    pub fn process_id(self) -> Option<u32> {
        let mut pid = 0;
        // SAFETY: `pid` is a valid out pointer for the call.
        let thread = unsafe { GetWindowThreadProcessId(self.0, &mut pid) };
        (thread != 0 && pid != 0).then_some(pid)
    }

    /// Whether the window's class name is exactly `expected`.
    pub fn class_is(self, expected: &str) -> bool {
        let mut class = [0u16; 64];
        // SAFETY: The buffer and its length in UTF-16 units describe the same array.
        let length = unsafe { GetClassNameW(self.0, class.as_mut_ptr(), class.len() as i32) };
        length > 0
            && class[..length as usize]
                .iter()
                .copied()
                .eq(expected.encode_utf16())
    }

    pub fn is_visible(self) -> bool {
        // SAFETY: Reads the window's state; the handle is validated by Windows.
        unsafe { IsWindowVisible(self.0) != 0 }
    }

    pub fn is_minimized(self) -> bool {
        // SAFETY: Reads the window's state; the handle is validated by Windows.
        unsafe { IsIconic(self.0) != 0 }
    }

    pub fn style(self) -> u32 {
        // SAFETY: Reads a documented window field; the handle is validated by Windows.
        unsafe { GetWindowLongW(self.0, GWL_STYLE) as u32 }
    }

    pub fn extended_style(self) -> u32 {
        // SAFETY: Reads a documented window field; the handle is validated by Windows.
        unsafe { GetWindowLongW(self.0, GWL_EXSTYLE) as u32 }
    }

    /// Whether another window owns this one (dialogs, tool palettes).
    pub fn has_owner(self) -> bool {
        // SAFETY: Reads the window's owner; the handle is validated by Windows.
        !unsafe { GetWindow(self.0, GW_OWNER) }.is_null()
    }

    /// Whether the window is cloaked: shown to the system but hidden by DWM, such as a window
    /// on another virtual desktop or a suspended Store app.
    pub fn is_cloaked(self) -> bool {
        let mut cloaked = 0u32;
        // SAFETY: The attribute is a `u32`, and the pointer and size describe `cloaked`.
        let status = unsafe {
            DwmGetWindowAttribute(
                self.0,
                DWMWA_CLOAKED as u32,
                (&raw mut cloaked).cast(),
                size_of::<u32>() as u32,
            )
        };
        status >= 0 && cloaked != 0
    }

    /// The window's bounds in screen coordinates.
    pub fn rect(self) -> Option<RECT> {
        let mut bounds = RECT::default();
        // SAFETY: `bounds` is a valid out pointer for the call.
        (unsafe { GetWindowRect(self.0, &mut bounds) } != 0).then_some(bounds)
    }

    /// Restores the window if minimized and brings it to the front.
    pub fn activate(self) {
        // SAFETY: Window state changes on a handle Windows validates; failures are harmless.
        unsafe {
            if IsIconic(self.0) != 0 {
                ShowWindowAsync(self.0, SW_RESTORE);
            }
            SetForegroundWindow(self.0);
        }
    }

    /// Asks the window to close, as its close button would. Returns whether the request was sent.
    pub fn request_close(self) -> bool {
        // SAFETY: Posts a parameterless message; the handle is validated by Windows.
        unsafe { PostMessageW(self.0, WM_CLOSE, 0, 0) != 0 }
    }

    /// The bounds of the monitor the window is mostly on.
    pub fn monitor_rect(self) -> Option<RECT> {
        // SAFETY: With MONITOR_DEFAULTTONEAREST the call returns a monitor for any window.
        let monitor = unsafe { MonitorFromWindow(self.0, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `info` is sized by `cbSize` as the call requires.
        (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(info.rcMonitor)
    }
}

/// Enumeration callback: `data` is the `*mut Vec<Window>` passed by `top_level` or `children`.
unsafe extern "system" fn collect(window: HWND, data: isize) -> i32 {
    // SAFETY: The enumerating function passes back the pointer we gave it, which points to a
    // live `Vec<Window>` that nothing else touches during the enumeration.
    let windows = unsafe { &mut *(data as *mut Vec<Window>) };
    windows.push(Window(window));
    1
}

/// Lets a process we started bring its window to the front, which Windows otherwise blocks.
pub fn allow_set_foreground(pid: u32) {
    // SAFETY: Takes a process ID; failure only means the window may open behind others.
    unsafe { AllowSetForegroundWindow(pid) };
}

pub fn current_process_id() -> u32 {
    // SAFETY: No arguments.
    unsafe { GetCurrentProcessId() }
}

/// The Windows session a process runs in.
pub fn process_session(pid: u32) -> Option<u32> {
    let mut session = 0;
    // SAFETY: `session` is a valid out pointer for the call.
    (unsafe { ProcessIdToSessionId(pid, &mut session) } != 0).then_some(session)
}

/// Whether a session is the active console or remote session and unlocked.
pub fn session_is_active_and_unlocked(session: u32) -> bool {
    let mut buffer = null_mut();
    let mut bytes = 0;
    // SAFETY: The out pointers are valid. On success Windows allocates `buffer`, freed below.
    let result = unsafe {
        WTSQuerySessionInformationW(
            null_mut(),
            session,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes,
        )
    };
    if result == 0 || buffer.is_null() {
        return false;
    }
    let active = (bytes as usize >= size_of::<WTSINFOEXW>()) && {
        // SAFETY: The buffer holds at least one WTSINFOEXW (checked above). It need not have
        // Rust's alignment for the structure, hence the unaligned read.
        let info = unsafe { buffer.cast::<WTSINFOEXW>().read_unaligned() };
        info.Level == 1 && {
            // SAFETY: Level 1 means the union holds WTSInfoExLevel1.
            let data = unsafe { info.Data.WTSInfoExLevel1 };
            data.SessionId == session
                && data.SessionState == WTSActive
                && data.SessionFlags == WTS_SESSIONSTATE_UNLOCK as i32
        }
    };
    // SAFETY: `buffer` was allocated by WTSQuerySessionInformationW and is freed exactly once.
    unsafe { WTSFreeMemory(buffer.cast()) };
    active
}

/// The IDs of all running processes.
pub fn process_ids() -> io::Result<Vec<u32>> {
    // SAFETY: Takes a snapshot of the process list; the handle is checked below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: A valid snapshot handle that we own and close once, on drop.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut pids = Vec::new();
    // SAFETY: `entry` is sized by `dwSize` as the call requires.
    let mut next = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    while next != 0 {
        pids.push(entry.th32ProcessID);
        // SAFETY: As above.
        next = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
        return Err(error);
    }
    Ok(pids)
}

/// A process opened for limited queries: its times, image path and package identity.
pub struct Process(OwnedHandle);

impl AsRawHandle for Process {
    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.0.as_raw_handle()
    }
}

impl Process {
    pub fn open(pid: u32) -> Option<Self> {
        // SAFETY: Opens a query-only handle; the result is checked below.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        // SAFETY: A valid process handle that we own and close once, on drop.
        (!handle.is_null()).then(|| Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    /// The creation time, which tells a process from a later one that reuses its ID.
    pub fn created(&self) -> Option<u64> {
        let (mut created, mut exited, mut kernel, mut user) = Default::default();
        // SAFETY: All four out pointers are valid FILETIMEs.
        let ok = unsafe {
            GetProcessTimes(
                self.0.as_raw_handle(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        let created: FILETIME = created;
        (ok != 0)
            .then_some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    /// The full path of the process's executable. `buffer` is reused between calls.
    pub fn executable(&self, buffer: &mut Vec<u16>) -> Option<PathBuf> {
        let query = |buffer: &mut Vec<u16>| {
            let mut length = buffer.len() as u32;
            // SAFETY: The buffer and its length in UTF-16 units describe the same allocation.
            let ok = unsafe {
                QueryFullProcessImageNameW(
                    self.0.as_raw_handle(),
                    0,
                    buffer.as_mut_ptr(),
                    &mut length,
                )
            };
            (ok != 0).then_some(length as usize)
        };
        let length = query(buffer).or_else(|| {
            let error = io::Error::last_os_error().raw_os_error();
            (error == Some(ERROR_INSUFFICIENT_BUFFER as i32)).then(|| {
                buffer.resize(32_768, 0);
                query(buffer)
            })?
        })?;
        Some(PathBuf::from(OsString::from_wide(buffer.get(..length)?)))
    }

    /// The Store app's AUMID, or its package family when the process has no single app.
    /// `None` for unpackaged processes.
    pub fn package_identity(&self) -> Option<String> {
        let mut buffer = [0u16; 512];
        let mut length = buffer.len() as u32;
        // SAFETY: The buffer and its length in UTF-16 units describe the same array.
        let failed = unsafe {
            GetPackageFamilyName(self.0.as_raw_handle(), &mut length, buffer.as_mut_ptr())
        };
        if failed != 0 {
            return None;
        }
        let family = String::from_utf16(buffer.get(..length.checked_sub(1)? as usize)?).ok()?;
        let mut length = buffer.len() as u32;
        // SAFETY: As above.
        let failed = unsafe {
            GetApplicationUserModelId(self.0.as_raw_handle(), &mut length, buffer.as_mut_ptr())
        };
        if failed == 0 {
            return String::from_utf16(buffer.get(..length.checked_sub(1)? as usize)?).ok();
        }
        Some(family)
    }
}

/// The tick count now and at the last keyboard or mouse input, both in milliseconds. The input
/// time is the low 32 bits of the tick count, so it wraps every 49.7 days.
pub fn input_ticks() -> Option<(u64, u32)> {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is sized by `cbSize` as the call requires.
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return None;
    }
    // SAFETY: No arguments.
    let now = unsafe { GetTickCount64() };
    Some((now, info.dwTime))
}
