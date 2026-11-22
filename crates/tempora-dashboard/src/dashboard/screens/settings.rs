//! Settings (`Settings` mockup). Recorder settings save immediately through `SetSettings`;
//! appearance and week start are dashboard preferences.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Weekday;
use eframe::egui::{self, Align, Layout, Rect, ThemePreference, Ui, vec2};
use egui_phosphor::regular as icons;
use tempora::model::{AppIdentity, CaptureMode, ControlRequest, Settings, database_path};

use super::Popup;
use crate::dashboard::nav::Page;
use crate::dashboard::theme::Tokens;
use crate::dashboard::widgets::{self, Button, ButtonKind, StatusKind};
use crate::dashboard::{APP_NAME, Dashboard, Job, format};

const SECTIONS: [&str; 6] = [
    "General",
    "Capture",
    "Apps & exclusions",
    "Data & backup",
    "Change history",
    "Recorder",
];
const IDLE_MINUTES: [u32; 5] = [2, 5, 10, 15, 30];

#[derive(Default)]
pub(in crate::dashboard) struct SettingsState {
    pub export: Option<Result<PathBuf, String>>,
    pub exporting: bool,
    pub backup: Option<Result<PathBuf, String>>,
    /// Section to scroll to on the next frame.
    pub scroll_to: Option<usize>,
    /// Database path and size, read once per visit.
    pub database: Option<(PathBuf, Option<u64>)>,
}

fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb < 10.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{mb:.0} MB")
    }
}

/// A count with thousands separators: 12,345.
fn grouped(count: impl ToString) -> String {
    count
        .to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",")
}

