//! App detail (`AppDetail` mockup): header, KPIs, daily trend, sessions, identity, actions.

use chrono::NaiveDate;
use chrono_tz::Tz;
use eframe::egui::{self, Align, Layout, RichText, Ui, vec2};
use egui_phosphor::regular as icons;
use tempora::{
    model::{CAPABILITY_RESET_APP, CAPABILITY_UNGROUP_APP, Category, ControlRequest},
    storage::{Report, TimelineKind},
};

use super::Popup;
use super::library::last_seen;
use crate::dashboard::nav::{Page, View};
use crate::dashboard::theme::Tokens;
use crate::dashboard::widgets::{
    self, Button, ButtonKind, Column, Kpi, PairedBar, RowStyle, Width,
};
use crate::dashboard::{AppInfo, Dashboard, format};

pub(in crate::dashboard) struct DetailVm {
    pub key: String,
    pub name: String,
    pub category: Category,
    pub source: format::Source,
    /// Present when the app has activity in the selected range.
    pub range: Option<RangeVm>,
    pub identity: Option<IdentityVm>,
    /// Focused time by local hour across the range; `None` without focus.
    pub hours: Option<[i64; 24]>,
}

pub(in crate::dashboard) struct RangeVm {
    pub focused_ms: i64,
    pub running_ms: i64,
    pub background_ms: i64,
    pub sessions: usize,
    pub average_ms: i64,
    pub last_seen_ms: i64,
    pub days: Vec<(NaiveDate, i64, i64, i64)>,
    pub session_rows: Vec<SessionVm>,
}

pub(in crate::dashboard) struct SessionVm {
    pub start_ms: i64,
    pub end_ms: i64,
    pub date: NaiveDate,
    pub running_ms: i64,
    pub focused_ms: i64,
}

pub(in crate::dashboard) struct IdentityVm {
    pub id: Option<String>,
    /// Every source key grouped under the app, the app's own key included.
    pub sources: Vec<String>,
    pub executables: Vec<String>,
    pub first_seen: Option<NaiveDate>,
    pub all_time_focused_ms: Option<i64>,
    /// The recognized name and category, when the user's edits replaced them.
    pub automatic: Option<(String, Category)>,
}

pub(in crate::dashboard) fn detail_vm(
    key: &str,
    report: Option<&Report>,
    info: Option<&AppInfo>,
    catalog_name: Option<(&str, Category)>,
    zone: Tz,
    today: NaiveDate,
) -> Option<DetailVm> {
    let app = report.and_then(|report| report.apps.iter().find(|app| app.key == key));
    let all_time = info.and_then(|info| info.all_time.as_ref());
    let (name, category) = app
        .map(|app| (app.name.clone(), app.category))
        .or_else(|| all_time.map(|app| (app.name.clone(), app.category)))
        .or_else(|| catalog_name.map(|(name, category)| (name.to_owned(), category)))?;
    let source = format::source(key);
    let range = app.map(|app| {
        let mut session_rows: Vec<_> = app
            .sessions
            .iter()
            .map(|session| SessionVm {
                start_ms: session.start_ms,
                end_ms: session.end_ms,
                date: format::local_date(session.start_ms, zone).unwrap_or(today),
                running_ms: session.totals.running_ms,
                focused_ms: session.totals.focused_ms,
            })
            .collect();
        session_rows.reverse();
        let count = app.sessions.len();
        RangeVm {
            focused_ms: app.totals.focused_ms,
            running_ms: app.totals.running_ms,
            background_ms: app.totals.background_ms,
            sessions: count,
            average_ms: if count == 0 {
                0
            } else {
                app.totals.running_ms / count as i64
            },
            last_seen_ms: app.last_seen_ms,
            days: app
                .days
                .iter()
                .map(|(date, totals)| {
                    (
                        *date,
                        totals.focused_ms,
                        totals.running_ms,
                        totals.background_ms,
                    )
                })
                .collect(),
            session_rows,
        }
    });
    let identity = info.map(|info| IdentityVm {
        id: source.id.clone(),
        sources: info.sources.clone(),
        executables: info
            .sources
            .iter()
            .filter_map(|key| format::source(key).path)
            .collect(),
        first_seen: all_time
            .and_then(|app| app.sessions.first())
            .and_then(|session| format::local_date(session.start_ms, zone)),
        all_time_focused_ms: all_time.map(|app| app.totals.focused_ms),
        automatic: info
            .automatic
            .clone()
            .filter(|automatic| *automatic != (name.clone(), category)),
    });
    let hours = report.and_then(|report| focus_by_hour(report, key, zone));
    Some(DetailVm {
        key: key.to_owned(),
        name,
        category,
        source,
        range,
        identity,
        hours,
    })
}

