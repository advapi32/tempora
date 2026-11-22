//! First run (`FirstRun` mockup): how it works, what to record, start at sign-in.
//! Everything stays in this draft until Finish sends one `SetSettings`.

use std::collections::BTreeSet;

use eframe::egui::{Align, Layout, RichText, Ui};
use egui_phosphor::regular as icons;
use tempora::model::{AppIdentity, CaptureMode, ControlRequest, Settings};

use crate::dashboard::theme::{self, Tokens};
use crate::dashboard::widgets::{self, Button, ButtonKind, DialogAction, StepState};
use crate::dashboard::{APP_NAME, Dashboard, format};

const STEPS: [&str; 3] = ["How it works", "What to record", "Start at sign-in"];

pub(in crate::dashboard) struct FirstRun {
    pub step: usize,
    pub mode: CaptureMode,
    /// All mode: unticked apps become exclusions.
    pub unticked: BTreeSet<String>,
    /// Selected mode: only ticked apps are recorded.
    pub ticked: BTreeSet<String>,
    /// `None` until the user touches it; then overrides the stored preference.
    pub sign_in: Option<bool>,
}

impl Default for FirstRun {
    fn default() -> Self {
        Self {
            step: 0,
            mode: CaptureMode::All,
            unticked: BTreeSet::new(),
            ticked: BTreeSet::new(),
            sign_in: None,
        }
    }
}

impl FirstRun {
    pub fn is_on(&self, key: &str) -> bool {
        match self.mode {
            CaptureMode::All => !self.unticked.contains(key),
            CaptureMode::Selected => self.ticked.contains(key),
        }
    }

    fn set(&mut self, key: &str, on: bool) {
        let set = match self.mode {
            CaptureMode::All => &mut self.unticked,
            CaptureMode::Selected => &mut self.ticked,
        };
        if on == (self.mode == CaptureMode::Selected) {
            set.insert(key.to_owned());
        } else {
            set.remove(key);
        }
    }

    fn ready(&self) -> bool {
        self.mode == CaptureMode::All || !self.ticked.is_empty()
    }

    /// The single settings write sent on Finish.
    pub fn settings(&self, base: &Settings, candidates: &[AppIdentity]) -> Settings {
        let mut settings = base.clone();
        settings.enabled = true;
        settings.paused = false;
        settings.mode = self.mode;
        settings.launch_at_sign_in = self.sign_in.unwrap_or(base.launch_at_sign_in);
        let known = |key: &String| candidates.iter().any(|app| app.key == *key);
        match self.mode {
            CaptureMode::All => settings
                .excluded
                .extend(self.unticked.iter().filter(|key| known(key)).cloned()),
            CaptureMode::Selected => {
                settings.selected = self
                    .ticked
                    .iter()
                    .filter(|key| known(key))
                    .cloned()
                    .collect();
            }
        }
        settings
    }
}

impl Dashboard {
    pub(in crate::dashboard) fn first_run_page(&mut self, ui: &mut Ui, _wide: bool) {
        let step = self.first_run.step;
        // The card is only as tall as its content; step 2's list sizes itself to the window.
        widgets::split_card(
            ui,
            300.0,
            |ui, fill| first_run_side(ui, step, fill),
            |ui| self.first_run_main(ui),
        );
    }

