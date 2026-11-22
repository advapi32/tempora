//! Overview (`Main` mockup): date controls, KPIs, focused time by day, top apps.

use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Align, Layout, RichText, Ui, vec2};
use egui_phosphor::regular as icons;
use tempora::{
    model::{Category, ControlRequest, DateRange},
    storage::Report,
};

use crate::dashboard::nav::{Page, Period, View};
use crate::dashboard::theme::{self, Tokens};
use crate::dashboard::widgets::{
    self, Button, ButtonKind, Column, Kpi, RowStyle, Segment, StackedBar, Width,
};
use crate::dashboard::{Dashboard, format};

const TOP_APPS: usize = 10;

/// Everything the Overview draws, filled from one range report.
pub(in crate::dashboard) struct OverviewVm {
    pub empty: bool,
    pub focused_ms: i64,
    /// Change against the preceding equivalent period. Needs the previous-period query.
    pub focused_delta: Option<String>,
    /// "vs. same point last week", "vs. previous month" and so on.
    pub delta_caption: Option<String>,
    pub average_ms: i64,
    pub active_days: usize,
    pub in_progress: bool,
    pub gaming_ms: i64,
    pub gaming_share: Option<u32>,
    pub idle_ms: i64,
    pub idle_excluded: bool,
    pub idle_threshold: Option<u32>,
    pub chips: Vec<(Category, i64)>,
    pub days: Vec<DayVm>,
    pub apps: Vec<AppRowVm>,
    pub app_count: usize,
}

pub(in crate::dashboard) struct DayVm {
    pub date: NaiveDate,
    pub categories: Vec<(Category, i64)>,
    pub total_ms: i64,
    pub today: bool,
    pub future: bool,
}

pub(in crate::dashboard) struct AppRowVm {
    pub key: String,
    pub name: String,
    /// The full source, for the name's tooltip.
    pub source: String,
    /// Tells this row apart from others shown with the same name. `None` when the name is unique.
    pub subtitle: Option<String>,
    pub category: Category,
    pub focused_ms: i64,
    pub running_ms: i64,
    pub share: Option<u32>,
    pub bar: f32,
}

pub(in crate::dashboard) fn overview_vm(
    report: &Report,
    view: &View,
    today: NaiveDate,
    idle_threshold: Option<u32>,
    previous: Option<(i64, bool)>,
) -> OverviewVm {
    let focused_ms: i64 = report
        .categories
        .iter()
        .map(|row| row.totals.focused_ms)
        .sum();
    let idle_ms: i64 = report.categories.iter().map(|row| row.totals.idle_ms).sum();
    let gaming_ms = report
        .categories
        .iter()
        .find(|row| row.category == Category::Games)
        .map_or(0, |row| row.totals.focused_ms);
    let active_days = report
        .days
        .iter()
        .filter(|day| day.date <= today && day.covered_ms > 0)
        .count();
    let chips = Category::ALL
        .into_iter()
        .filter_map(|category| {
            report
                .categories
                .iter()
                .find(|row| row.category == category && row.totals.running_ms > 0)
                .map(|row| (category, row.totals.focused_ms))
        })
        .collect();
    let days = report
        .days
        .iter()
        .map(|day| {
            let categories: Vec<_> = Category::ALL
                .into_iter()
                .filter_map(|category| {
                    day.categories
                        .iter()
                        .find(|row| row.category == category)
                        .map(|row| (category, row.totals.focused_ms))
                })
                .collect();
            DayVm {
                date: day.date,
                total_ms: categories
                    .iter()
                    .filter(|(category, _)| view.category.is_none_or(|only| only == *category))
                    .map(|(_, ms)| ms)
                    .sum(),
                categories,
                today: day.date == today,
                future: day.date > today,
            }
        })
        .collect();
    let mut ranked: Vec<_> = report
        .apps
        .iter()
        .filter(|app| app.totals.focused_ms > 0)
        .filter(|app| {
            view.category
                .is_none_or(|category| app.category == category)
        })
        .collect();
    ranked.sort_by_key(|app| std::cmp::Reverse(app.totals.focused_ms));
    let top = ranked.first().map_or(1, |app| app.totals.focused_ms.max(1));
    let shown: Vec<&tempora::storage::AppSummary> = ranked.iter().take(TOP_APPS).copied().collect();
    let names: Vec<(&str, &str)> = shown
        .iter()
        .map(|app| (app.key.as_str(), app.name.as_str()))
        .collect();
    let lines = format::disambiguation(&names);
    let apps = shown
        .iter()
        .map(|app| AppRowVm {
            key: app.key.clone(),
            name: app.name.clone(),
            source: format::source(&app.key).full(),
            subtitle: lines.get(&app.key).cloned(),
            category: app.category,
            focused_ms: app.totals.focused_ms,
            running_ms: app.totals.running_ms,
            share: format::percent(app.totals.focused_ms, focused_ms),
            bar: app.totals.focused_ms as f32 / top as f32,
        })
        .collect();
    OverviewVm {
        empty: report.coverage.is_none(),
        focused_ms,
        focused_delta: previous
            .filter(|(previous, _)| *previous > 0)
            .map(|(previous, _)| {
                let change =
                    ((focused_ms - previous) as f64 / previous as f64 * 100.0).round() as i64;
                if change > 0 {
                    format!("+{change}%")
                } else {
                    format!("{change}%")
                }
            }),
        delta_caption: previous
            .filter(|(previous, _)| *previous > 0)
            .map(|(_, partial)| {
                let unit = match view.period {
                    Period::Week => "week",
                    Period::Month => "month",
                    Period::Custom => "period",
                };
                if partial {
                    format!("vs. same point last {unit}")
                } else {
                    format!("vs. previous {unit}")
                }
            }),
        average_ms: if active_days == 0 {
            0
        } else {
            focused_ms / active_days as i64
        },
        active_days,
        in_progress: view.range.start <= today && today <= view.range.end,
        gaming_ms,
        gaming_share: format::percent(gaming_ms, focused_ms),
        idle_ms,
        idle_excluded: view.exclude_idle,
        idle_threshold,
        chips,
        days,
        apps,
        app_count: ranked.len(),
    }
}