/// A session's day: "Today", "Yesterday" or "Sat, Nov 21".
fn day_label(date: NaiveDate, today: NaiveDate) -> String {
    if date == today {
        "Today".into()
    } else if Some(date) == today.pred_opt() {
        "Yesterday".into()
    } else {
        date.format("%a, %b %-d").to_string()
    }
}

fn recognition(source: &format::Source) -> Option<&'static str> {
    match source.kind {
        "Steam" => Some("Recognized from Steam library"),
        "Riot" => Some("Recognized Riot game"),
        "Product" => Some("Recognized product"),
        "Package" => Some("Store app package"),
        _ => None,
    }
}

fn origin_label(view: Option<&View>) -> String {
    match view.map(|view| &view.page) {
        Some(Page::Overview) => "Overview".into(),
        Some(Page::Day(date)) => date.format("%a, %b %-d").to_string(),
        Some(Page::App(_)) => "App".into(),
        Some(Page::Settings) => "Settings".into(),
        Some(Page::Library) | None => "Library".into(),
    }
}

enum DetailAction {
    OpenDay(NaiveDate),
    Edit,
    Group,
    StopTracking,
    Delete,
    RemoveFromGroup(String),
    Reset,
}

impl Dashboard {
    pub(super) fn app_detail(&mut self, ui: &mut Ui, key: &str, wide: bool) {
        let report = self.current_report();
        let info = self.info(key);
        let catalog = self
            .apps
            .iter()
            .find(|app| app.key == key)
            .map(|app| (app.name.as_str(), app.category));
        let Some(vm) = detail_vm(
            key,
            report.as_deref(),
            info.as_deref(),
            catalog,
            self.zone,
            self.today(),
        ) else {
            self.report_status(ui);
            return;
        };
        let origin = origin_label(self.nav.origin());
        if widgets::breadcrumb(ui, &[&origin, &vm.name]) == Some(0) && !self.nav.go_back() {
            self.nav.section(Page::Library);
        }
        let mut action = self.detail_header(ui, &vm);
        self.report_status(ui);
        Self::dim_while_stale(ui, "app-detail", self.report_is_stale());
        if let Some(range) = &vm.range {
            let kpis = detail_kpis(range, self.zone, self.today());
            widgets::kpi_grid(ui, &kpis, if wide { 5 } else { 3 });
        }
        let online = self.recorder_online() && !self.pending;
        let correction_blocked = |capability| {
            if !self.recorder_online() {
                Some("The recorder isn't running")
            } else if !self.recorder_can(capability) {
                Some("Update the recorder to use this")
            } else if self.pending {
                Some("Wait for the current recorder action to finish")
            } else {
                None
            }
        };
        let ungroup_blocked = correction_blocked(CAPABILITY_UNGROUP_APP);
        let reset_blocked = correction_blocked(CAPABILITY_RESET_APP);
        let range_label = self.report_range_label();
        let main = |ui: &mut Ui, this: &mut Self| -> Option<DetailAction> {
            match &vm.range {
                Some(range) => {
                    let trend = this.trend_card(ui, &vm, range);
                    let sessions = this.sessions_card(ui, &vm, range);
                    trend.or(sessions)
                }
                None => {
                    let label = this.report_range_label();
                    widgets::empty_state(
                        ui,
                        "No activity in this range",
                        &format!("{} has no recorded sessions in {label}.", vm.name),
                        &[],
                    );
                    None
                }
            }
        };
        let side = |ui: &mut Ui| -> Option<DetailAction> {
            let identity_action = vm.identity.as_ref().and_then(|identity| {
                identity_card(ui, &vm, identity, ungroup_blocked, reset_blocked)
            });
            if let Some(hours) = &vm.hours {
                hours_card(ui, &vm, hours, &range_label);
            }
            let mut picked = None;
            widgets::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_enabled_ui(online, |ui| {
                    if widgets::menu_item(ui, "Stop tracking this app", icons::PROHIBIT, false)
                        .on_hover_text("Keeps the existing history")
                        .clicked()
                    {
                        picked = Some(DetailAction::StopTracking);
                    }
                    if widgets::menu_item(ui, "Delete history…", icons::TRASH, true).clicked() {
                        picked = Some(DetailAction::Delete);
                    }
                });
            });
            identity_action.or(picked)
        };
        let (main_action, side_action) = if wide {
            let gap = Tokens::GRID_GAP + 4.0;
            let width = ui.available_width();
            let side_width = (width * 0.3).max(300.0);
            let main_width = width - side_width - gap;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let main_action = ui
                    .allocate_ui_with_layout(
                        vec2(main_width, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(main_width);
                            main(ui, self)
                        },
                    )
                    .inner;
                let side_action = ui
                    .allocate_ui_with_layout(
                        vec2(side_width, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(side_width);
                            side(ui)
                        },
                    )
                    .inner;
                (main_action, side_action)
            })
            .inner
        } else {
            let main_action = main(ui, self);
            (main_action, side(ui))
        };
        action = action.or(main_action).or(side_action);
        match action {
            Some(DetailAction::OpenDay(date)) => {
                let key = vm.key.clone();
                self.nav
                    .drill_with(Page::Day(date), |view| view.lane_app = Some(key));
            }
            Some(DetailAction::Edit) => self.open_edit(&vm.key),
            Some(DetailAction::Group) => self.open_group(&vm.key),
            Some(DetailAction::StopTracking) => self.stop_tracking(&vm.key),
            Some(DetailAction::Delete) => self.open_delete(Some(&vm.key)),
            Some(DetailAction::RemoveFromGroup(source)) => self.action(
                ControlRequest::UngroupApp { key: source },
                "Removed from group",
            ),
            Some(DetailAction::Reset) => self.action(
                ControlRequest::ResetApp {
                    key: vm.key.clone(),
                },
                "Reset to automatic",
            ),
            None => {}
        }
    }

    fn detail_header(&mut self, ui: &mut Ui, vm: &DetailVm) -> Option<DetailAction> {
        let mut action = None;
        let running = self.recorder.as_ref().is_some_and(|status| {
            status.candidates.iter().any(|app| {
                app.key == vm.key
                    || vm
                        .identity
                        .as_ref()
                        .is_some_and(|identity| identity.sources.contains(&app.key))
            })
        });
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            vec2(width, 56.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                widgets::icon_tile(ui, Some(&vm.key), &vm.name, vm.category, 56.0);
                let actions_width = 210.0 + 3.0 * Tokens::CONTROL_GAP + 300.0;
                let text_width = (ui.available_width() - actions_width).max(200.0);
                ui.allocate_ui_with_layout(
                    vec2(text_width, 56.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.add(
                            egui::Label::new(crate::dashboard::theme::display(&vm.name, 24.0))
                                .truncate()
                                .selectable(false),
                        );
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            widgets::category_tag(ui, vm.category);
                            let colors = Tokens::get(ui);
                            if running {
                                widgets::tone_tag(ui, "Running now", colors.ok);
                            }
                            widgets::tone_tag(ui, &vm.source.line, colors.fg_muted);
                            if let Some(recognition) = recognition(&vm.source) {
                                widgets::tone_tag(ui, recognition, colors.fg_muted);
                            }
                        });
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = Tokens::CONTROL_GAP;
                    let online = self.recorder_online() && !self.pending;
                    let more = ui.add(Button::icon("More actions", icons::DOTS_THREE));
                    if more.clicked() {
                        self.popup = match self.popup {
                            Some(Popup::More) => None,
                            _ => Some(Popup::More),
                        };
                    }
                    if ui
                        .add_enabled(
                            online,
                            Button::new("Group with…", ButtonKind::Secondary)
                                .with_icon(icons::STACK),
                        )
                        .clicked()
                    {
                        action = Some(DetailAction::Group);
                    }
                    if ui
                        .add_enabled(
                            online,
                            Button::new("Edit app", ButtonKind::Secondary)
                                .with_icon(icons::PENCIL_SIMPLE),
                        )
                        .clicked()
                    {
                        action = Some(DetailAction::Edit);
                    }
                    self.range_control(ui);
                    if matches!(self.popup, Some(Popup::More)) {
                        let last_day = vm
                            .range
                            .as_ref()
                            .and_then(|range| format::local_date(range.last_seen_ms, self.zone));
                        let output = widgets::anchored_menu(
                            ui.ctx(),
                            egui::Id::new("detail-more"),
                            more.rect,
                            Align::Max,
                            |ui| {
                                ui.set_width(220.0);
                                let mut picked = None;
                                if let Some(day) = last_day
                                    && widgets::menu_item(
                                        ui,
                                        "Open last active day",
                                        icons::ROWS,
                                        false,
                                    )
                                    .clicked()
                                {
                                    picked = Some(DetailAction::OpenDay(day));
                                }
                                ui.add_enabled_ui(online, |ui| {
                                    if widgets::menu_item(
                                        ui,
                                        "Stop tracking this app",
                                        icons::PROHIBIT,
                                        false,
                                    )
                                    .clicked()
                                    {
                                        picked = Some(DetailAction::StopTracking);
                                    }
                                    if widgets::menu_item(ui, "Delete history…", icons::TRASH, true)
                                        .clicked()
                                    {
                                        picked = Some(DetailAction::Delete);
                                    }
                                });
                                picked
                            },
                        );
                        if output.inner.is_some() || output.dismissed {
                            self.popup = None;
                        }
                        if output.inner.is_some() {
                            action = output.inner;
                        }
                    }
                });
            },
        );
        action
    }

    fn trend_card(&mut self, ui: &mut Ui, vm: &DetailVm, range: &RangeVm) -> Option<DetailAction> {
        let colors = Tokens::get(ui);
        let color = colors.category(vm.category);
        let today = self.today();
        let mut opened = None;
        widgets::card(ui, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            ui.spacing_mut().item_spacing.y = 14.0;
            let mut table = self.nav.view.trend_table;
            widgets::section_header(ui, "Daily trend", None, |ui| {
                widgets::segmented(ui, &mut table, &[(false, "Chart"), (true, "Table")]);
                ui.add_space(6.0);
                widgets::legend_item(ui, Some(color.gamma_multiply(0.28)), "Running");
                widgets::legend_item(ui, Some(color), "Focused");
            });
            self.nav.view.trend_table = table;
            if table {
                let specs = [
                    Column::new("Day", Width::Flex(1.0, 120.0), Align::Min),
                    Column::new("Focused", Width::Fixed(90.0), Align::Max),
                    Column::new("Running", Width::Fixed(90.0), Align::Max),
                    Column::new("Background", Width::Fixed(100.0), Align::Max),
                ];
                let columns = widgets::columns(width, &specs);
                ui.label(widgets::text(
                    format!(
                        "{}, days with activity in {}",
                        vm.name,
                        self.report_range_label()
                    ),
                    Tokens::CAPTION,
                    colors.fg_subtle,
                ));
                ui.spacing_mut().item_spacing.y = 0.0;
                widgets::table_header(ui, &columns, &specs.map(|column| column.label));
                let active: Vec<_> = range
                    .days
                    .iter()
                    .filter(|(_, _, running, _)| *running > 0)
                    .collect();
                for (index, (date, focused, running, background)) in active.iter().enumerate() {
                    let day = date.format("%a, %b %-d").to_string();
                    let label = format!(
                        "{day}: {} focused, {} running, {} background. Open timeline.",
                        format::duration(*focused),
                        format::duration(*running),
                        format::duration(*background)
                    );
                    let row = widgets::table_row(
                        ui,
                        ui.id().with(("trend-row", date)),
                        &columns,
                        RowStyle {
                            height: 36.0,
                            separator: index + 1 < active.len(),
                            ..RowStyle::default()
                        },
                        &label,
                        |row| {
                            row.cell(0, |ui| {
                                widgets::truncated(ui, RichText::new(&day).size(Tokens::SECONDARY));
                            });
                            row.number(1, &format::duration(*focused), colors.fg);
                            row.number(2, &format::duration(*running), colors.fg_muted);
                            row.number(3, &format::duration(*background), colors.fg_muted);
                        },
                    );
                    if row.clicked() {
                        opened = Some(*date);
                    }
                }
            } else {
                let bars: Vec<_> = range
                    .days
                    .iter()
                    .map(|(date, focused, running, _)| PairedBar {
                        label: date.format("%-d").to_string(),
                        description: format!(
                            "{}: {} focused, {} running. Open timeline.",
                            date.format("%a %b %-d"),
                            format::duration(*focused),
                            format::duration(*running)
                        ),
                        front_ms: *focused,
                        back_ms: *running,
                        today: *date == today,
                    })
                    .collect();
                if let Some(index) = widgets::paired_bars(ui, &bars, color, 180.0) {
                    opened = Some(range.days[index].0);
                }
            }
        });
        opened.map(DetailAction::OpenDay)
    }

    fn sessions_card(
        &mut self,
        ui: &mut Ui,
        vm: &DetailVm,
        range: &RangeVm,
    ) -> Option<DetailAction> {
        let colors = Tokens::get(ui);
        let today = self.today();
        let mut opened = None;
        widgets::table_card(ui, true, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                crate::dashboard::theme::section(ui, "Sessions");
                ui.label(
                    widgets::mono(
                        format!("{} in {}", range.sessions, self.report_range_label()),
                        Tokens::CAPTION,
                    )
                    .color(colors.fg_subtle),
                );
            });
            ui.add_space(10.0);
            let specs = [
                Column::new("Date", Width::Fixed(110.0), Align::Min),
                Column::new("Time", Width::Fixed(116.0), Align::Min),
                Column::new("Running", Width::Fixed(76.0), Align::Max),
                Column::new("Focused", Width::Fixed(76.0), Align::Max),
                Column::new("In front", Width::Flex(1.0, 96.0), Align::Min),
                Column::new("", Width::Fixed(16.0), Align::Max),
            ];
            let columns = widgets::scrolled_columns(ui, width, &specs);
            widgets::table_header(ui, &columns, &specs.map(|column| column.label));
            // Explicit height: the column this card sits in has no height of its own, so
            // the scroll area would otherwise collapse to egui's 64 px minimum.
            super::fitted_list(
                self.viewport_bottom,
                ui,
                "detail-sessions",
                widgets::TITLED_TABLE_BOTTOM,
                |ui| {
                    let mut previous_day = None;
                    for session in &range.session_rows {
                        let date = day_label(session.date, today);
                        // Sessions are newest first, so a day's runs sit together: name the
                        // day once, on its first row.
                        let first_of_day = previous_day != Some(session.date);
                        previous_day = Some(session.date);
                        let time = format!(
                            "{} – {}",
                            format::local(session.start_ms, self.zone, "%H:%M"),
                            format::local(session.end_ms, self.zone, "%H:%M")
                        );
                        let label = format!(
                            "{date}, {time}: {} running, {} focused. Open the day.",
                            format::duration(session.running_ms),
                            format::duration(session.focused_ms),
                        );
                        let row = widgets::table_row(
                            ui,
                            ui.id().with(("session-row", session.start_ms)),
                            &columns,
                            RowStyle {
                                height: 42.0,
                                ..RowStyle::default()
                            },
                            &label,
                            |row| {
                                if first_of_day {
                                    row.cell(0, |ui| {
                                        widgets::truncated(
                                            ui,
                                            RichText::new(&date)
                                                .font(Tokens::medium(Tokens::SECONDARY)),
                                        );
                                    });
                                }
                                row.cell(1, |ui| {
                                    ui.label(
                                        widgets::mono(&time, Tokens::SECONDARY)
                                            .color(colors.fg_muted),
                                    );
                                });
                                row.number(2, &format::duration(session.running_ms), colors.fg);
                                row.number(3, &format::duration(session.focused_ms), colors.fg);
                                row.cell(4, |ui| {
                                    // Open versus in front: the full bar is the running time.
                                    let share = widgets::in_front_share(
                                        session.focused_ms,
                                        session.running_ms,
                                    );
                                    let label = format!(
                                        "{} in front of {} open",
                                        format::duration(session.focused_ms),
                                        format::duration(session.running_ms)
                                    );
                                    widgets::open_bar(
                                        ui,
                                        colors.category(vm.category),
                                        share,
                                        &label,
                                    );
                                });
                                row.cell(5, |ui| {
                                    ui.label(
                                        RichText::new(icons::CARET_RIGHT)
                                            .font(Tokens::icon(Tokens::CAPTION))
                                            .color(colors.fg_subtle),
                                    );
                                });
                            },
                        );
                        if row.clicked() {
                            opened = Some(DetailAction::OpenDay(session.date));
                        }
                    }
                },
            );
        });
        let _ = vm;
        opened
    }
}

