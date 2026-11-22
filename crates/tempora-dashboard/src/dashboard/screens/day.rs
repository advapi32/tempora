//! Day page: the lane of what was in front, the day's apps and the activity behind them.

mod activity;
mod activity_card;
mod lane;
mod model;
mod this_day;

use std::collections::HashMap;

use chrono::NaiveDate;
use eframe::egui::{
    self, Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, WidgetInfo, WidgetType, vec2,
};
use egui_phosphor::regular as icons;

use activity::{ActivityRow, Stretch, activity_rows};
use activity_card::*;
use lane::*;
pub(in crate::dashboard) use model::*;
use this_day::*;

use super::pointer_anchor;
use crate::dashboard::nav::{DayZoom, Page, Period, View};
use crate::dashboard::theme::Tokens;
use crate::dashboard::widgets::motion::{self, Ease};
use crate::dashboard::widgets::{self, Button, ButtonKind, LaneBlock};
use crate::dashboard::{Dashboard, format};

fn origin_crumb(origin: Option<&View>) -> String {
    match origin {
        Some(view) => {
            let page = match &view.page {
                Page::Overview => "Overview",
                Page::Library => "Library",
                Page::App(_) => "App",
                Page::Day(_) => "Timeline",
                Page::Settings => "Settings",
            };
            format!("{page} · {}", format::range_label(view.range, view.period))
        }
        None => "Timeline".into(),
    }
}

fn back_label(origin: &View) -> &'static str {
    match (&origin.page, origin.period) {
        (Page::Overview | Page::Library, Period::Week) => "Back to week",
        (Page::Overview | Page::Library, Period::Month) => "Back to month",
        (Page::App(_), _) => "Back to app",
        _ => "Back",
    }
}

enum DayAction {
    Back,
    Crumb,
    Shift(i64),
    /// The app to highlight in the lane, or none.
    SetHighlight(Option<String>),
    OpenApp(String),
    /// The app menu for a key, anchored at the pointer.
    Menu(String, Rect),
    Edit(String),
    /// A new lane zoom, from a pick, the wheel or a pan.
    Zoom(DayZoom),
}

