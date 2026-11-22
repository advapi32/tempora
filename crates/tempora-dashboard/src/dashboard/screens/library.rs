//! Library: searchable catalog with filters, sortable table, browser group and row actions.

use std::cmp::Reverse;

use chrono::{Days, NaiveDate};
use chrono_tz::Tz;
use eframe::egui::{self, Align, Key, Layout, Rect, RichText, Ui};
use egui_phosphor::regular as icons;
use tempora::{
    model::{AppIdentity, Category},
    storage::Report,
};

use super::Popup;
use crate::dashboard::nav::{LibraryFilter, LibrarySort, Page, View};
use crate::dashboard::theme::Tokens;
use crate::dashboard::widgets::{self, Button, ButtonKind, Column, RowStyle, Width};
use crate::dashboard::{Dashboard, Extras, format};
use tempora::storage::LibraryFlags;

pub(in crate::dashboard) struct LibraryRow {
    pub key: String,
    pub name: String,
    /// The full source, for the name's tooltip.
    pub source: String,
    /// Tells this row apart from others with the same name. `None` when the name is unique.
    pub subtitle: Option<String>,
    pub category: Category,
    /// `None` for catalog entries without activity in the range.
    pub focused_ms: Option<i64>,
    pub running_ms: Option<i64>,
    pub last_seen_ms: Option<i64>,
    pub live: bool,
    pub flags: LibraryFlags,
    /// Seven days of focused time ending at the range's last elapsed day.
    pub week: Option<[i64; 7]>,
}

pub(in crate::dashboard) struct BrowserGroup {
    pub focused_ms: i64,
    /// Union of every browser's running time, not a sum.
    pub running_ms: i64,
    pub members: Vec<LibraryRow>,
}

pub(in crate::dashboard) enum LibraryEntry {
    App(LibraryRow),
    Browsers(BrowserGroup),
}

pub(in crate::dashboard) struct LibraryVm {
    pub entries: Vec<LibraryEntry>,
    pub range_has_apps: bool,
}

fn sort_key(row: &LibraryRow, sort: LibrarySort) -> (Reverse<i64>, String) {
    let value = match sort {
        LibrarySort::Focused => row.focused_ms.unwrap_or(-1),
        LibrarySort::Running => row.running_ms.unwrap_or(-1),
        LibrarySort::LastSeen => row.last_seen_ms.unwrap_or(i64::MIN),
        LibrarySort::Name => 0,
    };
    (Reverse(value), row.name.to_lowercase())
}