pub(in crate::dashboard) fn overview_subtitle(view: &View, today: NaiveDate) -> String {
    let in_progress = view.range.start <= today && today <= view.range.end;
    let base = match view.period {
        Period::Week => format!("Week {}", view.range.start.iso_week().week()),
        Period::Month => view.range.start.format("%B %Y").to_string(),
        Period::Custom => {
            let days = (view.range.end - view.range.start).num_days() + 1;
            if days == 1 {
                view.range.start.format("%A").to_string()
            } else {
                format!("{days} days")
            }
        }
    };
    if in_progress {
        format!("{base} · today in progress")
    } else {
        base
    }
}

impl Dashboard {
    pub(super) fn overview(&mut self, ui: &mut Ui, wide: bool) {
        let colors = Tokens::get(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            self.date_controls(ui, true);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut exclude = self.nav.view.exclude_idle;
                if widgets::switch(ui, "Exclude estimated idle", &mut exclude).changed() {
                    self.nav.view.exclude_idle = exclude;
                }
            });
        });
        self.report_status(ui);
        let Some(report) = self.current_report() else {
            return;
        };
        let threshold = self
            .recorder
            .as_ref()
            .map(|status| status.settings.idle_minutes);
        let previous = self.extras().and_then(|extras| extras.previous);
        let vm = overview_vm(&report, &self.nav.view, self.today(), threshold, previous);
        if vm.empty {
            self.overview_empty(ui);
            return;
        }
        widgets::kpi_grid(ui, &kpis(&vm, colors), if wide { 4 } else { 2 });
        self.focus_by_day(ui, &vm);
        self.top_apps(ui, &vm);
    }

    fn overview_empty(&mut self, ui: &mut Ui) {
        let paused = self
            .recorder
            .as_ref()
            .is_some_and(|status| status.settings.paused);
        let message = if paused {
            "Recording is paused. Resume to start collecting new activity."
        } else if self.recorder.is_none() {
            "Start the recorder to collect activity."
        } else {
            "Try another date range, or open an app to begin a session."
        };
        // One click to the data, however far the range was stepped away from it.
        let latest = self.latest_activity_range();
        let mut actions = Vec::new();
        if latest.is_some() {
            actions.push(Button::new("Go to latest activity", ButtonKind::Secondary));
        }
        let resume = paused && self.recorder_online();
        if resume {
            actions.push(Button::new("Resume recording", ButtonKind::Primary));
        }
        match widgets::empty_state(ui, "No activity in this range", message, &actions) {
            Some(0) if latest.is_some() => {
                if let Some((period, range)) = latest {
                    self.nav.set_range(period, range);
                }
            }
            Some(_) if resume => {
                self.action(ControlRequest::SetPaused(false), "Recording resumed");
            }
            _ => {}
        }
    }

    /// The period of the current kind that contains the latest recorded activity, unless it's
    /// the one already shown. A custom range jumps to that activity's week.
    pub(super) fn latest_activity_range(&self) -> Option<(Period, DateRange)> {
        let (_, last) = self.extras()?.bounds?;
        let date = format::local_date(last, self.zone)?;
        let period = match self.nav.view.period {
            Period::Custom => Period::Week,
            period => period,
        };
        let range = crate::dashboard::nav::period_range(period, date, self.prefs.week_start);
        (range != self.nav.view.range).then_some((period, range))
    }

    fn focus_by_day(&mut self, ui: &mut Ui, vm: &OverviewVm) {
        // The chart takes about a quarter of the space below the KPIs so Top apps keeps rows.
        let below = self.viewport_bottom - ui.cursor().top();
        let plot = if below.is_finite() {
            (below * 0.25).clamp(120.0, 260.0)
        } else {
            260.0
        };
        widgets::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = Tokens::GRID_GAP;
            let mut table = self.nav.view.overview_table;
            widgets::section_header(
                ui,
                "Focused time by day",
                Some("Select a day to open its timeline. Select a category to filter."),
                |ui| {
                    widgets::segmented(ui, &mut table, &[(false, "Chart"), (true, "Table")]);
                },
            );
            self.nav.view.overview_table = table;
            self.category_chips(ui, vm);
            let opened = if table {
                day_table(ui, vm, self.nav.view.range, self.nav.view.period)
            } else {
                day_chart(ui, vm, self.nav.view.category, plot)
            };
            if let Some(date) = opened {
                self.nav.drill(Page::Day(date));
            }
        });
    }

    fn category_chips(&mut self, ui: &mut Ui, vm: &OverviewVm) {
        let colors = Tokens::get(ui);
        let selected = self.nav.view.category;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(Tokens::CONTROL_GAP, Tokens::CONTROL_GAP);
            for (category, focused) in &vm.chips {
                let on = selected == Some(*category);
                let chip = widgets::category_chip(
                    ui,
                    category.label(),
                    colors.category(*category),
                    Some(&format::duration(*focused)),
                    on,
                    selected.is_some(),
                );
                if chip.clicked() {
                    self.nav.view.category = if on { None } else { Some(*category) };
                }
            }
        });
    }

    fn top_apps(&mut self, ui: &mut Ui, vm: &OverviewVm) {
        let colors = Tokens::get(ui);
        widgets::table_card(ui, true, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                theme::section(ui, "Top apps");
                ui.label(
                    widgets::mono(
                        match vm.app_count {
                            1 => "1 app".to_owned(),
                            count => format!("{count} apps"),
                        },
                        Tokens::CAPTION,
                    )
                    .color(colors.fg_subtle),
                );
                if let Some(category) = self.nav.view.category
                    && widgets::filter_chip(ui, category.label()).clicked()
                {
                    self.nav.view.category = None;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = Button::new("Open Library", ButtonKind::Ghost)
                        .small()
                        .trailing(icons::ARROW_RIGHT);
                    if ui.add(open).clicked() {
                        self.nav.section(Page::Library);
                    }
                });
            });
            ui.add_space(12.0);
            if vm.apps.is_empty() {
                ui.label(widgets::text(
                    "No focused apps in this view.",
                    Tokens::SECONDARY,
                    colors.fg_muted,
                ));
                return;
            }
            let specs = [
                Column::new("App", Width::Flex(2.2, 200.0), Align::Min),
                Column::new("Category", Width::Fixed(130.0), Align::Min),
                Column::new("Focused", Width::Flex(2.0, 160.0), Align::Min),
                Column::new("Running", Width::Fixed(90.0), Align::Max),
                Column::new("Share", Width::Fixed(64.0), Align::Max),
            ];
            let columns = widgets::scrolled_columns(ui, width, &specs);
            widgets::table_header(ui, &columns, &specs.map(|column| column.label));
            let mut opened = None;
            let mut menu = None;
            let mut edit = None;
            let online = self.recorder_online();
            super::fitted_list(
                self.viewport_bottom,
                ui,
                "top-apps",
                widgets::TITLED_TABLE_BOTTOM,
                |ui| {
                    for app in &vm.apps {
                        let label = format!(
                            "{}, {}, {} focused, {} running{}. Open details. Right-click for options.",
                            app.name,
                            app.category.label(),
                            format::duration(app.focused_ms),
                            format::duration(app.running_ms),
                            app.share
                                .map(|share| format!(", {share}% of focused time"))
                                .unwrap_or_default(),
                        );
                        let row = widgets::table_row(
                            ui,
                            ui.id().with(("top-app", &app.key)),
                            &columns,
                            RowStyle::default(),
                            &label,
                            |row| {
                                row.cell(0, |ui| {
                                    widgets::app_cell_with(
                                        ui,
                                        Some(&app.key),
                                        &app.name,
                                        app.subtitle.as_deref(),
                                        app.category,
                                        30.0,
                                        |_| {},
                                    )
                                    .on_hover_text(app.source.as_str());
                                });
                                row.cell(1, |ui| widgets::category_label(ui, app.category));
                                let bar_width = row.width(2);
                                row.cell(2, |ui| {
                                    let value = format::duration(app.focused_ms);
                                    widgets::share_bar(
                                        ui,
                                        app.bar,
                                        colors.category(app.category),
                                        (bar_width - 64.0 - Tokens::CONTROL_GAP).max(0.0),
                                    );
                                    widgets::right_aligned_cell(
                                        ui,
                                        64.0,
                                        20.0,
                                        RichText::new(value)
                                            .font(Tokens::mono_medium(Tokens::SECONDARY)),
                                    );
                                });
                                row.number(3, &format::duration(app.running_ms), colors.fg_muted);
                                row.number(
                                    4,
                                    &app.share
                                        .map_or_else(String::new, |share| format!("{share}%")),
                                    colors.fg_subtle,
                                );
                            },
                        );
                        if row.clicked() {
                            opened = Some(app.key.clone());
                        } else if row.secondary_clicked()
                            && let Some(pos) = row.interact_pointer_pos()
                        {
                            menu = Some((app.key.clone(), super::pointer_anchor(pos)));
                        } else if online
                            && row.has_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::F2))
                        {
                            edit = Some(app.key.clone());
                        }
                    }
                },
            );
            if let Some(key) = opened {
                self.nav.drill(Page::App(key));
            }
            if let Some((key, anchor)) = menu {
                self.toggle_app_menu(key, anchor);
            }
            if let Some(key) = edit {
                self.open_edit(&key);
            }
        });
    }
}

