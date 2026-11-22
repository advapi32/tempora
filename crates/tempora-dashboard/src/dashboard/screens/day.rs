//! Day page: the lane of what was in front, the day's apps and the activity behind them.

mod activity;
mod activity_card;
mod lane;
mod model;
mod this_day;

use std::collections::HashMap;

use chrono::NaiveDate;
use eframe::egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use egui_phosphor::regular as icons;

use activity::{ActivityRow, Stretch, activity_rows};
use activity_card::*;
use lane::*;
pub(in crate::dashboard) use model::*;
use this_day::*;

use super::pointer_anchor;
use crate::dashboard::nav::{DayZoom, Page, Period, View};
use crate::dashboard::theme::Tokens;
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
        let report = self.day_report.get(&key).cloned();
        let vm = report
            .as_ref()
            .map(|report| day_vm(report, date, self.zone));
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
                for (index, (value, label)) in [(vm.focused_ms, "focused"), (vm.gaps_ms, "in gaps")]
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
                self.nav.view.zoom = zoom;
            });
        });
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
            None => {}
        }
    }

    fn lanes_card(
        &mut self,
        ui: &mut Ui,
        vm: &DayVm,
        (start, end): (i64, i64),
        palette: &HashMap<String, usize>,
        highlight: Option<&str>,
    ) -> Option<DayAction> {
        let colors = Tokens::get(ui);
        let span = (end - start).max(1) as f32;
        let at = |ms: i64| (ms - start) as f32 / span;
        let ticks = ticks((start, end), vm.day.0);
        let tick_positions: Vec<f32> = ticks.iter().map(|(at, _)| *at).collect();
        let visible = |block: &&BlockVm| block.end_ms > start && block.start_ms < end;
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
        let mut action = None;
        widgets::card(ui, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            // The lane spans the card; its title sits above it like every other card's.
            let lane_width = width;
            let mut clear_highlight = false;
            ui.spacing_mut().item_spacing.y = 14.0;
            widgets::section_header(ui, "What you were using", None, |ui| {
                // Always reserve the chip's height so highlighting doesn't shift the lane.
                ui.allocate_exact_size(vec2(0.0, widgets::FILTER_CHIP_HEIGHT), Sense::hover());
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
                merge_slivers(&visible_foreground, (start, end), lane_width, SLIVER_PX)
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
                                dimmed: is_dimmed(block, highlight),
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
                                dimmed: is_dimmed(run.dominant, highlight),
                            },
                            None,
                        ),
                    })
                    .unzip();
            let track = ui.cursor().min;
            let response = widgets::lane(ui, 44.0, &tick_positions, &foreground, None);
            let foreground_clicked = response.clicked;
            let foreground_context = response.context;
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
            if let Some((index, pos)) = foreground_context
                && let Some(key) = foreground_apps[index].clone()
            {
                action = Some(DayAction::Menu(key, pointer_anchor(pos)));
            }
            if let Some(index) = foreground_clicked
                && let Some(key) = foreground_apps[index].clone()
            {
                let next = if highlight == Some(key.as_str()) {
                    None
                } else {
                    Some(key)
                };
                action = Some(DayAction::SetHighlight(next));
            }
            if clear_highlight {
                action = Some(DayAction::SetHighlight(None));
            }
            // The legend is the summary: each top app with its focused time in the day.
            let mut top: Vec<(usize, &DayApp)> = vm
                .apps
                .iter()
                .filter_map(|app| palette.get(&app.key).map(|&index| (index, app)))
                .collect();
            top.sort_by_key(|(index, _)| *index);
            let other_ms = other_focus_ms(&vm.apps, palette);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for (index, app) in &top {
                    widgets::legend_item(
                        ui,
                        Some(Tokens::APP_PALETTE[*index]),
                        &format!("{} {}", app.name, format::duration(app.focused_ms)),
                    );
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
        });
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
    use chrono::Days;

    fn labels(ticks: Vec<(f32, String)>) -> Vec<String> {
        ticks.into_iter().map(|(_, label)| label).collect()
    }

    /// A day with one foreground block per `(from, to)` pair, offsets from midnight in ms.
    fn active_day(day: (i64, i64), spans: &[(i64, i64)]) -> DayVm {
        DayVm {
            day,
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
            labels(ticks(range, day.0)),
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
            labels(ticks(range, day.0)),
            ["09", "10", "11", "12", "13", "14", "15"]
        );
        assert_eq!(window(&active_day(day, &[]), DayZoom::Active), day);
        assert_eq!(
            labels(ticks(day, day.0)),
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
            labels(ticks(range, day.0)),
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
