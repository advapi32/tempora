//! Blocking dashboard jobs, run in order on a single background thread.

use super::{format, icons, nav};
use chrono::{Local, NaiveDate};
use chrono_tz::Tz;
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use tempora::{
    model::{AppIdentity, AppKey, ControlRequest, DateRange, RecorderStatus, database_path},
    pipe,
    storage::{AppSummary, Report, Store},
};

/// Projections computed with each range report.
#[derive(Default)]
pub(super) struct Extras {
    /// Focused time in the preceding equivalent period and whether it was clipped to the
    /// same elapsed point because the current period is in progress.
    pub previous: Option<(i64, bool)>,
    pub bounds: Option<(i64, i64)>,
    pub last_seen: HashMap<String, i64>,
    /// Daily focused time for the seven days ending at the range's last elapsed day.
    pub week: HashMap<String, [i64; 7]>,
    pub intervals: Option<i64>,
}

/// Stored identities and all-time summary of one effective app.
pub(super) struct AppInfo {
    pub all_time: Option<AppSummary>,
    pub sources: Vec<String>,
    /// Recognized name and category, before user edits.
    pub automatic: Option<(String, tempora::model::Category)>,
}

pub(super) struct DeletePreview {
    pub name: String,
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub sessions: usize,
    pub focused_ms: i64,
    /// Daily running time for an app, or daily focused time for all history.
    pub days: Option<Vec<(NaiveDate, i64)>>,
}

pub(super) struct RangeReport {
    pub(super) report: Report,
    pub(super) apps: Vec<AppIdentity>,
    pub(super) extras: Extras,
}