fn kpis<'a>(vm: &OverviewVm, colors: &Tokens) -> [Kpi<'a>; 4] {
    let in_progress = if vm.in_progress {
        " · today in progress"
    } else {
        ""
    };
    [
        Kpi {
            delta: vm.focused_delta.clone(),
            caption: vm.delta_caption.clone(),
            ..Kpi::new(
                "Focused time",
                icons::CLOCK,
                format::duration(vm.focused_ms),
            )
        },
        Kpi {
            caption: Some(match vm.active_days {
                0 => "No days with activity".into(),
                1 => format!("1 day with activity{in_progress}"),
                days => format!("{days} days with activity{in_progress}"),
            }),
            ..Kpi::new(
                "Daily average",
                icons::TREND_UP,
                format::duration(vm.average_ms),
            )
        },
        Kpi {
            share: vm
                .gaming_share
                .map(|share| (share as f32 / 100.0, colors.category(Category::Games))),
            caption: vm
                .gaming_share
                .map(|share| format!("{share}% of focused time")),
            ..Kpi::new(
                "Gaming",
                icons::GAME_CONTROLLER,
                format::duration(vm.gaming_ms),
            )
        },
        Kpi {
            caption: Some(if vm.idle_excluded {
                "Excluded from the totals on this page".into()
            } else {
                match vm.idle_threshold {
                    Some(minutes) => format!("Included in focused time · {minutes} min threshold"),
                    None => "Included in focused time".into(),
                }
            }),
            ..Kpi::new("Estimated idle", icons::MOON, format::duration(vm.idle_ms))
        },
    ]
}

