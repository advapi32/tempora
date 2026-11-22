mod app_detail;
mod day;
mod first_run;
mod library;
mod overview;
mod settings;

pub(super) use first_run::FirstRun;
pub(super) use overview::overview_subtitle;
pub(super) use settings::SettingsState;

use chrono::{Local, NaiveDate};
use eframe::egui::{self, Align, Ui};
use egui_phosphor::regular as icons;
use tempora::model::DateRange;

use super::nav::{Page, Period, Preset, period_range};
use super::widgets::{self, Button, Calendar, PickerAction, PickerInput};
use super::{Dashboard, format};

/// The one open popover. Each is drawn right after its anchor.
pub(super) enum Popup {
    Range(Calendar),
    Search(String),
    Categories,
    /// The app menu for a key, anchored at a button or at the pointer (zero-size rect).
    AppMenu(String, egui::Rect),
    More,
    AddApp(egui::Rect),
}

/// A zero-size anchor at the pointer, for the app menu opened by a right-click.
pub(super) fn pointer_anchor(pos: egui::Pos2) -> egui::Rect {
    egui::Rect::from_min_size(pos, egui::Vec2::ZERO)
}

/// Height a list may take so its card ends at the bottom of the window, or `None` when
/// even a short list wouldn't fit (then the page scrolls and the list doesn't).
pub(super) fn fill_height(viewport_bottom: f32, ui: &Ui, after: f32) -> Option<f32> {
    const MIN_LIST: f32 = 120.0;
    // Card border, the page's bottom margin, rounding slack. `after` adds the card's
    // bottom padding and anything else below the list.
    let below = 1.0 + 24.0 + 2.0;
    let height = viewport_bottom - ui.cursor().top() - below - after;
    (height >= MIN_LIST).then_some(height)
}

/// A list that fills the window and scrolls inside its card, so there is never a page
/// scroll bar and a list scroll bar at the same time.
pub(super) fn fitted_list<R>(
    viewport_bottom: f32,
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    after: f32,
    content: impl FnOnce(&mut Ui) -> R,
) -> R {
    match fill_height(viewport_bottom, ui, after) {
        Some(height) => {
            let area = egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, true]);
            widgets::scroll(ui, area, content).inner
        }
        None => content(ui),
    }
}

impl Dashboard {
    pub(super) fn content(&mut self, ui: &mut Ui, wide: bool) {
        match self.nav.page().clone() {
            Page::Overview => self.overview(ui, wide),
            Page::Library => self.library(ui),
            Page::App(key) => self.app_detail(ui, &key, wide),
            Page::Day(date) => self.day_timeline(ui, date, wide),
            Page::Settings => self.settings_page(ui, wide),
        }
    }

    pub(super) fn today(&self) -> NaiveDate {
        Local::now().date_naive()
    }

    /// Opens the app menu for `key` at `anchor`, or closes it when it is already open for `key`.
    pub(super) fn toggle_app_menu(&mut self, key: String, anchor: egui::Rect) {
        self.popup = match &self.popup {
            Some(Popup::AppMenu(open, _)) if *open == key => None,
            _ => Some(Popup::AppMenu(key, anchor)),
        };
    }

    /// Edit app (F2), Group with…, Open sessions | Stop tracking, Delete history…. Drawn once
    /// per frame by the dashboard, so every page can open it from a button or the pointer.
    pub(super) fn app_menu(&mut self, ctx: &egui::Context) {
        let Some(Popup::AppMenu(key, anchor)) = &self.popup else {
            return;
        };
        let (key, anchor) = (key.clone(), *anchor);
        // Buttons anchor the menu's right edge; a pointer anchor (zero width) opens it down and right.
        let align = if anchor.width() > 0.0 {
            Align::Max
        } else {
            Align::Min
        };
        let online = self.recorder_online() && !self.pending;
        let output = widgets::anchored_menu(ctx, egui::Id::new("app-menu"), anchor, align, |ui| {
            ui.set_width(232.0);
            let mut picked = None;
            ui.add_enabled_ui(online, |ui| {
                if widgets::menu_item_with_key(ui, "Edit app…", icons::PENCIL_SIMPLE, "F2")
                    .clicked()
                {
                    picked = Some(0);
                }
                if widgets::menu_item(ui, "Group with…", icons::STACK, false).clicked() {
                    picked = Some(1);
                }
            });
            if widgets::menu_item(ui, "Open sessions", icons::ROWS, false).clicked() {
                picked = Some(2);
            }
            ui.separator();
            ui.add_enabled_ui(online, |ui| {
                if widgets::menu_item(ui, "Stop tracking this app", icons::PROHIBIT, false)
                    .on_hover_text("Keeps the existing history")
                    .clicked()
                {
                    picked = Some(3);
                }
                if widgets::menu_item(ui, "Delete history…", icons::TRASH, true).clicked() {
                    picked = Some(4);
                }
            });
            picked
        });
        match output.inner {
            Some(0) => self.open_edit(&key),
            Some(1) => self.open_group(&key),
            Some(2) => {
                self.popup = None;
                self.nav.drill(Page::App(key));
            }
            Some(3) => self.stop_tracking(&key),
            Some(_) => self.open_delete(Some(&key)),
            None if output.dismissed => self.popup = None,
            None => {}
        }
    }

