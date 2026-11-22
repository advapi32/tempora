//! Native behavior for the custom title bar.
//!
//! The window has no native caption, so this hooks the window procedure and answers
//! `WM_NCHITTEST` for the areas the title bar publishes each frame:
//!
//! - Over the maximize button it answers `HTMAXBUTTON`, which is when Windows 11 shows its
//!   Snap Layouts flyout. The pointer messages over it then arrive as non-client messages
//!   that egui never sees, so the hover and pressed state and the click are handled here and
//!   handed back to the title bar.
//! - Over the bar's empty space it answers `HTCAPTION`, so Windows moves the window from the
//!   press itself, as from a native title bar: dragging a maximized window down restores it,
//!   a double-click maximizes, a right-click opens the window menu, and Snap works. Asking
//!   for a move after egui has seen the drag start comes too late to be reliable.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use eframe::egui;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWLP_WNDPROC, HTCAPTION, HTCLIENT, HTMAXBUTTON, IsZoomed, SW_MAXIMIZE,
    SW_RESTORE, SetWindowLongPtrW, ShowWindow, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST,
    WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE,
    WM_NCRBUTTONDOWN, WM_NCRBUTTONUP, WNDPROC,
};

/// The window procedure this hook replaced.
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
/// The maximize button in client pixels: left, top, right, bottom. Empty when not shown.
static BUTTON: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4];
/// The title bar's empty space in client pixels, where Windows moves the window. Empty when
/// there's none to claim.
static CAPTION: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4];
/// Controls on or over the title bar, in client pixels. They stay egui's.
static HOLES: Mutex<Vec<[i32; 4]>> = Mutex::new(Vec::new());
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

/// Publishes the title bar's empty space, in egui points: `area` minus the `holes` where
/// controls sit. Pass an empty `area` to claim none.
pub(super) fn set_caption(area: egui::Rect, holes: &[egui::Rect], pixels_per_point: f32) {
    let pixels = |rect: egui::Rect| {
        [rect.left(), rect.top(), rect.right(), rect.bottom()]
            .map(|edge| (edge * pixels_per_point).round() as i32)
    };
    for (slot, edge) in CAPTION.iter().zip(pixels(area)) {
        slot.store(edge, Relaxed);
    }
    if let Ok(mut published) = HOLES.lock() {
        published.clear();
        published.extend(holes.iter().map(|hole| pixels(*hole)));
    }
}

/// Hover and pressed state of the maximize button, as Windows reports it.
pub(super) fn maximize_state() -> (bool, bool) {
    let pressed = PRESSED.load(Relaxed);
    (HOVERED.load(Relaxed) || pressed, pressed)
}

/// The pointer position a non-client message carries, in client pixels.
fn client_point(hwnd: HWND, lparam: LPARAM) -> Option<(i32, i32)> {
    // Screen coordinates, signed 16-bit each (negative on monitors left of or above primary).
    let mut point = POINT {
        x: i32::from(lparam as u16 as i16),
        y: i32::from((lparam >> 16) as u16 as i16),
    };
    // SAFETY: `point` is a valid in/out pointer; the handle is validated by Windows.
    (unsafe { ScreenToClient(hwnd, &mut point) } != 0).then_some((point.x, point.y))
}

fn inside([left, top, right, bottom]: [i32; 4], (x, y): (i32, i32)) -> bool {
    (left..right).contains(&x) && (top..bottom).contains(&y)
}

fn over_button(hwnd: HWND, lparam: LPARAM) -> bool {
    client_point(hwnd, lparam)
        .is_some_and(|point| inside(BUTTON.each_ref().map(|edge| edge.load(Relaxed)), point))
}

/// Whether `point` is on the caption and clear of every hole.
fn on_caption(caption: [i32; 4], holes: &[[i32; 4]], point: (i32, i32)) -> bool {
    inside(caption, point) && !holes.iter().any(|hole| inside(*hole, point))
}

fn over_caption(hwnd: HWND, lparam: LPARAM) -> bool {
    let caption = CAPTION.each_ref().map(|edge| edge.load(Relaxed));
    client_point(hwnd, lparam).is_some_and(|point| {
        HOLES
            .lock()
            .is_ok_and(|holes| on_caption(caption, &holes, point))
    })
}

/// The window procedure this hook replaced.
fn previous() -> WNDPROC {
    // SAFETY: PREVIOUS holds the window procedure returned by SetWindowLongPtrW (set before
    // this procedure was installed), and a WNDPROC is a nullable function pointer of that size.
    unsafe { std::mem::transmute(PREVIOUS.load(Relaxed)) }
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
        WM_NCHITTEST => {
            if over_button(hwnd, lparam) {
                return HTMAXBUTTON as LRESULT;
            }
            // SAFETY: Forwards the message unchanged to the procedure we replaced.
            let hit = unsafe { CallWindowProcW(previous(), hwnd, message, wparam, lparam) };
            return if hit == HTCLIENT as LRESULT && over_caption(hwnd, lparam) {
                HTCAPTION as LRESULT
            } else {
                hit
            };
        }
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
    // SAFETY: Forwards the message unchanged to the procedure we replaced.
    unsafe { CallWindowProcW(previous(), hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caption_is_the_bar_minus_its_controls() {
        let bar = [0, 5, 1000, 70];
        let tab = [400, 20, 480, 50];
        assert!(on_caption(bar, &[tab], (200, 30)), "empty bar space");
        assert!(!on_caption(bar, &[tab], (420, 30)), "a tab stays egui's");
        assert!(
            !on_caption(bar, &[tab], (200, 2)),
            "the resize edge above the bar"
        );
        assert!(!on_caption([0; 4], &[], (200, 30)), "nothing claimed");
    }
}
