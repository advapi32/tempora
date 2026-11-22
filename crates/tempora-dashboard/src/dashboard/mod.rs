mod caption;
mod dialogs;
mod format;
#[cfg(debug_assertions)]
mod gallery;
mod icons;
#[cfg(debug_assertions)]
pub(super) use gallery::Gallery;
mod nav;
mod prefs;
mod screens;
mod shell;
mod theme;
mod widgets;
pub(super) mod window_geometry;
mod worker;

use std::collections::{HashMap, HashSet, hash_map::RandomState};
use std::hash::BuildHasher;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate};
use chrono_tz::Tz;
use eframe::egui;
use tempora::{
    model::{
        AppIdentity, CAPABILITY_TRACKED_UNDO, ControlRequest, DateRange, RecorderStatus, Settings,
    },
    storage::Report,
};

use dialogs::Dialog;
use nav::{Nav, Page};
use prefs::Preferences;
use screens::{FirstRun, Popup, SettingsState};
use shell::RecorderStartup;
use worker::{AppInfo, ControlKind, DeletePreview, Extras, Job, Update, Worker};

pub(super) const APP_NAME: &str = "Tempora";
/// The size the dashboard first opens at, in points.
pub(super) const WINDOW_DEFAULT: [f32; 2] = [1170.0, 760.0];
pub(super) const WINDOW_MIN: [f32; 2] = theme::Tokens::WINDOW_MIN;
/// Recorder status is re-read this often; reports refresh only when data changes.
const STATUS_PERIOD: Duration = Duration::from_secs(15);
/// Failed requests in a row before the dashboard marks the recorder unreachable.
const STATUS_RETRIES: u32 = 3;
/// Time between retries. Connecting to a busy pipe can also wait up to a second.
const STATUS_RETRY: Duration = Duration::from_secs(1);
/// The persistent connection's `data_version` is compared this often.
const POLL_PERIOD: Duration = Duration::from_secs(3);

/// Keeps the last successful value and any later error under their own keys.
/// Only one request can be in flight, and an invalidation leaves the last value visible.
pub(super) struct Cached<K, V> {
    loaded: Option<(K, V)>,
    error: Option<(K, String)>,
    in_flight: Option<K>,
    stale: bool,
}

impl<K: Clone + PartialEq, V> Default for Cached<K, V> {
    fn default() -> Self {
        Self {
            loaded: None,
            error: None,
            in_flight: None,
            stale: false,
        }
    }
}

impl<K: Clone + PartialEq, V> Cached<K, V> {
    pub fn get(&self, key: &K) -> Option<&V> {
        self.loaded
            .as_ref()
            .filter(|(loaded, _)| loaded == key)
            .map(|(_, value)| value)
    }

    pub fn error(&self, key: &K) -> Option<&str> {
        self.error
            .as_ref()
            .filter(|(failed, _)| failed == key)
            .map(|(_, error)| error.as_str())
    }

    pub fn loading(&self, key: &K) -> bool {
        self.in_flight.as_ref() == Some(key)
    }

    fn needs(&mut self, key: &K) -> bool {
        if self.in_flight.is_some() {
            return false;
        }
        let known = self.get(key).is_some() || self.error(key).is_some();
        if known && !self.stale {
            return false;
        }
        self.in_flight = Some(key.clone());
        self.stale = false;
        true
    }

    fn finish(&mut self, key: K, result: Result<V, String>) {
        self.in_flight = None;
        match result {
            Ok(value) => {
                self.loaded = Some((key, value));
                self.error = None;
            }
            Err(error) => self.error = Some((key, error)),
        }
    }

    fn invalidate(&mut self) {
        self.stale = true;
    }
}

pub(super) struct Toast {
    pub message: String,
    pub error: bool,
    pub undo: Option<&'static str>,
    pub shown: Instant,
}

impl Toast {
    /// Toasts fade on their own; ones that offer Undo or report an error stay longer.
    pub fn lifetime(&self) -> Duration {
        if self.undo.is_some() || self.error {
            Duration::from_secs(8)
        } else {
            Duration::from_secs(4)
        }
    }
}