fn show_in_folder(path: &Path) {
    let _ = Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

/// Resolves a stored key to a display identity using what the dashboard already has.
fn identity(key: &str, known: &[AppIdentity]) -> (String, String, tempora::model::Category) {
    let source = format::source(key);
    match known.iter().find(|app| app.key == key) {
        Some(app) => (
            app.name.clone(),
            source.path.unwrap_or(source.line),
            app.category,
        ),
        None => (
            source.line.clone(),
            source.path.unwrap_or(source.line),
            Default::default(),
        ),
    }
}

impl Dashboard {
    /// Applies a change locally and sends it to the recorder at once.
    fn save_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut settings = self.settings.clone();
        change(&mut settings);
        if settings != self.settings {
            self.settings = settings.clone();
            self.settings_dirty = true;
            self.action(ControlRequest::SetSettings(settings), "Settings saved");
        }
    }

    pub(super) fn settings_page(&mut self, ui: &mut Ui, wide: bool) {
        if self.settings_state.database.is_none() {
            self.settings_state.database = database_path().ok().map(|path| {
                (
                    path.clone(),
                    std::fs::metadata(&path).ok().map(|meta| meta.len()),
                )
            });
        }
        let online = self.recorder_online() && !self.pending;
        let mut jump = None;
        let sections = |ui: &mut Ui, this: &mut Self| {
            let mut rects = Vec::new();
            ui.spacing_mut().item_spacing.y = Tokens::GRID_GAP;
            rects.push(this.general_card(ui, online));
            rects.push(this.capture_card(ui, online));
            rects.push(this.apps_card(ui, online));
            rects.push(this.data_card(ui, online));
            rects.push(this.changes_card(ui, online));
            rects.push(this.recorder_card(ui));
            let colors = Tokens::get(ui);
            ui.label(widgets::text(
                format!(
                    "{APP_NAME} {} · recorder and dashboard run as separate processes. Closing this window doesn't stop recording.",
                    env!("CARGO_PKG_VERSION")
                ),
                Tokens::CAPTION,
                colors.fg_subtle,
            ));
            if let Some(index) = this.settings_state.scroll_to.take()
                && let Some(rect) = rects.get(index)
            {
                ui.scroll_to_rect(*rect, Some(Align::TOP));
            }
        };
        let mut nav = |ui: &mut Ui| {
            for (index, section) in SECTIONS.iter().enumerate() {
                if widgets::list_item(ui, section, None, false).clicked() {
                    jump = Some(index);
                }
            }
        };
        if wide {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 32.0;
                ui.allocate_ui_with_layout(vec2(200.0, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(200.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    nav(ui);
                });
                let width = ui.available_width();
                ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(width);
                    sections(ui, self);
                });
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (index, section) in SECTIONS.iter().enumerate() {
                    if ui
                        .add(Button::new(section, ButtonKind::Ghost).small())
                        .clicked()
                    {
                        jump = Some(index);
                    }
                }
            });
            sections(ui, self);
        }
        if jump.is_some() {
            self.settings_state.scroll_to = jump;
            ui.ctx().request_repaint();
        }
    }

    fn general_card(&mut self, ui: &mut Ui, online: bool) -> Rect {
        widgets::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 14.0;
            widgets::section_header(ui, "General", None, |_| {});
            let mut sign_in = self.settings.launch_at_sign_in;
            let changed = widgets::setting_row(
                ui,
                "Start at sign-in",
                "Runs the recorder in the tray when you sign in to Windows. Takes effect at your next sign-in.",
                |ui| ui.add_enabled_ui(online, |ui| widgets::switch(ui, "", &mut sign_in).changed()).inner,
            );
            if changed {
                self.save_settings(|settings| settings.launch_at_sign_in = sign_in);
            }
            let mut appearance = self.prefs.appearance;
            let changed = widgets::setting_row(
                ui,
                "Appearance",
                "Follows the Windows light or dark setting by default.",
                |ui| {
                    widgets::segmented(
                        ui,
                        &mut appearance,
                        &[
                            (ThemePreference::System, "System"),
                            (ThemePreference::Light, "Light"),
                            (ThemePreference::Dark, "Dark"),
                        ],
                    )
                },
            );
            if changed {
                self.prefs.appearance = appearance;
                self.prefs.apply(ui.ctx());
            }
            let mut week = self.prefs.week_start;
            let changed = widgets::setting_row(
                ui,
                "Week starts on",
                "Used by Overview and date pickers.",
                |ui| widgets::segmented(ui, &mut week, &[(Weekday::Mon, "Monday"), (Weekday::Sun, "Sunday")]),
            );
            if changed {
                self.prefs.week_start = week;
            }
        })
        .response
        .rect
    }

    fn capture_card(&mut self, ui: &mut Ui, online: bool) -> Rect {
        widgets::card(ui, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            ui.spacing_mut().item_spacing.y = 14.0;
            widgets::section_header(ui, "Capture", Some("Everything stays on this PC. Nothing is sent anywhere."), |_| {});
            let mut mode = self.settings.mode;
            ui.add_enabled_ui(online, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let card_width = (width - 12.0) / 2.0;
                    for (option, title, detail) in [
                        (CaptureMode::All, "All apps", "Record everything except the apps you exclude."),
                        (
                            CaptureMode::Selected,
                            "Only apps I pick",
                            "Record just the apps you pick. Others aren't stored at all.",
                        ),
                    ] {
                        if widgets::radio_card(ui, mode == option, title, detail, card_width).clicked() {
                            mode = option;
                        }
                    }
                });
            });
            let mut idle = self.settings.idle_minutes;
            let idle_changed = widgets::setting_row(
                ui,
                "Idle threshold",
                "After this long without keyboard, mouse or controller input, time counts as estimated idle. Applies from now on.",
                |ui| {
                    ui.add_enabled_ui(online, |ui| {
                        let labels: Vec<String> = IDLE_MINUTES.iter().map(|minutes| format!("{minutes} min")).collect();
                        let options: Vec<(u32, &str)> = IDLE_MINUTES
                            .iter()
                            .zip(&labels)
                            .map(|(minutes, label)| (*minutes, label.as_str()))
                            .collect();
                        widgets::segmented(ui, &mut idle, &options)
                    })
                    .inner
                },
            );
            if mode != self.settings.mode {
                self.save_settings(|settings| settings.mode = mode);
            }
            if idle_changed {
                self.save_settings(|settings| settings.idle_minutes = idle);
            }
        })
        .response
        .rect
    }

    fn apps_card(&mut self, ui: &mut Ui, online: bool) -> Rect {
        let selected_mode = self.settings.mode == CaptureMode::Selected;
        let keys: Vec<String> = if selected_mode {
            self.settings.selected.iter().cloned().collect()
        } else {
            self.settings.excluded.iter().cloned().collect()
        };
        let candidates: Vec<AppIdentity> = self
            .recorder
            .iter()
            .flat_map(|status| status.candidates.clone())
            .collect();
        let known: Vec<AppIdentity> = self.apps.iter().chain(&candidates).cloned().collect();
        let mut remove = None;
        let mut add_anchor = None;
        let rect = widgets::card(ui, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            ui.spacing_mut().item_spacing.y = 12.0;
            let (title, subtitle) = if selected_mode {
                ("Selected apps", "Only these apps are recorded.")
            } else {
                (
                    "Excluded apps",
                    "Never recorded. Existing history stays until you delete it.",
                )
            };
            widgets::section_header(ui, title, Some(subtitle), |ui| {
                let button = ui.add_enabled(
                    online && !candidates.is_empty(),
                    Button::new("Add from running apps", ButtonKind::Secondary)
                        .with_icon(icons::PLUS),
                );
                if button.clicked() {
                    add_anchor = Some(button.rect);
                }
            });
            let colors = Tokens::get(ui);
            if keys.is_empty() {
                ui.label(widgets::text(
                    if selected_mode {
                        "No apps selected yet. Add one from the apps running now."
                    } else {
                        "No apps excluded."
                    },
                    Tokens::SECONDARY,
                    colors.fg_muted,
                ));
            }
            for key in &keys {
                let (name, path, category) = identity(key, &known);
                ui.horizontal(|ui| {
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        vec2(width - 40.0, 40.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            widgets::app_cell(ui, Some(key), &name, Some(&path), category, 28.0);
                        },
                    );
                    if ui
                        .add_enabled(
                            online,
                            Button::new(&format!("Remove {name}"), ButtonKind::GhostIcon)
                                .small()
                                .with_icon(icons::X),
                        )
                        .clicked()
                    {
                        remove = Some(key.clone());
                    }
                });
            }
        })
        .response
        .rect;
        if let Some(anchor) = add_anchor {
            self.popup = match self.popup {
                Some(Popup::AddApp(_)) => None,
                _ => Some(Popup::AddApp(anchor)),
            };
        }
        if let Some(key) = remove {
            self.save_settings(|settings| {
                if selected_mode {
                    settings.selected.remove(&key);
                } else {
                    settings.excluded.remove(&key);
                }
            });
        }
        if let Some(Popup::AddApp(anchor)) = self.popup {
            let available: Vec<&AppIdentity> = candidates
                .iter()
                .filter(|app| !keys.contains(&app.key))
                .collect();
            let output = widgets::anchored_menu(
                ui.ctx(),
                egui::Id::new("add-running-app"),
                anchor,
                Align::Max,
                |ui| {
                    ui.set_width(260.0);
                    let mut picked = None;
                    if available.is_empty() {
                        ui.label(widgets::text(
                            "Every running app is already listed.",
                            Tokens::SECONDARY,
                            Tokens::get(ui).fg_muted,
                        ));
                    }
                    for app in available {
                        if widgets::list_item(ui, &app.name, Some(icons::PLUS), false).clicked() {
                            picked = Some(app.key.clone());
                        }
                    }
                    picked
                },
            );
            if let Some(key) = output.inner {
                self.popup = None;
                self.save_settings(|settings| {
                    if selected_mode {
                        settings.selected.insert(key);
                    } else {
                        settings.excluded.insert(key);
                    }
                });
            } else if output.dismissed {
                self.popup = None;
            }
        }
        rect
    }

    fn data_card(&mut self, ui: &mut Ui, online: bool) -> Rect {
        widgets::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 14.0;
            widgets::section_header(ui, "Data & backup", None, |_| {});
            let colors = Tokens::get(ui);
            let exporting = self.settings_state.exporting;
            let export = widgets::setting_row(
                ui,
                "Export sessions to CSV",
                "App names as shown, start and end timestamps, focused and running durations.",
                |ui| {
                    // One control: the range to export, then Export (right to left here).
                    widgets::button_group(ui, |ui| {
                        let clicked = ui
                            .add_enabled(
                                !exporting,
                                Button::new("Export…", ButtonKind::Secondary)
                                    .with_icon(icons::DOWNLOAD_SIMPLE)
                                    .segment(),
                            )
                            .clicked();
                        self.range_control_styled(ui, true);
                        clicked
                    })
                },
            );
            if export
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name(format!("tempora-{}.csv", self.nav.view.range.start))
                    .save_file()
            {
                self.settings_state.exporting = true;
                self.settings_state.export = None;
                self.send(Job::Export(path, self.nav.view.range, self.nav.view.exclude_idle));
            }
            if exporting {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(widgets::text("Exporting…", Tokens::SECONDARY, colors.fg_muted));
                });
            }
            let mut retry_export = false;
            match &self.settings_state.export {
                Some(Ok(path)) => {
                    if widgets::banner(
                        ui,
                        widgets::BannerKind::Info,
                        "Export complete",
                        &format!("Saved to {}", path.display()),
                        &[(Button::new("Show in folder", ButtonKind::Secondary).small(), true)],
                    )
                    .is_some()
                    {
                        show_in_folder(path);
                    }
                }
                Some(Err(error)) => {
                    retry_export = widgets::banner(
                        ui,
                        widgets::BannerKind::Error,
                        "Export failed. Nothing was written.",
                        error,
                        &[(Button::new("Dismiss", ButtonKind::Secondary).small(), true)],
                    )
                    .is_some();
                }
                None => {}
            }
            if retry_export {
                self.settings_state.export = None;
            }
            let backup = widgets::setting_row(
                ui,
                "Back up database",
                "A complete, consistent copy, safe to take while recording.",
                |ui| {
                    ui.add_enabled(online, Button::new("Back up now…", ButtonKind::Secondary).with_icon(icons::DATABASE))
                        .clicked()
                },
            );
            let mut choose = backup;
            match &self.settings_state.backup {
                Some(Ok(path)) => {
                    if widgets::banner(
                        ui,
                        widgets::BannerKind::Info,
                        "Backup complete",
                        &format!("Saved to {}", path.display()),
                        &[(Button::new("Show in folder", ButtonKind::Secondary).small(), true)],
                    )
                    .is_some()
                    {
                        show_in_folder(path);
                    }
                }
                Some(Err(error)) => {
                    choose |= widgets::banner(
                        ui,
                        widgets::BannerKind::Error,
                        "Backup failed. Nothing was written.",
                        error,
                        &[(Button::new("Choose another folder", ButtonKind::Secondary).small(), online)],
                    )
                    .is_some();
                }
                None => {}
            }
            if choose
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("SQLite", &["sqlite3"])
                    .set_file_name("tempora-backup.sqlite3")
                    .save_file()
            {
                self.pending = true;
                self.settings_state.backup = None;
                self.send(Job::Backup(path));
            }
            if widgets::setting_row(
                ui,
                "Delete an app's history",
                "Pick an app in the Library and choose Delete history. You'll see exactly what goes before anything is removed.",
                |ui| ui.add(Button::new("Open Library", ButtonKind::Secondary)).clicked(),
            ) {
                self.nav.section(Page::Library);
            }
            if widgets::setting_row(
                ui,
                "Delete all history",
                "Permanently removes every recorded session. You'll see a preview first.",
                |ui| {
                    ui.add_enabled(online, Button::new("Delete all history…", ButtonKind::Danger))
                        .clicked()
                },
            ) {
                self.open_delete(None);
            }
            if let Some((path, size)) = self.settings_state.database.clone() {
                let mut detail = path.display().to_string();
                if let Some(bytes) = size {
                    detail.push_str(&format!(" · {}", megabytes(bytes)));
                }
                if let Some(count) = self.extras().and_then(|extras| extras.intervals) {
                    detail.push_str(&format!(" · {} recorded spans", grouped(count)));
                }
                if widgets::setting_row(ui, "Database", &detail, |ui| {
                    ui.add(Button::new("Open folder", ButtonKind::Secondary)).clicked()
                }) {
                    show_in_folder(&path);
                }
            }
        })
        .response
        .rect
    }

    fn changes_card(&mut self, ui: &mut Ui, online: bool) -> Rect {
        widgets::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 14.0;
            widgets::section_header(
                ui,
                "Change history",
                Some("Renames, regroups and category changes. Undo restores the previous names and totals."),
                |_| {},
            );
            let corrections = self
                .extras()
                .map(|extras| extras.corrections.clone())
                .unwrap_or_default();
            let colors = Tokens::get(ui);
            if corrections.is_empty() {
                ui.label(widgets::text("No corrections yet.", Tokens::SECONDARY, colors.fg_muted));
            }
            let mut undo = false;
            for (index, correction) in corrections.iter().enumerate() {
                let newest = index == 0;
                let detail = if newest {
                    "Newest change. Undo restores the previous names, categories and grouping."
                } else {
                    "Undo the newer changes first."
                };
                undo |= widgets::setting_row(ui, &correction.apps.join(", "), detail, |ui| {
                    ui.add_enabled(online && newest, Button::new("Undo", ButtonKind::Secondary))
                        .clicked()
                });
            }
            if undo {
                self.action(ControlRequest::UndoCorrection, "Change undone");
            }
        })
        .response
        .rect
    }

    fn recorder_card(&mut self, ui: &mut Ui) -> Rect {
        let kind = self.status_kind();
        let paused = self.recorder.as_ref().map(|status| status.settings.paused);
        let mut toggle = false;
        let rect = widgets::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 14.0;
            widgets::section_header(ui, "Recorder", None, |_| {});
            let (title, detail) = match kind {
                StatusKind::Unreachable => (
                    "Can't reach the recorder",
                    "History stays readable. Start the recorder to keep tracking.",
                ),
                StatusKind::Paused => ("Paused", "Nothing is captured until you resume."),
                StatusKind::Recording => (
                    "Recording",
                    "Runs in the tray. Quit it from the tray icon's menu.",
                ),
                StatusKind::Attention => {
                    ("Needs attention", "See the message at the top of the page.")
                }
                StatusKind::Setup => ("Not set up", "Finish setup to start recording."),
                StatusKind::Connecting => ("Connecting…", "Waiting for the recorder to answer."),
            };
            widgets::setting_row(ui, title, detail, |ui| {
                if let Some(paused) = paused {
                    let label = if paused { "Resume" } else { "Pause" };
                    let icon = if paused { icons::PLAY } else { icons::PAUSE };
                    toggle = ui
                        .add_enabled(
                            !self.pending,
                            Button::new(label, ButtonKind::Secondary).with_icon(icon),
                        )
                        .clicked();
                }
            });
        })
        .response
        .rect;
        if toggle && let Some(paused) = paused {
            self.action(
                ControlRequest::SetPaused(!paused),
                if paused {
                    "Recording resumed"
                } else {
                    "Recording paused"
                },
            );
        }
        rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_identities_render_from_known_data() {
        assert_eq!(megabytes(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
        assert_eq!(megabytes(186 * 1024 * 1024), "186 MB");
        let known = [AppIdentity {
            key: r"exe:c:\games\game.exe".into(),
            name: "Game".into(),
            category: tempora::model::Category::Games,
        }];
        assert_eq!(identity(r"exe:c:\games\game.exe", &known).0, "Game");
        assert_eq!(identity(r"exe:c:\tools\x.exe", &known).0, "x.exe");
    }
}
