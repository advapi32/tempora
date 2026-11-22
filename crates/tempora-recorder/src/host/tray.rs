//! The recorder's hidden window: its notification-area icon and menu, and the Windows messages
//! it turns into [`Event`]s for the host.

use super::{APP_NAME, POLL_MS};
use anyhow::{Context, Result};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io,
    ptr::{null, null_mut},
};
use tempora::model::Settings;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        RemoteDesktop::{
            NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
            WTSUnRegisterSessionNotification,
        },
    },
    UI::{
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
            NIM_SETFOCUS, NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
            NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
        },
        WindowsAndMessaging::*,
    },
};

const TRAY_MESSAGE: u32 = WM_APP + 1;
const WAKE_MESSAGE: u32 = WM_APP + 2;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const TIMER_ID: usize = 1;
const ICON_ID: u32 = 1;

thread_local! {
    /// Events from the window procedure, drained by the host after each dispatched message.
    static EVENTS: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
    // SAFETY: The argument is a NUL-terminated UTF-16 string that outlives the call.
    static TASKBAR_CREATED: u32 = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
}

pub(super) enum Event {
    Tick,
    Open,
    Menu,
    Quit,
    SessionBlocked,
    SessionChanged,
    Sleep(bool),
    EndingSession,
    Shutdown(bool),
    RestoreTray,
}

/// The next event raised by the window procedure, if any.
pub(super) fn next_event() -> Option<Event> {
    EVENTS.with(|events| events.borrow_mut().pop_front())
}

