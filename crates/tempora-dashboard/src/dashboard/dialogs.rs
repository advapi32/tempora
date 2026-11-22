//! Correction and deletion dialogs (`Corrections` mockup). Each shows its preview before
//! committing through the recorder; corrections offer Undo afterwards.

use eframe::egui::{self, Align, Layout, RichText, Ui, vec2};
use egui_phosphor::regular as icons;
use tempora::model::{Category, ControlRequest};

use super::nav::Page;
use super::theme::Tokens;
use super::widgets::{self, Button, ButtonKind, DialogAction};
use super::{AppInfo, Dashboard, DeletePreview, Job, format};

pub(super) enum Dialog {
    Edit(EditDraft),
    Group(GroupDraft),
    Delete(DeleteDraft),
}

pub(super) struct EditDraft {
    pub key: String,
    pub name: String,
    pub category: Category,
    /// Name and category are prefilled once the app's info arrives.
    pub prefilled: bool,
}

pub(super) struct GroupDraft {
    pub source: String,
    pub target: Option<String>,
    pub search: String,
}

pub(super) struct DeleteDraft {
    pub key: Option<String>,
    pub preview: Option<Result<DeletePreview, String>>,
    /// Data version the preview was computed at; a newer one forces a recheck.
    pub version: Option<i64>,
    /// The preview was refreshed because the data changed before confirming.
    pub rechecked: bool,
}

impl Dialog {
    /// Apps whose identity and totals the dialog previews.
    pub fn info_keys(&self) -> Vec<String> {
        match self {
            Self::Edit(edit) => vec![edit.key.clone()],
            Self::Group(group) => std::iter::once(group.source.clone())
                .chain(group.target.clone())
                .collect(),
            Self::Delete(delete) => delete.key.iter().cloned().collect(),
        }
    }
}

/// What a dialog asks for when it closes.
enum Outcome {
    Keep,
    Close,
    /// The delete preview is stale: fetch it again before allowing the commit.
    Recheck,
    Commit(ControlRequest, &'static str),
}

/// Two-column preview rows: muted label, value.
/// A titled block of label / value rows with an aligned label column.
fn preview_rows(ui: &mut Ui, title: &str, rows: &[(&str, String)]) {
    const ROW: f32 = 24.0;
    const LABEL: f32 = 110.0;
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    egui::Frame::new()
        .fill(colors.muted)
        .stroke(egui::Stroke::new(1.0, colors.border))
        .corner_radius(Tokens::POPOVER_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(width - 30.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).font(Tokens::medium(Tokens::SECONDARY)));
            for (label, value) in rows {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW), egui::Sense::hover());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Label,
                        true,
                        format!("{label}: {value}"),
                    )
                });
                ui.painter().text(
                    egui::pos2(rect.left(), rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    *label,
                    egui::FontId::proportional(Tokens::SECONDARY),
                    colors.fg_muted,
                );
                let mut job = egui::text::LayoutJob::simple_singleline(
                    value.clone(),
                    Tokens::mono(Tokens::SECONDARY),
                    colors.fg,
                );
                job.wrap.max_width = (rect.width() - LABEL).max(0.0);
                job.wrap.max_rows = 1;
                let galley = ui.painter().layout_job(job);
                let y = rect.center().y - galley.size().y / 2.0;
                ui.painter()
                    .galley(egui::pos2(rect.left() + LABEL, y), galley, colors.fg);
            }
        });
}

fn description(ui: &mut Ui, text: &str) {
    ui.label(widgets::text(
        text,
        Tokens::SECONDARY,
        Tokens::get(ui).fg_muted,
    ));
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(Tokens::medium(Tokens::CAPTION))
            .color(Tokens::get(ui).fg_muted),
    );
}

fn span(info: &AppInfo, zone: chrono_tz::Tz) -> Option<String> {
    let app = info.all_time.as_ref()?;
    let first = app
        .sessions
        .first()
        .and_then(|session| format::local_date(session.start_ms, zone));
    format::dates_span(first, format::local_date(app.last_seen_ms, zone))
}

impl Dashboard {
    pub(super) fn open_edit(&mut self, key: &str) {
        self.popup = None;
        self.dialog = Some(Dialog::Edit(EditDraft {
            key: key.to_owned(),
            name: String::new(),
            category: Category::Other,
            prefilled: false,
        }));
    }

