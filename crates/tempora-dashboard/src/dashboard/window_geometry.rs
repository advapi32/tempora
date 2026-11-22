//! The dashboard window's size, position and maximized state across launches.
//!
//! eframe's own window persistence restores whatever it saved, including a minimized window
//! (0×0 at −32000, −32000), so it stays off. This module records the geometry only while the
//! window is shown normally and validates it before the next window is created: the size must
//! be at least the minimum, and the position must still be on a connected monitor.

use std::path::{Path, PathBuf};

use eframe::egui::ViewportInfo;
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "window.json";
/// Larger than any real monitor, in points: a guard against a corrupt file.
const MAX_SIDE: f32 = 16_384.0;
/// Height of the strip along the top that must be on a monitor, in pixels, so the title bar
/// can be grabbed.
const GRAB_STRIP: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowGeometry {
    /// Outer top-left corner, in points. `None` lets Windows place the window.
    pub position: Option<[f32; 2]>,
    /// Inner size, in points.
    pub size: [f32; 2],
    /// Pixels per point when the geometry was recorded, to check the position against monitors.
    pub scale: f32,
    pub maximized: bool,
}

impl WindowGeometry {
    /// Updates the geometry from the current viewport. Minimized and fullscreen windows are
    /// ignored, and a maximized window keeps the last normal size and position.
    pub fn observe(
        current: Option<Self>,
        info: &ViewportInfo,
        default_size: [f32; 2],
    ) -> Option<Self> {
        if info.minimized == Some(true) || info.fullscreen == Some(true) {
            return current;
        }
        if info.maximized == Some(true) {
            let mut geometry = current.unwrap_or(Self {
                position: None,
                size: default_size,
                scale: info.native_pixels_per_point.unwrap_or(1.0),
                maximized: true,
            });
            geometry.maximized = true;
            return Some(geometry);
        }
        let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) else {
            return current;
        };
        Some(Self {
            position: Some([outer.min.x, outer.min.y]),
            size: [inner.width(), inner.height()],
            scale: info.native_pixels_per_point.unwrap_or(1.0),
            maximized: false,
        })
    }

    /// A geometry that is safe to restore, or `None` to use the defaults. `on_screen` says
    /// whether a rectangle in pixels (left, top, right, bottom) overlaps a connected monitor.
    pub fn sanitized(
        self,
        min_size: [f32; 2],
        on_screen: impl Fn([f32; 4]) -> bool,
    ) -> Option<Self> {
        let [width, height] = self.size;
        let finite = [width, height, self.scale]
            .iter()
            .chain(self.position.iter().flatten())
            .all(|value| value.is_finite());
        if !finite || self.scale <= 0.0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let size = [width.max(min_size[0]), height.max(min_size[1])];
        let position = self.position.filter(|[x, y]| {
            let left = x * self.scale;
            let top = y * self.scale;
            on_screen([left, top, left + size[0] * self.scale, top + GRAB_STRIP])
        });
        Some(Self {
            position,
            size,
            ..self
        })
    }
}

fn file(app_name: &str) -> Option<PathBuf> {
    eframe::storage_dir(app_name).map(|directory| directory.join(FILE_NAME))
}

/// The saved geometry, validated against the minimum size and the connected monitors.
pub(crate) fn load(app_name: &str, min_size: [f32; 2]) -> Option<WindowGeometry> {
    let text = std::fs::read_to_string(file(app_name)?).ok()?;
    let geometry: WindowGeometry = serde_json::from_str(&text).ok()?;
    geometry.sanitized(min_size, overlaps_a_monitor)
}

/// Writes the geometry through a temporary file, so a crash never leaves half a file.
pub(crate) fn save(app_name: &str, geometry: &WindowGeometry) {
    let Some(path) = file(app_name) else {
        return;
    };
    let _ = write(&path, geometry);
}

fn write(path: &Path, geometry: &WindowGeometry) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(geometry)?)?;
    std::fs::rename(&temporary, path)
}

#[cfg(windows)]
fn overlaps_a_monitor([left, top, right, bottom]: [f32; 4]) -> bool {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect},
    };
    let rect = RECT {
        left: left.round() as i32,
        top: top.round() as i32,
        right: right.round() as i32,
        bottom: bottom.round() as i32,
    };
    // SAFETY: `rect` is a valid RECT for the duration of the call.
    !unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_null()
}

