//! The Activity card: one row per stretch of use, breaks and folded runs of quick switches.

use std::collections::HashMap;

use chrono_tz::Tz;
use eframe::egui::{self, Align, Response, RichText, Ui};
use egui_phosphor::regular as icons;

use crate::dashboard::format;
use crate::dashboard::screens::pointer_anchor;
use crate::dashboard::theme::{self, Tokens};
use crate::dashboard::widgets::{self, Column, Columns, RowStyle, Width};

use super::*;

pub(super) fn activity_card(
    ui: &mut Ui,
    rows: &[ActivityRow],
    palette: &HashMap<String, usize>,
    zone: Tz,
    viewport_bottom: f32,
    online: bool,
) -> Option<DayAction> {
    let mut action = None;
    widgets::table_card(ui, true, |ui| {
        let width = ui.available_width();
        ui.set_min_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        theme::section(ui, "Activity");
        ui.add_space(10.0);
        let specs = [
            Column::new("Time", Width::Fixed(160.0), Align::Min),
            Column::new("Activity", Width::Flex(1.0, 140.0), Align::Min),
            Column::new("Duration", Width::Fixed(72.0), Align::Max),
        ];
        let columns = widgets::scrolled_columns(ui, width, &specs);
        widgets::table_header(ui, &columns, &specs.map(|column| column.label));
        // Explicit height so the list scrolls inside its card (see This day).
        crate::dashboard::screens::fitted_list(
            viewport_bottom,
            ui,
            "activity",
            widgets::TITLED_TABLE_BOTTOM,
            |ui| {
                for (index, row) in rows.iter().enumerate() {
                    if let Some(next) =
                        activity_row(ui, &columns, index, row, palette, zone, online)
                    {
                        action = Some(next);
                    }
                }
            },
        );
    });
    action
}

pub(super) fn activity_row(
    ui: &mut Ui,
    columns: &Columns,
    index: usize,
    row: &ActivityRow,
    palette: &HashMap<String, usize>,
    zone: Tz,
    online: bool,
) -> Option<DayAction> {
    let colors = Tokens::get(ui);
    match row {
        ActivityRow::Focus(stretch) => {
            let range = span_label(stretch.start_ms, stretch.end_ms, zone, "%H:%M");
            let duration = format::duration(stretch.end_ms - stretch.start_ms);
            let swatch = match &stretch.key {
                Some(key) => app_color(Some(key), palette),
                None => colors.border,
            };
            let label = match &stretch.key {
                Some(_) => format!(
                    "{}, {range}, {duration}. Open details. Right-click for options.",
                    stretch.name
                ),
                None => format!("{}, {range}, {duration}.", stretch.name),
            };
            let response = widgets::table_row(
                ui,
                ui.id().with(("activity", index)),
                columns,
                RowStyle {
                    height: 34.0,
                    clickable: stretch.key.is_some(),
                    ..RowStyle::default()
                },
                &label,
                |row| {
                    row.cell(0, |ui| {
                        ui.label(widgets::mono(&range, Tokens::SECONDARY).color(colors.fg_muted));
                    });
                    row.cell(1, |ui| {
                        widgets::swatch(ui, swatch, 10.0);
                        widgets::truncated(
                            ui,
                            RichText::new(&stretch.name).size(Tokens::SECONDARY),
                        );
                    });
                    row.number(2, &duration, colors.fg);
                },
            );
            stretch
                .key
                .as_ref()
                .and_then(|key| row_action(ui, &response, key, online))
        }
        ActivityRow::Pause { start_ms, end_ms } => {
            break_row(ui, columns, index, "Paused", (*start_ms, *end_ms), zone);
            None
        }
        ActivityRow::Gap { start_ms, end_ms } => {
            break_row(
                ui,
                columns,
                index,
                "Not recording",
                (*start_ms, *end_ms),
                zone,
            );
            None
        }
        ActivityRow::QuickSwitches {
            start_ms,
            end_ms,
            total_ms,
            names,
            more,
            folded,
        } => {
            let id = ui.id().with(("activity-quick", *start_ms));
            let mut expanded = ui
                .ctx()
                .data_mut(|data| data.get_temp::<bool>(id))
                .unwrap_or(false);
            let count = folded.len();
            let title = format!("{count} quick switch{}", if count == 1 { "" } else { "es" });
            let mut detail = names.join(", ");
            if *more > 0 {
                detail.push_str(&format!(", +{more} more"));
            }
            let range = span_label(*start_ms, *end_ms, zone, "%H:%M");
            let duration = format::duration(*total_ms);
            let label = format!(
                "{title}, {duration}. {}",
                if expanded {
                    "Collapse."
                } else {
                    "Expand to show each."
                }
            );
            let caret = if expanded {
                icons::CARET_DOWN
            } else {
                icons::CARET_RIGHT
            };
            let response = widgets::table_row(
                ui,
                ui.id().with(("activity", index)),
                columns,
                RowStyle {
                    height: 34.0,
                    ..RowStyle::default()
                },
                &label,
                |row| {
                    row.cell(0, |ui| {
                        ui.label(widgets::mono(&range, Tokens::SECONDARY).color(colors.fg_muted));
                    });
                    row.cell(1, |ui| {
                        ui.label(RichText::new(caret).color(colors.fg_muted));
                        widgets::swatch(ui, colors.border, 10.0);
                        widgets::truncated(
                            ui,
                            RichText::new(format!("{title}, {detail}"))
                                .size(Tokens::SECONDARY)
                                .color(colors.fg_muted),
                        );
                    });
                    row.number(2, &duration, colors.fg);
                },
            );
            if response.clicked() {
                expanded = !expanded;
                ui.ctx().data_mut(|data| data.insert_temp(id, expanded));
            }
            let mut action = None;
            if expanded {
                for (sub, stretch) in folded.iter().enumerate() {
                    if let Some(next) = folded_row(
                        ui,
                        columns,
                        (*start_ms, sub),
                        stretch,
                        palette,
                        zone,
                        online,
                    ) {
                        action = Some(next);
                    }
                }
            }
            action
        }
    }
}

