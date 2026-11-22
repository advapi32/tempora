//! Native behavior for the custom title bar's maximize button.
//!
//! Windows 11 shows its Snap Layouts flyout only when hit testing reports the pointer over a
//! maximize button (`HTMAXBUTTON`). The window has no native caption, so this hooks the
//! window procedure: over the button's rectangle, published by the title bar each frame,
//! `WM_NCHITTEST` answers `HTMAXBUTTON`. The pointer messages over it then arrive as
//! non-client messages that egui never sees, so the hover and pressed state and the click
//! are handled here and handed back to the title bar.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering::Relaxed};

use eframe::egui;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWLP_WNDPROC, HTMAXBUTTON, IsZoomed, SW_MAXIMIZE, SW_RESTORE,
    SetWindowLongPtrW, ShowWindow, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
    WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCRBUTTONDOWN,
    WM_NCRBUTTONUP, WNDPROC,
};

/// The window procedure this hook replaced.
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
/// The maximize button in client pixels: left, top, right, bottom. Empty when not shown.
static BUTTON: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4];
static HOVERED: AtomicBool = AtomicBool::new(false);
static PRESSED: AtomicBool = AtomicBool::new(false);
static TRACKING: AtomicBool = AtomicBool::new(false);
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// Hooks the dashboard window. Without a window handle the title bar keeps working, just
/// without Snap Layouts.
pub(super) fn install(context: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = context.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    if CONTEXT.set(context.egui_ctx.clone()).is_err() {
        return;
    }
    let hwnd = handle.hwnd.get() as HWND;
    // The window lives on this thread for the life of the process; the hook stays with it.
    // SAFETY: `hwnd` is eframe's live window on this thread, and `window_proc` has the window
    // procedure signature. The previous procedure is kept and every unhandled message goes to it.
    let previous =
        unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, window_proc as *const () as isize) };
    PREVIOUS.store(previous, Relaxed);
}

/// Whether the hook runs, so the maximize button takes its state from `maximize_state`.
pub(super) fn installed() -> bool {
    PREVIOUS.load(Relaxed) != 0
}

/// Publishes where the maximize button is, in egui points.
pub(super) fn set_maximize_button(rect: egui::Rect, pixels_per_point: f32) {
    let edges = [rect.left(), rect.top(), rect.right(), rect.bottom()];
    for (slot, edge) in BUTTON.iter().zip(edges) {
        slot.store((edge * pixels_per_point).round() as i32, Relaxed);
    }
}

/// Hover and pressed state of the maximize button, as Windows reports it.
pub(super) fn maximize_state() -> (bool, bool) {
    let pressed = PRESSED.load(Relaxed);
    (HOVERED.load(Relaxed) || pressed, pressed)
}

fn over_button(hwnd: HWND, lparam: LPARAM) -> bool {
    // Screen coordinates, signed 16-bit each (negative on monitors left of or above primary).
    let mut point = POINT {
        x: i32::from(lparam as u16 as i16),
        y: i32::from((lparam >> 16) as u16 as i16),
    };
    // SAFETY: `point` is a valid in/out pointer; the handle is validated by Windows.
    if unsafe { ScreenToClient(hwnd, &mut point) } == 0 {
        return false;
    }
    let [left, top, right, bottom] = BUTTON.each_ref().map(|edge| edge.load(Relaxed));
    (left..right).contains(&point.x) && (top..bottom).contains(&point.y)
}

fn set(flag: &AtomicBool, value: bool) {
    if flag.swap(value, Relaxed) != value
        && let Some(context) = CONTEXT.get()
    {
        context.request_repaint();
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let on_button = wparam == HTMAXBUTTON as WPARAM;
    match message {
        WM_NCHITTEST if over_button(hwnd, lparam) => return HTMAXBUTTON as LRESULT,
        WM_NCMOUSEMOVE => {
            set(&HOVERED, on_button);
            if on_button {
                if !TRACKING.swap(true, Relaxed) {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: `track` is sized by `cbSize` and names our own window.
                    unsafe { TrackMouseEvent(&mut track) };
                }
                return 0;
            }
        }
        WM_NCMOUSELEAVE => {
            TRACKING.store(false, Relaxed);
            set(&HOVERED, false);
            set(&PRESSED, false);
        }
        WM_MOUSEMOVE => set(&HOVERED, false),
        WM_LBUTTONUP => set(&PRESSED, false),
        // Swallowed: Windows would otherwise draw and track a classic caption button.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if on_button => {
            set(&PRESSED, true);
            return 0;
        }
        WM_NCLBUTTONUP if on_button => {
            if PRESSED.load(Relaxed) {
                // SAFETY: State query and change on the window this procedure belongs to.
                let zoomed = unsafe { IsZoomed(hwnd) } != 0;
                // SAFETY: As above.
                unsafe { ShowWindow(hwnd, if zoomed { SW_RESTORE } else { SW_MAXIMIZE }) };
            }
            set(&PRESSED, false);
            return 0;
        }
        WM_NCLBUTTONUP => set(&PRESSED, false),
        WM_NCRBUTTONDOWN | WM_NCRBUTTONUP if on_button => return 0,
        _ => {}
    }
    // SAFETY: PREVIOUS holds the window procedure returned by SetWindowLongPtrW (set before
    // this procedure was installed), and a WNDPROC is a nullable function pointer of that size.
    let previous: WNDPROC = unsafe { std::mem::transmute(PREVIOUS.load(Relaxed)) };
    // SAFETY: Forwards the message unchanged to the procedure we replaced.
    unsafe { CallWindowProcW(previous, hwnd, message, wparam, lparam) }
}