pub(in crate::dashboard) fn library_vm(
    report: &Report,
    catalog: &[AppIdentity],
    view: &View,
    live: &dyn Fn(&str) -> bool,
    extras: Option<&Extras>,
) -> LibraryVm {
    let query = view.search.trim().to_lowercase();
    let flags = |key: &str| {
        extras
            .and_then(|extras| extras.flags.get(key).copied())
            .unwrap_or_default()
    };
    let passes = |category: Category, name: &str, key: &str| {
        let tab = match view.library {
            LibraryFilter::All => true,
            LibraryFilter::Games => category == Category::Games,
            LibraryFilter::Apps => category != Category::Games,
        };
        tab && view.category.is_none_or(|only| only == category)
            && (query.is_empty()
                || name.to_lowercase().contains(&query)
                || key.to_lowercase().contains(&query))
    };
    let mut rows: Vec<LibraryRow> = report
        .apps
        .iter()
        .filter(|app| passes(app.category, &app.name, &app.key))
        .map(|app| LibraryRow {
            key: app.key.clone(),
            name: app.name.clone(),
            source: format::source(&app.key).full(),
            subtitle: None,
            category: app.category,
            focused_ms: Some(app.totals.focused_ms),
            running_ms: Some(app.totals.running_ms),
            last_seen_ms: Some(app.last_seen_ms),
            live: live(&app.key),
            flags: flags(&app.key),
            week: extras.and_then(|extras| extras.week.get(&app.key).copied()),
        })
        .collect();
    // Inactive catalog entries stay findable by search.
    if !query.is_empty() {
        rows.extend(
            catalog
                .iter()
                .filter(|app| !report.apps.iter().any(|active| active.key == app.key))
                .filter(|app| passes(app.category, &app.name, &app.key))
                .map(|app| LibraryRow {
                    key: app.key.clone(),
                    name: app.name.clone(),
                    source: format::source(&app.key).full(),
                    subtitle: None,
                    category: app.category,
                    focused_ms: None,
                    running_ms: None,
                    last_seen_ms: extras.and_then(|extras| extras.last_seen.get(&app.key).copied()),
                    live: live(&app.key),
                    flags: flags(&app.key),
                    week: None,
                }),
        );
    }
    let names: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.key.as_str(), row.name.as_str()))
        .collect();
    let lines = format::disambiguation(&names);
    for row in &mut rows {
        row.subtitle = lines.get(&row.key).cloned();
    }
    rows.sort_by_key(|row| sort_key(row, view.sort));
    let browsers = rows
        .iter()
        .filter(|row| row.category == Category::Browsers && row.focused_ms.is_some())
        .count();
    let mut entries = Vec::new();
    if browsers >= 2 {
        let (members, others): (Vec<_>, Vec<_>) = rows
            .into_iter()
            .partition(|row| row.category == Category::Browsers && row.focused_ms.is_some());
        let totals = report
            .categories
            .iter()
            .find(|row| row.category == Category::Browsers)
            .map(|row| row.totals)
            .unwrap_or_default();
        let group = BrowserGroup {
            focused_ms: totals.focused_ms,
            running_ms: totals.running_ms,
            members,
        };
        let group_key = match view.sort {
            LibrarySort::Focused => group.focused_ms,
            LibrarySort::Running => group.running_ms,
            LibrarySort::LastSeen => group
                .members
                .iter()
                .filter_map(|row| row.last_seen_ms)
                .max()
                .unwrap_or(i64::MIN),
            LibrarySort::Name => 0,
        };
        let position = others
            .iter()
            .position(|row| sort_key(row, view.sort) > (Reverse(group_key), "browsers".into()))
            .unwrap_or(others.len());
        let mut others = others
            .into_iter()
            .map(LibraryEntry::App)
            .collect::<Vec<_>>();
        others.insert(position, LibraryEntry::Browsers(group));
        entries = others;
    } else {
        entries.extend(rows.into_iter().map(LibraryEntry::App));
    }
    LibraryVm {
        entries,
        range_has_apps: !report.apps.is_empty(),
    }
}

/// "Today 14:22", "Yesterday 09:10" or "Nov 12".
pub(in crate::dashboard) fn last_seen(ms: i64, zone: Tz, today: NaiveDate) -> String {
    match format::local_date(ms, zone) {
        Some(date) if date == today => format!("Today {}", format::local(ms, zone, "%H:%M")),
        Some(date) if Some(date) == today.checked_sub_days(Days::new(1)) => {
            format!("Yesterday {}", format::local(ms, zone, "%H:%M"))
        }
        Some(date) => date.format("%b %-d").to_string(),
        None => String::new(),
    }
}

const ROW_HEIGHT: f32 = 50.0;
/// The card's bottom padding, the caption under the table and the gap above it.
const LIBRARY_CAPTION: f32 = 16.0 + Tokens::GRID_GAP + widgets::PLAIN_TABLE_BOTTOM;

enum RowAction {
    Open(String),
    Menu(String, Rect),
    Edit(String),
    Toggle,
}