/// A paused or not-recording row: a hatch swatch, no action.
pub(super) fn break_row(
    ui: &mut Ui,
    columns: &Columns,
    index: usize,
    name: &str,
    (start_ms, end_ms): (i64, i64),
    zone: Tz,
) {
    let colors = Tokens::get(ui);
    let range = span_label(start_ms, end_ms, zone, "%H:%M");
    let duration = format::duration(end_ms - start_ms);
    let label = format!("{name}, {range}, {duration}.");
    widgets::table_row(
        ui,
        ui.id().with(("activity", index)),
        columns,
        RowStyle {
            height: 34.0,
            clickable: false,
            ..RowStyle::default()
        },
        &label,
        |row| {
            row.cell(0, |ui| {
                ui.label(widgets::mono(&range, Tokens::SECONDARY).color(colors.fg_muted));
            });
            row.cell(1, |ui| {
                hatch_swatch(ui);
                widgets::truncated(
                    ui,
                    RichText::new(name)
                        .size(Tokens::SECONDARY)
                        .color(colors.fg_muted),
                );
            });
            row.number(2, &duration, colors.fg_muted);
        },
    );
}

/// One app switch folded into a quick-switch row, shown when the row is expanded. Its time has
/// seconds, since these stretches are short. An app opens its details and takes the app menu;
/// the desktop does nothing.
pub(super) fn folded_row(
    ui: &mut Ui,
    columns: &Columns,
    (run, sub): (i64, usize),
    stretch: &Stretch,
    palette: &HashMap<String, usize>,
    zone: Tz,
    online: bool,
) -> Option<DayAction> {
    let colors = Tokens::get(ui);
    let range = span_label(stretch.start_ms, stretch.end_ms, zone, "%H:%M:%S");
    let duration = format::duration(stretch.end_ms - stretch.start_ms);
    let swatch = match &stretch.key {
        Some(key) => app_color(Some(key), palette),
        None => colors.border,
    };
    let label = match &stretch.key {
        Some(_) => format!(
            "{}, {range}, {duration}. Open details. Right-click for options.",
            stretch.name
        ),
        None => format!("{}, {range}, {duration}.", stretch.name),
    };
    let response = widgets::table_row(
        ui,
        ui.id().with(("activity-folded", run, sub)),
        columns,
        RowStyle {
            height: 30.0,
            indent: 16.0,
            clickable: stretch.key.is_some(),
            ..RowStyle::default()
        },
        &label,
        |row| {
            row.cell(0, |ui| {
                ui.label(widgets::mono(&range, Tokens::CAPTION).color(colors.fg_subtle));
            });
            row.cell(1, |ui| {
                widgets::swatch(ui, swatch, 10.0);
                widgets::truncated(
                    ui,
                    RichText::new(&stretch.name)
                        .size(Tokens::SECONDARY)
                        .color(colors.fg_muted),
                );
            });
            row.number(2, &duration, colors.fg_muted);
        },
    );
    stretch
        .key
        .as_ref()
        .and_then(|key| row_action(ui, &response, key, online))
}

/// What a This day app row does: a click opens the app, a right-click opens its menu at the
/// pointer, and F2 edits it while the recorder is online.
pub(super) fn row_action(ui: &Ui, row: &Response, key: &str, online: bool) -> Option<DayAction> {
    if row.clicked() {
        Some(DayAction::OpenApp(key.to_owned()))
    } else if row.secondary_clicked() {
        row.interact_pointer_pos()
            .map(|pos| DayAction::Menu(key.to_owned(), pointer_anchor(pos)))
    } else if online && row.has_focus() && ui.input(|input| input.key_pressed(egui::Key::F2)) {
        Some(DayAction::Edit(key.to_owned()))
    } else {
        None
    }
}