fn detail_kpis<'a>(range: &RangeVm, zone: Tz, today: NaiveDate) -> [Kpi<'a>; 5] {
    let kpi = |label, icon, value: String, caption: String| Kpi {
        value_size: 24.0,
        caption: Some(caption),
        ..Kpi::new(label, icon, value)
    };
    let duration = |label, icon, ms: i64, caption: &str| Kpi {
        roll_ms: Some(ms),
        ..kpi(label, icon, format::duration(ms), caption.to_owned())
    };
    let seen_day = format::local_date(range.last_seen_ms, zone);
    let seen = last_seen(range.last_seen_ms, zone, today);
    let (seen_value, seen_caption) = if seen_day == Some(today) {
        (
            format::local(range.last_seen_ms, zone, "%H:%M"),
            "Today".to_owned(),
        )
    } else {
        (seen, format::local(range.last_seen_ms, zone, "%H:%M"))
    };
    [
        duration(
            "Focused",
            icons::CLOCK,
            range.focused_ms,
            "In the foreground",
        ),
        duration("Running", icons::PLAY, range.running_ms, "Any process open"),
        duration(
            "Background",
            icons::STACK,
            range.background_ms,
            "Another app had focus",
        ),
        kpi(
            "Sessions",
            icons::LIST_BULLETS,
            range.sessions.to_string(),
            format!("Avg {}", format::duration(range.average_ms)),
        ),
        kpi("Last seen", icons::EYE, seen_value, seen_caption),
    ]
}