impl Dashboard {
    pub(super) fn day_timeline(&mut self, ui: &mut Ui, date: NaiveDate, wide: bool) {
        let colors = Tokens::get(ui);
        let mut action = None;
        let origin = self.nav.origin().cloned();
        let day_title = date.format("%A, %b %-d").to_string();
        let crumb = origin_crumb(origin.as_ref());
        if widgets::breadcrumb(ui, &[&crumb, &day_title]) == Some(0) {
            action = Some(DayAction::Crumb);
        }
        let key = (date, self.nav.view.exclude_idle);
        // The previous day stays up while this one loads, drawn as the day it is.
        let shown = self
            .day_report
            .shown(&key)
            .map(|(loaded, report)| (loaded.0, report.clone()));
        let stale = shown.as_ref().is_some_and(|(loaded, _)| *loaded != date);
        let report = shown.as_ref().map(|(_, report)| report.clone());
        let vm = shown
            .as_ref()
            .map(|(loaded, report)| day_vm(report, *loaded, self.zone));
        let today = self.today();
        let online = self.recorder_online();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(origin) = &origin
                && ui
                    .add(
                        Button::new(back_label(origin), ButtonKind::Secondary)
                            .with_icon(icons::ARROW_LEFT),
                    )
                    .clicked()
            {
                action = Some(DayAction::Back);
            }
            let previous = date.pred_opt().unwrap_or(date).format("%A %b %-d");
            if ui
                .add(Button::icon(
                    &format!("Previous day, {previous}"),
                    icons::CARET_LEFT,
                ))
                .clicked()
            {
                action = Some(DayAction::Shift(-1));
            }
            let next = date.succ_opt().unwrap_or(date).format("%A %b %-d");
            if ui
                .add_enabled(
                    date < today,
                    Button::icon(&format!("Next day, {next}"), icons::CARET_RIGHT),
                )
                .clicked()
            {
                action = Some(DayAction::Shift(1));
            }
            if let Some(vm) = &vm {
                ui.add_space(8.0);
                // Scoped, so only the totals fade and the zoom presets beside them stay crisp.
                ui.scope(|ui| {
                    Self::dim_while_stale(ui, "day-totals", stale);
                    for (index, (value, label)) in
                        [(vm.focused_ms, "focused"), (vm.gaps_ms, "in gaps")]
                            .into_iter()
                            .enumerate()
                    {
                        let number = ui.label(
                            RichText::new(format::duration(value))
                                .font(Tokens::mono_medium(Tokens::SECONDARY)),
                        );
                        if index == 0 && vm.unknown_ms >= MINUTE {
                            number.on_hover_text(format!(
                                "Another {} couldn't be attributed to an app.",
                                format::duration(vm.unknown_ms)
                            ));
                        }
                        ui.label(widgets::text(label, Tokens::SECONDARY, colors.fg_muted));
                        ui.add_space(8.0);
                    }
                });
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut zoom = self.nav.view.zoom;
                widgets::segmented(
                    ui,
                    &mut zoom,
                    &[
                        (DayZoom::Active, "Active hours"),
                        (DayZoom::FullDay, "24 hours"),
                        (DayZoom::Evening, "Evening"),
                    ],
                );
                if zoom != self.nav.view.zoom {
                    self.nav.view.set_zoom(zoom);
                }
            });
        });
        Self::dim_while_stale(ui, "day-body", stale);
        if let Some(error) = self.day_report.error(&key) {
            widgets::banner(
                ui,
                widgets::BannerKind::Error,
                "This day is unavailable",
                error,
                &[],
            );
        }
        match &vm {
            Some(vm) if vm.foreground.is_empty() => {
                widgets::empty_state(
                    ui,
                    "Nothing recorded this day",
                    "The recorder wasn't running, or every app was excluded.",
                    &[],
                );
            }
            Some(vm) => {
                let lane_window = window(vm, self.nav.view.zoom);
                let palette = app_colors(&vm.apps);
                let rows = report
                    .as_ref()
                    .map(|report| activity_rows(&report.timeline, lane_window, &names(report)))
                    .unwrap_or_default();
                let highlight = highlight_key(vm, self.nav.view.lane_app.as_deref());
                if let Some(next) =
                    self.lanes_card(ui, vm, lane_window, &palette, highlight.as_deref())
                {
                    action = Some(next);
                }
                let viewport_bottom = self.viewport_bottom;
                let zone = self.zone;
                let next = if wide {
                    let gap = Tokens::GRID_GAP + 4.0;
                    let width = ui.available_width();
                    let side = (width * 0.3).max(300.0);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        let first = ui
                            .allocate_ui_with_layout(
                                vec2(width - side - gap, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(width - side - gap);
                                    activity_card(
                                        ui,
                                        &rows,
                                        &palette,
                                        zone,
                                        viewport_bottom,
                                        online,
                                    )
                                },
                            )
                            .inner;
                        let second = ui
                            .allocate_ui_with_layout(
                                vec2(side, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(side);
                                    this_day_card(ui, vm, viewport_bottom, online)
                                },
                            )
                            .inner;
                        first.or(second)
                    })
                    .inner
                } else {
                    let first = activity_card(ui, &rows, &palette, zone, viewport_bottom, online);
                    first.or_else(|| this_day_card(ui, vm, viewport_bottom, online))
                };
                if let Some(next) = next {
                    action = Some(next);
                }
            }
            None if self.day_report.loading(&key) => {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| ui.spinner());
            }
            None => {}
        }
        match action {
            Some(DayAction::Back) => {
                self.nav.go_back();
            }
            Some(DayAction::Crumb) => {
                if !self.nav.go_back() {
                    self.nav.section(Page::Overview);
                }
            }
            Some(DayAction::Shift(days)) => {
                if let Some(next) = date.checked_add_signed(chrono::Duration::days(days)) {
                    self.nav.view.page = Page::Day(next);
                }
            }
            Some(DayAction::SetHighlight(key)) => self.nav.view.lane_app = key,
            Some(DayAction::OpenApp(key)) => self.nav.drill(Page::App(key)),
            Some(DayAction::Menu(key, anchor)) => self.toggle_app_menu(key, anchor),
            // Only produced while the recorder is online (see `row_action`).
            Some(DayAction::Edit(key)) => self.open_edit(&key),
            Some(DayAction::Zoom(zoom)) => self.nav.view.set_zoom(zoom),
            None => {}
        }
    }

    fn lanes_card(
        &mut self,
        ui: &mut Ui,
        vm: &DayVm,
        target: (i64, i64),
        palette: &HashMap<String, usize>,
        highlight: Option<&str>,
    ) -> Option<DayAction> {
        let colors = Tokens::get(ui);
        let ctx = ui.ctx().clone();
        let day_start = vm.day.0;
        // The lane eases to a new zoom, so it glides rather than jumps.
        let window_id = ui.id().with("lane-window");
        let glide = |part: usize, ms: i64| {
            let offset = motion::tween(
                &ctx,
                window_id.with(part),
                (ms - day_start) as f32,
                Ease::FOLLOW,
            );
            day_start + offset.round() as i64
        };
        let shown = (glide(0, target.0), glide(1, target.1));
        let at = |ms: i64| position(ms, shown);
        let ms_at = |at: f32| shown.0 + ((shown.1 - shown.0) as f64 * f64::from(at)).round() as i64;
        // Labels for where the lane is going, placed where it is now.
        let ticks: Vec<(f32, String)> = tick_times(target, vm)
            .into_iter()
            .map(|(ms, label)| (at(ms), label))
            .collect();
        let tick_positions: Vec<f32> = ticks.iter().map(|(at, _)| *at).collect();
        // Blocks in either window, so none pop in or out while the lane glides.
        let reach = (shown.0.min(target.0), shown.1.max(target.1));
        let visible = |block: &&BlockVm| block.end_ms > reach.0 && block.start_ms < reach.1;
        let zone = self.zone;
        let describe = |block: &BlockVm| {
            format!(
                "{} – {}, {}, {}",
                format::local(block.start_ms, zone, "%H:%M"),
                format::local(block.end_ms, zone, "%H:%M"),
                block.label(),
                format::duration(block.end_ms - block.start_ms)
            )
        };
        // A legend entry under the pointer previews its highlight. The legend is drawn below
        // the lane, so this frame uses what it reported last frame.
        let preview_id = ui.id().with("legend-preview");
        let previewed: Option<String> = ui.data(|data| data.get_temp(preview_id));
        let shown_highlight = previewed.as_deref().or(highlight);
        let mut action = None;
        let zoomed = self.nav.view.zoomed();
        // The preset a reset would go back to.
        let preset = if zoomed {
            self.nav.view.zoom_base
        } else {
            self.nav.view.zoom
        };
        // The highlight before a click on the lane, so a double-click, whose two clicks each
        // toggle it, can put it back.
        let lane_app = self.nav.view.lane_app.clone();
        let before_click = ui.id().with("highlight-before-click");
        let now = ui.input(|input| input.time);
        let mut restore_highlight = None;
        let base = match self.nav.view.zoom_base {
            DayZoom::FullDay => "24 hours",
            DayZoom::Evening => "the evening",
            DayZoom::Active | DayZoom::Custom { .. } => "active hours",
        };
        let mut reset = false;
        widgets::card(ui, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            // The lane spans the card; its title sits above it like every other card's.
            let lane_width = width;
            let mut clear_highlight = false;
            ui.spacing_mut().item_spacing.y = 14.0;
            let hint = if zoomed {
                "Shift+scroll pans. Double-click or press Esc to zoom back out."
            } else {
                "Drag across the lane to zoom in. Ctrl+scroll zooms, Shift+scroll pans."
            };
            widgets::section_header(ui, "What you were using", Some(hint), |ui| {
                // Always reserve the chip's height so highlighting doesn't shift the lane.
                ui.allocate_exact_size(vec2(0.0, widgets::FILTER_CHIP_HEIGHT), Sense::hover());
                if zoomed
                    && ui
                        .add(
                            Button::new("Reset zoom", ButtonKind::Ghost)
                                .small()
                                .with_icon(icons::MAGNIFYING_GLASS_MINUS),
                        )
                        .on_hover_text(format!("Back to {base}"))
                        .clicked()
                {
                    reset = true;
                }
                if let Some(name) = highlight
                    .and_then(|key| vm.apps.iter().find(|app| app.key == key))
                    .map(|app| app.name.as_str())
                    && widgets::highlight_chip(ui, name).clicked()
                {
                    clear_highlight = true;
                }
            });
            widgets::lane_axis(ui, &ticks);
            // Slivers too thin to see merge into one block per run.
            let visible_foreground: Vec<&BlockVm> = vm.foreground.iter().filter(visible).collect();
            // The app of each block, for a right-click; merged runs have none.
            let (foreground, foreground_apps): (Vec<LaneBlock>, Vec<Option<String>>) =
                merge_slivers(&visible_foreground, target, lane_width, SLIVER_PX)
                    .into_iter()
                    .map(|item| match item {
                        LaneItem::Block(block) => (
                            LaneBlock {
                                start: at(block.start_ms),
                                end: at(block.end_ms),
                                style: style(block.kind, block.key.as_deref(), palette),
                                label: Some(block.name.clone()),
                                description: format!(
                                    "{}{}",
                                    describe(block),
                                    click_hint(block, highlight)
                                ),
                                tooltip: Some(block_tip(block, highlight, zone)),
                                dimmed: is_dimmed(block, shown_highlight),
                            },
                            block.key.clone(),
                        ),
                        LaneItem::Run(run) => (
                            LaneBlock {
                                start: at(run.start_ms),
                                end: at(run.end_ms),
                                style: style(
                                    run.dominant.kind,
                                    run.dominant.key.as_deref(),
                                    palette,
                                ),
                                label: None,
                                description: run_description(&run, zone),
                                tooltip: Some(run_tip(&run, zone)),
                                dimmed: is_dimmed(run.dominant, shown_highlight),
                            },
                            None,
                        ),
                    })
                    .unzip();
            let time = |at: f32| format::local(ms_at(at), zone, "%H:%M");
            let span = |from: f32, to: f32| {
                let (start, end) = fit(ms_at(from), ms_at(to) - ms_at(from), vm.day);
                format!(
                    "{} · {}",
                    span_label(start, end, zone, "%H:%M"),
                    format::duration(end - start)
                )
            };
            let scale = widgets::LaneScale {
                time: &time,
                span: &span,
            };
            let track = ui.cursor().min;
            let response =
                widgets::lane(ui, 44.0, &tick_positions, &foreground, None, Some(&scale));
            if foreground.is_empty() {
                let rect = Rect::from_min_size(track, vec2(lane_width, 44.0));
                ui.new_child(UiBuilder::new().max_rect(rect))
                    .centered_and_justified(|ui| {
                        ui.label(widgets::text(
                            "Nothing recorded in this window",
                            Tokens::SECONDARY,
                            colors.fg_muted,
                        ))
                    });
            }
            if let Some((index, pos)) = response.context
                && let Some(key) = foreground_apps[index].clone()
            {
                action = Some(DayAction::Menu(key, pointer_anchor(pos)));
            }
            if response.reset {
                if let Some((at, previous)) =
                    ui.data(|data| data.get_temp::<(f64, Option<String>)>(before_click))
                    && now - at < 1.0
                {
                    restore_highlight = Some(previous);
                }
            } else if let Some(index) = response.clicked
                && let Some(key) = foreground_apps[index].clone()
            {
                ui.data_mut(|data| data.insert_temp(before_click, (now, lane_app.clone())));
                let next = if highlight == Some(key.as_str()) {
                    None
                } else {
                    Some(key)
                };
                action = Some(DayAction::SetHighlight(next));
            }
            if let Some((from, to)) = response.picked {
                let (start, end) = (ms_at(from), ms_at(to));
                action = Some(DayAction::Zoom(zoom_for(
                    fit(start, end - start, vm.day),
                    vm,
                    preset,
                )));
            }
            if let Some((pointer, factor)) = response.zoom {
                let zoomed = zoom_window(target, vm.day, ms_at(pointer), pointer, factor);
                action = Some(DayAction::Zoom(zoom_for(zoomed, vm, preset)));
            }
            if let Some(by) = response.pan {
                let by = ((shown.1 - shown.0) as f64 * f64::from(by)).round() as i64;
                action = Some(DayAction::Zoom(zoom_for(
                    pan_window(target, vm.day, by),
                    vm,
                    preset,
                )));
            }
            // A double-click resets the zoom alongside the clicks it is made of, which toggle
            // a highlight on and off again; Escape resets it unless it is cancelling a drag.
            let escape =
                !response.dragging && ui.input(|input| input.key_pressed(egui::Key::Escape));
            if response.reset || escape {
                reset = true;
            }
            if clear_highlight {
                action = Some(DayAction::SetHighlight(None));
            }
            // The legend is the summary: each top app with its focused time in the day. Hover
            // an app to preview its highlight; click to keep it.
            let mut top: Vec<(usize, &DayApp)> = vm
                .apps
                .iter()
                .filter_map(|app| palette.get(&app.key).map(|&index| (index, app)))
                .collect();
            top.sort_by_key(|(index, _)| *index);
            let other_ms = other_focus_ms(&vm.apps, palette);
            let mut hovered_app = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for (index, app) in &top {
                    let on = highlight == Some(app.key.as_str());
                    let color = Tokens::APP_PALETTE[*index];
                    let faded = shown_highlight.is_some_and(|key| key != app.key);
                    let item = widgets::legend_item(
                        ui,
                        Some(if faded {
                            color.gamma_multiply(0.3)
                        } else {
                            color
                        }),
                        &format!("{} {}", app.name, format::duration(app.focused_ms)),
                    );
                    let item = ui.interact(
                        item.rect,
                        ui.id().with(("legend", &app.key)),
                        Sense::click(),
                    );
                    item.widget_info(|| {
                        let verb = if on { "Stop highlighting" } else { "Highlight" };
                        WidgetInfo::labeled(
                            WidgetType::Button,
                            ui.is_enabled(),
                            format!("{verb} {}", app.name),
                        )
                    });
                    widgets::focus_ring(ui, &item, 3);
                    if item.hovered() {
                        hovered_app = Some(app.key.clone());
                    }
                    if item.clicked() {
                        action = Some(DayAction::SetHighlight((!on).then(|| app.key.clone())));
                    }
                }
                if other_ms > 0 {
                    widgets::legend_item(
                        ui,
                        Some(Tokens::APP_OTHER),
                        &format!("Other apps {}", format::duration(other_ms)),
                    );
                }
                if vm.gaps_ms > 0 {
                    widgets::legend_item(ui, None, "Not recording");
                }
            });
            ui.data_mut(|data| match hovered_app {
                Some(key) => {
                    data.insert_temp(preview_id, key);
                }
                None => data.remove::<String>(preview_id),
            });
        });
        if reset && self.dialog.is_none() && self.popup.is_none() {
            self.nav.view.reset_zoom();
        }
        if let Some(previous) = restore_highlight {
            self.nav.view.lane_app = previous;
        }
        action
    }
}