pub(super) struct Dashboard {
    jobs: Sender<Job>,
    updates: Receiver<Update>,
    report: Cached<(DateRange, bool), Arc<Report>>,
    day_report: Cached<(NaiveDate, bool), Arc<Report>>,
    infos: HashMap<String, Cached<String, Arc<AppInfo>>>,
    apps: Vec<AppIdentity>,
    prefs: Preferences,
    /// The window's last normal geometry, and what was last written to disk.
    window: Option<window_geometry::WindowGeometry>,
    saved_window: Option<window_geometry::WindowGeometry>,
    recorder: Option<RecorderStatus>,
    recorder_startup: Option<RecorderStartup>,
    today_focus: Option<(NaiveDate, i64)>,
    settings: Settings,
    settings_dirty: bool,
    recorder_error: Option<tempora::pipe::RequestError>,
    toast: Option<Toast>,
    /// Corrections made by this dashboard. Older recorders support one legacy undo (None).
    undo_stack: Vec<Option<u64>>,
    pending: bool,
    status_pending: bool,
    /// Status requests that failed in a row while the recorder was last known online.
    status_failures: u32,
    next_refresh: Instant,
    next_poll: Instant,
    data_version: Option<i64>,
    extras: Option<((DateRange, bool), Arc<Extras>)>,
    /// Days with recorded history, by calendar month (first day).
    covered: HashMap<NaiveDate, Arc<HashSet<NaiveDate>>>,
    covered_pending: HashSet<NaiveDate>,
    nav: Nav,
    zone: Tz,
    /// Bottom of the visible content area this frame; lists fill the window to it.
    viewport_bottom: f32,
    dialog: Option<Dialog>,
    popup: Option<Popup>,
    first_run: FirstRun,
    settings_state: SettingsState,
}

impl Dashboard {
    pub fn new(context: &eframe::CreationContext<'_>) -> Self {
        theme::setup(&context.egui_ctx);
        caption::install(context);
        let prefs = Preferences::load(context.storage);
        prefs.apply(&context.egui_ctx);
        let zone = iana_time_zone::get_timezone()
            .ok()
            .and_then(|zone| zone.parse().ok())
            .unwrap_or(chrono_tz::UTC);
        let today = Local::now().date_naive();
        let (jobs, receiver) = mpsc::channel();
        let (sender, updates) = mpsc::channel();
        let egui = context.egui_ctx.clone();
        std::thread::spawn(move || {
            let mut worker = Worker::new(zone);
            while let Ok(job) = receiver.recv() {
                if sender.send(worker.run(job)).is_err() {
                    break;
                }
                egui.request_repaint();
            }
        });
        let mut dashboard = Self::build(jobs, updates, prefs, zone, today);
        dashboard.refresh();
        dashboard
    }

    fn build(
        jobs: Sender<Job>,
        updates: Receiver<Update>,
        prefs: Preferences,
        zone: Tz,
        today: NaiveDate,
    ) -> Self {
        Self {
            jobs,
            updates,
            report: Cached::default(),
            day_report: Cached::default(),
            infos: HashMap::new(),
            apps: Vec::new(),
            nav: Nav::new(today, prefs.week_start),
            prefs,
            window: None,
            saved_window: None,
            recorder: None,
            recorder_startup: None,
            today_focus: None,
            settings: Settings::default(),
            settings_dirty: false,
            recorder_error: None,
            toast: None,
            undo_stack: Vec::new(),
            pending: false,
            status_pending: false,
            status_failures: 0,
            next_refresh: Instant::now(),
            next_poll: Instant::now(),
            data_version: None,
            extras: None,
            covered: HashMap::new(),
            covered_pending: HashSet::new(),
            zone,
            viewport_bottom: f32::INFINITY,
            dialog: None,
            popup: None,
            first_run: FirstRun::default(),
            settings_state: SettingsState::default(),
        }
    }

    /// A dashboard with no worker, for headless rendering tests.
    #[cfg(test)]
    pub(super) fn for_test(ctx: &egui::Context) -> Self {
        theme::setup(ctx);
        let (jobs, _) = mpsc::channel();
        let (_, updates) = mpsc::channel();
        Self::build(
            jobs,
            updates,
            Preferences::default(),
            chrono_tz::UTC,
            Local::now().date_naive(),
        )
    }