/// `ungroup_blocked` is the reason "Remove from group" is unavailable, or `None` when it can
/// be used.
fn identity_card(
    ui: &mut Ui,
    vm: &DetailVm,
    identity: &IdentityVm,
    ungroup_blocked: Option<&str>,
    reset_blocked: Option<&str>,
) -> Option<DetailAction> {
    let colors = Tokens::get(ui);
    widgets::card(ui, |ui| {
        let mut picked = None;
        let width = ui.available_width();
        ui.set_min_width(width);
        ui.spacing_mut().item_spacing.y = 10.0;
        crate::dashboard::theme::section(ui, "Identity");
        let mut rows: Vec<(&str, Vec<String>, bool)> =
            vec![("Source", vec![vm.source.kind.to_owned()], false)];
        if let Some(id) = &identity.id {
            rows.push((
                if vm.source.kind == "Steam" {
                    "App ID"
                } else {
                    "ID"
                },
                vec![id.clone()],
                true,
            ));
        }
        // A grouped app lists its sources below, each with Remove from group; listing the
        // executables here too would show the same paths twice.
        if !identity.executables.is_empty() && identity.sources.len() <= 1 {
            rows.push(("Executables", identity.executables.clone(), true));
        }
        if let Some(first) = identity.first_seen {
            rows.push((
                "First seen",
                vec![first.format("%b %-d, %Y").to_string()],
                false,
            ));
        }
        if let Some(focused) = identity.all_time_focused_ms {
            rows.push((
                "All-time",
                vec![format!("{} focused", format::duration(focused))],
                false,
            ));
        }
        for (label, values, mono) in rows {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.allocate_ui_with_layout(
                    vec2(100.0, 18.0),
                    Layout::left_to_right(Align::Min),
                    |ui| {
                        ui.label(widgets::text(label, Tokens::SECONDARY, colors.fg_subtle));
                    },
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for value in values {
                        let text = if mono {
                            widgets::mono(value, Tokens::CAPTION)
                        } else {
                            RichText::new(value).size(Tokens::SECONDARY)
                        };
                        widgets::truncated(ui, text);
                    }
                });
            });
        }
        if let Some((name, category)) = &identity.automatic {
            // Right to left, like the group's sources: Reset at the edge, the value beside
            // its label.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response = ui.add_enabled(
                    reset_blocked.is_none(),
                    Button::new("Reset", ButtonKind::Ghost).small(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        reset_blocked.is_none(),
                        format!("Reset to {name}, {}", category.label()),
                    )
                });
                let response = match reset_blocked {
                    Some(reason) => response.on_disabled_hover_text(reason),
                    None => response.on_hover_text("Use the automatic name and category"),
                };
                if response.clicked() {
                    picked = Some(DetailAction::Reset);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.allocate_ui_with_layout(
                        vec2(100.0, 18.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.label(widgets::text(
                                "Automatic",
                                Tokens::SECONDARY,
                                colors.fg_subtle,
                            ));
                        },
                    );
                    widgets::truncated(
                        ui,
                        RichText::new(format!("{name} · {}", category.label()))
                            .size(Tokens::SECONDARY),
                    );
                });
            });
        }
        if identity.sources.len() > 1 {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(widgets::text(
                    format!("Includes {} sources", identity.sources.len()),
                    Tokens::CAPTION,
                    colors.fg_subtle,
                ));
                for key in &identity.sources {
                    let source = format::source(key);
                    let display = source.path.unwrap_or(source.line);
                    // Right to left: the button sits at the edge, the key takes the rest.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // The app's own key is the group's representative: no button.
                        if key != &vm.key {
                            let response = ui.add_enabled(
                                ungroup_blocked.is_none(),
                                Button::new("Remove from group", ButtonKind::Ghost).small(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ungroup_blocked.is_none(),
                                    format!("Remove {display} from group"),
                                )
                            });
                            let response = match ungroup_blocked {
                                Some(reason) => response.on_disabled_hover_text(reason),
                                None => response,
                            };
                            if response.clicked() {
                                picked = Some(DetailAction::RemoveFromGroup(key.clone()));
                            }
                        }
                        widgets::truncated(ui, widgets::mono(display.clone(), Tokens::CAPTION));
                    });
                }
            });
        }
        ui.separator();
        ui.label(widgets::text(
            "Your edits always win over automatic matches. Earlier reports use the current name and grouping.",
            Tokens::CAPTION,
            colors.fg_subtle,
        ));
        picked
    })
    .inner
}

