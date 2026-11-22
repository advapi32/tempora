use chrono::Weekday;
use eframe::egui::{Context, ThemePreference};
use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "dashboard_preferences";

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub appearance: ThemePreference,
    pub week_start: Weekday,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: ThemePreference::System,
            week_start: Weekday::Mon,
        }
    }
}

impl Preferences {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_default()
    }

    pub fn apply(&self, ctx: &Context) {
        ctx.set_theme(self.appearance);
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, self);
    }
}
