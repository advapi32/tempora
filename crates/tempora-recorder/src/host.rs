mod capture;
mod startup;
mod tray;

use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    process::{Child, Command},
    time::{Duration, Instant},
};
use tempora::{
    model::{
        AppIdentity, CAPABILITY_RESET_APP, CAPABILITY_TRACKED_UNDO, CAPABILITY_UNGROUP_APP,
        ControlRequest, ControlResponse, RecorderStatus, Settings, database_path,
    },
    observation::{Engine, WindowsObserver, capture_clock},
    pipe::Server,
    storage::Store,
    win::{self, Window},
};
use tray::{Event, Indicator, MenuChoice, TrayWindow};
use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

const APP_NAME: &str = "Tempora";
const POLL_MS: u32 = 125;
const FOCUS_PERIOD: Duration = Duration::from_secs(2);
const PROCESS_PERIOD: Duration = Duration::from_secs(10);
const PAUSE_CHECKPOINT: Duration = Duration::from_secs(30);

pub fn run() {
    let quiet = std::env::args_os().any(|arg| arg == "--sign-in");
    if let Err(error) = run_recorder()
        && !quiet
    {
        tray::show_error(&format!("The recorder could not start.\n\n{error:#}"));
    }
}

fn run_recorder() -> Result<()> {
    let database = database_path()?;
    fs::create_dir_all(database.parent().context("Missing history folder")?)?;
    let _ownership = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .share_mode(0)
        .open(database.with_extension("writer.lock"))
    {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => return Ok(()),
        Err(error) => return Err(error).context("Could not acquire local history ownership"),
    };

    let store = Store::open_writer(&database)?;
    let settings = store.settings()?;
    let (observer, fingerprinted) = capture::prepare_observer(&store, &settings)?;
    let mut host = Host {
        session_eligible: observer.session_eligible(),
        settings: store.settings()?,
        store,
        observer,
        fingerprinted,
        pending_fingerprints: HashMap::new(),
        pipe: Server::bind().context("Could not create the local recorder control channel")?,
        engine: Engine::default(),
        next_focus: Instant::now(),
        next_processes: Instant::now(),
        next_pause_checkpoint: Instant::now() + PAUSE_CHECKPOINT,
        last_recording_boundary: None,
        pause_start: None,
        candidates: Vec::new(),
        last_candidates: None,
        undo_tokens: Vec::new(),
        dashboard: None,
        asleep: false,
        ending_session: false,
        quitting: false,
        recording_problem: None,
        settings_problem: None,
        startup_problem: None,
        dashboard_problem: None,
    };

    let mut window = TrayWindow::new()?;
    host.reconcile_pause();
    window.refresh(host.indicator(), host.problem());
    'messages: while tray::pump_message()? {
        while let Some(event) = tray::next_event() {
            match event {
                Event::Tick => host.tick(),
                Event::Open => host.open_dashboard(),
                Event::Menu => match window.menu(host.indicator(), &host.settings) {
                    Some(MenuChoice::OpenDashboard) => host.open_dashboard(),
                    Some(MenuChoice::TogglePause) if host.settings.enabled => {
                        let _ = host.control(ControlRequest::SetPaused(!host.settings.paused));
                    }
                    // Before setup, the pause item reads "Finish setup…" and opens the dashboard.
                    Some(MenuChoice::TogglePause) => host.open_dashboard(),
                    Some(MenuChoice::Quit) => break 'messages,
                    None => {}
                },
                Event::Quit => break 'messages,
                Event::SessionBlocked => {
                    host.session_eligible = false;
                    host.reconcile_pause();
                }
                Event::SessionChanged => {
                    host.session_eligible = host.observer.session_eligible();
                    host.reconcile_pause();
                    host.next_focus = Instant::now();
                }
                Event::Sleep(asleep) => {
                    host.asleep = asleep;
                    host.reconcile_pause();
                    host.next_focus = Instant::now();
                }
                Event::EndingSession => {
                    host.ending_session = true;
                    host.reconcile_pause();
                }
                Event::Shutdown(true) => break 'messages,
                Event::Shutdown(false) => {
                    host.ending_session = false;
                    host.reconcile_pause();
                }
                Event::RestoreTray => window.restore(),
            }
            window.refresh(host.indicator(), host.problem());
            if host.quitting && host.pipe.idle() {
                break 'messages;
            }
        }
    }
    host.flush_pause(capture_clock());
    host.close_dashboard();
    Ok(())
}