/// Waits for the next window message and dispatches it to the window procedure. Returns
/// `Ok(false)` once the thread's message loop is told to quit.
pub(super) fn pump_message() -> Result<bool> {
    let mut message = MSG::default();
    // SAFETY: `message` is a valid out pointer; a null window reads every message for the thread.
    let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
    if result == -1 {
        return Err(io::Error::last_os_error()).context("Recorder message loop failed");
    }
    if result == 0 {
        return Ok(false);
    }
    // SAFETY: `message` was just filled in by GetMessageW.
    unsafe {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    Ok(true)
}

/// Shows a modal error box, for failures before the tray icon exists.
pub(super) fn show_error(text: &str) {
    let text = wide(text);
    let caption = wide(APP_NAME);
    // SAFETY: Both strings are NUL-terminated UTF-16 that outlive the call.
    unsafe {
        MessageBoxW(
            null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Indicator {
    Recording,
    Paused,
    Attention,
}

impl Indicator {
    // The loaded icon is kept as a window property, so it is released with the window.
    const ICON_PROPERTY: *const u16 = windows_sys::w!("Tempora.TrayIcon");

    fn label(self) -> &'static str {
        match self {
            Self::Recording => "Recording",
            Self::Paused => "Paused",
            Self::Attention => "Needs attention",
        }
    }

    /// The state's icon from the executable's resources, at the small-icon size for the
    /// window's DPI. Null when loading fails.
    fn load_icon(self, hwnd: HWND) -> HICON {
        use windows_sys::Win32::UI::HiDpi::{
            GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi,
        };

        let resource = match self {
            Self::Recording => 101,
            Self::Paused => 102,
            Self::Attention => 103,
        };
        // SAFETY: Reads DPI and metrics for our own window. The resource ID is passed as an
        // integer resource name (MAKEINTRESOURCE), which LoadImageW accepts in place of a string.
        unsafe {
            let dpi = match GetDpiForWindow(hwnd) {
                0 => GetDpiForSystem(),
                dpi => dpi,
            };
            let size = GetSystemMetricsForDpi(SM_CXSMICON, dpi);
            let module = GetModuleHandleW(null());
            LoadImageW(module, resource as *const u16, IMAGE_ICON, size, size, 0)
        }
    }

    /// A shared system icon for when the resource can't be loaded. It must not be destroyed.
    fn stock_icon(self) -> HICON {
        let name = match self {
            Self::Recording => IDI_INFORMATION,
            Self::Paused => IDI_APPLICATION,
            Self::Attention => IDI_WARNING,
        };
        // SAFETY: Loads a predefined system icon by its integer ID.
        unsafe { LoadIconW(null_mut(), name) }
    }

    fn release_icon(hwnd: HWND) {
        // SAFETY: The property only ever holds an icon we loaded with LoadImageW and stored with
        // SetPropW; removing it first means it is destroyed exactly once.
        unsafe {
            let previous = RemovePropW(hwnd, Self::ICON_PROPERTY);
            if !previous.is_null() {
                DestroyIcon(previous);
            }
        }
    }
}

/// What the user picked in the tray menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum MenuChoice {
    OpenDashboard,
    TogglePause,
    Quit,
}

impl MenuChoice {
    const ALL: [Self; 3] = [Self::OpenDashboard, Self::TogglePause, Self::Quit];

    fn id(self) -> usize {
        match self {
            Self::OpenDashboard => 1,
            Self::TogglePause => 2,
            Self::Quit => 3,
        }
    }

    fn from_id(id: i32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.id() as i32 == id)
    }
}

/// A popup menu, destroyed on drop.
struct PopupMenu(HMENU);

impl PopupMenu {
    fn new() -> Option<Self> {
        // SAFETY: No arguments; the handle is checked below.
        let menu = unsafe { CreatePopupMenu() };
        (!menu.is_null()).then_some(Self(menu))
    }

    fn item(&self, flags: MENU_ITEM_FLAGS, id: usize, label: Option<&str>) {
        let label = label.map(wide);
        let text = label.as_ref().map_or(null(), |label| label.as_ptr());
        // SAFETY: The menu is live, and the label (when given) is NUL-terminated UTF-16 that
        // outlives the call; the menu copies it.
        unsafe { AppendMenuW(self.0, flags, id, text) };
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        // SAFETY: We created the menu and destroy it once.
        unsafe { DestroyMenu(self.0) };
    }
}

pub(super) struct TrayWindow {
    hwnd: HWND,
    indicator: Option<Indicator>,
    tip: String,
    problem: Option<String>,
    registered_session: bool,
}

impl TrayWindow {
    pub(super) fn new() -> Result<Self> {
        let class_name = wide("TemporaRecorder");
        // SAFETY: A null name returns this executable's module handle.
        let instance = unsafe { GetModuleHandleW(null()) };
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..WNDCLASSW::default()
        };
        // SAFETY: `class` is fully initialized and its name outlives the call.
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(io::Error::last_os_error())
                .context("Could not register the recorder window");
        }
        let title = wide(APP_NAME);
        // SAFETY: The class was registered above; both strings outlive the call.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if hwnd.is_null() {
            return Err(io::Error::last_os_error()).context("Could not create the tray window");
        }
        // SAFETY: `hwnd` is our live window; unregistered in Drop.
        let registered_session =
            unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } != 0;
        // Constructed before the timer so Drop cleans up if scheduling fails.
        let window = Self {
            hwnd,
            indicator: None,
            tip: String::new(),
            problem: None,
            registered_session,
        };
        // SAFETY: `hwnd` is our live window; the timer posts WM_TIMER to it until Drop.
        if unsafe { SetTimer(hwnd, TIMER_ID, POLL_MS, None) } == 0 {
            return Err(io::Error::last_os_error()).context("Could not schedule recorder polling");
        }
        Ok(window)
    }

    fn tray_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..NOTIFYICONDATAW::default()
        }
    }

    pub(super) fn refresh(&mut self, status: Indicator, problem: Option<&str>) {
        self.problem = problem.map(str::to_owned);
        // The tooltip explains attention with the actual problem (disk full, locked
        // database, setup unfinished). Windows caps tips at 127 characters.
        let tip = match (status, problem) {
            (Indicator::Attention, Some(problem)) => format!("{APP_NAME} - {problem}"),
            (Indicator::Attention, None) => format!("{APP_NAME} - Finish setup in the dashboard"),
            _ => format!("{APP_NAME} - {}", status.label()),
        };
        let tip: String = tip.chars().take(127).collect();
        if self.indicator == Some(status) && self.tip == tip {
            return;
        }
        let mut icon = self.tray_data();
        icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        icon.uCallbackMessage = TRAY_MESSAGE;
        let loaded = status.load_icon(self.hwnd);
        icon.hIcon = if loaded.is_null() {
            status.stock_icon()
        } else {
            loaded
        };
        let wide_tip: Vec<u16> = wide(&tip).into_iter().take(127).collect();
        icon.szTip[..wide_tip.len()].copy_from_slice(&wide_tip);
        let change = if self.indicator.is_none() {
            NIM_ADD
        } else {
            NIM_MODIFY
        };
        // SAFETY: `icon` is a fully initialized NOTIFYICONDATAW for our window.
        let updated = unsafe { Shell_NotifyIconW(change, &icon) } != 0;
        if updated {
            Indicator::release_icon(self.hwnd);
            self.indicator = Some(status);
            self.tip = tip;
            if change == NIM_ADD {
                icon.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                // SAFETY: As above.
                unsafe { Shell_NotifyIconW(NIM_SETVERSION, &icon) };
            }
        }
        // The shell copies the icon, so ours is kept on the window (and released with the next
        // one) or destroyed now if it couldn't be stored.
        if !loaded.is_null() {
            // SAFETY: `loaded` came from LoadImageW and is ours until stored on our window.
            let stored =
                updated && unsafe { SetPropW(self.hwnd, Indicator::ICON_PROPERTY, loaded) } != 0;
            if !stored {
                // SAFETY: Not stored anywhere, so this is the only release.
                unsafe { DestroyIcon(loaded) };
            }
        }
    }

    /// Adds the icon again after Explorer restarts.
    pub(super) fn restore(&mut self) {
        if let Some(status) = self.indicator.take() {
            let problem = self.problem.clone();
            self.refresh(status, problem.as_deref());
        }
    }

    /// Shows the tray menu at the icon and returns the choice, if any.
    pub(super) fn menu(&self, status: Indicator, settings: &Settings) -> Option<MenuChoice> {
        let menu = PopupMenu::new()?;
        let pause_label = if !settings.enabled {
            "Finish setup…"
        } else if settings.paused {
            "Resume recording"
        } else {
            "Pause recording"
        };
        menu.item(MF_STRING | MF_GRAYED, 0, Some(status.label()));
        menu.item(MF_SEPARATOR, 0, None);
        menu.item(
            MF_STRING,
            MenuChoice::OpenDashboard.id(),
            Some("Open dashboard"),
        );
        menu.item(MF_STRING, MenuChoice::TogglePause.id(), Some(pause_label));
        menu.item(MF_SEPARATOR, 0, None);
        menu.item(MF_STRING, MenuChoice::Quit.id(), Some("Quit"));
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..NOTIFYICONIDENTIFIER::default()
        };
        let mut location = POINT::default();
        let mut bounds = RECT::default();
        // SAFETY: Both pointers are valid for the calls; the menu and window are live. The
        // foreground switch and the trailing WM_NULL are the documented way to make a tray
        // menu close when the user clicks elsewhere.
        let selection = unsafe {
            // Open dashboard is the default item, drawn bold.
            SetMenuDefaultItem(menu.0, MenuChoice::OpenDashboard.id() as u32, 0);
            if Shell_NotifyIconGetRect(&identifier, &mut bounds) >= 0 {
                location.x = bounds.left;
                location.y = bounds.top;
            } else {
                GetCursorPos(&mut location);
            }
            SetForegroundWindow(self.hwnd);
            let selection = TrackPopupMenu(
                menu.0,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                location.x,
                location.y,
                0,
                self.hwnd,
                null(),
            );
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
            selection
        };
        if selection == 0 {
            // Dismissed: give keyboard focus back to the notification area.
            // SAFETY: As in `refresh`.
            unsafe { Shell_NotifyIconW(NIM_SETFOCUS, &self.tray_data()) };
        }
        MenuChoice::from_id(selection)
    }
}

