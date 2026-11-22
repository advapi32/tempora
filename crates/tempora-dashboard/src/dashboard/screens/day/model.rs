//! The day's view model: the foreground blocks within the day, its apps, the lane window and
//! its ticks, and the merging of slivers too thin to see.

use std::collections::HashMap;

use chrono::{Days, NaiveDate};
use chrono_tz::Tz;
use tempora::storage::{Report, TimelineKind};

use crate::dashboard::format;
use crate::dashboard::nav::{ClockTime, DayZoom};
use crate::dashboard::theme::Tokens;

pub(super) const MINUTE: i64 = 60_000;

pub(super) const HOUR: i64 = 3_600_000;

/// Active hours snap to this many minutes when the activity spans at most `FINE_SPAN_MAX`.
pub(super) const FINE_SNAP: i64 = 5 * MINUTE;

/// Active hours snap to this many minutes for longer activity.
pub(super) const COARSE_SNAP: i64 = 15 * MINUTE;

/// The longest activity span that still snaps to `FINE_SNAP`.
pub(super) const FINE_SPAN_MAX: i64 = 2 * HOUR;

/// The shortest active hours window.
pub(super) const MIN_WINDOW: i64 = 15 * MINUTE;

/// Tick steps for the lane axis, finest first.
pub(super) const TICK_STEPS: [i64; 8] = [
    5 * MINUTE,
    10 * MINUTE,
    15 * MINUTE,
    30 * MINUTE,
    HOUR,
    2 * HOUR,
    3 * HOUR,
    6 * HOUR,
];

/// The most tick intervals a lane axis shows.
pub(super) const MAX_TICK_INTERVALS: i64 = 8;

/// The narrowest the lane zooms to.
pub(super) const MIN_ZOOM: i64 = 10 * MINUTE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::dashboard) enum BlockKind {
    App,
    Desktop,
    Paused,
    Gap,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::dashboard) struct BlockVm {
    pub start_ms: i64,
    pub end_ms: i64,
    pub kind: BlockKind,
    pub name: String,
    /// The app whose focus this is, for foreground focus blocks.
    pub key: Option<String>,
}

impl BlockVm {
    pub(in crate::dashboard) fn label(&self) -> &str {
        &self.name
    }
}

/// An app active this day.
pub(in crate::dashboard) struct DayApp {
    pub key: String,
    pub name: String,
    pub focused_ms: i64,
    pub running_ms: i64,
}

pub(in crate::dashboard) struct DayVm {
    pub day: (i64, i64),
    /// The calendar day and the zone it is in, for turning clock times into instants.
    pub date: NaiveDate,
    pub zone: Tz,
    pub focused_ms: i64,
    pub unknown_ms: i64,
    pub gaps_ms: i64,
    pub foreground: Vec<BlockVm>,
    /// Apps active this day, most focused first; apps never in front come last.
    pub apps: Vec<DayApp>,
}

pub(super) fn names(report: &Report) -> HashMap<&str, &str> {
    report
        .apps
        .iter()
        .map(|app| (app.key.as_str(), app.name.as_str()))
        .collect()
}

pub(super) fn clip(start: i64, end: i64, bounds: (i64, i64)) -> Option<(i64, i64)> {
    let (start, end) = (start.max(bounds.0), end.min(bounds.1));
    (start < end).then_some((start, end))
}

