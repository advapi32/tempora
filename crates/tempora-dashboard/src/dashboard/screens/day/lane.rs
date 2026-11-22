//! How the lane draws: block colors and dimming, the highlight, and block tooltips.

use std::collections::HashMap;

use chrono_tz::Tz;
use eframe::egui::{self, Sense, Ui, vec2};

use crate::dashboard::format;
use crate::dashboard::theme::Tokens;
use crate::dashboard::widgets::{self, LaneStyle};

use super::*;

/// "{start}–{end} · N switches: brave 3m, vesktop 1m, Spotify <1m", naming at most four apps.
pub(super) fn run_description(run: &SliverRun, zone: Tz) -> String {
    let mut names: Vec<String> = run
        .apps
        .iter()
        .take(RUN_APPS_SHOWN)
        .map(|(name, ms)| format!("{name} {}", format::duration(*ms)))
        .collect();
    let more = run.apps.len().saturating_sub(RUN_APPS_SHOWN);
    if more > 0 {
        names.push(format!("+{more} more"));
    }
    format!(
        "{}–{} · {} switches: {}",
        format::local(run.start_ms, zone, "%H:%M"),
        format::local(run.end_ms, zone, "%H:%M"),
        run.switches,
        names.join(", ")
    )
}

/// Tooltip for one block: what it was, when, how long, and what a click does.
pub(super) fn block_tip(block: &BlockVm, highlight: Option<&str>, zone: Tz) -> widgets::LaneTip {
    widgets::LaneTip {
        title: block.label().to_owned(),
        lines: vec![format!(
            "{} · {}",
            span_label(block.start_ms, block.end_ms, zone, "%H:%M"),
            format::duration(block.end_ms - block.start_ms)
        )],
        hint: block.key.as_deref().map(|key| {
            if highlight == Some(key) {
                "Click to stop highlighting · Right-click for options".to_owned()
            } else {
                "Click to highlight · Right-click for options".to_owned()
            }
        }),
    }
}

/// Tooltip for a run of quick switches: the span and the apps in it.
pub(super) fn run_tip(run: &SliverRun, zone: Tz) -> widgets::LaneTip {
    let mut lines = vec![span_label(run.start_ms, run.end_ms, zone, "%H:%M")];
    lines.extend(
        run.apps
            .iter()
            .take(RUN_APPS_SHOWN)
            .map(|(name, ms)| format!("{name} · {}", format::duration(*ms))),
    );
    let more = run.apps.len().saturating_sub(RUN_APPS_SHOWN);
    if more > 0 {
        lines.push(format!("+{more} more"));
    }
    widgets::LaneTip {
        title: format!("{} quick switches", run.switches),
        lines,
        hint: None,
    }
}

/// An app's color: its palette rank, else grey.
pub(super) fn app_color(key: Option<&str>, palette: &HashMap<String, usize>) -> egui::Color32 {
    key.and_then(|key| palette.get(key))
        .map_or(Tokens::APP_OTHER, |&index| Tokens::APP_PALETTE[index])
}

pub(super) fn style(
    kind: BlockKind,
    key: Option<&str>,
    palette: &HashMap<String, usize>,
) -> LaneStyle {
    match kind {
        BlockKind::App => LaneStyle::Solid(app_color(key, palette)),
        BlockKind::Desktop => LaneStyle::Neutral,
        BlockKind::Paused | BlockKind::Gap => LaneStyle::Gap,
    }
}

/// A focus block whose app isn't the running lane's app. Blocks without an app, and every
/// block when there is no running lane app, are never dimmed.
pub(super) fn is_dimmed(block: &BlockVm, lane_key: Option<&str>) -> bool {
    match (block.key.as_deref(), lane_key) {
        (Some(key), Some(lane)) => key != lane,
        _ => false,
    }
}

/// The app to highlight: the one the view picked, if it had focus this day. An app that was only
/// open has nothing in front to show, so it is not highlighted.
pub(super) fn highlight_key(vm: &DayVm, picked: Option<&str>) -> Option<String> {
    let key = picked?;
    vm.apps
        .iter()
        .any(|app| app.key == key && app.focused_ms > 0)
        .then(|| key.to_owned())
}

/// The tooltip's last sentence for an app block: what a click does.
pub(super) fn click_hint(block: &BlockVm, highlight: Option<&str>) -> &'static str {
    match block.key.as_deref() {
        Some(key) if highlight == Some(key) => " Click to stop highlighting.",
        Some(_) => " Click to highlight.",
        None => "",
    }
}

pub(super) fn span_label(start_ms: i64, end_ms: i64, zone: Tz, pattern: &str) -> String {
    format!(
        "{} – {}",
        format::local(start_ms, zone, pattern),
        format::local(end_ms, zone, pattern)
    )
}

pub(super) fn hatch_swatch(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    widgets::hatch_gap(ui, rect);
}