struct Host {
    store: Store,
    settings: Settings,
    observer: WindowsObserver,
    /// `exe:` keys whose grouping check completed this session, including keys with no fingerprint.
    fingerprinted: HashSet<String>,
    /// Fingerprints read for exclusion checks and available to reuse after a successful write.
    pending_fingerprints: HashMap<String, Option<String>>,
    pipe: Server,
    engine: Engine,
    next_focus: Instant,
    next_processes: Instant,
    next_pause_checkpoint: Instant,
    /// UTC time and local offset at the boundary where a pause can start.
    last_recording_boundary: Option<(i64, i32)>,
    pause_start: Option<(i64, i32)>,
    candidates: Vec<AppIdentity>,
    last_candidates: Option<Instant>,
    /// Tokens for the contiguous set of tracked corrections at the top of the database stack.
    undo_tokens: Vec<u64>,
    dashboard: Option<Child>,
    session_eligible: bool,
    asleep: bool,
    ending_session: bool,
    quitting: bool,
    recording_problem: Option<String>,
    settings_problem: Option<String>,
    startup_problem: Option<String>,
    dashboard_problem: Option<String>,
}

impl Host {
    fn tick(&mut self) {
        if let Some(request) = self.pipe.poll() {
            let response = self.control(request);
            self.pipe.respond(&response);
        }
        if self.quitting {
            return;
        }
        let now = Instant::now();
        if now >= self.next_focus {
            self.next_focus = now + FOCUS_PERIOD;
            if !self.asleep && !self.ending_session {
                self.session_eligible = self.observer.session_eligible();
            }
            self.reconcile_pause();
            if self.recording() {
                self.record_sample(now);
            }
        }
        if now >= self.next_pause_checkpoint {
            self.flush_pause(capture_clock());
            self.next_pause_checkpoint = now + PAUSE_CHECKPOINT;
        }
    }

    fn eligible(&self) -> bool {
        self.session_eligible && !self.asleep && !self.ending_session
    }

    fn recording(&self) -> bool {
        self.settings.is_recording() && self.eligible()
    }

    fn change_settings(&mut self, updated: Settings) -> Result<()> {
        anyhow::ensure!(
            updated.idle_minutes > 0,
            "Idle threshold must be at least one minute"
        );
        if self.settings == updated {
            return Ok(());
        }
        let startup_changed = updated.enabled
            && (!self.settings.enabled
                || self.settings.launch_at_sign_in != updated.launch_at_sign_in);
        if startup_changed {
            if let Err(error) = startup::configure(updated.launch_at_sign_in) {
                self.startup_problem = Some(error.to_string());
                return Err(error);
            }
            self.startup_problem = None;
        }
        if let Err(error) = self.store.save_settings(&updated) {
            self.settings_problem = Some("Could not save recorder settings".into());
            if startup_changed {
                self.startup_problem =
                    startup::configure(self.settings.enabled && self.settings.launch_at_sign_in)
                        .err()
                        .map(|e| e.to_string());
            }
            return Err(error.into());
        }
        self.settings_problem = None;
        let mut previous_capture = self.settings.clone();
        previous_capture.launch_at_sign_in = updated.launch_at_sign_in;
        let capture_changed = previous_capture != updated;
        self.settings = updated;
        if capture_changed {
            self.pending_fingerprints.clear();
            self.reconcile_pause();
            self.reset_observation();
        }
        Ok(())
    }

    fn problem(&self) -> Option<&str> {
        self.recording_problem
            .as_deref()
            .or(self.settings_problem.as_deref())
            .or(self.startup_problem.as_deref())
            .or(self.dashboard_problem.as_deref())
    }

    fn indicator(&self) -> Indicator {
        if self.problem().is_some() || !self.settings.enabled {
            Indicator::Attention
        } else if self.recording() {
            Indicator::Recording
        } else {
            Indicator::Paused
        }
    }