pub(in crate::dashboard) fn day_vm(report: &Report, date: NaiveDate, zone: Tz) -> DayVm {
    let day = (
        format::day_start(date, zone),
        format::day_start(date + Days::new(1), zone),
    );
    let names = names(report);
    let mut foreground = Vec::new();
    let (mut unknown_ms, mut gaps_ms) = (0, 0);
    for entry in &report.timeline {
        let Some((start, end)) = clip(entry.start_ms, entry.end_ms, day) else {
            continue;
        };
        let (kind, name, key) = match &entry.kind {
            TimelineKind::Focus(key) => {
                let name = names.get(key.as_str()).copied().unwrap_or("App");
                (BlockKind::App, name.to_owned(), Some(key.clone()))
            }
            TimelineKind::Desktop => (BlockKind::Desktop, "Desktop or excluded app".into(), None),
            // Unknown focus stays empty track; only its time is counted.
            TimelineKind::Unknown => {
                unknown_ms += end - start;
                continue;
            }
            TimelineKind::Paused => {
                gaps_ms += end - start;
                (BlockKind::Paused, "Paused".into(), None)
            }
            TimelineKind::Gap => {
                gaps_ms += end - start;
                (BlockKind::Gap, "Not recording".into(), None)
            }
            // Idle overlays focus; it is not drawn in the lane.
            TimelineKind::Idle => continue,
        };
        foreground.push(BlockVm {
            start_ms: start,
            end_ms: end,
            kind,
            name,
            key,
        });
    }
    let mut apps: Vec<DayApp> = report
        .apps
        .iter()
        .map(|app| DayApp {
            key: app.key.clone(),
            name: app.name.clone(),
            focused_ms: app.totals.focused_ms,
            running_ms: app.totals.running_ms,
        })
        .collect();
    apps.sort_by_key(|app| {
        (
            app.focused_ms == 0,
            std::cmp::Reverse(app.focused_ms),
            std::cmp::Reverse(app.running_ms),
        )
    });
    DayVm {
        day,
        date,
        zone,
        focused_ms: report
            .categories
            .iter()
            .map(|row| row.totals.focused_ms)
            .sum(),
        unknown_ms,
        gaps_ms,
        foreground,
        apps,
    }
}

/// Foreground app blocks narrower than this, with gaps this small, merge into one run.
pub(super) const SLIVER_PX: f32 = 4.0;

/// Apps named in a merged run's description before "+N more".
pub(super) const RUN_APPS_SHOWN: usize = 4;

/// A foreground lane item: a block as it is, or a run of slivers merged into one.
pub(super) enum LaneItem<'a> {
    Block(&'a BlockVm),
    Run(SliverRun<'a>),
}

pub(super) struct SliverRun<'a> {
    pub(super) start_ms: i64,
    pub(super) end_ms: i64,
    /// The first block of the app with the most time in the run.
    pub(super) dominant: &'a BlockVm,
    /// Blocks in the run.
    pub(super) switches: usize,
    /// Apps by time in the run, most first, ties in order of first appearance.
    pub(super) apps: Vec<(String, i64)>,
}

/// Merges runs of adjacent app blocks that would each be narrower than `min_px` (and whose gaps
/// are under `min_px`) into one run. Gap, paused and desktop blocks break runs and are never
/// merged, and a lone narrow block stays as it is. `blocks` are in time order.
pub(super) fn merge_slivers<'a>(
    blocks: &[&'a BlockVm],
    window: (i64, i64),
    lane_px: f32,
    min_px: f32,
) -> Vec<LaneItem<'a>> {
    let span = (window.1 - window.0).max(1) as f32;
    let px = |ms: i64| ms as f32 / span * lane_px;
    let is_sliver = |block: &BlockVm| {
        matches!(block.kind, BlockKind::App) && px(block.end_ms - block.start_ms) < min_px
    };
    let mut items = Vec::new();
    let mut index = 0;
    while index < blocks.len() {
        let mut end = index + 1;
        if is_sliver(blocks[index]) {
            while end < blocks.len()
                && is_sliver(blocks[end])
                && px(blocks[end].start_ms - blocks[end - 1].end_ms) < min_px
            {
                end += 1;
            }
        }
        if end - index >= 2 {
            items.push(LaneItem::Run(sliver_run(&blocks[index..end])));
        } else {
            items.push(LaneItem::Block(blocks[index]));
        }
        index = end;
    }
    items
}