    fn send(&mut self, job: Job) {
        if self.jobs.send(job).is_err() {
            self.pending = false;
            self.notify("The dashboard worker stopped. Reopen the dashboard.", true);
        }
    }

    fn notify(&mut self, message: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            message: message.into(),
            error,
            undo: None,
            shown: Instant::now(),
        });
    }

    fn request_status(&mut self) {
        if !self.status_pending {
            self.status_pending = true;
            self.send(Job::Status);
        }
    }

    /// Marks every cached projection stale; `sync` re-requests what is on screen.
    fn refresh(&mut self) {
        self.request_status();
        self.report.invalidate();
        self.day_report.invalidate();
        for info in self.infos.values_mut() {
            info.invalidate();
        }
    }

    /// Requests the covered days of a calendar month once per data version.
    pub(super) fn want_covered(&mut self, month: NaiveDate) {
        if !self.covered.contains_key(&month) && self.covered_pending.insert(month) {
            self.send(Job::Covered(month));
        }
    }

    fn extras(&self) -> Option<Arc<Extras>> {
        self.extras
            .as_ref()
            .filter(|(key, _)| *key == self.report_key())
            .map(|(_, extras)| extras.clone())
    }

    fn report_key(&self) -> (DateRange, bool) {
        (self.nav.view.range, self.nav.view.exclude_idle)
    }

    pub(super) fn current_report(&self) -> Option<Arc<Report>> {
        self.report.get(&self.report_key()).cloned()
    }

    fn info(&self, key: &str) -> Option<Arc<AppInfo>> {
        self.infos
            .get(key)
            .and_then(|info| info.get(&key.to_owned()).cloned())
    }

    pub(super) fn want_info(&mut self, key: &str) {
        let key = key.to_owned();
        if self.infos.entry(key.clone()).or_default().needs(&key) {
            self.send(Job::AppInfo(key));
        }
    }

    /// Requests what the current page shows. Cheap; no database work on this thread.
    fn sync(&mut self) {
        let key = self.report_key();
        if self.report.needs(&key) {
            self.send(Job::Report(key.0, key.1));
        }
        match self.nav.page().clone() {
            Page::Day(date) => {
                let key = (date, self.nav.view.exclude_idle);
                if self.day_report.needs(&key) {
                    self.send(Job::DayReport(date, key.1));
                }
            }
            Page::App(app) => self.want_info(&app),
            _ => {}
        }
        if let Some(dialog) = &self.dialog {
            for key in dialog.info_keys() {
                self.want_info(&key);
            }
        }
    }

    fn action(&mut self, command: ControlRequest, success: &'static str) {
        if matches!(command, ControlRequest::DeleteHistory { .. }) {
            self.undo_stack.clear();
        }
        let command = if command.is_app_correction() && self.recorder_can(CAPABILITY_TRACKED_UNDO) {
            ControlRequest::TrackCorrection {
                token: RandomState::new().hash_one(()),
                command: Box::new(command),
            }
        } else {
            command
        };
        self.pending = true;
        self.send(Job::Control(command, success));
    }

    pub(super) fn recorder_can(&self, capability: &str) -> bool {
        self.recorder
            .as_ref()
            .is_some_and(|status| status.capabilities.iter().any(|known| known == capability))
    }

    pub(super) fn recorder_online(&self) -> bool {
        self.recorder.is_some()
    }

    fn undo(&mut self) {
        if !self.recorder_online() || self.pending {
            return;
        }
        let command = match self.undo_stack.last() {
            Some(Some(token)) => ControlRequest::UndoTrackedCorrection { token: *token },
            Some(None) => ControlRequest::UndoCorrection,
            None => return,
        };
        self.toast = None;
        self.action(command, "Change undone");
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Update::Status(result) => {
                    self.status_pending = false;
                    match result {
                        Ok(status) => {
                            if !self.settings_dirty {
                                self.settings = status.settings.clone();
                            }
                            if status.recording {
                                self.send(Job::TodayFocus(Local::now().date_naive()));
                            }
                            self.recorder = Some(status);
                            self.status_failures = 0;
                            if self.recorder_error.take().is_some() {
                                self.report.invalidate();
                            }
                        }
                        // The recorder can't answer while its tray menu is open or a backup
                        // runs. Keep the last known state through a few quick retries instead
                        // of flashing "can't reach the recorder".
                        Err(_)
                            if self.recorder.is_some()
                                && self.status_failures + 1 < STATUS_RETRIES =>
                        {
                            self.status_failures += 1;
                            self.next_refresh = Instant::now() + STATUS_RETRY;
                        }
                        Err(error) => {
                            self.status_failures = 0;
                            self.recorder = None;
                            self.recorder_error = Some(error);
                        }
                    }
                }
                Update::TodayFocus(date, focus) => {
                    self.today_focus = focus.map(|total| (date, total));
                }
                Update::Report(range, exclude_idle, result) => {
                    let result = result.map(|loaded| {
                        self.apps = loaded.apps;
                        self.extras = Some(((range, exclude_idle), Arc::new(loaded.extras)));
                        Arc::new(loaded.report)
                    });
                    self.report.finish((range, exclude_idle), result);
                }
                Update::DayReport(date, exclude_idle, result) => {
                    self.day_report
                        .finish((date, exclude_idle), result.map(Arc::new));
                }
                Update::AppInfo(key, result) => {
                    self.infos
                        .entry(key.clone())
                        .or_default()
                        .finish(key, result.map(Arc::new));
                }
                Update::DeletePreview(key, result) => {
                    if let Some(Dialog::Delete(draft)) = &mut self.dialog
                        && draft.key == key
                    {
                        draft.preview = Some(result);
                        draft.version = self.data_version;
                    }
                }
                Update::Done(kind, result) => {
                    self.pending = false;
                    match result {
                        Ok(message) => {
                            let undo = match kind {
                                ControlKind::Correction(token) => {
                                    if token.is_none() {
                                        self.undo_stack.clear();
                                    }
                                    self.undo_stack.push(token);
                                    Some("Undo")
                                }
                                ControlKind::Undo => {
                                    self.undo_stack.pop();
                                    (!self.undo_stack.is_empty()).then_some("Undo previous")
                                }
                                ControlKind::Settings => {
                                    self.settings_dirty = false;
                                    None
                                }
                                ControlKind::Other => None,
                            };
                            self.toast = Some(Toast {
                                message: message.into(),
                                error: false,
                                undo,
                                shown: Instant::now(),
                            });
                            if matches!(kind, ControlKind::Correction(_) | ControlKind::Undo) {
                                self.infos.clear();
                                icons::clear(ctx);
                            }
                            self.refresh();
                        }
                        Err(error) => {
                            if kind == ControlKind::Undo {
                                self.undo_stack.clear();
                            }
                            self.notify(error, true);
                        }
                    }
                }
                Update::Exported(result) => {
                    self.settings_state.exporting = false;
                    self.settings_state.export = Some(result);
                }
                Update::Icon(key, image) => icons::store(ctx, key, image),
                Update::Covered(month, days) => {
                    self.covered_pending.remove(&month);
                    if let Some(days) = days {
                        self.covered.insert(month, Arc::new(days));
                    }
                }
                Update::Version(version) => {
                    if version.is_some()
                        && self.data_version.is_some()
                        && version != self.data_version
                    {
                        self.covered.clear();
                        self.refresh();
                    }
                    self.data_version = version;
                }
                Update::BackedUp(result) => {
                    self.pending = false;
                    self.settings_state.backup = Some(result);
                }
            }
        }
    }

    /// Mouse buttons 4 and 5 and Alt+Left/Right step through page history. While a popup or
    /// dialog is open, the same press closes it instead.
    fn history_navigation(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let (back, forward) = ctx.input_mut(|input| {
            let mut back = input.pointer.button_pressed(egui::PointerButton::Extra1);
            let mut forward = input.pointer.button_pressed(egui::PointerButton::Extra2);
            if !typing {
                back |= input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft);
                forward |= input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight);
            }
            (back, forward)
        });
        if !back && !forward {
            return;
        }
        if self.dialog.is_some() || self.popup.is_some() {
            self.dialog = None;
            self.popup = None;
        } else if back {
            self.nav.history_back();
        } else {
            self.nav.history_forward();
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() || self.popup.is_some() {
            return;
        }
        if !ctx.text_edit_focused()
            && !self.undo_stack.is_empty()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z))
        {
            self.undo();
        }
    }
}