fn day_chart(ui: &mut Ui, vm: &OverviewVm, only: Option<Category>, plot: f32) -> Option<NaiveDate> {
    let colors = Tokens::get(ui);
    let week = vm.days.len() <= 7;
    let bars: Vec<_> = vm
        .days
        .iter()
        .map(|day| {
            let date = day.date.format("%a %b %-d");
            StackedBar {
                label: if week {
                    day.date.format("%a %-d").to_string()
                } else {
                    day.date.day().to_string()
                },
                description: if day.future {
                    format!("{date}: no data yet")
                } else {
                    format!(
                        "{date}{}: {} focused. Open timeline.",
                        if day.today { " (today so far)" } else { "" },
                        format::duration(day.total_ms)
                    )
                },
                total: (!day.future && day.total_ms > 0).then(|| format::duration(day.total_ms)),
                segments: day
                    .categories
                    .iter()
                    .map(|(category, ms)| Segment {
                        color: colors.category(*category),
                        ms: *ms,
                        dimmed: only.is_some_and(|only| only != *category),
                        name: category.label(),
                    })
                    .collect(),
                today: day.today,
                future: day.future,
                slice_detail: Some(if day.today {
                    format!("{} · today so far", day.date.format("%A, %b %-d"))
                } else {
                    day.date.format("%A, %b %-d").to_string()
                }),
            }
        })
        .collect();
    widgets::stacked_bars(ui, &bars, week, plot).map(|index| vm.days[index].date)
}