/// Totals a run's time per app. Callers pass at least two blocks.
pub(super) fn sliver_run<'a>(blocks: &[&'a BlockVm]) -> SliverRun<'a> {
    let mut apps: Vec<(&'a BlockVm, i64)> = Vec::new();
    for block in blocks {
        let ms = block.end_ms - block.start_ms;
        match apps.iter_mut().find(|(first, _)| first.key == block.key) {
            Some(app) => app.1 += ms,
            None => apps.push((block, ms)),
        }
    }
    // Strictly greater, so a tie keeps the app that appeared first.
    let mut dominant = apps[0];
    for app in &apps[1..] {
        if app.1 > dominant.1 {
            dominant = *app;
        }
    }
    let mut by_time = apps.clone();
    by_time.sort_by_key(|(_, ms)| std::cmp::Reverse(*ms));
    SliverRun {
        start_ms: blocks[0].start_ms,
        end_ms: blocks[blocks.len() - 1].end_ms,
        dominant: dominant.0,
        switches: blocks.len(),
        apps: by_time
            .into_iter()
            .map(|(first, ms)| (first.name.clone(), ms))
            .collect(),
    }
}

/// The five apps with the most focused time this day get a palette color each, by rank. Ties
/// go to the name that sorts first. Apps never in front get none.
pub(super) fn app_colors(apps: &[DayApp]) -> HashMap<String, usize> {
    let mut ranked: Vec<&DayApp> = apps.iter().filter(|app| app.focused_ms > 0).collect();
    ranked.sort_by(|a, b| {
        b.focused_ms
            .cmp(&a.focused_ms)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.key.cmp(&b.key))
    });
    ranked
        .into_iter()
        .take(Tokens::APP_PALETTE.len())
        .enumerate()
        .map(|(index, app)| (app.key.clone(), index))
        .collect()
}

/// Focused time of the apps outside the palette.
pub(super) fn other_focus_ms(apps: &[DayApp], palette: &HashMap<String, usize>) -> i64 {
    apps.iter()
        .filter(|app| !palette.contains_key(&app.key))
        .map(|app| app.focused_ms)
        .sum()
}

/// The lane window. Active hours fits the recorded activity: floored and ceiled to 5 minutes
/// when the activity spans two hours or less, else to 15 minutes. It is at least 15 minutes
/// wide, extending the end and then moving the start back, and stays inside the day.
pub(super) fn window(vm: &DayVm, zoom: DayZoom) -> (i64, i64) {
    match zoom {
        DayZoom::FullDay => vm.day,
        DayZoom::Custom { from, to } => {
            let start = format::clock_instant(vm.date, from.ms, from.repeat, vm.zone);
            let end = format::clock_instant(vm.date, to.ms, to.repeat, vm.zone);
            fit(start, end - start, vm.day)
        }
        DayZoom::Evening => (
            format::clock_instant(vm.date, 17 * HOUR, false, vm.zone).min(vm.day.1),
            vm.day.1,
        ),
        DayZoom::Active => {
            let (first, last) = vm
                .foreground
                .iter()
                .fold((i64::MAX, i64::MIN), |(first, last), block| {
                    (first.min(block.start_ms), last.max(block.end_ms))
                });
            if first > last {
                return vm.day;
            }
            let snap = if last - first <= FINE_SPAN_MAX {
                FINE_SNAP
            } else {
                COARSE_SNAP
            };
            let floor = |ms: i64| vm.day.0 + (ms - vm.day.0).div_euclid(snap) * snap;
            let ceil = |ms: i64| vm.day.0 + (ms - vm.day.0 + snap - 1).div_euclid(snap) * snap;
            let mut start = floor(first).max(vm.day.0);
            let mut end = ceil(last).min(vm.day.1);
            if end - start < MIN_WINDOW {
                end = (start + MIN_WINDOW).min(vm.day.1);
                start = (end - MIN_WINDOW).max(vm.day.0);
            }
            (start, end)
        }
    }
}

/// A window `span` long starting near `start`, at least `MIN_ZOOM` and at most the day, moved
/// inside the day if it would stick out.
pub(super) fn fit(start: i64, span: i64, day: (i64, i64)) -> (i64, i64) {
    let length = (day.1 - day.0).max(1);
    let span = span.clamp(MIN_ZOOM.min(length), length);
    let start = start.clamp(day.0, day.1 - span);
    (start, start + span)
}