    fn first_run_main(&mut self, ui: &mut Ui) {
        let colors = Tokens::get(ui);
        let step = self.first_run.step;
        ui.spacing_mut().item_spacing.y = 22.0;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(widgets::mono(format!("Step {} of 3", step + 1), Tokens::CAPTION).color(colors.fg_subtle));
            ui.label(theme::display(
                match step {
                    0 => format!("Welcome to {APP_NAME}"),
                    1 => format!("Choose what {APP_NAME} records"),
                    _ => "Start recording at sign-in?".to_owned(),
                },
                24.0,
            ));
            ui.label(widgets::text(
                match step {
                    0 => "See how much you play, and what else is open while you do.",
                    1 => "Nothing is recorded until you finish setup. You can change this later in Settings.",
                    _ => "The recorder runs in the tray. You can change this later in Settings.",
                },
                Tokens::BODY,
                colors.fg_muted,
            ));
        });
        match step {
            0 => how_it_works(ui),
            1 => self.what_to_record(ui),
            _ => self.sign_in_step(ui),
        }
        let footnote = match (step, self.first_run.mode) {
            (1, CaptureMode::All) => "Unticked apps won't be recorded.",
            (1, CaptureMode::Selected) => "Only ticked apps will be recorded.",
            _ => "",
        };
        let last = step == STEPS.len() - 1;
        let online = self.recorder_online() && !self.pending;
        let ready = self.first_run.ready() && online;
        // Directly under the content, 24 px below it (22 from the item spacing plus 2).
        ui.add_space(2.0);
        let confirm = Button::new(
            if last { "Finish" } else { "Continue" },
            ButtonKind::Primary,
        );
        let back_label = if step == 0 { "" } else { "Back" };
        ui.horizontal(|ui| {
            match widgets::wizard_footer(ui, back_label, confirm, ready || step == 0, footnote) {
                Some(DialogAction::Cancel) => self.first_run.step = step.saturating_sub(1),
                Some(DialogAction::Confirm) if last => self.finish_first_run(),
                Some(DialogAction::Confirm) => self.first_run.step += 1,
                None => {}
            }
        });
    }

    fn what_to_record(&mut self, ui: &mut Ui) {
        let colors = Tokens::get(ui);
        let width = ui.available_width();
        let card_width = (width - 12.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (mode, title, detail) in [
                (
                    CaptureMode::All,
                    "All apps",
                    "Untick anything you'd rather not track.",
                ),
                (
                    CaptureMode::Selected,
                    "Only apps I pick",
                    "Tick the apps you want tracked.",
                ),
            ] {
                if widgets::radio_card(ui, self.first_run.mode == mode, title, detail, card_width)
                    .clicked()
                {
                    self.first_run.mode = mode;
                }
            }
        });
        let candidates: Vec<AppIdentity> = self
            .recorder
            .iter()
            .flat_map(|status| status.candidates.clone())
            .collect();
        let on = candidates
            .iter()
            .filter(|app| self.first_run.is_on(&app.key))
            .count();
        let summary = match self.first_run.mode {
            CaptureMode::All => format!(
                "{} excluded · new apps recorded automatically",
                candidates.len() - on
            ),
            CaptureMode::Selected => format!("{on} selected · new apps ignored"),
        };
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Detected on this PC").font(Tokens::medium(Tokens::SECONDARY)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(widgets::mono(summary, Tokens::CAPTION).color(colors.fg_subtle));
                });
            });
            let height = (ui.available_height() - 80.0).clamp(160.0, 360.0);
            widgets::bordered_list(ui, height, |ui| {
                if candidates.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(widgets::text(
                            "No open apps found. Apps you open later appear in Settings.",
                            Tokens::SECONDARY,
                            colors.fg_muted,
                        ));
                    });
                }
                for app in &candidates {
                    let mut checked = self.first_run.is_on(&app.key);
                    let sub = format::source(&app.key).line;
                    if widgets::app_check_row(
                        ui,
                        &mut checked,
                        &app.key,
                        &app.name,
                        &sub,
                        app.category,
                    )
                    .changed()
                    {
                        self.first_run.set(&app.key, checked);
                    }
                }
            });
        });
    }

    fn sign_in_step(&mut self, ui: &mut Ui) {
        let stored = self.settings.launch_at_sign_in;
        let mut sign_in = self.first_run.sign_in.unwrap_or(stored);
        if widgets::checkbox(
            ui,
            &mut sign_in,
            "Start the recorder when I sign in",
            Some("Takes effect at your next sign-in."),
        )
        .changed()
        {
            self.first_run.sign_in = Some(sign_in);
        }
    }

    fn finish_first_run(&mut self) {
        let candidates = self
            .recorder
            .as_ref()
            .map(|status| status.candidates.clone())
            .unwrap_or_default();
        let settings = self.first_run.settings(&self.settings, &candidates);
        self.action(ControlRequest::SetSettings(settings), "Recording started");
    }
}

/// The side pane: the steps, then the privacy points at the bottom of the pane. `fill` is the
/// height the main pane used last frame; the privacy points end there, or directly after the
/// steps when the steps are taller.
fn first_run_side(ui: &mut Ui, step: usize, fill: f32) {
    ui.spacing_mut().item_spacing.y = 4.0;
    for (index, label) in STEPS.iter().enumerate() {
        let state = match index.cmp(&step) {
            std::cmp::Ordering::Less => StepState::Done,
            std::cmp::Ordering::Equal => StepState::Current,
            std::cmp::Ordering::Greater => StepState::Upcoming,
        };
        widgets::step_item(ui, index + 1, label, state);
    }
    let id = ui.id().with("privacy-height");
    let measured = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(120.0);
    let target_bottom = ui.max_rect().top() + fill;
    let gap = (target_bottom - measured - ui.min_rect().bottom()).max(32.0);
    ui.add_space(gap);
    let block = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 14.0;
        for (icon, text) in [
            (icons::LOCK_SIMPLE, "Your history stays on this PC."),
            (icons::EYE_SLASH, "Only app names and times are recorded."),
            (icons::HAND, "Pause or quit from the tray icon."),
        ] {
            widgets::icon_line(ui, icon, text);
        }
    });
    let height = block.response.rect.height();
    if (height - measured).abs() > 0.5 {
        ui.data_mut(|data| data.insert_temp(id, height));
        ui.ctx().request_repaint();
    }
}

fn how_it_works(ui: &mut Ui) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 14.0;
        for (icon, text) in [
            (
                icons::APP_WINDOW,
                "Time is counted per game and app, both while you use it and while it sits open in the background.",
            ),
            (
                icons::MOON,
                "Time away from your PC can be left out of the totals.",
            ),
            (
                icons::CHART_BAR,
                "Wrong name or category? Fix it once and your past days update too.",
            ),
        ] {
            widgets::icon_line(ui, icon, text);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempora::model::Category;

    fn app(key: &str) -> AppIdentity {
        AppIdentity {
            key: key.into(),
            name: key.into(),
            category: Category::Other,
        }
    }

    #[test]
    fn unticked_apps_become_exclusions_and_ticked_apps_the_allowlist() {
        let candidates = [app("a"), app("b"), app("c")];
        let mut draft = FirstRun::default();
        draft.set("b", false);
        let settings = draft.settings(&Settings::default(), &candidates);
        assert!(settings.enabled && !settings.paused);
        assert_eq!(settings.excluded.iter().collect::<Vec<_>>(), ["b"]);

        draft.mode = CaptureMode::Selected;
        assert!(!draft.ready());
        draft.set("c", true);
        assert!(draft.is_on("c") && !draft.is_on("a"));
        let settings = draft.settings(&Settings::default(), &candidates);
        assert_eq!(settings.mode, CaptureMode::Selected);
        assert_eq!(settings.selected.iter().collect::<Vec<_>>(), ["c"]);
        assert_eq!(
            settings.launch_at_sign_in,
            Settings::default().launch_at_sign_in
        );
    }
}
