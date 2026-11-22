//! The This day card: the day's apps with their focused time.

use eframe::egui::{Align, RichText, Ui};
use egui_phosphor::regular as icons;

use crate::dashboard::format;
use crate::dashboard::theme::{self, Tokens};
use crate::dashboard::widgets::{self, Column, RowStyle, Width};

use super::*;

pub(super) fn this_day_card(
    ui: &mut Ui,
    vm: &DayVm,
    viewport_bottom: f32,
    online: bool,
) -> Option<DayAction> {
    let colors = Tokens::get(ui);
    // The same per-app colors as the lane and the Activity list.
    let palette = app_colors(&vm.apps);
    let mut action = None;
    widgets::table_card(ui, true, |ui| {
        let width = ui.available_width();
        ui.set_min_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        theme::section(ui, "This day");
        ui.add_space(10.0);
        let specs = [
            Column::new("App", Width::Flex(1.0, 120.0), Align::Min),
            // Wide enough for "running 12h 05m" on the apps that were never in front.
            Column::new("Focused", Width::Fixed(120.0), Align::Max),
        ];
        let columns = widgets::scrolled_columns(ui, width, &specs);
        crate::dashboard::screens::fitted_list(
            viewport_bottom,
            ui,
            "this-day",
            widgets::TITLED_TABLE_BOTTOM,
            |ui| {
                let (focused, unfocused): (Vec<&DayApp>, Vec<&DayApp>) =
                    vm.apps.iter().partition(|app| app.focused_ms > 0);
                for app in focused {
                    let label = format!(
                        "{}, {} focused. Open details. Right-click for options.",
                        app.name,
                        format::duration(app.focused_ms)
                    );
                    let row = widgets::table_row(
                        ui,
                        ui.id().with(("this-day", &app.key)),
                        &columns,
                        RowStyle {
                            height: 34.0,
                            ..RowStyle::default()
                        },
                        &label,
                        |row| {
                            row.cell(0, |ui| {
                                widgets::swatch(ui, app_color(Some(&app.key), &palette), 10.0);
                                widgets::truncated(
                                    ui,
                                    RichText::new(&app.name).size(Tokens::SECONDARY),
                                );
                            });
                            row.number(1, &format::duration(app.focused_ms), colors.fg);
                        },
                    );
                    if let Some(next) = row_action(ui, &row, &app.key, online) {
                        action = Some(next);
                    }
                }
                if unfocused.is_empty() {
                    return;
                }
                // Apps that never had focus: collapsed, with their running time when open.
                let toggle_id = ui.id().with("this-day-also-running");
                let mut expanded = ui
                    .ctx()
                    .data_mut(|data| data.get_temp::<bool>(toggle_id))
                    .unwrap_or(false);
                let label = format!(
                    "Also running, {} apps with no focus. {}",
                    unfocused.len(),
                    if expanded { "Collapse." } else { "Expand." }
                );
                let toggle = widgets::table_row(
                    ui,
                    ui.id().with("also-running"),
                    &columns,
                    RowStyle {
                        height: 34.0,
                        clickable: true,
                        ..RowStyle::default()
                    },
                    &label,
                    |row| {
                        row.cell(0, |ui| {
                            let caret = if expanded {
                                icons::CARET_DOWN
                            } else {
                                icons::CARET_RIGHT
                            };
                            ui.label(RichText::new(caret).color(colors.fg_muted));
                            widgets::truncated(
                                ui,
                                RichText::new(format!("Also running · {}", unfocused.len()))
                                    .size(Tokens::SECONDARY)
                                    .color(colors.fg_muted),
                            );
                        });
                    },
                )
                .on_hover_text("Apps that were running today but never in front");
                if toggle.clicked() {
                    expanded = !expanded;
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(toggle_id, expanded));
                }
                if !expanded {
                    return;
                }
                for app in unfocused {
                    let label = format!(
                        "{}, {} running, no focus. Open details. Right-click for options.",
                        app.name,
                        format::duration(app.running_ms)
                    );
                    let row = widgets::table_row(
                        ui,
                        ui.id().with(("this-day", &app.key)),
                        &columns,
                        RowStyle {
                            height: 34.0,
                            clickable: true,
                            ..RowStyle::default()
                        },
                        &label,
                        |row| {
                            row.cell(0, |ui| {
                                widgets::swatch(ui, app_color(Some(&app.key), &palette), 10.0);
                                widgets::truncated(
                                    ui,
                                    RichText::new(&app.name)
                                        .size(Tokens::SECONDARY)
                                        .color(colors.fg_muted),
                                );
                            });
                            // Running time, not focus: labelled so it can't read as focused.
                            row.number(
                                1,
                                &format!("running {}", format::duration(app.running_ms)),
                                colors.fg_muted,
                            );
                        },
                    );
                    if let Some(next) = row_action(ui, &row, &app.key, online) {
                        action = Some(next);
                    }
                }
            },
        );
    });
    action
}