/// Splits the app's focus spans at local hour boundaries.
fn focus_by_hour(report: &Report, key: &str, zone: Tz) -> Option<[i64; 24]> {
    use chrono::{TimeZone, Timelike};
    let mut hours = [0i64; 24];
    for entry in &report.timeline {
        if !matches!(&entry.kind, TimelineKind::Focus(owner) if owner == key) {
            continue;
        }
        let mut at = entry.start_ms;
        while at < entry.end_ms {
            let local = zone.timestamp_millis_opt(at).single()?;
            let hour = local.hour() as usize;
            let into_hour = i64::from(local.minute()) * 60_000
                + i64::from(local.second()) * 1_000
                + i64::from(local.timestamp_subsec_millis());
            let next = (at - into_hour + 3_600_000).min(entry.end_ms);
            hours[hour] += next - at;
            at = next;
        }
    }
    hours.iter().any(|ms| *ms > 0).then_some(hours)
}

fn hours_card(ui: &mut Ui, vm: &DetailVm, hours: &[i64; 24], range: &str) {
    let colors = Tokens::get(ui);
    widgets::card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 12.0;
        widgets::section_header(ui, "Time of day", None, |ui| {
            ui.label(widgets::text(
                format!("Focused, {range}"),
                Tokens::CAPTION,
                colors.fg_subtle,
            ));
        });
        let peak = hours
            .iter()
            .enumerate()
            .max_by_key(|(_, ms)| **ms)
            .map_or(0, |(hour, _)| hour);
        let active: Vec<usize> = (0..24).filter(|hour| hours[*hour] > 0).collect();
        let summary = format!(
            "Focused between {:02}:00 and {:02}:00, peaking at {peak:02}:00",
            active.first().copied().unwrap_or(0),
            active.last().map_or(0, |hour| hour + 1)
        );
        widgets::hour_histogram(ui, hours, colors.category(vm.category), &summary);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempora::storage::{AppSummary, Session, TimelineEntry, Totals};

    const MINUTE: i64 = 60_000;

    #[test]
    fn identity_offers_the_automatic_identity_only_when_edits_replaced_it() {
        let key = r"exe:c:\code\code.exe";
        let today = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let vm = |automatic: (&str, Category)| {
            let info = AppInfo {
                all_time: None,
                sources: vec![key.into()],
                automatic: Some((automatic.0.into(), automatic.1)),
            };
            detail_vm(
                key,
                None,
                Some(&info),
                Some(("Visual Studio Code", Category::Productivity)),
                chrono_tz::UTC,
                today,
            )
            .and_then(|vm| vm.identity)
            .and_then(|identity| identity.automatic)
        };
        assert_eq!(
            vm(("Code", Category::Other)),
            Some(("Code".into(), Category::Other))
        );
        assert_eq!(
            vm(("Visual Studio Code", Category::Productivity)),
            None,
            "nothing to reset"
        );
    }

    #[test]
    fn sessions_list_newest_first_with_their_average() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let start = |hour: i64| {
            today
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis()
                + hour * 60 * MINUTE
        };
        let session = |from, to, idle| Session {
            start_ms: from,
            end_ms: to,
            totals: Totals {
                running_ms: to - from,
                focused_ms: to - from,
                idle_ms: idle,
                ..Totals::default()
            },
        };
        let app = AppSummary {
            key: "steam:1".into(),
            name: "Game".into(),
            category: Category::Games,
            totals: Totals {
                running_ms: 3 * 60 * MINUTE,
                ..Totals::default()
            },
            last_seen_ms: start(5),
            days: vec![(today, Totals::default())],
            sessions: vec![
                session(start(1), start(2), 5 * MINUTE),
                session(start(3), start(5), 0),
            ],
        };
        let report = Report {
            apps: vec![app],
            ..Report::default()
        };
        let vm = detail_vm("steam:1", Some(&report), None, None, chrono_tz::UTC, today).unwrap();
        let range = vm.range.unwrap();
        assert_eq!(range.sessions, 2);
        assert_eq!(range.average_ms, 90 * MINUTE);
        assert_eq!(range.session_rows[0].start_ms, start(3), "newest first");
        assert_eq!(range.session_rows[1].start_ms, start(1));
        assert_eq!(vm.source.line, "Steam · App 1");
        assert!(vm.identity.is_none(), "identity waits for the info query");
    }

    #[test]
    fn session_days_read_today_and_yesterday() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(today.pred_opt().unwrap(), today), "Yesterday");
        assert_eq!(
            day_label(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(), today),
            "Fri, Nov 20"
        );
    }

    #[test]
    fn focus_splits_at_local_hours() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 20)
            .unwrap()
            .and_hms_opt(20, 30, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let report = Report {
            timeline: vec![TimelineEntry {
                start_ms: start,
                end_ms: start + 90 * MINUTE,
                kind: TimelineKind::Focus("a".into()),
            }],
            ..Report::default()
        };
        let hours = focus_by_hour(&report, "a", chrono_tz::UTC).unwrap();
        assert_eq!(
            (hours[20], hours[21], hours[22]),
            (30 * MINUTE, 60 * MINUTE, 0)
        );
        assert!(focus_by_hour(&report, "b", chrono_tz::UTC).is_none());
    }
}