#[cfg(not(windows))]
fn overlaps_a_monitor(_rect: [f32; 4]) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2, vec2};

    const MIN: [f32; 2] = [1100.0, 720.0];
    const DEFAULT: [f32; 2] = [1170.0, 760.0];

    fn normal(x: f32, y: f32, width: f32, height: f32) -> ViewportInfo {
        ViewportInfo {
            native_pixels_per_point: Some(1.5),
            outer_rect: Some(Rect::from_min_size(
                pos2(x, y),
                vec2(width + 2.0, height + 32.0),
            )),
            inner_rect: Some(Rect::from_min_size(
                pos2(x + 1.0, y + 31.0),
                vec2(width, height),
            )),
            minimized: Some(false),
            maximized: Some(false),
            fullscreen: Some(false),
            ..ViewportInfo::default()
        }
    }

    #[test]
    fn a_normal_window_records_its_position_size_and_scale() {
        let geometry = WindowGeometry::observe(None, &normal(100.0, 50.0, 1200.0, 800.0), DEFAULT);
        assert_eq!(
            geometry,
            Some(WindowGeometry {
                position: Some([100.0, 50.0]),
                size: [1200.0, 800.0],
                scale: 1.5,
                maximized: false,
            })
        );
    }

    #[test]
    fn a_minimized_window_keeps_the_last_normal_geometry() {
        let last = WindowGeometry::observe(None, &normal(100.0, 50.0, 1200.0, 800.0), DEFAULT);
        let minimized = ViewportInfo {
            minimized: Some(true),
            outer_rect: Some(Rect::from_min_size(
                pos2(-32000.0, -32000.0),
                vec2(0.0, 0.0),
            )),
            inner_rect: Some(Rect::from_min_size(
                pos2(-32000.0, -32000.0),
                vec2(0.0, 0.0),
            )),
            ..ViewportInfo::default()
        };
        assert_eq!(WindowGeometry::observe(last, &minimized, DEFAULT), last);
    }

    #[test]
    fn a_maximized_window_keeps_the_normal_rect_and_remembers_the_state() {
        let last = WindowGeometry::observe(None, &normal(100.0, 50.0, 1200.0, 800.0), DEFAULT);
        let maximized = ViewportInfo {
            maximized: Some(true),
            ..normal(0.0, 0.0, 2560.0, 1400.0)
        };
        let geometry = WindowGeometry::observe(last, &maximized, DEFAULT).unwrap();
        assert!(geometry.maximized);
        assert_eq!(geometry.size, [1200.0, 800.0]);
        assert_eq!(geometry.position, Some([100.0, 50.0]));

        let first = WindowGeometry::observe(None, &maximized, DEFAULT).unwrap();
        assert_eq!(
            (first.size, first.position, first.maximized),
            (DEFAULT, None, true)
        );
    }

    #[test]
    fn restoring_grows_to_the_minimum_and_drops_an_offscreen_position() {
        let saved = WindowGeometry {
            position: Some([-32000.0, -32000.0]),
            size: [0.0, 0.0],
            scale: 1.0,
            maximized: false,
        };
        let restored = saved
            .sanitized(MIN, |[left, top, ..]| left >= 0.0 && top >= 0.0)
            .unwrap();
        assert_eq!(restored.size, MIN);
        assert_eq!(restored.position, None);
    }

    #[test]
    fn restoring_keeps_an_onscreen_position_checked_in_pixels() {
        let saved = WindowGeometry {
            position: Some([100.0, 50.0]),
            size: [1200.0, 800.0],
            scale: 1.5,
            maximized: false,
        };
        let restored = saved
            .sanitized(MIN, |rect| {
                rect == [150.0, 75.0, 150.0 + 1800.0, 75.0 + GRAB_STRIP]
            })
            .unwrap();
        assert_eq!(restored, saved);
    }

    #[test]
    fn a_corrupt_geometry_is_rejected() {
        let corrupt = |size: [f32; 2], scale: f32| WindowGeometry {
            position: None,
            size,
            scale,
            maximized: false,
        };
        assert_eq!(
            corrupt([f32::NAN, 800.0], 1.0).sanitized(MIN, |_| true),
            None
        );
        assert_eq!(corrupt([1e9, 800.0], 1.0).sanitized(MIN, |_| true), None);
        assert_eq!(corrupt([1200.0, 800.0], 0.0).sanitized(MIN, |_| true), None);
    }
}