    pub(super) fn open_group(&mut self, key: &str) {
        self.popup = None;
        self.dialog = Some(Dialog::Group(GroupDraft {
            source: key.to_owned(),
            target: None,
            search: String::new(),
        }));
    }

    pub(super) fn open_delete(&mut self, key: Option<&str>) {
        self.popup = None;
        let key = key.map(str::to_owned);
        self.dialog = Some(Dialog::Delete(DeleteDraft {
            key: key.clone(),
            preview: None,
            version: None,
            rechecked: false,
        }));
        self.send(Job::DeletePreview(key));
    }

    pub(super) fn stop_tracking(&mut self, key: &str) {
        self.popup = None;
        self.pending = true;
        self.send(Job::Exclude(key.to_owned()));
    }

    pub(super) fn draw_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let title = match &dialog {
            Dialog::Edit(_) => "Edit app".to_owned(),
            Dialog::Group(_) => "Group entries".to_owned(),
            Dialog::Delete(draft) => match (&draft.key, &draft.preview) {
                (None, _) => "Delete all recorded history?".to_owned(),
                (Some(_), Some(Ok(preview))) => format!("Delete {} history?", preview.name),
                (Some(_), _) => "Delete app history?".to_owned(),
            },
        };
        let mut outcome = Outcome::Keep;
        let closed = widgets::modal(ctx, egui::Id::new("correction-dialog"), &title, |ui| {
            ui.spacing_mut().item_spacing.y = 14.0;
            outcome = match &mut dialog {
                Dialog::Edit(draft) => self.edit_body(ui, draft),
                Dialog::Group(draft) => self.group_body(ui, draft),
                Dialog::Delete(draft) => self.delete_body(ui, draft),
            };
            !matches!(outcome, Outcome::Keep)
        });
        match outcome {
            Outcome::Commit(request, message) => {
                self.after_commit(&request);
                self.action(request, message);
            }
            Outcome::Close => {}
            Outcome::Recheck => {
                if let Dialog::Delete(draft) = &mut dialog {
                    draft.preview = None;
                    draft.rechecked = true;
                    self.send(Job::DeletePreview(draft.key.clone()));
                }
                self.dialog = Some(dialog);
            }
            Outcome::Keep if closed => {}
            Outcome::Keep => self.dialog = Some(dialog),
        }
    }

    /// Keeps the page valid after a merge or deletion removes the app being shown.
    fn after_commit(&mut self, request: &ControlRequest) {
        match (request, self.nav.page().clone()) {
            (ControlRequest::GroupApps { source, target }, Page::App(shown))
                if shown == *source =>
            {
                self.nav.view.page = Page::App(target.clone());
            }
            (ControlRequest::DeleteHistory { key: Some(key) }, Page::App(shown))
                if shown == *key =>
            {
                self.nav.section(Page::Library);
            }
            _ => {}
        }
    }

    fn edit_body(&self, ui: &mut Ui, draft: &mut EditDraft) -> Outcome {
        let info = self.info(&draft.key);
        let current = self.apps.iter().find(|app| app.key == draft.key);
        let original = info
            .as_ref()
            .and_then(|info| info.all_time.as_ref())
            .map(|app| (app.name.clone(), app.category))
            .or_else(|| current.map(|app| (app.name.clone(), app.category)));
        if !draft.prefilled
            && let Some((name, category)) = &original
        {
            draft.name = name.clone();
            draft.category = *category;
            draft.prefilled = true;
        }
        description(
            ui,
            "Applies to all recorded history and to future sightings of this app.",
        );
        if let Some(info) = &info {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                label(ui, "Source");
                for key in &info.sources {
                    let source = format::source(key);
                    widgets::truncated(
                        ui,
                        widgets::mono(source.path.unwrap_or(source.line), Tokens::CAPTION),
                    );
                }
            });
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            label(ui, "Display name");
            let width = ui.available_width();
            widgets::text_field(
                ui,
                egui::Id::new("edit-name"),
                &mut draft.name,
                "App name",
                width,
            );
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            label(ui, "Category");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(Tokens::CONTROL_GAP, Tokens::CONTROL_GAP);
                let colors = Tokens::get(ui);
                for category in Category::ALL {
                    if widgets::radio_chip(
                        ui,
                        category.label(),
                        colors.category(category),
                        draft.category == category,
                    )
                    .clicked()
                    {
                        draft.category = category;
                    }
                }
            });
        });
        let automatic = info.as_ref().and_then(|info| info.automatic.clone());
        if let (Some((name, category)), Some(original)) = (&automatic, &original)
            && (name != &original.0 || *category != original.1)
            && self.recorder_can(tempora::model::CAPABILITY_RESET_APP)
        {
            let mut reset = false;
            ui.horizontal(|ui| {
                let colors = Tokens::get(ui);
                ui.label(widgets::text(
                    format!("Automatic: {name} · {}", category.label()),
                    Tokens::CAPTION,
                    colors.fg_subtle,
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    reset = ui
                        .add_enabled(
                            self.recorder_online() && !self.pending,
                            Button::new("Reset to automatic", ButtonKind::Ghost).small(),
                        )
                        .clicked();
                });
            });
            if reset {
                return Outcome::Commit(
                    ControlRequest::ResetApp {
                        key: draft.key.clone(),
                    },
                    "Reset to automatic",
                );
            }
        }
        let changed = original.as_ref().is_none_or(|(name, category)| {
            name.trim() != draft.name.trim() || *category != draft.category
        });
        let ready =
            self.recorder_online() && !self.pending && !draft.name.trim().is_empty() && changed;
        match widgets::dialog_footer(
            ui,
            Button::new("Save", ButtonKind::Primary),
            ready,
            "You can undo this.",
        ) {
            Some(DialogAction::Confirm) => Outcome::Commit(
                ControlRequest::EditApp {
                    key: draft.key.clone(),
                    name: draft.name.trim().to_owned(),
                    category: draft.category,
                },
                "App updated",
            ),
            Some(DialogAction::Cancel) => Outcome::Close,
            None => Outcome::Keep,
        }
    }

    fn group_body(&self, ui: &mut Ui, draft: &mut GroupDraft) -> Outcome {
        let colors = Tokens::get(ui);
        description(
            ui,
            "Count these as one app in every report. The original observations are kept.",
        );
        let entry = |ui: &mut Ui, key: &str, removable: bool| -> bool {
            let info = self.info(key);
            let app = self.apps.iter().find(|app| app.key == key);
            let name = info
                .as_ref()
                .and_then(|info| info.all_time.as_ref().map(|app| app.name.clone()))
                .or_else(|| app.map(|app| app.name.clone()))
                .unwrap_or_else(|| key.to_owned());
            let category = app.map_or(Category::Other, |app| app.category);
            let mut removed = false;
            ui.horizontal(|ui| {
                let width = ui.available_width();
                let detail = info.as_ref().and_then(|info| {
                    let focused = info.all_time.as_ref()?.totals.focused_ms;
                    Some(format!(
                        "{} · {}",
                        format::duration(focused),
                        span(info, self.zone).unwrap_or_default()
                    ))
                });
                let actions = if removable { 40.0 } else { 0.0 };
                ui.allocate_ui_with_layout(
                    vec2(width - actions, 40.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        widgets::app_cell_with(
                            ui,
                            Some(key),
                            &name,
                            Some(&format::source(key).line),
                            category,
                            28.0,
                            |ui| {
                                if let Some(detail) = &detail {
                                    ui.label(
                                        widgets::mono(detail, Tokens::CAPTION)
                                            .color(colors.fg_subtle),
                                    );
                                }
                            },
                        );
                    },
                );
                if removable {
                    removed = ui
                        .add(
                            Button::new(&format!("Remove {name}"), ButtonKind::GhostIcon)
                                .small()
                                .with_icon(icons::X),
                        )
                        .clicked();
                }
            });
            removed
        };
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            label(ui, "Entries");
            entry(ui, &draft.source, false);
            if let Some(target) = draft.target.clone()
                && entry(ui, &target, true)
            {
                draft.target = None;
            }
            if draft.target.is_none() {
                let width = ui.available_width() - 22.0;
                widgets::search_field(
                    ui,
                    egui::Id::new("group-search"),
                    &mut draft.search,
                    "Add another entry…",
                    width,
                );
                let query = draft.search.trim().to_lowercase();
                if !query.is_empty() {
                    let matches: Vec<_> = self
                        .apps
                        .iter()
                        .filter(|app| app.key != draft.source)
                        .filter(|app| {
                            app.name.to_lowercase().contains(&query)
                                || app.key.to_lowercase().contains(&query)
                        })
                        .take(6)
                        .collect();
                    if matches.is_empty() {
                        ui.label(widgets::text(
                            "No other entries match.",
                            Tokens::SECONDARY,
                            colors.fg_muted,
                        ));
                    }
                    for app in matches {
                        if widgets::list_item(ui, &app.name, Some(icons::PLUS), false).clicked() {
                            draft.target = Some(app.key.clone());
                            draft.search.clear();
                        }
                    }
                }
            }
        });
        if let Some(target) = &draft.target
            && let Some(app) = self.apps.iter().find(|app| app.key == *target)
        {
            preview_rows(
                ui,
                "After grouping",
                &[
                    ("Name", app.name.clone()),
                    ("Category", app.category.label().to_owned()),
                ],
            );
            ui.label(widgets::text(
                "The group uses this entry's name and category. Rename it afterwards with Edit app.",
                Tokens::CAPTION,
                colors.fg_subtle,
            ));
            let summaries = self
                .info(&draft.source)
                .zip(self.info(target))
                .and_then(|(source, target)| source.all_time.clone().zip(target.all_time.clone()));
            if let Some((source, target)) = summaries {
                let totals = group_totals(&source, &target);
                preview_rows(
                    ui,
                    "Totals after grouping",
                    &[
                        (
                            "Focused",
                            format!(
                                "{} → {}",
                                format::duration(totals.focused.0),
                                format::duration(totals.focused.1)
                            ),
                        ),
                        (
                            "Running",
                            format!(
                                "{} → {}",
                                format::duration(totals.running.0),
                                format::duration(totals.running.1)
                            ),
                        ),
                        (
                            "Sessions",
                            format!("{} → {}", totals.sessions.0, totals.sessions.1),
                        ),
                    ],
                );
                let overlap = totals.running.0 - totals.running.1;
                if overlap >= 60_000 {
                    ui.label(widgets::text(
                        format!(
                            "{} of overlapping running time is counted once.",
                            format::duration(overlap)
                        ),
                        Tokens::CAPTION,
                        colors.fg_subtle,
                    ));
                }
            }
        }
        let ready = self.recorder_online() && !self.pending && draft.target.is_some();
        match widgets::dialog_footer(
            ui,
            Button::new("Group 2 entries", ButtonKind::Primary),
            ready,
            "You can undo this.",
        ) {
            Some(DialogAction::Confirm) => match &draft.target {
                Some(target) => Outcome::Commit(
                    ControlRequest::GroupApps {
                        source: draft.source.clone(),
                        target: target.clone(),
                    },
                    "Entries grouped",
                ),
                None => Outcome::Keep,
            },
            Some(DialogAction::Cancel) => Outcome::Close,
            None => Outcome::Keep,
        }
    }

    fn delete_body(&self, ui: &mut Ui, draft: &mut DeleteDraft) -> Outcome {
        let colors = Tokens::get(ui);
        if draft.rechecked {
            widgets::banner(
                ui,
                widgets::BannerKind::Info,
                "Activity changed since the preview",
                "Review the updated numbers before deleting.",
                &[],
            );
        }
        let mut sessions = None;
        match &draft.preview {
            None => {
                ui.spinner();
            }
            Some(Err(error)) => {
                widgets::banner(
                    ui,
                    widgets::BannerKind::Error,
                    "Couldn't prepare the preview",
                    error,
                    &[],
                );
            }
            Some(Ok(preview)) => {
                sessions = Some(preview.sessions);
                let count = match preview.sessions {
                    1 => "1 session".to_owned(),
                    count => format!("{count} sessions"),
                };
                let dates = format::dates_span(
                    preview
                        .first_ms
                        .and_then(|ms| format::local_date(ms, self.zone)),
                    preview
                        .last_ms
                        .and_then(|ms| format::local_date(ms, self.zone)),
                );
                let span = dates
                    .map(|dates| format!(" from {dates}"))
                    .unwrap_or_default();
                let focused = format::duration(preview.focused_ms);
                let summary = match &draft.key {
                    Some(_) => format!(
                        "This permanently deletes {count} of {}{span} ({focused} of focused time). This can't be undone.",
                        preview.name
                    ),
                    None => format!(
                        "This permanently deletes all {count} recorded{span} ({focused} of focused time), for every app. This can't be undone."
                    ),
                };
                ui.add(egui::Label::new(widgets::text(summary, Tokens::BODY, colors.fg)).wrap());
                // Only a long history shows a pattern worth drawing.
                if let Some(days) = preview.days.as_ref().filter(|days| days.len() >= 21) {
                    let active = days.iter().filter(|(_, ms)| *ms > 0).count();
                    widgets::heat_strip(
                        ui,
                        days,
                        &format!("Activity on {active} of {} days", days.len()),
                    );
                }
                if draft.key.is_some() {
                    ui.label(widgets::text(
                        format!(
                            "Want to keep the history? Stop tracking {} instead to stop recording it.",
                            preview.name
                        ),
                        Tokens::CAPTION,
                        colors.fg_subtle,
                    ));
                }
            }
        }
        let confirm_label = match sessions {
            Some(1) => "Delete 1 session".to_owned(),
            Some(count) if count > 0 => format!("Delete {count} sessions"),
            _ => "Delete history".to_owned(),
        };
        let ready = self.recorder_online() && !self.pending && matches!(draft.preview, Some(Ok(_)));
        match widgets::dialog_footer(
            ui,
            Button::new(&confirm_label, ButtonKind::Danger),
            ready,
            "",
        ) {
            Some(DialogAction::Confirm) if draft.version != self.data_version => Outcome::Recheck,
            Some(DialogAction::Confirm) => Outcome::Commit(
                ControlRequest::DeleteHistory {
                    key: draft.key.clone(),
                },
                "History deleted",
            ),
            Some(DialogAction::Cancel) => Outcome::Close,
            None => Outcome::Keep,
        }
    }
}