impl eframe::App for Dashboard {
    /// Worker results and refresh scheduling; also runs while the window is hidden.
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.window = context.input(|input| {
            window_geometry::WindowGeometry::observe(self.window, input.viewport(), WINDOW_DEFAULT)
        });
        self.receive(context);
        let now = Instant::now();
        if now >= self.next_refresh {
            self.request_status();
            self.next_refresh = now + STATUS_PERIOD;
        }
        if now >= self.next_poll {
            self.send(Job::Poll);
            self.next_poll = now + POLL_PERIOD;
        }
        self.sync();
        context.request_repaint_after(
            self.next_poll
                .min(self.next_refresh)
                .saturating_duration_since(now),
        );
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        for key in icons::take_wanted(&context) {
            self.send(Job::Icon(key));
        }
        self.nav.observe();
        self.history_navigation(&context);
        self.shortcuts(&context);
        self.draw_shell(ui);
        self.draw_dialog(&context);
        self.draw_toast(&context);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.prefs.save(storage);
        if let Some(window) = self.window
            && self.saved_window != self.window
        {
            window_geometry::save(APP_NAME, &window);
            self.saved_window = self.window;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_walks_this_dashboards_corrections_newest_first() {
        let ctx = egui::Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        dashboard.recorder = Some(RecorderStatus {
            settings: Settings::default(),
            candidates: Vec::new(),
            problem: None,
            recording: false,
            capabilities: vec![CAPABILITY_TRACKED_UNDO.into()],
        });
        let (send, jobs) = mpsc::channel();
        dashboard.jobs = send;
        let (updates, receive) = mpsc::channel();
        dashboard.updates = receive;
        let next_command = || {
            jobs.try_iter().find_map(|job| match job {
                Job::Control(command, _) => Some(command),
                _ => None,
            })
        };
        let shortcut = |dashboard: &mut Dashboard| {
            let modifiers = egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            };
            ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Z,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |_| dashboard.shortcuts(&ctx),
            )
            .textures_delta
            .clear();
        };
        shortcut(&mut dashboard);
        assert!(next_command().is_none(), "fresh dashboard");