impl Drop for TrayWindow {
    fn drop(&mut self) {
        // SAFETY: Undoes what `new` and `refresh` set up on our own window, once each.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.tray_data());
            Indicator::release_icon(self.hwnd);
            KillTimer(self.hwnd, TIMER_ID);
            if self.registered_session {
                WTSUnRegisterSessionNotification(self.hwnd);
            }
            DestroyWindow(self.hwnd);
        }
    }
}

fn submit(hwnd: HWND, event: Event) {
    let needs_wake = EVENTS.with(|events| {
        let mut events = events.borrow_mut();
        let was_empty = events.is_empty();
        events.push_back(event);
        was_empty
    });
    if needs_wake {
        // Wakes GetMessageW so the host drains the queue promptly.
        // SAFETY: Posting a private message to our own window.
        unsafe { PostMessageW(hwnd, WAKE_MESSAGE, 0, 0) };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if TASKBAR_CREATED.with(|taskbar| message == *taskbar) {
        submit(hwnd, Event::RestoreTray);
        return 0;
    }
    match message {
        TRAY_MESSAGE => {
            match (lparam & 0xffff) as u32 {
                WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => submit(hwnd, Event::Open),
                WM_CONTEXTMENU => submit(hwnd, Event::Menu),
                WM_RBUTTONUP if lparam >> 16 == 0 => submit(hwnd, Event::Menu),
                _ => {}
            }
            0
        }
        WM_TIMER => {
            submit(hwnd, Event::Tick);
            0
        }
        WAKE_MESSAGE => 0,
        WM_CLOSE => {
            submit(hwnd, Event::Quit);
            0
        }
        WM_QUERYENDSESSION => {
            submit(hwnd, Event::EndingSession);
            1
        }
        WM_ENDSESSION => {
            submit(hwnd, Event::Shutdown(wparam != 0));
            0
        }
        WM_WTSSESSION_CHANGE => {
            match wparam as u32 {
                WTS_SESSION_LOGOFF => submit(hwnd, Event::Quit),
                WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => {
                    submit(hwnd, Event::SessionBlocked)
                }
                WTS_SESSION_UNLOCK | WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
                    submit(hwnd, Event::SessionChanged)
                }
                _ => {}
            }
            0
        }
        WM_POWERBROADCAST => {
            match wparam as u32 {
                PBT_APMSUSPEND => submit(hwnd, Event::Sleep(true)),
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => submit(hwnd, Event::Sleep(false)),
                _ => {}
            }
            1
        }
        // SAFETY: Forwards a message Windows gave us, unchanged, to the default procedure.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

pub(super) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_choices_round_trip_through_their_ids() {
        for choice in MenuChoice::ALL {
            assert_eq!(MenuChoice::from_id(choice.id() as i32), Some(choice));
        }
        assert_eq!(MenuChoice::from_id(0), None, "a dismissed menu returns 0");
    }
}