pub(super) enum Job {
    Status,
    TodayFocus(NaiveDate),
    Report(DateRange, bool),
    DayReport(NaiveDate, bool),
    AppInfo(String),
    DeletePreview(Option<String>),
    Exclude(String),
    Control(ControlRequest, &'static str),
    Export(PathBuf, DateRange, bool),
    Backup(PathBuf),
    Icon(String),
    Poll,
    Covered(NaiveDate),
}

pub(super) enum Update {
    Status(Result<RecorderStatus, pipe::RequestError>),
    TodayFocus(NaiveDate, Option<i64>),
    Report(DateRange, bool, Result<RangeReport, String>),
    DayReport(NaiveDate, bool, Result<Report, String>),
    AppInfo(String, Result<AppInfo, String>),
    DeletePreview(Option<String>, Result<DeletePreview, String>),
    Done(ControlKind, Result<&'static str, String>),
    Exported(Result<PathBuf, String>),
    BackedUp(Result<PathBuf, String>),
    Icon(String, Option<egui::ColorImage>),
    Version(Option<i64>),
    Covered(NaiveDate, Option<HashSet<NaiveDate>>),
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ControlKind {
    Correction(Option<u64>),
    /// A correction the recorder saved nothing for. It has no undo step to offer.
    Unchanged,
    Undo,
    Settings,
    Other,
}

impl ControlKind {
    fn of(request: &ControlRequest) -> Self {
        match request {
            ControlRequest::TrackCorrection { token, .. } => Self::Correction(Some(*token)),
            request if request.is_app_correction() => Self::Correction(None),
            ControlRequest::UndoCorrection | ControlRequest::UndoTrackedCorrection { .. } => {
                Self::Undo
            }
            ControlRequest::SetSettings(_) => Self::Settings,
            _ => Self::Other,
        }
    }

    /// The kind once the recorder has answered. A correction that changed nothing is `Unchanged`:
    /// undoing it would revert the previous, real correction, so it gets no undo token.
    fn settled(self, changed: bool) -> Self {
        match self {
            Self::Correction(_) if !changed => Self::Unchanged,
            kind => kind,
        }
    }
}

pub(super) struct Worker {
    zone: Tz,
    /// Reuse one read-only connection so comparisons of PRAGMA data_version are meaningful.
    store: Option<Store>,
}

impl Worker {
    pub(super) fn new(zone: Tz) -> Self {
        Self { zone, store: None }
    }

    fn store(&mut self) -> Result<&Store, String> {
        if self.store.is_none() {
            let path = database_path().map_err(|error| error.to_string())?;
            self.store = Some(Store::open_reader(&path).map_err(|error| error.to_string())?);
        }
        Ok(self
            .store
            .as_ref()
            .expect("the read-only connection just opened or was already cached"))
    }

    fn exclude_app(&mut self, key: &str) -> Result<&'static str, String> {
        let mut settings = pipe::status().map_err(|error| error.to_string())?.settings;
        let sources = self
            .store()?
            .source_keys(key)
            .map_err(|error| error.to_string())?;
        settings.excluded.extend(sources);
        control(&ControlRequest::SetSettings(settings))?;
        Ok("App excluded from future recording")
    }

    pub(super) fn run(&mut self, job: Job) -> Update {
        let zone = self.zone;
        match job {
            Job::Status => Update::Status(pipe::status()),
            Job::TodayFocus(date) => {
                let focus = self
                    .store()
                    .and_then(|store| {
                        store
                            .report(day_range(date), zone, false)
                            .map_err(|error| error.to_string())
                    })
                    .ok()
                    .filter(|report| report.coverage.is_some())
                    .map(|report| {
                        report
                            .categories
                            .iter()
                            .map(|category| category.totals.focused_ms)
                            .sum()
                    });
                Update::TodayFocus(date, focus)
            }
            Job::Report(range, exclude) => Update::Report(
                range,
                exclude,
                (|| {
                    let store = self.store()?;
                    let report = store
                        .report(range, zone, exclude)
                        .map_err(|error| error.to_string())?;
                    let apps = store.apps().map_err(|error| error.to_string())?;
                    let extras = extras(store, range, zone, exclude);
                    Ok(RangeReport {
                        report,
                        apps,
                        extras,
                    })
                })(),
            ),
            Job::DayReport(date, exclude) => Update::DayReport(
                date,
                exclude,
                self.store().and_then(|store| {
                    store
                        .report(day_range(date), zone, exclude)
                        .map_err(|error| error.to_string())
                }),
            ),
            Job::AppInfo(key) => {
                let result = self.store().and_then(|store| {
                    let all_time = store.preview(&key).map_err(|error| error.to_string())?;
                    let sources = store.source_keys(&key).map_err(|error| error.to_string())?;
                    let automatic = store.automatic_identity(&key).ok().flatten();
                    Ok(AppInfo {
                        all_time,
                        sources,
                        automatic,
                    })
                });
                Update::AppInfo(key, result)
            }
            Job::Control(command, success) => {
                let kind = ControlKind::of(&command);
                match pipe::command_changed(&command) {
                    Ok(changed) => Update::Done(kind.settled(changed), Ok(success)),
                    Err(error) => Update::Done(kind, Err(error.to_string())),
                }
            }
            Job::Export(path, range, exclude) => Update::Exported(self.store().and_then(|store| {
                store
                    .export_csv(&path, range, zone, exclude)
                    .map(|()| path)
                    .map_err(|error| error.to_string())
            })),
            Job::Backup(path) => {
                let command = ControlRequest::Backup { path: path.clone() };
                Update::BackedUp(control(&command).map(|()| path))
            }
            Job::Exclude(key) => Update::Done(ControlKind::Settings, self.exclude_app(&key)),
            Job::Covered(month) => {
                let days = (|| {
                    let end = nav::shift_month(month, 1)?.pred_opt()?;
                    let store = self.store().ok()?;
                    let report = store
                        .report(DateRange { start: month, end }, zone, false)
                        .ok()?;
                    Some(
                        report
                            .days
                            .iter()
                            .filter(|day| day.covered_ms > 0)
                            .map(|day| day.date)
                            .collect(),
                    )
                })();
                Update::Covered(month, days)
            }
            Job::Poll => {
                let version = self
                    .store()
                    .and_then(|store| store.data_version().map_err(|error| error.to_string()));
                if version.is_err() {
                    self.store = None;
                }
                Update::Version(version.ok())
            }
            Job::Icon(key) => {
                let image = self
                    .icon_candidates(&key)
                    .iter()
                    .find_map(|path| icons::extract(path))
                    .or_else(|| {
                        // Store apps often have no usable icon in their executables; ask the
                        // shell for the one the Start menu shows.
                        std::iter::once(key.clone())
                            .chain(
                                self.store()
                                    .and_then(|store| {
                                        store.source_keys(&key).map_err(|e| e.to_string())
                                    })
                                    .unwrap_or_default(),
                            )
                            .find_map(|source| {
                                AppKey::parse(&source)
                                    .package()
                                    .and_then(icons::package_icon)
                            })
                    });
                Update::Icon(key, image)
            }
            Job::DeletePreview(key) => {
                let preview = self
                    .store()
                    .and_then(|store| delete_preview(store, key.as_deref(), zone));
                Update::DeletePreview(key, preview)
            }
        }
    }

    /// Executables that may carry an app's icon, best first: recorded executables of the key
    /// and the identities grouped under it, then package, Steam, Riot and product executables.
    fn icon_candidates(&mut self, key: &str) -> Vec<PathBuf> {
        let mut keys = vec![key.to_owned()];
        if let Ok(sources) = self
            .store()
            .and_then(|store| store.source_keys(key).map_err(|error| error.to_string()))
        {
            keys.extend(sources);
        }
        let mut candidates: Vec<PathBuf> = keys
            .iter()
            .filter_map(|source| format::source(source).path.map(PathBuf::from))
            .collect();
        for package in keys
            .iter()
            .filter_map(|source| AppKey::parse(source).package())
        {
            candidates.extend(icons::package_executables(package));
        }
        for app_id in keys
            .iter()
            .filter_map(|source| AppKey::parse(source).steam_id())
        {
            candidates.extend(icons::steam_executables(app_id));
        }
        for id in keys
            .iter()
            .filter_map(|source| AppKey::parse(source).riot_id())
        {
            candidates.extend(icons::riot_executables(id));
        }
        for id in keys
            .iter()
            .filter_map(|source| AppKey::parse(source).product())
        {
            candidates.extend(icons::product_executables(id));
        }
        candidates
    }
}

/// Focused time in the period before `range`: the previous month for a whole month,
/// otherwise the same number of days. An in-progress range compares up to the same
/// elapsed point.
fn previous_focus(store: &Store, range: DateRange, zone: Tz, exclude: bool) -> Option<(i64, bool)> {
    use chrono::Datelike;
    let days = (range.end - range.start).num_days() + 1;
    let whole_month = range.start.day() == 1
        && range.end.succ_opt().is_some_and(|next| next.day() == 1)
        && range.start.month() == range.end.month();
    let previous = if whole_month {
        let start = nav::shift_month(range.start, -1)?;
        DateRange {
            start,
            end: range.start.pred_opt()?,
        }
    } else {
        DateRange {
            start: range.start - chrono::Days::new(days as u64),
            end: range.start.pred_opt()?,
        }
    };
    let start = format::day_start(previous.start, zone);
    let end = format::day_start(previous.end.succ_opt()?, zone);
    let now = chrono::Utc::now().timestamp_millis();
    let current_start = format::day_start(range.start, zone);
    let current_end = format::day_start(range.end.succ_opt()?, zone);
    let partial = current_start <= now && now < current_end;
    let end = if partial {
        (start + (now - current_start)).min(end)
    } else {
        end
    };
    store
        .focused_between(start, end, exclude)
        .ok()
        .map(|focused| (focused, partial))
}

/// Daily focused time per app for the seven days ending at the range's last elapsed day.
fn week_buckets(store: &Store, range: DateRange, zone: Tz) -> HashMap<String, [i64; 7]> {
    let end = range.end.min(Local::now().date_naive());
    let week = DateRange {
        start: end - chrono::Days::new(6),
        end,
    };
    let Ok(report) = store.report(week, zone, false) else {
        return HashMap::new();
    };
    report
        .apps
        .into_iter()
        .map(|app| {
            let mut days = [0; 7];
            for (index, (_, totals)) in app.days.iter().take(7).enumerate() {
                days[index] = totals.focused_ms;
            }
            (app.key, days)
        })
        .collect()
}

fn extras(store: &Store, range: DateRange, zone: Tz, exclude: bool) -> Extras {
    Extras {
        previous: previous_focus(store, range, zone, exclude),
        bounds: store.history_bounds().ok().flatten(),
        last_seen: store.last_seen_all().unwrap_or_default(),
        week: week_buckets(store, range, zone),
        intervals: store.interval_count().ok(),
    }
}

fn control(command: &ControlRequest) -> Result<(), String> {
    pipe::command(command).map_err(|error| error.to_string())
}

fn day_range(date: NaiveDate) -> DateRange {
    DateRange {
        start: date,
        end: date,
    }
}

/// Running time per local day, splitting sessions at midnight, first to last day.
fn session_days(sessions: &[tempora::storage::Session], zone: Tz) -> Vec<(NaiveDate, i64)> {
    let mut days = std::collections::BTreeMap::<NaiveDate, i64>::new();
    for session in sessions {
        let mut at = session.start_ms;
        while at < session.end_ms {
            let Some(date) = format::local_date(at, zone) else {
                break;
            };
            let Some(next_day) = date.succ_opt() else {
                break;
            };
            let next = format::day_start(next_day, zone)
                .min(session.end_ms)
                .max(at + 1);
            *days.entry(date).or_default() += next - at;
            at = next;
        }
    }
    let (Some(first), Some(last)) = (
        days.keys().next().copied(),
        days.keys().next_back().copied(),
    ) else {
        return Vec::new();
    };
    first
        .iter_days()
        .take_while(|date| *date <= last)
        .map(|date| (date, days.get(&date).copied().unwrap_or(0)))
        .collect()
}

fn delete_preview(store: &Store, key: Option<&str>, zone: Tz) -> Result<DeletePreview, String> {
    if let Some(key) = key {
        let app = store
            .preview(key)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "This app is no longer in the library.".to_string())?;
        return Ok(DeletePreview {
            name: app.name,
            first_ms: app.sessions.first().map(|session| session.start_ms),
            last_ms: Some(app.last_seen_ms),
            sessions: app.sessions.len(),
            focused_ms: app.totals.focused_ms,
            days: Some(session_days(&app.sessions, zone)),
        });
    }
    let range = DateRange {
        start: NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid date"),
        end: Local::now().date_naive(),
    };
    let report = store
        .report(range, zone, false)
        .map_err(|error| error.to_string())?;
    let days = report.coverage.map(|(start, end)| {
        let (first, last) = (
            format::local_date(start, zone),
            format::local_date(end, zone),
        );
        report
            .days
            .iter()
            .filter(|day| first.is_none_or(|first| day.date >= first))
            .filter(|day| last.is_none_or(|last| day.date <= last))
            .map(|day| {
                let focused = day.categories.iter().map(|row| row.totals.focused_ms).sum();
                (day.date, focused)
            })
            .collect()
    });
    Ok(DeletePreview {
        name: "All recorded activity".into(),
        first_ms: report.coverage.map(|bounds| bounds.0),
        last_ms: report.coverage.map(|bounds| bounds.1),
        sessions: report.apps.iter().map(|app| app.sessions.len()).sum(),
        focused_ms: report
            .categories
            .iter()
            .map(|row| row.totals.focused_ms)
            .sum(),
        days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correction_that_changed_nothing_settles_as_unchanged() {
        assert!(matches!(
            ControlKind::Correction(Some(7)).settled(false),
            ControlKind::Unchanged
        ));
        assert!(matches!(
            ControlKind::Correction(None).settled(false),
            ControlKind::Unchanged
        ));
        assert!(matches!(
            ControlKind::Correction(Some(7)).settled(true),
            ControlKind::Correction(Some(7))
        ));
        assert!(matches!(ControlKind::Undo.settled(true), ControlKind::Undo));
    }

    #[test]
    fn delete_previews_show_app_running_time_and_overall_focused_time() -> anyhow::Result<()> {
        use tempora::model::{Category, Interval, IntervalKind};

        let store = Store::open_writer(std::path::Path::new(":memory:"))?;
        let app = AppIdentity {
            key: "game".into(),
            name: "Game".into(),
            category: Category::Games,
        };
        let first = NaiveDate::from_ymd_opt(2024, 11, 1).unwrap();
        let second = first.succ_opt().unwrap();
        let zone = chrono_tz::UTC;
        let midnight = format::day_start(second, zone);
        store.record(&[
            Interval {
                start_ms: midnight - 60_000,
                end_ms: midnight,
                offset_seconds: 0,
                kind: IntervalKind::Focus(app.clone()),
            },
            Interval {
                start_ms: midnight - 60_000,
                end_ms: midnight + 60_000,
                offset_seconds: 0,
                kind: IntervalKind::Presence(app),
            },
        ])?;

        let app = delete_preview(&store, Some("game"), zone).map_err(anyhow::Error::msg)?;
        assert_eq!(app.days, Some(vec![(first, 60_000), (second, 60_000)]));
        let all = delete_preview(&store, None, zone).map_err(anyhow::Error::msg)?;
        assert_eq!(all.days, Some(vec![(first, 60_000), (second, 0)]));
        assert_eq!(app.focused_ms, all.focused_ms);
        assert!(matches!(
            delete_preview(&store, Some("missing"), zone),
            Err(error) if error == "This app is no longer in the library."
        ));
        Ok(())
    }
}