#[cfg(test)]
fn example_app_names() -> HashMap<&'static str, &'static str> {
    [
        ("brave", "brave"),
        ("vesktop", "vesktop"),
        ("spotify", "Spotify"),
        ("steam", "Steam"),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::nav::ClockTime;
    use chrono::Days;

    /// A day with nothing recorded in `zone`, for checking windows and ticks.
    fn day_in(date: NaiveDate, zone: chrono_tz::Tz) -> DayVm {
        DayVm {
            day: (
                format::day_start(date, zone),
                format::day_start(date + Days::new(1), zone),
            ),
            date,
            zone,
            focused_ms: 0,
            unknown_ms: 0,
            gaps_ms: 0,
            foreground: Vec::new(),
            apps: Vec::new(),
        }
    }

    fn labels(ticks: Vec<(f32, String)>) -> Vec<String> {
        ticks.into_iter().map(|(_, label)| label).collect()
    }

    /// A day with one foreground block per `(from, to)` pair, offsets from midnight in ms.
    fn active_day(day: (i64, i64), spans: &[(i64, i64)]) -> DayVm {
        DayVm {
            day,
            date: chrono::DateTime::from_timestamp_millis(day.0)
                .unwrap()
                .date_naive(),
            zone: chrono_tz::UTC,
            focused_ms: 0,
            unknown_ms: 0,
            gaps_ms: 0,
            foreground: spans
                .iter()
                .map(|&(from, to)| BlockVm {
                    start_ms: day.0 + from,
                    end_ms: day.0 + to,
                    kind: BlockKind::Paused,
                    name: String::new(),
                    key: None,
                })
                .collect(),
            apps: Vec::new(),
        }
    }

    fn test_day() -> (i64, i64) {
        let date = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        (
            format::day_start(date, chrono_tz::UTC),
            format::day_start(date + Days::new(1), chrono_tz::UTC),
        )
    }

    #[test]
    fn active_hours_fit_five_minute_steps_for_short_activity() {
        let day = test_day();
        // 17:46 to 17:48 floors to 17:45 and ceils to 17:50: 5 minutes, so the window grows
        // to the 15 minute minimum.
        let short = active_day(day, &[(17 * HOUR + 46 * MINUTE, 17 * HOUR + 48 * MINUTE)]);
        let range = window(&short, DayZoom::Active);
        assert_eq!(range, (day.0 + 17 * HOUR + 45 * MINUTE, day.0 + 18 * HOUR));
        assert_eq!(
            labels(ticks(range, &short)),
            ["17:45", "17:50", "17:55", "18:00"],
            "5 minute ticks"
        );
    }

    #[test]
    fn active_hours_snap_to_fifteen_minutes_over_two_hours() {
        let day = test_day();
        // 09:05 to 15:01 spans six hours: floors to 09:00 and ceils to 15:15.
        let spread = active_day(
            day,
            &[
                (9 * HOUR + 5 * MINUTE, 9 * HOUR + 10 * MINUTE),
                (14 * HOUR, 15 * HOUR + MINUTE),
            ],
        );
        let range = window(&spread, DayZoom::Active);
        assert_eq!(range, (day.0 + 9 * HOUR, day.0 + 15 * HOUR + 15 * MINUTE));
        assert_eq!(
            labels(ticks(range, &spread)),
            ["09", "10", "11", "12", "13", "14", "15"]
        );
        assert_eq!(window(&active_day(day, &[]), DayZoom::Active), day);
        assert_eq!(
            labels(ticks(day, &active_day(day, &[]))),
            ["00", "03", "06", "09", "12", "15", "18", "21", "24"],
            "a full day keeps hour labels, every 3 hours"
        );
    }

    #[test]
    fn short_session_gets_a_fifteen_minute_minimum() {
        let day = test_day();
        // A 24-minute session from 02:12 to 02:36 floors to 02:10 and ceils to 02:40.
        let session = active_day(day, &[(2 * HOUR + 12 * MINUTE, 2 * HOUR + 36 * MINUTE)]);
        let range = window(&session, DayZoom::Active);
        assert_eq!(
            range,
            (
                day.0 + 2 * HOUR + 10 * MINUTE,
                day.0 + 2 * HOUR + 40 * MINUTE
            )
        );
        assert_eq!(
            labels(ticks(range, &session)),
            [
                "02:10", "02:15", "02:20", "02:25", "02:30", "02:35", "02:40"
            ]
        );
        // A 2 minute blip still gets the 15 minute minimum, extended from its start.
        let blip = active_day(day, &[(10 * HOUR, 10 * HOUR + 2 * MINUTE)]);
        assert_eq!(
            window(&blip, DayZoom::Active),
            (day.0 + 10 * HOUR, day.0 + 10 * HOUR + 15 * MINUTE)
        );
    }

    #[test]
    fn active_hours_clamp_to_the_day_at_midnight() {
        let day = test_day();
        // Late in the day: the end stops at midnight and the start moves back to keep 15 minutes.
        assert_eq!(
            window(
                &active_day(day, &[(23 * HOUR + 50 * MINUTE, 24 * HOUR - MINUTE)]),
                DayZoom::Active
            ),
            (day.1 - 15 * MINUTE, day.1)
        );
        // Early in the day: the start stays at midnight and the end extends to 15 minutes.
        assert_eq!(
            window(&active_day(day, &[(0, 5 * MINUTE)]), DayZoom::Active),
            (day.0, day.0 + 15 * MINUTE)
        );
    }

    fn app_block(from: i64, to: i64, key: &str) -> BlockVm {
        BlockVm {
            start_ms: from,
            end_ms: to,
            kind: BlockKind::App,
            name: example_app_names()[key].into(),
            key: Some(key.into()),
        }
    }

    /// A 100 000 ms window over a 1000 px lane: 100 ms is one pixel.
    const WINDOW: (i64, i64) = (0, 100_000);
    const LANE_PX: f32 = 1000.0;

    fn merged(blocks: &[BlockVm], lane_px: f32) -> Vec<LaneItem<'_>> {
        let refs: Vec<&BlockVm> = blocks.iter().collect();
        merge_slivers(&refs, WINDOW, lane_px, SLIVER_PX)
    }

    #[test]
    fn zooming_keeps_the_pointed_time_in_place_and_stays_inside_the_day() {
        let day = test_day();
        let window = (day.0 + 10 * HOUR, day.0 + 14 * HOUR);
        let noon = day.0 + 12 * HOUR;
        assert_eq!(
            zoom_window(window, day, noon, 0.5, 2.0),
            (day.0 + 11 * HOUR, day.0 + 13 * HOUR),
            "twice as close around noon in the middle"
        );
        assert_eq!(
            zoom_window(window, day, noon, 0.25, 2.0),
            (
                day.0 + 11 * HOUR + 30 * MINUTE,
                day.0 + 13 * HOUR + 30 * MINUTE
            ),
            "noon stays a quarter of the way across"
        );
        let close = zoom_window(window, day, noon, 0.5, 1_000.0);
        assert_eq!(
            close.1 - close.0,
            MIN_ZOOM,
            "never narrower than the minimum"
        );
        assert_eq!(
            zoom_window(window, day, noon, 0.5, 0.1),
            day,
            "zooming out stops at the whole day"
        );
        assert_eq!(
            zoom_for(day, &active_day(day, &[]), DayZoom::FullDay),
            DayZoom::FullDay,
            "zooming out to the whole day from 24 hours is just 24 hours"
        );
        assert!(
            matches!(
                zoom_for(day, &active_day(day, &[]), DayZoom::Evening),
                DayZoom::Custom { .. }
            ),
            "from Evening it stays a custom zoom, so a reset goes back to Evening"
        );
    }

    #[test]
    fn a_picked_zoom_keeps_its_clock_times_across_a_daylight_saving_change() {
        let zone: chrono_tz::Tz = chrono_tz::Europe::Berlin;
        let day_of = |date: NaiveDate| DayVm {
            day: (
                format::day_start(date, zone),
                format::day_start(date + Days::new(1), zone),
            ),
            date,
            zone,
            focused_ms: 0,
            unknown_ms: 0,
            gaps_ms: 0,
            foreground: Vec::new(),
            apps: Vec::new(),
        };
        let clock = |instant| format::local(instant, zone, "%H:%M");
        // Clocks in Berlin spring forward from 02:00 to 03:00 on Sunday, March 29, 2026.
        let saturday = NaiveDate::from_ymd_opt(2026, 3, 28).unwrap();
        let sunday = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let (before, change) = (day_of(saturday), day_of(sunday));
        let picked = (
            format::clock_instant(saturday, 9 * HOUR, false, zone),
            format::clock_instant(saturday, 10 * HOUR, false, zone),
        );
        let zoom = zoom_for(picked, &before, DayZoom::Active);
        assert_eq!(
            zoom,
            DayZoom::Custom {
                from: ClockTime::at(9 * HOUR),
                to: ClockTime::at(10 * HOUR),
                length: HOUR,
            }
        );
        let (start, end) = window(&change, zoom);
        assert_eq!(
            (clock(start), clock(end)),
            ("09:00".to_owned(), "10:00".to_owned()),
            "the same clock hours on the shorter day"
        );
        assert_eq!(labels(ticks((start, end), &change))[0], "09:00");
        let full = ticks(change.day, &change);
        assert_eq!(
            labels(full.clone()),
            ["00", "03", "06", "09", "12", "15", "18", "21", "24"],
            "ticks keep true clock labels on a 23-hour day"
        );
        assert!(
            (full[1].0 - 2.0 / 23.0).abs() < 1e-4,
            "03:00 comes two hours after midnight: {}",
            full[1].0
        );
        assert_eq!(clock(window(&change, DayZoom::Evening).0), "17:00");
    }

    #[test]
    fn a_zoom_into_the_repeated_hour_stays_in_the_occurrence_it_was_picked_in() {
        let zone = chrono_tz::Europe::Berlin;
        // Clocks in Berlin fall back from 03:00 to 02:00 on Sunday, October 25, 2026.
        let date = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let vm = day_in(date, zone);
        let second = |ms| format::clock_instants(date, ms, zone)[1];
        let picked = (
            second(2 * HOUR + 15 * MINUTE),
            second(2 * HOUR + 45 * MINUTE),
        );
        let zoom = zoom_for(picked, &vm, DayZoom::Active);
        assert!(
            matches!(
                zoom,
                DayZoom::Custom {
                    from: ClockTime { repeat: true, .. },
                    to: ClockTime { repeat: true, .. },
                    ..
                }
            ),
            "{zoom:?}"
        );
        assert_eq!(
            window(&vm, zoom),
            picked,
            "the second 02:15 to 02:45, not the hour before"
        );
        assert_eq!(
            labels(ticks(picked, &vm)),
            [
                "02:15", "02:20", "02:25", "02:30", "02:35", "02:40", "02:45"
            ],
            "the repeated hour still has its ticks"
        );
    }

    #[test]
    fn a_span_across_the_autumn_change_keeps_its_length_on_the_next_day() {
        let zone = chrono_tz::Europe::Berlin;
        // Clocks in Berlin fall back from 03:00 to 02:00 on Sunday, October 25, 2026.
        let date = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let instants = |ms| format::clock_instants(date, ms, zone);
        // From the first 02:45 to the second 02:15: half an hour.
        let picked = (
            instants(2 * HOUR + 45 * MINUTE)[0],
            instants(2 * HOUR + 15 * MINUTE)[1],
        );
        assert_eq!(picked.1 - picked.0, 30 * MINUTE);
        let zoom = zoom_for(picked, &day_in(date, zone), DayZoom::Active);
        let next = date + Days::new(1);
        let (start, end) = window(&day_in(next, zone), zoom);
        assert_eq!(
            end - start,
            30 * MINUTE,
            "reversed clock endpoints fall back to the picked length"
        );
        assert_eq!(format::local(start, zone, "%H:%M"), "02:45");
    }

    #[test]
    fn a_skipped_hour_gets_no_tick_and_the_real_one_keeps_its_label() {
        let zone = chrono_tz::Europe::Berlin;
        // Clocks in Berlin spring forward from 02:00 to 03:00 on Sunday, March 29, 2026.
        let date = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let vm = day_in(date, zone);
        let at = |ms| format::clock_instant(date, ms, false, zone);
        let window = (at(HOUR + 30 * MINUTE), at(3 * HOUR + 30 * MINUTE));
        let ticks = ticks(window, &vm);
        assert_eq!(
            labels(ticks.clone()),
            [
                "01:30", "01:40", "01:50", "03:00", "03:10", "03:20", "03:30"
            ],
            "02:00 to 02:50 never happen that day"
        );
        let three = ticks.iter().find(|(_, label)| label == "03:00").unwrap();
        assert!(
            (three.0 - 0.5).abs() < 1e-4,
            "03:00 is half an hour after 01:30: {}",
            three.0
        );
    }

    #[test]
    fn panning_stops_at_the_edges_of_the_day() {
        let day = test_day();
        let window = (day.0 + HOUR, day.0 + 3 * HOUR);
        assert_eq!(
            pan_window(window, day, 30 * MINUTE),
            (day.0 + HOUR + 30 * MINUTE, day.0 + 3 * HOUR + 30 * MINUTE)
        );
        assert_eq!(
            pan_window(window, day, -5 * HOUR),
            (day.0, day.0 + 2 * HOUR)
        );
        assert_eq!(
            pan_window(window, day, 30 * HOUR),
            (day.1 - 2 * HOUR, day.1)
        );
    }

    #[test]
    fn a_picked_zoom_keeps_its_hours_on_another_day() {
        let day = test_day();
        let zoom = zoom_for(
            (day.0 + 9 * HOUR, day.0 + 10 * HOUR),
            &active_day(day, &[]),
            DayZoom::Active,
        );
        assert_eq!(
            window(&active_day(day, &[]), zoom),
            (day.0 + 9 * HOUR, day.0 + 10 * HOUR)
        );
        let next = (day.1, day.1 + 24 * HOUR);
        assert_eq!(
            window(&active_day(next, &[]), zoom),
            (next.0 + 9 * HOUR, next.0 + 10 * HOUR)
        );
        assert_eq!(
            fit(day.0 + 9 * HOUR, MINUTE, day),
            (day.0 + 9 * HOUR, day.0 + 9 * HOUR + MIN_ZOOM),
            "a tiny pick widens to the minimum"
        );
        assert_eq!(
            fit(day.1 - MINUTE, HOUR, day),
            (day.1 - HOUR, day.1),
            "a pick at midnight stays in the day"
        );
    }

    #[test]
    fn a_run_of_slivers_merges_and_keeps_the_dominant_app() {
        // Each block is under 4 px and each gap is zero, so the four blocks form one run.
        let blocks = [
            app_block(0, 100, "vesktop"),
            app_block(100, 250, "spotify"),
            app_block(250, 350, "brave"),
            app_block(350, 500, "brave"),
        ];
        let items = merged(&blocks, LANE_PX);
        assert_eq!(items.len(), 1);
        let LaneItem::Run(run) = &items[0] else {
            panic!("the slivers merge into a run");
        };
        assert_eq!(run.dominant.key.as_deref(), Some("brave"));
        assert_eq!((run.start_ms, run.end_ms), (0, 500));
        assert_eq!(run.switches, 4);
        assert_eq!(
            run.apps,
            [
                ("brave".to_owned(), 250),
                ("Spotify".to_owned(), 150),
                ("vesktop".to_owned(), 100),
            ]
        );
    }

    #[test]
    fn a_tie_goes_to_the_app_that_appeared_first() {
        let blocks = [app_block(0, 100, "vesktop"), app_block(100, 200, "spotify")];
        let items = merged(&blocks, LANE_PX);
        let LaneItem::Run(run) = &items[0] else {
            panic!("two slivers merge");
        };
        assert_eq!(run.dominant.key.as_deref(), Some("vesktop"));
    }

    #[test]
    fn a_wide_block_breaks_a_run_and_a_lone_sliver_stays() {
        let blocks = [
            app_block(0, 100, "brave"),
            app_block(100, 200, "vesktop"),
            // 10 000 ms is 100 px wide.
            app_block(200, 10_200, "spotify"),
            app_block(10_200, 10_300, "brave"),
        ];
        let items = merged(&blocks, LANE_PX);
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], LaneItem::Run(run) if run.switches == 2));
        assert!(
            matches!(&items[1], LaneItem::Block(block) if block.key.as_deref() == Some("spotify"))
        );
        assert!(matches!(&items[2], LaneItem::Block(block) if block.start_ms == 10_200));
    }

    #[test]
    fn gaps_and_paused_blocks_break_a_run() {
        let paused = BlockVm {
            start_ms: 100,
            end_ms: 200,
            kind: BlockKind::Paused,
            name: "Paused".into(),
            key: None,
        };
        let blocks = [
            app_block(0, 100, "brave"),
            paused,
            app_block(200, 300, "brave"),
        ];
        assert_eq!(merged(&blocks, LANE_PX).len(), 3, "paused is never merged");
        // A gap of 19 px separates two slivers, so neither joins a run.
        let apart = [app_block(0, 100, "brave"), app_block(2_000, 2_100, "brave")];
        let items = merged(&apart, LANE_PX);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| matches!(item, LaneItem::Block(_))));
    }

    #[test]
    fn nothing_merges_when_the_lane_is_wide_enough() {
        let blocks = [app_block(0, 100, "brave"), app_block(100, 200, "vesktop")];
        // At 100 000 px each 100 ms block is 100 px wide.
        let items = merged(&blocks, 100_000.0);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| matches!(item, LaneItem::Block(_))));
    }

    #[test]
    fn a_run_describes_its_switches_and_names_at_most_four_apps() {
        let apps = [
            ("brave", 3 * MINUTE),
            ("vesktop", MINUTE),
            ("spotify", 30 * 1_000),
        ];
        let run = SliverRun {
            start_ms: 10 * HOUR,
            end_ms: 10 * HOUR + 5 * MINUTE,
            dominant: &app_block(0, 1, "brave"),
            switches: 9,
            apps: apps
                .iter()
                .map(|(key, ms)| (example_app_names()[key].to_owned(), *ms))
                .collect(),
        };
        assert_eq!(
            run_description(&run, chrono_tz::UTC),
            "10:00–10:05 · 9 switches: brave 3m, vesktop 1m, Spotify <1m"
        );
        let many = SliverRun {
            apps: (0..6)
                .map(|index| (format!("App {index}"), MINUTE))
                .collect(),
            ..run
        };
        assert!(
            run_description(&many, chrono_tz::UTC)
                .ends_with("App 0 1m, App 1 1m, App 2 1m, App 3 1m, +2 more")
        );
    }

    fn day_app(key: &str, name: &str, focused_min: i64) -> DayApp {
        DayApp {
            key: key.into(),
            name: name.into(),
            focused_ms: focused_min * MINUTE,
            running_ms: focused_min * MINUTE,
        }
    }

    #[test]
    fn app_colors_take_the_five_most_focused_apps_in_rank_order() {
        let apps = vec![
            day_app("mail", "Mail", 10),
            day_app("steam", "Steam", 50),
            day_app("notes", "Notes", 0),
            day_app("code", "Code", 40),
            day_app("spotify", "Spotify", 30),
            day_app("brave", "Brave", 40),
            day_app("vesktop", "Vesktop", 20),
        ];
        let colors = app_colors(&apps);
        let ranks = |key: &str| colors.get(key).copied();
        assert_eq!(ranks("steam"), Some(0), "most focused first");
        assert_eq!(
            ranks("brave"),
            Some(1),
            "a tie goes to the name that sorts first"
        );
        assert_eq!(ranks("code"), Some(2));
        assert_eq!(ranks("spotify"), Some(3));
        assert_eq!(ranks("vesktop"), Some(4));
        assert_eq!(ranks("mail"), None, "the sixth app is grey");
        assert_eq!(ranks("notes"), None, "an app never in front has no color");
        assert_eq!(colors.len(), 5);
        assert_eq!(other_focus_ms(&apps, &colors), 10 * MINUTE);
    }

    #[test]
    fn app_colors_do_not_depend_on_input_order() {
        let apps = vec![
            day_app("a", "Alpha", 20),
            day_app("b", "Beta", 20),
            day_app("c", "Gamma", 5),
        ];
        let mut reversed = apps
            .iter()
            .map(|app| day_app(&app.key, &app.name, app.focused_ms / MINUTE))
            .collect::<Vec<_>>();
        reversed.reverse();
        assert_eq!(app_colors(&apps), app_colors(&reversed));
        assert_eq!(app_colors(&apps)["a"], 0);
        assert_eq!(app_colors(&apps)["b"], 1);
    }

    #[test]
    fn a_highlight_needs_focus_that_day() {
        let vm = DayVm {
            day: (0, 1),
            date: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            zone: chrono_tz::UTC,
            focused_ms: 0,
            unknown_ms: 0,
            gaps_ms: 0,
            foreground: Vec::new(),
            apps: vec![day_app("steam", "Steam", 60), day_app("brave", "Brave", 0)],
        };
        assert_eq!(highlight_key(&vm, Some("steam")).as_deref(), Some("steam"));
        assert_eq!(
            highlight_key(&vm, Some("brave")),
            None,
            "open but never in front: nothing to highlight"
        );
        assert_eq!(highlight_key(&vm, Some("missing")), None);
        assert_eq!(highlight_key(&vm, None), None);
        let steam = app_block(0, 100, "steam");
        assert_eq!(
            click_hint(&steam, Some("steam")),
            " Click to stop highlighting."
        );
        assert_eq!(click_hint(&steam, Some("brave")), " Click to highlight.");
    }

    #[test]
    fn only_other_apps_are_dimmed_and_only_with_a_running_app() {
        let brave = app_block(0, 100, "brave");
        let desktop = BlockVm {
            key: None,
            kind: BlockKind::Desktop,
            ..brave.clone()
        };
        assert!(is_dimmed(&brave, Some("vesktop")));
        assert!(!is_dimmed(&brave, Some("brave")));
        assert!(!is_dimmed(&brave, None), "no running app, nothing dimmed");
        assert!(
            !is_dimmed(&desktop, Some("vesktop")),
            "desktop is never dimmed"
        );
    }
}