        let mut tokens = Vec::new();
        for key in ["steam:1", "steam:2"] {
            dashboard.action(ControlRequest::ResetApp { key: key.into() }, "App reset");
            let Some(ControlRequest::TrackCorrection { token, command }) = next_command() else {
                panic!("app correction should be tracked");
            };
            assert!(matches!(*command, ControlRequest::ResetApp { .. }));
            updates
                .send(Update::Done(
                    ControlKind::Correction(Some(token)),
                    Ok("App reset"),
                ))
                .unwrap();
            dashboard.receive(&ctx);
            tokens.push(token);
        }
        for (token, label) in [(tokens[1], Some("Undo previous")), (tokens[0], None)] {
            shortcut(&mut dashboard);
            assert!(matches!(
                next_command(),
                Some(ControlRequest::UndoTrackedCorrection {token: request_token}) if request_token == token
            ));
            updates
                .send(Update::Done(ControlKind::Undo, Ok("Change undone")))
                .unwrap();
            dashboard.receive(&ctx);
            assert_eq!(dashboard.toast.as_ref().unwrap().undo, label);
        }
        shortcut(&mut dashboard);
        assert!(
            next_command().is_none(),
            "pre-session corrections stay untouched"
        );
    }

    #[test]
    fn cache_coalesces_and_never_serves_another_key() {
        let mut cache = Cached::<u32, &str>::default();
        assert!(cache.needs(&1));
        assert!(!cache.needs(&2), "one request in flight");
        cache.finish(1, Ok("one"));
        assert_eq!(cache.get(&2), None);
        assert!(cache.needs(&2));
        cache.finish(2, Ok("two"));
        assert!(!cache.needs(&2));
        cache.invalidate();
        assert_eq!(cache.get(&2), Some(&"two"), "stale data stays visible");
        assert!(cache.needs(&2));
        cache.finish(2, Err("could not refresh".into()));
        assert_eq!(cache.get(&2), Some(&"two"));
        assert_eq!(cache.error(&2), Some("could not refresh"));
        assert_eq!(cache.error(&1), None);
        assert!(!cache.needs(&2), "an error waits for the next invalidation");
    }
}