    fn control(&mut self, request: ControlRequest) -> ControlResponse {
        if self.quitting {
            return ControlResponse::Error("The recorder is shutting down".into());
        }
        let (request, token) = match request {
            ControlRequest::TrackCorrection { token, command } if command.is_app_correction() => {
                (*command, Some(token))
            }
            request => (request, None),
        };
        let is_correction = request.is_app_correction();
        // A correction that changed nothing gets no undo token: undoing it would revert the
        // previous, real correction.
        let mut recorded = true;
        let result = match request {
            ControlRequest::Status => {
                let now = Instant::now();
                if self.eligible()
                    && !self.recording()
                    && self
                        .last_candidates
                        .is_none_or(|last| now.duration_since(last) >= PROCESS_PERIOD)
                    && let Some(apps) = self.observer.sample(true).processes
                {
                    self.candidates = apps;
                    self.last_candidates = Some(now);
                }
                return ControlResponse::Status(RecorderStatus {
                    settings: self.settings.clone(),
                    candidates: self.candidates.clone(),
                    problem: self.problem().map(str::to_owned),
                    recording: self.recording(),
                    capabilities: vec![
                        CAPABILITY_RESET_APP.into(),
                        CAPABILITY_UNGROUP_APP.into(),
                        CAPABILITY_TRACKED_UNDO.into(),
                    ],
                });
            }
            ControlRequest::SetSettings(settings) => self.change_settings(settings),
            ControlRequest::SetPaused(paused) => {
                let mut updated = self.settings.clone();
                updated.paused = paused;
                self.change_settings(updated)
            }
            ControlRequest::EditApp {
                key,
                name,
                category,
            } => self
                .store
                .edit_app(&key, &name, category)
                .map(|created| recorded = created)
                .map_err(Into::into),
            ControlRequest::GroupApps { source, target } => {
                self.store.group_apps(&source, &target).map_err(Into::into)
            }
            ControlRequest::TrackCorrection { .. } => {
                Err(anyhow::anyhow!("Only an app correction can be tracked"))
            }
            ControlRequest::UndoTrackedCorrection { token }
                if self.undo_tokens.last() != Some(&token) =>
            {
                Err(anyhow::anyhow!(
                    "That change is no longer the latest app change"
                ))
            }
            ControlRequest::UndoCorrection | ControlRequest::UndoTrackedCorrection { .. } => {
                match self.store.undo_correction() {
                    Ok(true) => {
                        self.undo_tokens.pop();
                        Ok(())
                    }
                    Ok(false) => Err(anyhow::anyhow!("There are no app corrections to undo")),
                    Err(error) => Err(error.into()),
                }
            }
            ControlRequest::DeleteHistory { key } => {
                // Deletion can prune snapshots from any point in the correction stack.
                self.undo_tokens.clear();
                self.store
                    .delete_history(key.as_deref())
                    .map(|_| {
                        self.fingerprinted.clear();
                        self.pending_fingerprints.clear();
                        self.reset_observation();
                        if self.pause_start.is_some() && key.is_none() {
                            self.pause_start = Some(capture_clock());
                        }
                    })
                    .map_err(Into::into)
            }
            ControlRequest::Backup { path } => self.store.backup(&path).map_err(Into::into),
            ControlRequest::ResetApp { key } => self.store.reset_app(&key).map_err(Into::into),
            ControlRequest::UngroupApp { key } => self.store.ungroup_app(&key).map_err(Into::into),
            ControlRequest::Quit => {
                self.quitting = true;
                self.flush_pause(capture_clock());
                Ok(())
            }
        };
        match result {
            Ok(()) => {
                if is_correction && recorded {
                    match token {
                        Some(token) => self.undo_tokens.push(token),
                        None => self.undo_tokens.clear(),
                    }
                }
                success_reply(is_correction, recorded)
            }
            Err(error) => ControlResponse::Error(error.to_string()),
        }
    }

    fn open_dashboard(&mut self) {
        let result = self.activate_or_launch_dashboard();
        self.dashboard_problem = result.err().map(|_| "Could not open the dashboard".into());
    }

    fn activate_or_launch_dashboard(&mut self) -> Result<()> {
        if let Some(child) = &mut self.dashboard
            && child.try_wait()?.is_none()
        {
            if let Some(window) = dashboard_window(child.id()) {
                window.activate();
            }
            return Ok(());
        }
        let executable = std::env::current_exe()?.with_file_name("tempora.exe");
        let child = Command::new(executable).spawn()?;
        win::allow_set_foreground(child.id());
        self.dashboard = Some(child);
        Ok(())
    }

    fn close_dashboard(&mut self) {
        let Some(mut child) = self.dashboard.take() else {
            return;
        };
        let started = Instant::now();
        let mut close_sent = false;
        while started.elapsed() < Duration::from_secs(2) && matches!(child.try_wait(), Ok(None)) {
            if !close_sent && let Some(window) = dashboard_window(child.id()) {
                close_sent = window.request_close();
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if matches!(child.try_wait(), Ok(None)) && child.kill().is_ok() {
            let _ = child.wait();
        }
    }
}

/// The dashboard's visible top-level window.
fn dashboard_window(pid: u32) -> Option<Window> {
    Window::top_level()
        .into_iter()
        .find(|window| window.process_id() == Some(pid) && window.is_visible())
}

/// The reply to a request that succeeded. A correction that recorded nothing answers
/// `Unchanged`, so the dashboard keeps its undo stack rather than storing a token for it.
fn success_reply(correction: bool, recorded: bool) -> ControlResponse {
    if correction && !recorded {
        ControlResponse::Unchanged
    } else {
        ControlResponse::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correction_that_recorded_nothing_replies_unchanged() {
        assert!(matches!(
            success_reply(true, false),
            ControlResponse::Unchanged
        ));
        assert!(matches!(success_reply(true, true), ControlResponse::Ok));
        assert!(matches!(success_reply(false, true), ControlResponse::Ok));
    }
}