impl Dashboard {
    pub(super) fn library(&mut self, ui: &mut Ui) {
        self.library_toolbar(ui);
        self.report_status(ui);
        let Some(report) = self.current_report() else {
            return;
        };
        let candidates: Vec<String> = self
            .recorder
            .iter()
            .flat_map(|status| status.candidates.iter().map(|app| app.key.clone()))
            .collect();
        let live = |key: &str| candidates.iter().any(|candidate| candidate == key);
        let extras = self.extras();
        let vm = library_vm(
            &report,
            &self.apps,
            &self.nav.view,
            &live,
            extras.as_deref(),
        );
        if !vm.range_has_apps && self.nav.view.search.trim().is_empty() {
            // One click to the data, however far the range was stepped away from it.
            let latest = self.latest_activity_range();
            let actions: Vec<Button> = latest
                .iter()
                .map(|_| Button::new("Go to latest activity", ButtonKind::Secondary))
                .collect();
            if widgets::empty_state(
                ui,
                "No apps recorded in this range",
                "Pick another range, or search to find apps recorded at other times.",
                &actions,
            )
            .is_some()
                && let Some((period, range)) = latest
            {
                self.nav.set_range(period, range);
            }
            return;
        }
        let colors = Tokens::get(ui);
        let action = widgets::table_card(ui, false, |ui| self.library_table(ui, &vm)).inner;
        ui.label(widgets::text(
            "Running time unions every process of an app, so it can exceed focused time. Launchers and updaters never extend a game's session.",
            Tokens::CAPTION,
            colors.fg_subtle,
        ));
        match action {
            Some(RowAction::Open(key)) => self.nav.drill(Page::App(key)),
            Some(RowAction::Menu(key, anchor)) => self.toggle_app_menu(key, anchor),
            Some(RowAction::Edit(key)) => {
                if self.recorder_online() {
                    self.open_edit(&key);
                }
            }
            Some(RowAction::Toggle) => {
                self.nav.view.browsers_expanded = !self.nav.view.browsers_expanded;
            }
            None => {}
        }
    }

