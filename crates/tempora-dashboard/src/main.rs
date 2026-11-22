#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod dashboard;

#[cfg(windows)]
fn main() -> eframe::Result {
    use dashboard::{WINDOW_DEFAULT, WINDOW_MIN, window_geometry};

    let gallery = cfg!(debug_assertions) && std::env::args_os().any(|arg| arg == "--gallery");
    let title = if gallery {
        "Tempora — Component gallery"
    } else {
        dashboard::APP_NAME
    };
    // The dashboard keeps its own window geometry (see `window_geometry`), validated here
    // before the window exists, so the window opens in place without a jump.
    let restored = (!gallery)
        .then(|| window_geometry::load(dashboard::APP_NAME, WINDOW_MIN))
        .flatten();
    eframe::run_native(
        title,
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size(WINDOW_DEFAULT)
                .with_min_inner_size(WINDOW_MIN)
                // The dashboard draws its own title bar (see `shell::top_bar`); egui-winit keeps
                // the Windows shadow, snapping and rounded corners for an undecorated window.
                // The gallery has no title bar of its own.
                .with_decorations(gallery)
                .with_icon(eframe::egui::IconData {
                    rgba: include_bytes!("../../../assets/icons/app-64.rgba").to_vec(),
                    width: 64,
                    height: 64,
                }),
            renderer: eframe::Renderer::Glow,
            // eframe would save a minimized window as 0x0 at -32000,-32000 and restore it.
            persist_window: false,
            // `persist_window: false` only stops saving: eframe 0.36 still applies geometry it
            // saved earlier (a minimized one comes back as 64x64 points off screen). This hook
            // runs after that and replaces it with the validated geometry or the defaults.
            window_builder: Some(Box::new(move |builder| {
                let mut builder = builder
                    .with_inner_size(restored.map_or(WINDOW_DEFAULT, |geometry| geometry.size));
                builder.position = restored
                    .and_then(|geometry| geometry.position)
                    .map(|[x, y]| eframe::egui::pos2(x, y));
                if restored.is_some_and(|geometry| geometry.maximized) {
                    builder = builder.with_maximized(true);
                }
                builder
            })),
            ..Default::default()
        },
        Box::new(move |context| {
            log_display_scale(&context.egui_ctx);
            #[cfg(debug_assertions)]
            if gallery {
                return Ok(Box::new(dashboard::Gallery::new(&context.egui_ctx)));
            }
            Ok(Box::new(dashboard::Dashboard::new(context)))
        }),
    )
}

#[cfg(windows)]
fn log_display_scale(ctx: &eframe::egui::Context) {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use windows_sys::Win32::UI::HiDpi::{
        AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        GetThreadDpiAwarenessContext,
    };

    let Some(path) = std::env::var_os("TEMPORA_DPI_LOG") else {
        return;
    };
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let log = Mutex::new((file, None));
    ctx.on_begin_pass("display scale log", Arc::new(move |ctx| {
        let Some(native) = ctx.native_pixels_per_point() else {
            return;
        };
        let scale = (native, ctx.pixels_per_point(), ctx.zoom_factor());
        let Ok(mut log) = log.lock() else {
            return;
        };
        if log.1 == Some(scale) {
            return;
        }
        log.1 = Some(scale);
        // SAFETY: Compares two opaque DPI-context tokens; the callback runs on the window thread,
        // whose context GetThreadDpiAwarenessContext reports.
        let per_monitor_v2 = unsafe {
            AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            ) != 0
        };
        let (native, pixels, zoom) = scale;
        let _ = writeln!(
            log.0,
            "native_scale={native} pixels_per_point={pixels} zoom_factor={zoom} per_monitor_v2={per_monitor_v2}"
        );
    }));
}

#[cfg(not(windows))]
fn main() {}