    /// Fades what a page draws next while it is still the previous range's or day's. A quick
    /// load swaps the content before anything changes, so there is no flash; a slow one fades
    /// the old numbers back so they don't pass for the new ones. Call it every frame with
    /// whether the content is stale; `id` tells the page's faded areas apart.
    pub(super) fn dim_while_stale(ui: &mut Ui, id: &str, stale: bool) {
        const WAIT: f64 = 0.15;
        const FADE: f64 = 0.2;
        const STALE_OPACITY: f32 = 0.45;
        if !stale {
            return;
        }
        let ctx = ui.ctx().clone();
        let key = egui::Id::new(("stale-since", id));
        let now = ctx.input(|input| input.time);
        let frame = ctx.cumulative_frame_nr();
        let since = ctx
            .data(|data| data.get_temp::<(f64, u64)>(key))
            .filter(|(_, seen)| seen + 1 >= frame)
            .map_or(now, |(since, _)| since);
        ctx.data_mut(|data| data.insert_temp(key, (since, frame)));
        let waited = now - since;
        if waited < WAIT {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(WAIT - waited));
            return;
        }
        let faded = if widgets::motion::enabled(&ctx) {
            ((waited - WAIT) / FADE).min(1.0) as f32
        } else {
            1.0
        };
        if faded < 1.0 {
            ctx.request_repaint();
        }
        ui.multiply_opacity(widgets::motion::lerp(1.0, STALE_OPACITY, faded));
    }

    /// Loading and error states shared by report-backed pages.
    pub(super) fn report_status(&self, ui: &mut Ui) {
        let key = (self.nav.view.range, self.nav.view.exclude_idle);
        if let Some(error) = self.report.error(&key) {
            widgets::banner(
                ui,
                widgets::BannerKind::Error,
                "History unavailable",
                error,
                &[],
            );
        } else if self.report.loading(&key) && self.report.shown(&key).is_none() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.spinner());
        }
    }

    /// Prev / range / next and, on Overview, the period control.
    pub(super) fn date_controls(&mut self, ui: &mut Ui, with_period: bool) {
        let today = self.today();
        let week_start = self.prefs.week_start;
        let previous = self.nav.adjacent(false, week_start, today);
        let next = self.nav.adjacent(true, week_start, today);
        let previous_button = Button::icon("Previous period", icons::CARET_LEFT);
        if ui
            .add_enabled(previous.is_some(), previous_button)
            .clicked()
            && let Some(range) = previous
        {
            let period = self.nav.view.period;
            self.nav.set_range(period, range);
        }
        self.range_control(ui);
        let next_button = Button::icon("Next period", icons::CARET_RIGHT);
        if ui.add_enabled(next.is_some(), next_button).clicked()
            && let Some(range) = next
        {
            let period = self.nav.view.period;
            self.nav.set_range(period, range);
        }
        if with_period {
            ui.add_space(4.0);
            let mut period = self.nav.view.period;
            let changed = widgets::segmented(
                ui,
                &mut period,
                &[
                    (Period::Week, "Week"),
                    (Period::Month, "Month"),
                    (Period::Custom, "Custom"),
                ],
            );
            if changed {
                let anchor = self.nav.view.range.end.min(today);
                match period {
                    Period::Custom => {
                        let range = self.nav.view.range;
                        self.popup = Some(Popup::Range(Calendar::new(range.start, range.end)));
                    }
                    _ => self
                        .nav
                        .set_range(period, period_range(period, anchor, week_start)),
                }
            }
        }
    }

    /// The range button and its presets + calendar popover.
    pub(super) fn range_control(&mut self, ui: &mut Ui) {
        self.range_control_styled(ui, false);
    }

    /// The range picker; `grouped` draws its button as a segment of a `button_group`.
    pub(super) fn range_control_styled(&mut self, ui: &mut Ui, grouped: bool) {
        let label = format::range_label(self.nav.view.range, self.nav.view.period);
        let button = widgets::range_button(ui, &label, grouped);
        if button.clicked() {
            self.popup = match self.popup {
                Some(Popup::Range(_)) => None,
                _ => {
                    let range = self.nav.view.range;
                    Some(Popup::Range(Calendar::new(range.start, range.end)))
                }
            };
        }
        if let Some(Popup::Range(calendar)) = &self.popup {
            let month = calendar.month;
            self.want_covered(month);
        }
        let history = self
            .extras()
            .and_then(|extras| extras.bounds)
            .and_then(|(first, last)| {
                format::dates_span(
                    format::local_date(first, self.zone),
                    format::local_date(last, self.zone),
                )
            })
            .map(|span| format!("History available {span}"));
        let covered_days = match &self.popup {
            Some(Popup::Range(calendar)) => self.covered.get(&calendar.month).cloned(),
            _ => None,
        };
        let Some(Popup::Range(calendar)) = &mut self.popup else {
            return;
        };
        let covered_check = covered_days.map(|days| move |date: NaiveDate| days.contains(&date));
        let today = Local::now().date_naive();
        let week_start = self.prefs.week_start;
        let current = self.nav.view.range;
        let resolved: Vec<_> = Preset::ALL
            .iter()
            .map(|preset| preset.resolve(today, week_start))
            .collect();
        let presets: Vec<_> = Preset::ALL
            .iter()
            .zip(&resolved)
            .map(|(preset, (_, range))| (preset.label(), *range == current))
            .collect();
        let output = widgets::anchored_popover(
            ui.ctx(),
            egui::Id::new("range-picker"),
            button.rect,
            Align::Min,
            |ui| {
                widgets::range_picker(
                    ui,
                    calendar,
                    &PickerInput {
                        presets: &presets,
                        history: history.as_deref(),
                        today,
                        week_start,
                        covered: covered_check
                            .as_ref()
                            .map(|check| check as &dyn Fn(NaiveDate) -> bool),
                    },
                )
            },
        );
        let close = match output.inner {
            Some(PickerAction::Preset(index)) => {
                let (period, range) = resolved[index];
                self.nav.set_range(period, range);
                true
            }
            Some(PickerAction::Apply) => {
                let (start, end) = calendar.selection();
                let range = DateRange { start, end };
                let period = [Period::Week, Period::Month]
                    .into_iter()
                    .find(|period| period_range(*period, start, week_start) == range)
                    .unwrap_or(Period::Custom);
                self.nav.set_range(period, range);
                true
            }
            Some(PickerAction::Cancel) => true,
            None => output.dismissed,
        };
        if close {
            self.popup = None;
            button.request_focus();
        }
    }
}

#[cfg(test)]
mod render_tests {
    use std::sync::Arc;

    use eframe::egui::{self, Context, FullOutput, Rect, Shape, Theme, pos2, vec2};
    use tempora::{
        model::{AppIdentity, Category, RecorderStatus, Settings},
        storage::{AppSummary, CategorySummary, DaySummary, Report, Totals},
    };

    use super::Popup;
    use crate::dashboard::Dashboard;
    use crate::dashboard::nav::Page;

    const HOUR: i64 = 3_600_000;