    fn library_toolbar(&mut self, ui: &mut Ui) {
        let colors = Tokens::get(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let width = (ui.available_width() * 0.32).clamp(260.0, 360.0);
            widgets::search_field(
                ui,
                egui::Id::new("library-search"),
                &mut self.nav.view.search,
                "Search by name, executable or path",
                width,
            );
            let mut filter = self.nav.view.library;
            let options = [
                (LibraryFilter::All, "All"),
                (LibraryFilter::Games, "Games"),
                (LibraryFilter::Apps, "Apps"),
            ];
            if widgets::segmented(ui, &mut filter, &options) {
                self.nav.view.library = filter;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = Tokens::CONTROL_GAP;
                let label = self
                    .nav
                    .view
                    .category
                    .map_or("All categories", Category::label);
                let button = ui.add(
                    Button::new(label, ButtonKind::Secondary)
                        .with_icon(icons::FUNNEL_SIMPLE)
                        .trailing(icons::CARET_DOWN),
                );
                if button.clicked() {
                    self.popup = match self.popup {
                        Some(Popup::Categories) => None,
                        _ => Some(Popup::Categories),
                    };
                }
                if matches!(self.popup, Some(Popup::Categories)) {
                    let selected = self.nav.view.category;
                    let output = widgets::anchored_menu(
                        ui.ctx(),
                        egui::Id::new("library-categories"),
                        button.rect,
                        Align::Max,
                        |ui| {
                            let mut picked = None;
                            if widgets::list_item(ui, "All categories", None, selected.is_none())
                                .clicked()
                            {
                                picked = Some(None);
                            }
                            for category in Category::ALL {
                                let item = widgets::swatch_item(
                                    ui,
                                    category.label(),
                                    colors.category(category),
                                    selected == Some(category),
                                );
                                if item.clicked() {
                                    picked = Some(Some(category));
                                }
                            }
                            picked
                        },
                    );
                    if let Some(category) = output.inner {
                        self.nav.view.category = category;
                        self.popup = None;
                    } else if output.dismissed {
                        self.popup = None;
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.range_control(ui);
                    });
                });
            });
        });
    }

    fn library_table(&mut self, ui: &mut Ui, vm: &LibraryVm) -> Option<RowAction> {
        let colors = Tokens::get(ui);
        let width = ui.available_width();
        ui.set_min_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        let specs = [
            Column::new("App", Width::Flex(2.6, 260.0), Align::Min),
            Column::new("Category", Width::Fixed(140.0), Align::Min),
            Column::new("Focused", Width::Fixed(96.0), Align::Max),
            Column::new("Running", Width::Fixed(96.0), Align::Max),
            Column::new("Last seen", Width::Fixed(132.0), Align::Min),
            Column::new("This week", Width::Fixed(64.0), Align::Min),
            Column::new("Actions", Width::Fixed(36.0), Align::Max),
        ];
        let columns = widgets::scrolled_columns(ui, width, &specs);
        let sorts = [
            Some(LibrarySort::Name),
            None,
            Some(LibrarySort::Focused),
            Some(LibrarySort::Running),
            Some(LibrarySort::LastSeen),
            None,
            None,
        ];
        let active = sorts
            .iter()
            .position(|sort| *sort == Some(self.nav.view.sort))
            .unwrap_or(2);
        let clicked = widgets::sortable_header(
            ui,
            &columns,
            &[
                "App",
                "Category",
                "Focused",
                "Running",
                "Last seen",
                "This week",
                "",
            ],
            &sorts.map(|sort| sort.is_some()),
            active,
            self.nav.view.sort != LibrarySort::Name,
        );
        if let Some(sort) = clicked.and_then(|index| sorts[index]) {
            self.nav.view.sort = sort;
        }
        if vm.entries.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(widgets::text(
                    format!("No apps match “{}”.", self.nav.view.search.trim()),
                    Tokens::BODY,
                    colors.fg_subtle,
                ));
            });
            ui.add_space(40.0);
            return None;
        }
        let today = self.today();
        let mut action = None;
        super::fitted_list(
            self.viewport_bottom,
            ui,
            "library-rows",
            LIBRARY_CAPTION,
            |ui| {
                for entry in &vm.entries {
                    match entry {
                        LibraryEntry::App(row) => {
                            if let Some(next) = self.library_row(ui, &columns, row, 0.0, today) {
                                action = Some(next);
                            }
                        }
                        LibraryEntry::Browsers(group) => {
                            if let Some(next) = self.browser_group_row(ui, &columns, group) {
                                action = Some(next);
                            }
                            if self.nav.view.browsers_expanded {
                                for row in &group.members {
                                    if let Some(next) =
                                        self.library_row(ui, &columns, row, 28.0, today)
                                    {
                                        action = Some(next);
                                    }
                                }
                            }
                        }
                    }
                }
            },
        );
        action
    }

    fn browser_group_row(
        &self,
        ui: &mut Ui,
        columns: &widgets::Columns,
        group: &BrowserGroup,
    ) -> Option<RowAction> {
        let colors = Tokens::get(ui);
        let expanded = self.nav.view.browsers_expanded;
        let label = format!(
            "Browsers, {} apps, {} focused, {} running combined. {}",
            group.members.len(),
            format::duration(group.focused_ms),
            format::duration(group.running_ms),
            if expanded { "Collapse." } else { "Expand." }
        );
        let mut toggled = false;
        let row = widgets::table_row(
            ui,
            ui.id().with("browser-group"),
            columns,
            RowStyle {
                height: ROW_HEIGHT,
                ..RowStyle::default()
            },
            &label,
            |row| {
                row.cell(0, |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let chevron = Button {
                        min_width: 24.0,
                        ..Button::new(
                            if expanded {
                                "Hide browsers"
                            } else {
                                "Show browsers"
                            },
                            ButtonKind::GhostIcon,
                        )
                        .small()
                        .with_icon(if expanded {
                            icons::CARET_DOWN
                        } else {
                            icons::CARET_RIGHT
                        })
                    };
                    toggled = ui.add(chevron).clicked();
                    widgets::app_cell(
                        ui,
                        None,
                        "Browsers",
                        Some(&format!(
                            "{} browsers · running time is combined",
                            group.members.len()
                        )),
                        Category::Browsers,
                        32.0,
                    );
                });
                row.cell(1, |ui| widgets::category_label(ui, Category::Browsers));
                row.number(2, &format::duration(group.focused_ms), colors.fg);
                row.number(3, &format::duration(group.running_ms), colors.fg_muted);
            },
        );
        (toggled || row.clicked()).then_some(RowAction::Toggle)
    }

    fn library_row(
        &self,
        ui: &mut Ui,
        columns: &widgets::Columns,
        row: &LibraryRow,
        indent: f32,
        today: NaiveDate,
    ) -> Option<RowAction> {
        let online = self.recorder_online();
        let colors = Tokens::get(ui);
        let last_seen = row.last_seen_ms.map(|ms| last_seen(ms, self.zone, today));
        let label = format!(
            "{}{}, {}, {}{}. Open details.",
            row.name,
            if row.live { ", running now" } else { "" },
            row.category.label(),
            match (row.focused_ms, row.running_ms) {
                (Some(focused), Some(running)) => format!(
                    "{} focused, {} running",
                    format::duration(focused),
                    format::duration(running)
                ),
                _ => "no activity in this range".into(),
            },
            last_seen
                .as_ref()
                .map(|seen| format!(", last seen {seen}"))
                .unwrap_or_default(),
        );
        let mut menu = None;
        let response = widgets::table_row(
            ui,
            ui.id().with(("library-row", &row.key)),
            columns,
            RowStyle {
                height: ROW_HEIGHT,
                indent,
                ..RowStyle::default()
            },
            &label,
            |cells| {
                cells.cell(0, |ui| {
                    widgets::app_cell_with(
                        ui,
                        Some(&row.key),
                        &row.name,
                        row.subtitle.as_deref(),
                        row.category,
                        32.0,
                        |ui| {
                            if row.live {
                                widgets::live_dot(ui);
                            }
                            if row.flags.edited {
                                widgets::tone_tag(ui, "Edited", colors.fg_muted);
                            }
                        },
                    )
                    .on_hover_text(row.source.as_str());
                });
                cells.cell(1, |ui| widgets::category_label(ui, row.category));
                let dash = "—".to_owned();
                cells.number(
                    2,
                    &row.focused_ms.map_or(dash.clone(), format::duration),
                    colors.fg,
                );
                cells.number(
                    3,
                    &row.running_ms.map_or(dash, format::duration),
                    colors.fg_muted,
                );
                if let Some(seen) = &last_seen {
                    cells.cell(4, |ui| {
                        widgets::truncated(
                            ui,
                            RichText::new(seen)
                                .font(Tokens::mono(Tokens::SECONDARY))
                                .color(colors.fg_muted),
                        );
                    });
                }
                if let Some(week) = row.week {
                    cells.cell(5, |ui| {
                        let max = week.iter().copied().max().unwrap_or(0).max(1);
                        let scaled = week.map(|ms| (ms * 255 / max) as u8);
                        widgets::sparkline(ui, row.category, scaled);
                    });
                }
                let actions_label = format!("Actions for {}", row.name);
                cells.cell(6, |ui| {
                    let button = Button {
                        min_width: 32.0,
                        ..Button::new(&actions_label, ButtonKind::GhostIcon)
                            .small()
                            .with_icon(icons::DOTS_THREE)
                    };
                    let response = ui.add(button);
                    if response.clicked() {
                        menu = Some(response.rect);
                    }
                });
            },
        );
        if let Some(anchor) = menu {
            return Some(RowAction::Menu(row.key.clone(), anchor));
        }
        if online && response.has_focus() && ui.input(|input| input.key_pressed(Key::F2)) {
            return Some(RowAction::Edit(row.key.clone()));
        }
        response.clicked().then(|| RowAction::Open(row.key.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::nav::Nav;
    use chrono::Weekday;
    use tempora::storage::{AppSummary, CategorySummary, Totals};

    const HOUR: i64 = 3_600_000;

    fn app(key: &str, category: Category, focused: i64, running: i64) -> AppSummary {
        AppSummary {
            key: key.into(),
            name: key.trim_start_matches("exe:").into(),
            category,
            totals: Totals {
                focused_ms: focused,
                running_ms: running,
                ..Totals::default()
            },
            last_seen_ms: focused,
            days: Vec::new(),
            sessions: Vec::new(),
        }
    }

    fn report() -> Report {
        Report {
            apps: vec![
                app("exe:firefox", Category::Browsers, HOUR, 3 * HOUR),
                app("exe:chrome", Category::Browsers, HOUR / 2, 2 * HOUR),
                app("steam:1", Category::Games, 3 * HOUR, 3 * HOUR),
                app("exe:code", Category::Productivity, HOUR / 4, HOUR),
            ],
            categories: vec![CategorySummary {
                category: Category::Browsers,
                totals: Totals {
                    focused_ms: 3 * HOUR / 2,
                    running_ms: 4 * HOUR,
                    ..Totals::default()
                },
            }],
            ..Report::default()
        }
    }

    #[test]
    fn browsers_group_with_unioned_running_time_in_sort_order() {
        let nav = Nav::new(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(), Weekday::Mon);
        let vm = library_vm(&report(), &[], &nav.view, &|key| key == "steam:1", None);
        assert_eq!(vm.entries.len(), 3, "two apps plus the browser group");
        let LibraryEntry::App(first) = &vm.entries[0] else {
            panic!("game first")
        };
        assert!(first.live);
        let LibraryEntry::Browsers(group) = &vm.entries[1] else {
            panic!("browsers second")
        };
        assert_eq!(
            group.running_ms,
            4 * HOUR,
            "union from the category, not 5h"
        );
        assert_eq!(group.members.len(), 2);
        assert!(matches!(&vm.entries[2], LibraryEntry::App(row) if row.key == "exe:code"));
    }

    #[test]
    fn search_finds_inactive_catalog_entries_and_filters_apply() {
        let mut nav = Nav::new(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(), Weekday::Mon);
        let catalog = vec![AppIdentity {
            key: r"exe:c:\tools\factorio.exe".into(),
            name: "Factorio".into(),
            category: Category::Games,
        }];
        nav.view.search = "factorio".into();
        let vm = library_vm(&report(), &catalog, &nav.view, &|_| false, None);
        assert!(matches!(&vm.entries[..], [LibraryEntry::App(row)] if row.focused_ms.is_none()));
        nav.view.search.clear();
        nav.view.library = LibraryFilter::Apps;
        let vm = library_vm(&report(), &catalog, &nav.view, &|_| false, None);
        assert_eq!(vm.entries.len(), 2);
        nav.view.category = Some(Category::Productivity);
        let vm = library_vm(&report(), &catalog, &nav.view, &|_| false, None);
        assert_eq!(vm.entries.len(), 1);
    }

    #[test]
    fn edited_flag_tags_rows_and_fills_inactive_last_seen() {
        let mut nav = Nav::new(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(), Weekday::Mon);
        let catalog = vec![AppIdentity {
            key: r"exe:d:\old\tool.exe".into(),
            name: "Old tool".into(),
            category: Category::Other,
        }];
        let mut extras = Extras::default();
        extras
            .flags
            .insert("steam:1".into(), LibraryFlags { edited: true });
        extras.last_seen.insert(catalog[0].key.clone(), 42);
        extras.week.insert("steam:1".into(), [0, 1, 2, 3, 4, 5, 6]);
        let vm = library_vm(&report(), &catalog, &nav.view, &|_| false, Some(&extras));
        let LibraryEntry::App(game) = &vm.entries[0] else {
            panic!("game first")
        };
        assert!(game.flags.edited);
        assert_eq!(game.week, Some([0, 1, 2, 3, 4, 5, 6]));
        nav.view.search = "old".into();
        let vm = library_vm(&report(), &catalog, &nav.view, &|_| false, Some(&extras));
        assert!(
            matches!(&vm.entries[..], [LibraryEntry::App(row)] if row.last_seen_ms == Some(42))
        );
    }
}