/// `window` zoomed by `factor` (above 1 zooms in) so that `pivot` stays at lane position `at`.
pub(super) fn zoom_window(
    window: (i64, i64),
    day: (i64, i64),
    pivot: i64,
    at: f32,
    factor: f32,
) -> (i64, i64) {
    let span = ((window.1 - window.0) as f64 / f64::from(factor.max(0.01))).round() as i64;
    let span = span.clamp(MIN_ZOOM.min(day.1 - day.0), day.1 - day.0);
    fit(
        pivot - (span as f64 * f64::from(at)).round() as i64,
        span,
        day,
    )
}

/// `window` moved later by `by` milliseconds, or earlier when negative, staying inside the day.
pub(super) fn pan_window(window: (i64, i64), day: (i64, i64), by: i64) -> (i64, i64) {
    fit(window.0 + by, window.1 - window.0, day)
}

/// The zoom that shows `window`: the whole day, or a custom span of it in clock times.
pub(super) fn zoom_for(window: (i64, i64), vm: &DayVm) -> DayZoom {
    if window == vm.day {
        DayZoom::FullDay
    } else {
        DayZoom::Custom {
            from: clock(window.0, vm),
            to: clock(window.1, vm),
        }
    }
}

/// What the clock reads at `instant` during the day, with the day's end as 24:00, and
/// whether it is the second time it reads that on a day the clocks fall back.
fn clock(instant: i64, vm: &DayVm) -> ClockTime {
    if instant >= vm.day.1 {
        return ClockTime::at(24 * HOUR);
    }
    if instant <= vm.day.0 {
        return ClockTime::at(0);
    }
    let ms = format::clock_ms(instant, vm.zone);
    let instants = format::clock_instants(vm.date, ms, vm.zone);
    ClockTime {
        ms,
        repeat: instants.len() == 2 && instants[1] == instant,
    }
}

/// Tick times and labels across the window: the finest step from 5 minutes to 6 hours that
/// gives at most eight intervals. Ticks fall on whole clock times, labelled `HH:MM` under an
/// hour's step and `HH` otherwise. On a daylight saving day a skipped time gets no tick and a
/// repeated one gets a tick for each occurrence, so labels agree with the pointer's readout.
pub(super) fn tick_times(window: (i64, i64), vm: &DayVm) -> Vec<(i64, String)> {
    let span = (window.1 - window.0).max(1);
    let step = TICK_STEPS
        .into_iter()
        .find(|&step| (span + step - 1) / step <= MAX_TICK_INTERVALS)
        .unwrap_or(6 * HOUR);
    let mut ticks: Vec<(i64, String)> = (0..=24 * HOUR / step)
        .map(|index| index * step)
        .flat_map(|reading| {
            let minutes = reading / MINUTE;
            let label = if step < HOUR {
                format!("{:02}:{:02}", minutes / 60, minutes % 60)
            } else {
                format!("{:02}", minutes / 60)
            };
            format::clock_instants(vm.date, reading, vm.zone)
                .into_iter()
                .filter(|at| (window.0..=window.1).contains(at))
                .map(move |at| (at, label.clone()))
        })
        .collect();
    ticks.sort_by_key(|(at, _)| *at);
    ticks.dedup_by_key(|(at, _)| *at);
    ticks
}

/// Tick labels across the window, at their positions in it.
#[cfg(test)]
pub(super) fn ticks(window: (i64, i64), vm: &DayVm) -> Vec<(f32, String)> {
    tick_times(window, vm)
        .into_iter()
        .map(|(at, label)| (position(at, window), label))
        .collect()
}

/// Where `ms` falls in `window`, 0 at its start and 1 at its end.
pub(super) fn position(ms: i64, window: (i64, i64)) -> f32 {
    ((ms - window.0) as f64 / (window.1 - window.0).max(1) as f64) as f32
}