/// Before → after totals for grouping two entries: focus is exclusive so it adds up;
/// running time and sessions are the union of both entries' sessions.
pub(super) struct GroupTotals {
    pub focused: (i64, i64),
    pub running: (i64, i64),
    pub sessions: (usize, usize),
}

pub(super) fn group_totals(
    source: &tempora::storage::AppSummary,
    target: &tempora::storage::AppSummary,
) -> GroupTotals {
    let mut spans: Vec<(i64, i64)> = source
        .sessions
        .iter()
        .chain(&target.sessions)
        .map(|session| (session.start_ms, session.end_ms))
        .collect();
    spans.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    GroupTotals {
        focused: (
            source.totals.focused_ms + target.totals.focused_ms,
            source.totals.focused_ms + target.totals.focused_ms,
        ),
        running: (
            source.totals.running_ms + target.totals.running_ms,
            merged.iter().map(|(start, end)| end - start).sum(),
        ),
        sessions: (source.sessions.len() + target.sessions.len(), merged.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempora::storage::{AppSummary, Session, Totals};

    fn summary(sessions: &[(i64, i64)], focused: i64) -> AppSummary {
        AppSummary {
            key: String::new(),
            name: String::new(),
            category: Category::Games,
            totals: Totals {
                focused_ms: focused,
                running_ms: sessions.iter().map(|(start, end)| end - start).sum(),
                ..Totals::default()
            },
            last_seen_ms: 0,
            days: Vec::new(),
            sessions: sessions
                .iter()
                .map(|(start_ms, end_ms)| Session {
                    start_ms: *start_ms,
                    end_ms: *end_ms,
                    totals: Totals::default(),
                })
                .collect(),
        }
    }

    #[test]
    fn grouping_counts_overlapping_running_time_once() {
        let totals = group_totals(
            &summary(&[(0, 60), (100, 160)], 50),
            &summary(&[(40, 90)], 30),
        );
        assert_eq!(totals.focused, (80, 80));
        assert_eq!(totals.running, (170, 150));
        assert_eq!(totals.sessions, (3, 2));
    }
}