/// The chart's text equivalent: focused time per day and category.
fn day_table(
    ui: &mut Ui,
    vm: &OverviewVm,
    range: tempora::model::DateRange,
    period: Period,
) -> Option<NaiveDate> {
    let colors = Tokens::get(ui);
    let categories: Vec<Category> = vm.chips.iter().map(|(category, _)| *category).collect();
    let width = ui.available_width();
    let mut specs = vec![Column::new("Day", Width::Flex(1.0, 120.0), Align::Min)];
    specs.extend(
        categories
            .iter()
            .map(|category| Column::new(category.label(), Width::Fixed(96.0), Align::Max)),
    );
    specs.push(Column::new("Total", Width::Fixed(80.0), Align::Max));
    let columns = widgets::scrolled_columns(ui, width, &specs);
    ui.label(widgets::text(
        format!(
            "Focused time per day and category, {}",
            format::range_label(range, period)
        ),
        Tokens::CAPTION,
        colors.fg_subtle,
    ));
    let labels: Vec<&str> = specs.iter().map(|column| column.label).collect();
    let mut opened = None;
    let area = egui::ScrollArea::vertical()
        .max_height(340.0)
        .id_salt("day-table");
    widgets::scroll(ui, area, |ui| {
        // Rows abut; the card's 16 px item gap would sit under each row.
        ui.spacing_mut().item_spacing.y = 0.0;
        widgets::table_header(ui, &columns, &labels);
        for (index, day) in vm.days.iter().enumerate() {
            let value = |category: Category| {
                day.categories
                    .iter()
                    .find(|(row, _)| *row == category)
                    .map_or(0, |(_, ms)| *ms)
            };
            let day_label = day.date.format("%a, %b %-d").to_string();
            let label = if day.future {
                format!("{day_label}: no data yet")
            } else {
                let parts: Vec<_> = categories
                    .iter()
                    .map(|category| {
                        format!(
                            "{} {}",
                            category.label(),
                            format::duration(value(*category))
                        )
                    })
                    .collect();
                format!(
                    "{day_label}: {}; total {}. Open timeline.",
                    parts.join(", "),
                    format::duration(day.total_ms)
                )
            };
            let row = widgets::table_row(
                ui,
                ui.id().with(("day-row", day.date)),
                &columns,
                RowStyle {
                    height: 36.0,
                    clickable: !day.future,
                    separator: index + 1 < vm.days.len(),
                    ..RowStyle::default()
                },
                &label,
                |row| {
                    row.cell(0, |ui| {
                        let text = RichText::new(&day_label).size(Tokens::SECONDARY);
                        widgets::truncated(
                            ui,
                            if day.today {
                                text.font(Tokens::medium(Tokens::SECONDARY))
                            } else {
                                text
                            },
                        );
                    });
                    for (index, category) in categories.iter().enumerate() {
                        let text = if day.future {
                            "—".to_owned()
                        } else {
                            format::duration(value(*category))
                        };
                        row.number(index + 1, &text, colors.fg_muted);
                    }
                    let total = if day.future {
                        "—".to_owned()
                    } else {
                        format::duration(day.total_ms)
                    };
                    row.number(categories.len() + 1, &total, colors.fg);
                },
            );
            if row.clicked() {
                opened = Some(day.date);
            }
        }
    });
    opened
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::nav::Nav;
    use chrono::Weekday;
    use tempora::storage::{AppSummary, CategorySummary, DaySummary, Totals};

    const HOUR: i64 = 3_600_000;

    fn totals(focused: i64, running: i64, idle: i64) -> Totals {
        Totals {
            focused_ms: focused,
            running_ms: running,
            idle_ms: idle,
            ..Totals::default()
        }
    }

    fn app(key: &str, category: Category, focused: i64) -> AppSummary {
        AppSummary {
            key: key.into(),
            name: key.into(),
            category,
            totals: totals(focused, focused * 2, 0),
            last_seen_ms: 0,
            days: Vec::new(),
            sessions: Vec::new(),
        }
    }

    #[test]
    fn adapter_binds_report_values_and_filters_by_category() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let mut nav = Nav::new(today, Weekday::Mon);
        let day = |offset: u64, games: i64| DaySummary {
            date: nav.view.range.start + chrono::Days::new(offset),
            categories: vec![
                CategorySummary {
                    category: Category::Games,
                    totals: totals(games, games, 0),
                },
                CategorySummary {
                    category: Category::Browsers,
                    totals: totals(HOUR, HOUR, 0),
                },
            ],
            covered_ms: if games > 0 { 1 } else { 0 },
        };
        let report = Report {
            apps: vec![
                app("steam:1", Category::Games, 3 * HOUR),
                app(r"exe:c:\firefox.exe", Category::Browsers, HOUR),
            ],
            categories: vec![
                CategorySummary {
                    category: Category::Games,
                    totals: totals(3 * HOUR, 3 * HOUR, HOUR / 2),
                },
                CategorySummary {
                    category: Category::Browsers,
                    totals: totals(HOUR, HOUR, 0),
                },
            ],
            days: (0..7)
                .map(|offset| day(offset, if offset < 2 { HOUR } else { 0 }))
                .collect(),
            timeline: Vec::new(),
            coverage: Some((0, 1)),
        };
        let vm = overview_vm(&report, &nav.view, today, Some(5), None);
        assert_eq!(vm.focused_ms, 4 * HOUR);
        assert_eq!(vm.active_days, 2);
        assert_eq!(vm.average_ms, 2 * HOUR);
        assert_eq!(vm.gaming_share, Some(75));
        assert_eq!(vm.idle_ms, HOUR / 2);
        assert_eq!(
            vm.focused_delta, None,
            "no comparison without a previous period"
        );
        let compared = overview_vm(&report, &nav.view, today, Some(5), Some((2 * HOUR, true)));
        assert_eq!(compared.focused_delta.as_deref(), Some("+100%"));
        assert_eq!(
            compared.delta_caption.as_deref(),
            Some("vs. same point last week")
        );
        assert_eq!(
            overview_vm(&report, &nav.view, today, None, Some((0, false))).focused_delta,
            None,
            "zero baseline"
        );
        assert!(vm.in_progress);
        assert_eq!(vm.days.iter().filter(|day| day.future).count(), 4);
        assert_eq!(vm.apps[0].source, "Steam · App 1");
        assert_eq!(
            vm.apps[0].subtitle, None,
            "a unique name has no second line"
        );
        assert_eq!(vm.apps[1].source, r"c:\firefox.exe");
        assert_eq!(vm.apps[1].subtitle, None);
        assert_eq!(vm.apps[1].share, Some(25));

        nav.view.category = Some(Category::Browsers);
        let vm = overview_vm(&report, &nav.view, today, None, None);
        assert_eq!(vm.app_count, 1);
        assert_eq!(
            vm.days[0].total_ms, HOUR,
            "day totals follow the category filter"
        );
    }
}