    fn frame(ctx: &Context, dashboard: &mut Dashboard, size: [f32; 2]) -> FullOutput {
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(size[0], size[1]))),
            ..egui::RawInput::default()
        };
        for _ in 0..2 {
            ctx.run_ui(input(), |ui| dashboard.draw_shell(ui))
                .textures_delta
                .clear();
        }
        let mut output = ctx.run_ui(input(), |ui| dashboard.draw_shell(ui));
        output.textures_delta.clear();
        output
    }

    fn texts(output: &FullOutput) -> Vec<String> {
        fn collect(shape: &Shape, out: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => out.push(text.galley.text().to_owned()),
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut out);
        }
        out
    }

    fn text_rects(output: &FullOutput, label: &str) -> Vec<Rect> {
        fn collect(shape: &Shape, label: &str, out: &mut Vec<Rect>) {
            match shape {
                Shape::Text(text) if text.galley.text() == label => {
                    out.push(text.visual_bounding_rect());
                }
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, label, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, label, &mut out);
        }
        out
    }

    /// Heights of the card frames whose top edge lines up with the "Focused time" label row.
    fn card_heights(output: &FullOutput, dashboard: &Dashboard) -> Vec<f32> {
        let _ = dashboard;
        let label_top = text_rects(output, "Focused time")[0].top();
        let mut heights = Vec::new();
        fn collect(shape: &Shape, top: f32, out: &mut Vec<f32>) {
            match shape {
                Shape::Rect(rect)
                    if rect.corner_radius.nw == crate::dashboard::theme::Tokens::CARD_RADIUS
                        && (rect.rect.top() - top).abs() < 24.0
                        && rect.rect.top() < top =>
                {
                    out.push(rect.rect.height());
                }
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, top, out)),
                _ => {}
            }
        }
        for clipped in &output.shapes {
            collect(&clipped.shape, label_top, &mut heights);
        }
        heights
    }

    fn has(output: &FullOutput, needle: &str) -> bool {
        texts(output).iter().any(|text| text.contains(needle))
    }

    fn status(enabled: bool) -> RecorderStatus {
        RecorderStatus {
            settings: Settings {
                enabled,
                ..Settings::default()
            },
            candidates: vec![AppIdentity {
                key: r"exe:c:\apps\discord.exe".into(),
                name: "Discord".into(),
                category: Category::Communication,
            }],
            problem: None,
            recording: enabled,
            capabilities: Vec::new(),
        }
    }

    fn report(dashboard: &Dashboard) -> Report {
        let range = dashboard.nav.view.range;
        let totals = |focused| Totals {
            focused_ms: focused,
            running_ms: focused * 2,
            ..Totals::default()
        };
        let app = |key: &str, name: &str, category, focused| AppSummary {
            key: key.into(),
            name: name.into(),
            category,
            totals: totals(focused),
            last_seen_ms: 0,
            days: Vec::new(),
            sessions: Vec::new(),
        };
        Report {
            apps: vec![
                app(
                    "steam:1086940",
                    "A game with a deliberately very long name that must truncate",
                    Category::Games,
                    3 * HOUR,
                ),
                app(r"exe:c:\firefox.exe", "Firefox", Category::Browsers, HOUR),
            ],
            categories: vec![
                CategorySummary {
                    category: Category::Games,
                    totals: totals(3 * HOUR),
                },
                CategorySummary {
                    category: Category::Browsers,
                    totals: totals(HOUR),
                },
            ],
            days: (0..7)
                .map(|offset| DaySummary {
                    date: range.start + chrono::Days::new(offset),
                    categories: vec![CategorySummary {
                        category: Category::Games,
                        totals: totals(HOUR / 2),
                    }],
                    covered_ms: 1,
                })
                .collect(),
            timeline: Vec::new(),
            coverage: Some((0, 1)),
        }
    }

    #[test]
    fn first_run_shows_each_step_without_writing() {
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(false));
        for (step, expected) in [
            (0, "Welcome to Tempora"),
            (1, "Detected on this PC"),
            (2, "Start the recorder when I sign in"),
        ] {
            dashboard.first_run.step = step;
            let output = frame(&ctx, &mut dashboard, [1100.0, 720.0]);
            assert!(has(&output, expected), "step {step}: {:?}", texts(&output));
            assert!(has(&output, "Your history stays on this PC."));
            if step == 1 {
                assert!(has(&output, "Discord") && has(&output, "discord.exe"));
            }
        }
        assert!(!dashboard.pending, "nothing is sent before Finish");
    }

    #[test]
    fn overview_and_library_render_in_both_themes_and_layouts() {
        for theme in [Theme::Light, Theme::Dark] {
            for size in [[1100.0, 1600.0], [1440.0, 1600.0]] {
                let ctx = Context::default();
                let mut dashboard = Dashboard::for_test(&ctx);
                ctx.set_theme(theme);
                dashboard.recorder = Some(status(true));
                let key = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
                let report = Arc::new(report(&dashboard));
                dashboard.report.finish(key, Ok(report));
                let output = frame(&ctx, &mut dashboard, size);
                for label in [
                    "Focused time by day",
                    "Top apps",
                    "Daily average",
                    "Firefox",
                    "75% of focused time",
                ] {
                    assert!(has(&output, label), "{label} in {theme:?} {size:?}");
                }
                let x = |label: &str| text_rects(&output, label)[0].left();
                assert!(
                    x("Chart") < x("Table"),
                    "segmented options read left to right"
                );
                let chart = text_rects(&output, "Chart")[0];
                let table = text_rects(&output, "Table")[0];
                assert!(
                    table.left() - chart.right() < 40.0,
                    "segmented stays compact"
                );
                let kpi_cards = card_heights(&output, &dashboard);
                let per_row = if size[0] - 64.0 >= 1280.0 { 4 } else { 2 };
                assert_eq!(kpi_cards.len(), per_row, "first KPI row: {kpi_cards:?}");
                assert!(
                    kpi_cards
                        .windows(2)
                        .all(|pair| (pair[0] - pair[1]).abs() < 0.5),
                    "equal KPI heights: {kpi_cards:?}"
                );
                dashboard.nav.section(Page::Library);
                let output = frame(&ctx, &mut dashboard, size);
                assert!(
                    has(&output, "Running time unions"),
                    "library in {theme:?} {size:?}"
                );
                assert!(
                    !has(&output, "Steam · App 1086940"),
                    "library rows show one line; the source is in the tooltip"
                );
                assert!(!has(&output, "Needs review"), "the review tab is gone");
                assert!(!has(&output, "Duplicate?"), "no duplicate tag");
                let origin = dashboard.nav.view.clone();
                dashboard.nav.drill(Page::App("steam:1086940".into()));
                let detail = frame(&ctx, &mut dashboard, size);
                for label in [
                    "Daily trend",
                    "Sessions",
                    "In the foreground",
                    "In front",
                    "Recognized from Steam library",
                    "Delete history…",
                ] {
                    assert!(
                        has(&detail, label),
                        "detail {label} in {theme:?} {size:?}: {:?}",
                        texts(&detail)
                    );
                }
                assert!(dashboard.nav.go_back());
                assert_eq!(dashboard.nav.view, origin, "Back restores the Library view");
                let name = text_rects(&output, "Firefox")[0];
                assert!(
                    text_rects(&output, "firefox.exe").is_empty(),
                    "a unique name gets one line and no source line"
                );
                let tile = text_rects(&output, "F")
                    .into_iter()
                    .filter(|letter| letter.right() < name.left())
                    .min_by(|a, b| (name.left() - a.right()).total_cmp(&(name.left() - b.right())))
                    .expect("Firefox tile letter");
                assert!(
                    (name.center().y - tile.center().y).abs() < 4.0,
                    "single name line centered on the tile: {:?} vs {:?}",
                    name.center(),
                    tile.center()
                );
            }
        }
    }

    #[test]
    fn month_chart_shows_the_daily_total_in_each_slice_tooltip() {
        use crate::dashboard::widgets::{self, Segment, StackedBar};

        let ctx = Context::default();
        let _dashboard = Dashboard::for_test(&ctx);
        let bars: Vec<_> = (0..30)
            .map(|index| StackedBar {
                label: format!("day-{index}"),
                description: "2h 07m focused. Open timeline.".into(),
                total: Some("2h 07m".into()),
                segments: vec![
                    Segment {
                        color: egui::Color32::RED,
                        ms: 2 * HOUR,
                        dimmed: false,
                        name: "Games",
                    },
                    Segment {
                        color: egui::Color32::BLUE,
                        ms: 7 * 60_000,
                        dimmed: false,
                        name: "Browsers",
                    },
                ],
                today: false,
                future: false,
                slice_detail: Some("Sunday, Nov 1".into()),
            })
            .collect();
        let chart_frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    widgets::stacked_bars(ui, &bars, false, 160.0);
                },
            );
            output.textures_delta.clear();
            output
        };
        let output = chart_frame(vec![]);
        let label = text_rects(&output, "day-0")[0];
        let pointer = pos2(label.center().x, label.top() - 40.0);
        chart_frame(vec![egui::Event::PointerMoved(pointer)]);
        let output = chart_frame(vec![]);
        for label in ["Games", "2h", "Sunday, Nov 1 · 2h 07m total"] {
            assert!(!text_rects(&output, label).is_empty(), "{label}");
        }
    }

    #[test]
    fn grouped_app_shows_its_running_source_and_blocks_corrections_while_busy() {
        use crate::dashboard::AppInfo;
        use tempora::model::{CAPABILITY_RESET_APP, CAPABILITY_UNGROUP_APP, ControlRequest};

        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        let key = "steam:1086940".to_owned();
        let source = r"exe:c:\games\a-copy.exe".to_owned();
        dashboard.recorder = Some(RecorderStatus {
            candidates: vec![AppIdentity {
                key: source.clone(),
                name: "Running copy".into(),
                category: Category::Games,
            }],
            capabilities: vec![CAPABILITY_UNGROUP_APP.into(), CAPABILITY_RESET_APP.into()],
            ..status(true)
        });
        let (send, jobs) = std::sync::mpsc::channel();
        dashboard.jobs = send;
        let report = report(&dashboard);
        let app = report.apps[0].clone();
        let key_report = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
        dashboard.report.finish(key_report, Ok(Arc::new(report)));
        dashboard.infos.entry(key.clone()).or_default().finish(
            key.clone(),
            Ok(Arc::new(AppInfo {
                all_time: Some(app),
                sources: vec![key.clone(), source.clone()],
                automatic: Some(("Original game".into(), Category::Games)),
            })),
        );
        dashboard.nav.drill(Page::App(key.clone()));
        dashboard.pending = true;
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        assert!(has(&output, "Includes 2 sources"), "{:?}", texts(&output));
        assert!(has(&output, "Running now"), "the grouped copy is running");
        let remove = text_rects(&output, "Remove from group");
        assert_eq!(remove.len(), 1, "only the copy can be removed");
        let reset = text_rects(&output, "Reset")[0].center();
        for at in [remove[0].center(), reset] {
            pointer_click(&ctx, &mut dashboard, at, egui::PointerButton::Primary);
        }
        assert!(
            jobs.try_recv().is_err(),
            "both commands wait for the recorder"
        );

        dashboard.pending = false;
        frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        pointer_click(&ctx, &mut dashboard, reset, egui::PointerButton::Primary);
        assert!(matches!(
            jobs.try_recv(),
            Ok(crate::dashboard::Job::Control(ControlRequest::ResetApp { key: reset_key }, _))
                if reset_key == key
        ));
    }

    #[test]
    fn day_page_shows_what_you_used_and_back_restores_the_range() {
        use tempora::storage::{Session, TimelineEntry, TimelineKind};
        for theme in [Theme::Light, Theme::Dark] {
            for size in [[1100.0, 1600.0], [1440.0, 1600.0]] {
                let ctx = Context::default();
                ctx.set_theme(theme);
                let mut dashboard = Dashboard::for_test(&ctx);
                dashboard.recorder = Some(status(true));
                let date = dashboard.nav.view.range.start;
                let base = crate::dashboard::format::day_start(date, dashboard.zone) + 20 * HOUR;
                let mut report = report(&dashboard);
                report.apps[0].sessions = vec![Session {
                    start_ms: base,
                    end_ms: base + 2 * HOUR,
                    totals: Totals {
                        running_ms: 2 * HOUR,
                        focused_ms: HOUR,
                        ..Totals::default()
                    },
                }];
                report.timeline = vec![
                    TimelineEntry {
                        start_ms: base,
                        end_ms: base + HOUR,
                        kind: TimelineKind::Focus("steam:1086940".into()),
                    },
                    TimelineEntry {
                        start_ms: base + HOUR,
                        end_ms: base + 2 * HOUR,
                        kind: TimelineKind::Paused,
                    },
                ];
                // An app with an entry but no focus: open all day, never in front.
                report.apps.push(tempora::storage::AppSummary {
                    key: r"exe:c:\idle.exe".into(),
                    name: "Idle Tool".into(),
                    category: Category::Other,
                    totals: Totals::default(),
                    last_seen_ms: base,
                    days: Vec::new(),
                    sessions: Vec::new(),
                });
                dashboard
                    .day_report
                    .finish((date, false), Ok(Arc::new(report)));
                let origin = dashboard.nav.view.clone();
                dashboard.nav.drill(Page::Day(date));
                let output = frame(&ctx, &mut dashboard, size);
                for label in [
                    "What you were using",
                    "Activity",
                    "Back to week",
                    "This day",
                    "in gaps",
                    "20:00 – 21:00",
                ] {
                    assert!(
                        has(&output, label),
                        "day {label} in {theme:?} {size:?}: {:?}",
                        texts(&output)
                    );
                }
                assert!(!has(&output, "When it was open"), "no running lane");
                assert!(!has(&output, "List"), "no list view toggle");
                assert_eq!(
                    texts(&output)
                        .iter()
                        .filter(|text| *text == "Timeline")
                        .count(),
                    1,
                    "only the navigation item says Timeline; the Day page has no Timeline toggle"
                );
                // The highlight chip adds one more copy of the app's name.
                let name = "A game with a deliberately very long name that must truncate";
                let copies =
                    |output: &FullOutput| texts(output).iter().filter(|text| *text == name).count();
                let plain = copies(&output);
                dashboard.nav.view.lane_app = Some("steam:1086940".into());
                let output = frame(&ctx, &mut dashboard, size);
                assert_eq!(
                    copies(&output),
                    plain + 1,
                    "an app in front is highlighted: {:?}",
                    texts(&output)
                );
                dashboard.nav.view.lane_app = Some(r"exe:c:\idle.exe".into());
                let output = frame(&ctx, &mut dashboard, size);
                assert_eq!(
                    copies(&output),
                    plain,
                    "an app that was only open is not highlighted"
                );
                dashboard.nav.view.lane_app = None;
                assert!(dashboard.nav.go_back());
                assert_eq!(dashboard.nav.view, origin, "Back returns to the same range");
            }
        }
    }

    #[test]
    fn dialogs_show_their_previews_before_committing() {
        use crate::dashboard::{AppInfo, DeletePreview, dialogs::Dialog};
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        let report = report(&dashboard);
        let key = "steam:1086940".to_owned();
        let mut app = report.apps[0].clone();
        app.last_seen_ms = 1_000;
        dashboard.apps = report
            .apps
            .iter()
            .map(|app| AppIdentity {
                key: app.key.clone(),
                name: app.name.clone(),
                category: app.category,
            })
            .collect();
        dashboard.infos.entry(key.clone()).or_default().finish(
            key.clone(),
            Ok(Arc::new(AppInfo {
                all_time: Some(app),
                sources: vec![key.clone()],
                automatic: None,
            })),
        );
        let draw = |dashboard: &mut Dashboard| {
            let input = || egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 900.0))),
                ..egui::RawInput::default()
            };
            let mut last = None;
            for _ in 0..3 {
                let mut output = ctx.run_ui(input(), |ui| {
                    dashboard.draw_shell(ui);
                    dashboard.draw_dialog(ui.ctx());
                });
                output.textures_delta.clear();
                last = Some(output);
            }
            last.unwrap()
        };

        dashboard.open_edit(&key);
        let output = draw(&mut dashboard);
        for label in ["Edit app", "Display name", "Save", "Productivity"] {
            assert!(has(&output, label), "edit {label}: {:?}", texts(&output));
        }
        assert!(matches!(&dashboard.dialog, Some(Dialog::Edit(draft)) if draft.prefilled));
        assert!(!has(&output, "What changes"), "no shifting preview box");

        dashboard.dialog = None;
        dashboard.open_group(&key);
        if let Some(Dialog::Group(draft)) = &mut dashboard.dialog {
            draft.target = Some(r"exe:c:\firefox.exe".into());
        }
        let output = draw(&mut dashboard);
        for label in [
            "Group entries",
            "Entries",
            "After grouping",
            "Group 2 entries",
            "You can undo this.",
        ] {
            assert!(has(&output, label), "group {label}: {:?}", texts(&output));
        }

        dashboard.dialog = None;
        dashboard.open_delete(None);
        if let Some(Dialog::Delete(draft)) = &mut dashboard.dialog {
            let start = dashboard.nav.view.range.start;
            draft.preview = Some(Ok(DeletePreview {
                name: "All recorded activity".into(),
                first_ms: Some(0),
                last_ms: Some(1),
                sessions: 214,
                focused_ms: 4 * HOUR,
                days: Some(
                    (0..30)
                        .map(|offset| (start - chrono::Days::new(offset), HOUR))
                        .rev()
                        .collect(),
                ),
            }));
        }
        let output = draw(&mut dashboard);
        for label in [
            "Delete all recorded history?",
            "This permanently deletes all 214 sessions",
            "Delete 214 sessions",
        ] {
            assert!(has(&output, label), "delete {label}: {:?}", texts(&output));
        }
    }

    #[test]
    fn settings_shows_every_section_in_both_layouts() {
        for size in [[1100.0, 2600.0], [1440.0, 2600.0]] {
            let ctx = Context::default();
            let mut dashboard = Dashboard::for_test(&ctx);
            let mut recorder = status(true);
            recorder
                .settings
                .excluded
                .insert(r"exe:c:\apps\discord.exe".into());
            dashboard.settings = recorder.settings.clone();
            dashboard.recorder = Some(recorder);
            dashboard.nav.section(Page::Settings);
            let output = frame(&ctx, &mut dashboard, size);
            for label in [
                "Start at sign-in",
                "Appearance",
                "Week starts on",
                "Idle threshold",
                "Excluded apps",
                "Discord",
                "Export sessions to CSV",
                "Back up database",
                "Delete all history…",
                "Recorder",
                "Closing this window doesn't stop recording.",
            ] {
                assert!(
                    has(&output, label),
                    "settings {label} at {size:?}: {:?}",
                    texts(&output)
                );
            }
        }
    }

    fn pointer_frame(
        ctx: &Context,
        dashboard: &mut Dashboard,
        events: Vec<egui::Event>,
    ) -> FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 1600.0))),
                events,
                ..egui::RawInput::default()
            },
            |ui| dashboard.draw_shell(ui),
        );
        output.textures_delta.clear();
        output
    }

    /// Hover, then press and release on `at`, one frame per step.
    fn pointer_click(
        ctx: &Context,
        dashboard: &mut Dashboard,
        at: egui::Pos2,
        button: egui::PointerButton,
    ) {
        let event = |pressed| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let steps = [
            vec![egui::Event::PointerMoved(at)],
            vec![event(true)],
            vec![event(false)],
        ];
        for events in steps {
            pointer_frame(ctx, dashboard, events);
        }
    }

    fn secondary_click(ctx: &Context, dashboard: &mut Dashboard, at: egui::Pos2) {
        pointer_click(ctx, dashboard, at, egui::PointerButton::Secondary);
    }

    #[test]
    fn title_bar_drag_uses_the_press_position_when_the_first_move_arrives_with_it() {
        for from_tab in [true, false] {
            let ctx = Context::default();
            let mut dashboard = Dashboard::for_test(&ctx);
            dashboard.recorder = Some(status(true));
            let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
            let tab = text_rects(&output, "Library")[0].center();
            let start = if from_tab { tab } else { pos2(tab.x, 65.0) };
            let moved = pos2(tab.x + 30.0, 65.0);
            pointer_frame(&ctx, &mut dashboard, vec![egui::Event::PointerMoved(start)]);
            let press = pointer_frame(
                &ctx,
                &mut dashboard,
                vec![
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerMoved(moved),
                ],
            );
            let drag = pointer_frame(
                &ctx,
                &mut dashboard,
                vec![egui::Event::PointerMoved(moved + vec2(1.0, 0.0))],
            );
            let started = [press, drag].iter().any(|frame| {
                frame.viewport_output.values().any(|viewport| {
                    viewport
                        .commands
                        .iter()
                        .any(|command| matches!(command, egui::ViewportCommand::StartDrag))
                })
            });
            assert_eq!(started, !from_tab, "drag starting at {start:?}");
        }
    }

    /// A right-click on a Top apps row opens the app menu for that row's app, anchored at the
    /// pointer, and the menu is drawn on the page.
    #[test]
    fn right_click_on_a_top_app_opens_the_app_menu() {
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        let key = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
        dashboard
            .report
            .finish(key, Ok(Arc::new(report(&dashboard))));
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        assert!(has(&output, "Top apps"));
        let at = text_rects(&output, "Firefox")[0].center();
        secondary_click(&ctx, &mut dashboard, at);
        match &dashboard.popup {
            Some(Popup::AppMenu(app, anchor)) => {
                assert_eq!(
                    app, r"exe:c:\firefox.exe",
                    "the menu is for the clicked row"
                );
                assert_eq!(anchor.min, at, "anchored at the pointer");
                assert_eq!(anchor.size(), egui::Vec2::ZERO, "zero-size anchor");
            }
            _ => panic!("a right-click on a top app opens the app menu"),
        }
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        assert!(
            has(&output, "Edit app…"),
            "the menu draws: {:?}",
            texts(&output)
        );
        assert!(has(&output, "Stop tracking this app"));
    }

    /// A right-click on a Library row opens the same app menu as its ⋯ button, at the pointer.
    #[test]
    fn right_click_on_a_library_row_opens_the_app_menu() {
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        let key = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
        dashboard
            .report
            .finish(key, Ok(Arc::new(report(&dashboard))));
        dashboard.nav.section(Page::Library);
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        let rows = text_rects(&output, "Firefox");
        assert_eq!(
            rows.len(),
            1,
            "one Library row for Firefox: {:?}",
            texts(&output)
        );
        let at = rows[0].center();
        secondary_click(&ctx, &mut dashboard, at);
        match &dashboard.popup {
            Some(Popup::AppMenu(app, anchor)) => {
                assert_eq!(
                    app, r"exe:c:\firefox.exe",
                    "the menu is for the clicked row"
                );
                assert_eq!(anchor.min, at, "anchored at the pointer");
            }
            _ => panic!("a right-click on a Library row opens the app menu"),
        }
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        assert!(
            has(&output, "Edit app…"),
            "the menu draws: {:?}",
            texts(&output)
        );
    }

    /// A right-click on a This day row opens the app menu for that app, on the day page.
    #[test]
    fn right_click_on_a_this_day_row_opens_the_app_menu() {
        use tempora::storage::{TimelineEntry, TimelineKind};
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        let date = dashboard.nav.view.range.start;
        let base = crate::dashboard::format::day_start(date, dashboard.zone) + 20 * HOUR;
        let mut report = report(&dashboard);
        report.timeline = vec![TimelineEntry {
            start_ms: base,
            end_ms: base + HOUR,
            kind: TimelineKind::Focus("steam:1086940".into()),
        }];
        dashboard
            .day_report
            .finish((date, false), Ok(Arc::new(report)));
        dashboard.nav.drill(Page::Day(date));
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        assert!(has(&output, "This day"), "{:?}", texts(&output));
        let rows = text_rects(&output, "Firefox");
        assert_eq!(rows.len(), 1, "one This day row for Firefox");
        secondary_click(&ctx, &mut dashboard, rows[0].center());
        assert!(
            matches!(&dashboard.popup, Some(Popup::AppMenu(app, _)) if app == r"exe:c:\firefox.exe"),
            "a right-click on the This day row opens its app menu"
        );
    }

    fn drag(ctx: &Context, dashboard: &mut Dashboard, from: egui::Pos2, to: egui::Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        pointer_frame(ctx, dashboard, vec![egui::Event::PointerMoved(from)]);
        pointer_frame(ctx, dashboard, vec![button(from, true)]);
        // A real mouse moves a few pixels each frame.
        let steps = ((to - from).length() / 4.0).ceil().max(1.0) as usize;
        for step in 1..=steps {
            let at = from.lerp(to, step as f32 / steps as f32);
            pointer_frame(ctx, dashboard, vec![egui::Event::PointerMoved(at)]);
        }
    }

    fn release(ctx: &Context, dashboard: &mut Dashboard, at: egui::Pos2) -> FullOutput {
        pointer_frame(
            ctx,
            dashboard,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    /// The last move and the release in one frame, as a real mouse often sends them.
    fn release_after_move(ctx: &Context, dashboard: &mut Dashboard, at: egui::Pos2) {
        pointer_frame(
            ctx,
            dashboard,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }

    fn escape() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    #[test]
    fn dragging_across_days_totals_them_until_escape() {
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        // A past week, so every day has recorded time.
        let today = dashboard.today();
        let week_start = dashboard.prefs.week_start;
        let past = crate::dashboard::nav::period_range(
            crate::dashboard::nav::Period::Week,
            today - chrono::Days::new(7),
            week_start,
        );
        dashboard
            .nav
            .set_range(crate::dashboard::nav::Period::Week, past);
        let key = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
        let report = report(&dashboard);
        dashboard.report.finish(key, Ok(Arc::new(report)));
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        let day_label = |offset: u64| {
            (past.start + chrono::Days::new(offset))
                .format("%a %-d")
                .to_string()
        };
        let first = text_rects(&output, &day_label(0))[0];
        let third = text_rects(&output, &day_label(2))[0];
        let at = |label: Rect| pos2(label.center().x, label.top() - 40.0);
        let summary = "3 days · 1h 30m · 30m avg";
        drag(&ctx, &mut dashboard, at(first), at(third));
        let output = pointer_frame(&ctx, &mut dashboard, vec![]);
        assert!(
            has(&output, summary),
            "while dragging: {:?}",
            texts(&output)
        );
        release(&ctx, &mut dashboard, at(third));
        let output = pointer_frame(&ctx, &mut dashboard, vec![]);
        assert!(has(&output, summary), "kept after the release");
        assert_eq!(
            dashboard.nav.page(),
            &Page::Overview,
            "a drag opens no timeline"
        );
        pointer_frame(
            &ctx,
            &mut dashboard,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let output = pointer_frame(&ctx, &mut dashboard, vec![]);
        assert!(!has(&output, summary), "Escape clears the pick");

        let second = text_rects(&output, &day_label(1))[0];
        drag(&ctx, &mut dashboard, at(first), at(second));
        release_after_move(&ctx, &mut dashboard, at(third));
        let output = pointer_frame(&ctx, &mut dashboard, vec![]);
        assert!(
            has(&output, summary),
            "a release that arrives with the last move ends on that day: {:?}",
            texts(&output)
        );
    }

    #[test]
    fn the_lane_zooms_to_a_dragged_span_and_the_wheel_and_resets_follow() {
        use crate::dashboard::nav::DayZoom;
        use tempora::storage::{TimelineEntry, TimelineKind};
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        let date = dashboard.nav.view.range.start;
        let midnight = crate::dashboard::format::day_start(date, dashboard.zone);
        let base = midnight + 20 * HOUR;
        let mut report = report(&dashboard);
        report.timeline = vec![
            TimelineEntry {
                start_ms: base,
                end_ms: base + HOUR,
                kind: TimelineKind::Focus("steam:1086940".into()),
            },
            TimelineEntry {
                start_ms: base + HOUR,
                end_ms: base + 2 * HOUR,
                kind: TimelineKind::Paused,
            },
        ];
        dashboard
            .day_report
            .finish((date, false), Ok(Arc::new(report)));
        dashboard.nav.drill(Page::Day(date));
        // Active hours run 20:00 to 22:00, labelled at each quarter hour.
        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        let topmost = |label: &str| {
            text_rects(&output, label)
                .into_iter()
                .min_by(|a, b| a.top().total_cmp(&b.top()))
                .unwrap_or_else(|| panic!("{label} tick: {:?}", texts(&output)))
        };
        let (start, end) = (topmost("20:00"), topmost("22:00"));
        // The lane sits under the axis row: half the row, the card's gap, half the lane.
        let y = start.center().y + 8.0 + 14.0 + 22.0;
        let x = |share: f32| start.left() + (end.right() - start.left()) * share;
        let span = |dashboard: &Dashboard| match dashboard.nav.view.zoom {
            DayZoom::Custom { from, to } => (from.ms, to.ms),
            other => panic!("expected a custom zoom, got {other:?}"),
        };
        let near = |actual: i64, expected: i64| (actual - expected).abs() <= 2 * 60_000;

        drag(&ctx, &mut dashboard, pos2(x(0.25), y), pos2(x(0.75), y));
        pointer_frame(&ctx, &mut dashboard, escape());
        release(&ctx, &mut dashboard, pos2(x(0.75), y));
        assert_eq!(
            dashboard.nav.view.zoom,
            DayZoom::Active,
            "Escape cancels the drag instead of zooming"
        );

        drag(&ctx, &mut dashboard, pos2(x(0.25), y), pos2(x(0.6), y));
        release_after_move(&ctx, &mut dashboard, pos2(x(0.75), y));
        let (from, to) = span(&dashboard);
        assert!(
            near(from, 20 * HOUR + 30 * 60_000) && near(to, 21 * HOUR + HOUR / 2),
            "picked 20:30 to 21:30: {from} {to}"
        );

        let zoomed = dashboard.nav.view.zoom;
        drag(&ctx, &mut dashboard, pos2(x(0.4), y), pos2(x(0.6), y));
        pointer_frame(&ctx, &mut dashboard, escape());
        release(&ctx, &mut dashboard, pos2(x(0.6), y));
        assert_eq!(
            dashboard.nav.view.zoom, zoomed,
            "Escape cancels a second drag and keeps the zoom it started from"
        );

        pointer_frame(
            &ctx,
            &mut dashboard,
            vec![
                egui::Event::PointerMoved(pos2(x(0.5), y)),
                egui::Event::Zoom(2.0),
            ],
        );
        let (from, to) = span(&dashboard);
        assert!(
            near(to - from, HOUR / 2),
            "Ctrl+wheel halves the span: {from} {to}"
        );
        assert!(
            near(from, 20 * HOUR + 45 * 60_000),
            "around the pointer: {from}"
        );

        let output = frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        assert!(has(&output, "Reset zoom"), "zoomed: {:?}", texts(&output));

        let key = |key| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        };
        pointer_frame(&ctx, &mut dashboard, key(egui::Key::Escape));
        assert_eq!(
            dashboard.nav.view.zoom,
            DayZoom::Active,
            "Escape goes back to the preset"
        );

        drag(&ctx, &mut dashboard, pos2(x(0.25), y), pos2(x(0.75), y));
        release(&ctx, &mut dashboard, pos2(x(0.75), y));
        span(&dashboard);
        for _ in 0..2 {
            pointer_click(
                &ctx,
                &mut dashboard,
                pos2(x(0.25), y),
                egui::PointerButton::Primary,
            );
        }
        assert_eq!(
            dashboard.nav.view.zoom,
            DayZoom::Active,
            "a double-click zooms back out"
        );
        assert_eq!(
            dashboard.nav.view.lane_app, None,
            "and its two clicks leave the highlight as it was"
        );
    }

    #[test]
    fn a_day_still_loading_shows_the_previous_day_as_itself_then_fades_it() {
        use tempora::storage::{TimelineEntry, TimelineKind};
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        let date = dashboard.nav.view.range.start;
        let base = crate::dashboard::format::day_start(date, dashboard.zone) + 20 * HOUR;
        let mut report = report(&dashboard);
        report.timeline = vec![TimelineEntry {
            start_ms: base,
            end_ms: base + HOUR,
            kind: TimelineKind::Focus("steam:1086940".into()),
        }];
        dashboard
            .day_report
            .finish((date, false), Ok(Arc::new(report)));
        dashboard.nav.drill(Page::Day(date));
        frame(&ctx, &mut dashboard, [1100.0, 1600.0]);
        // The next day hasn't loaded yet.
        let next = date + chrono::Days::new(1);
        dashboard.nav.view.page = Page::Day(next);
        let draw = |dashboard: &mut Dashboard, second: f64| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 1600.0))),
                    time: Some(second),
                    ..egui::RawInput::default()
                },
                |ui| dashboard.draw_shell(ui),
            );
            output.textures_delta.clear();
            output
        };
        // egui fades text by scaling its glyph colors, so this reads their alpha.
        fn alphas(output: &FullOutput, label: &str) -> Vec<u8> {
            fn walk(shape: &Shape, label: &str, out: &mut Vec<u8>) {
                match shape {
                    Shape::Text(text) if text.galley.text() == label => {
                        let alpha = text
                            .galley
                            .rows
                            .iter()
                            .flat_map(|row| row.row.visuals.mesh.vertices.iter())
                            .map(|vertex| vertex.color.a())
                            .max()
                            .unwrap_or(0);
                        out.push(alpha);
                    }
                    Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, label, out)),
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for clipped in &output.shapes {
                walk(&clipped.shape, label, &mut out);
            }
            out
        }
        let output = draw(&mut dashboard, 10.0);
        assert!(
            has(&output, &next.format("%A, %b %-d").to_string()),
            "the new day's title is up"
        );
        assert!(
            !has(&output, "Nothing recorded this day"),
            "the previous day is drawn as itself, not clipped to the new one: {:?}",
            texts(&output)
        );
        let focused = alphas(&output, "focused");
        assert!(
            !focused.is_empty() && focused.iter().all(|alpha| *alpha == 255),
            "a quick load swaps without fading: {focused:?}"
        );
        let output = draw(&mut dashboard, 11.0);
        let focused = alphas(&output, "focused");
        assert!(
            !focused.is_empty() && focused.iter().all(|alpha| *alpha < 200),
            "a slow load fades the old totals back: {focused:?}"
        );
    }

    /// An idle, unfocused window schedules no repaint: with motion on, the entrances settle
    /// and nothing animates or polls the UI continuously (the worker wakes it only when
    /// results arrive).
    #[test]
    fn idle_pages_do_not_request_continuous_repaints() {
        for page in 0..4 {
            let ctx = Context::default();
            let mut dashboard = Dashboard::for_test(&ctx);
            crate::dashboard::widgets::motion::set_enabled(&ctx, true);
            dashboard.recorder = Some(status(true));
            let key = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
            let report = Arc::new(report(&dashboard));
            let date = dashboard.nav.view.range.start;
            dashboard.report.finish(key, Ok(report.clone()));
            dashboard.day_report.finish((date, false), Ok(report));
            let page = [
                Page::Overview,
                Page::Library,
                Page::Settings,
                Page::Day(date),
            ][page]
                .clone();
            dashboard.nav.section(page.clone());
            // A clock that advances lets one-off animations (entrances, scroll bar fade-in)
            // finish.
            let input = |second: f64| egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 720.0))),
                focused: false,
                time: Some(second),
                ..egui::RawInput::default()
            };
            let mut delay = std::time::Duration::ZERO;
            for second in 0..4 {
                let mut output =
                    ctx.run_ui(input(f64::from(second)), |ui| dashboard.draw_shell(ui));
                output.textures_delta.clear();
                delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            }
            assert!(
                delay >= std::time::Duration::from_secs(1),
                "{page:?} keeps repainting: next frame in {delay:?}"
            );
        }
    }

    /// Long tables scroll inside their card: the page itself ends above the window edge.
    #[test]
    fn long_tables_fit_the_window_instead_of_scrolling_the_page() {
        for page in [Page::Overview, Page::Library] {
            let ctx = Context::default();
            let mut dashboard = Dashboard::for_test(&ctx);
            dashboard.recorder = Some(status(true));
            let key = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
            let mut report = report(&dashboard);
            let template = report.apps[0].clone();
            for index in 0..30 {
                let mut app = template.clone();
                app.key = format!("steam:{index}");
                app.name = format!("Game {index}");
                report.apps.push(app);
            }
            dashboard.report.finish(key, Ok(Arc::new(report)));
            dashboard.nav.section(page.clone());
            let output = frame(&ctx, &mut dashboard, [1440.0, 1000.0]);
            let mut bottom = 0.0f32;
            fn walk(shape: &Shape, bottom: &mut f32) {
                match shape {
                    Shape::Rect(rect)
                        if rect.corner_radius.nw
                            == crate::dashboard::theme::Tokens::CARD_RADIUS =>
                    {
                        *bottom = bottom.max(rect.rect.bottom());
                    }
                    Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, bottom)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut bottom);
            }
            assert!(bottom > 400.0, "{page:?} drew its cards");
            assert!(
                bottom <= 1000.0,
                "{page:?} page runs past the window: {bottom}"
            );
            assert!(has(&output, "Game 0"), "{page:?} shows the first rows");
        }
    }

    #[test]
    fn toasts_expire_after_their_lifetime() {
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.toast = Some(crate::dashboard::Toast {
            message: "Saved".into(),
            error: false,
            undo: None,
            shown: std::time::Instant::now() - std::time::Duration::from_secs(5),
        });
        ctx.run_ui(egui::RawInput::default(), |ui| {
            dashboard.draw_toast(ui.ctx())
        })
        .textures_delta
        .clear();
        assert!(dashboard.toast.is_none(), "a plain toast is gone after 4 s");
        dashboard.toast = Some(crate::dashboard::Toast {
            message: "App updated".into(),
            error: false,
            undo: Some("Undo"),
            shown: std::time::Instant::now() - std::time::Duration::from_secs(5),
        });
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            dashboard.draw_toast(ui.ctx())
        });
        output.textures_delta.clear();
        assert!(dashboard.toast.is_some(), "an undo toast stays for 8 s");
    }

    /// Inner list thumbs use the scrollbar color and start at the top of their track, and
    /// a page whose list fits shows no page scroll bar.
    #[test]
    fn list_thumbs_are_visible_and_the_page_does_not_scroll() {
        let ctx = Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(status(true));
        let key = (dashboard.nav.view.range, dashboard.nav.view.exclude_idle);
        let mut report = report(&dashboard);
        let template = report.apps[0].clone();
        for index in 0..30 {
            let mut app = template.clone();
            app.key = format!("steam:{index}");
            app.name = format!("Game {index}");
            report.apps.push(app);
        }
        dashboard.report.finish(key, Ok(Arc::new(report)));
        dashboard.nav.section(Page::Library);
        let colors =
            crate::dashboard::theme::Tokens::for_dark(ctx.global_style().visuals.dark_mode);
        let mut rects = Vec::new();
        for second in 0..6 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1440.0, 1000.0))),
                time: Some(f64::from(second)),
                ..egui::RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| dashboard.draw_shell(ui));
            output.textures_delta.clear();
            rects.clear();
            fn walk(shape: &Shape, out: &mut Vec<(Rect, egui::Color32)>) {
                match shape {
                    Shape::Rect(rect) => out.push((rect.rect, rect.fill)),
                    Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut rects);
            }
        }
        let bars: Vec<_> = rects
            .iter()
            .filter(|(rect, _)| rect.width() <= 12.0 && rect.height() > 20.0)
            .collect();
        let thumb = bars
            .iter()
            .find(|(_, fill)| *fill == colors.scroll_thumb || *fill == colors.scroll_thumb_hover)
            .expect("the list thumb uses the scrollbar color");
        let track = bars
            .iter()
            .find(|(rect, fill)| {
                *fill == colors.muted && (rect.left() - thumb.0.left()).abs() < 0.5
            })
            .expect("track behind the thumb");
        assert!(
            (thumb.0.top() - track.0.top()).abs() < 1.0,
            "thumb starts at the top: {thumb:?} {track:?}"
        );
        assert!(thumb.0.height() < track.0.height(), "the list scrolls");
        assert!(
            bars.iter().all(|(rect, _)| rect.right() < 1430.0),
            "no page scroll bar at the window edge: {bars:?}"
        );
    }
}
