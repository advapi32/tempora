//! The recorder's sampling, recognition updates and history writes.

use super::{Host, PAUSE_CHECKPOINT, PROCESS_PERIOD};
use anyhow::Result;
use std::{
    collections::{BTreeSet, HashSet},
    time::Instant,
};
use tempora::{
    model::{AppIdentity, AppKey, Interval, IntervalKind, Settings},
    observation::{WindowsObserver, capture_clock},
    storage::{self, Store, write_problem},
};

pub(super) fn prepare_observer(
    store: &Store,
    settings: &Settings,
) -> Result<(WindowsObserver, HashSet<String>)> {
    let mut observer = WindowsObserver::new()?;
    let qualified = store.qualified_keys()?;
    observer.remember(qualified.iter().cloned());
    observer.remember_fullscreen(store.fullscreen_keys().unwrap_or_default());
    for key in qualified
        .iter()
        .filter(|key| AppKey::parse(key).package().is_some())
    {
        if let Some(identity) = observer.reidentify_package(key) {
            let _ = store.refresh_automatic(&identity.key, &identity.name, identity.category);
        }
    }
    // Settings can name executables with no history. Their Steam aliases need the same settings.
    let executable_keys = qualified
        .iter()
        .chain(&settings.excluded)
        .chain(&settings.selected)
        .filter(|key| AppKey::parse(key).is_executable())
        .collect::<BTreeSet<_>>();
    for key in executable_keys {
        if let Some(identity) = observer.reidentify(key) {
            refresh_executable(store, key, &identity)?;
        }
    }
    let fingerprinted = group_historical_executables(store, |key| observer.fingerprint(key))?;
    Ok((observer, fingerprinted))
}

impl Host {
    pub(super) fn record_sample(&mut self, now: Instant) {
        let include_processes = now >= self.next_processes;
        let sample = self.observer.sample(include_processes);
        // A new Steam key must inherit the original key's capture settings before observation.
        if let Err(error) = self.refresh_observed_steam_aliases() {
            self.recording_problem = Some(write_problem(&error).into());
            self.reset_observation();
            return;
        }
        if include_processes {
            if let Some(apps) = &sample.processes {
                self.candidates.clone_from(apps);
                self.last_candidates = Some(now);
            }
            self.next_processes = now + PROCESS_PERIOD;
        }
        let offset = sample.local_offset_seconds;
        let previous_boundary = self.engine.latest_verified_utc_ms();
        let mut intervals = self.engine.observe(sample, &self.settings);
        if let Err(error) = self.apply_group_exclusions(&mut intervals) {
            // An unavailable group lookup prevents the sample from being written.
            self.recording_problem = Some(write_problem(&error).into());
            self.reset_observation();
            return;
        }
        if !self.write_intervals(&intervals) {
            self.reset_observation();
            return;
        }

        if intervals
            .iter()
            .any(|interval| matches!(interval.kind, IntervalKind::Gap))
        {
            self.next_processes = now;
        }
        // The write above registers new apps. Group them before attaching fullscreen evidence.
        self.group_new_executables(&intervals);
        self.record_fullscreen_evidence();

        let boundary = self.engine.latest_verified_utc_ms();
        if boundary != previous_boundary || self.last_recording_boundary.is_none() {
            self.last_recording_boundary = boundary.map(|end| (end, offset));
        }
    }

    fn refresh_observed_steam_aliases(&mut self) -> storage::Result<()> {
        let aliases = self.observer.steam_aliases();
        if aliases.is_empty() {
            return Ok(());
        }
        for (key, identity) in aliases.iter() {
            self.store.migrate_steam_alias(key, identity)?;
        }
        let settings = self.store.settings()?;
        // Keep the aliases queued until all migrations and the settings reload succeed.
        aliases.clear();
        if settings != self.settings {
            self.settings = settings;
            self.pending_fingerprints.clear();
            self.reset_observation();
        }
        Ok(())
    }

    fn apply_group_exclusions(&mut self, intervals: &mut Vec<Interval>) -> storage::Result<()> {
        filter_group_exclusions(&self.store, &self.settings, intervals, |key| {
            if !AppKey::parse(key).is_executable() || self.fingerprinted.contains(key) {
                return None;
            }
            self.pending_fingerprints
                .entry(key.to_owned())
                .or_insert_with(|| self.observer.fingerprint(key))
                .clone()
        })
    }

    /// Stores fingerprints and groups executables with older installs when the evidence matches.
    /// Failed writes are retried on the next batch that contains the executable.
    fn group_new_executables(&mut self, intervals: &[Interval]) {
        let mut seen = HashSet::new();
        for key in intervals.iter().filter_map(interval_app_key) {
            if !AppKey::parse(key).is_executable()
                || self.fingerprinted.contains(key)
                || !seen.insert(key)
            {
                continue;
            }
            let fingerprint = self
                .pending_fingerprints
                .remove(key)
                .unwrap_or_else(|| self.observer.fingerprint(key));
            if self.store.auto_group(key, fingerprint.as_deref()).is_ok() {
                self.fingerprinted.insert(key.to_owned());
            }
        }
    }

    fn record_fullscreen_evidence(&mut self) {
        let Some(key) = self.observer.last_fullscreen() else {
            return;
        };
        if !self.settings.allows(key) {
            return;
        }
        if !self.settings.excluded.is_empty()
            && !matches!(
                self.store.group_is_excluded(key, None, &self.settings),
                Ok(false)
            )
        {
            return;
        }
        if !matches!(self.store.mark_fullscreen(key), Ok(true)) {
            return;
        }

        // New fullscreen evidence can change the app's category while it is still running.
        let key = key.to_owned();
        self.observer.confirm_fullscreen(&key);
        if let Some(identity) = self.observer.reidentify(&key) {
            let _ = self
                .store
                .refresh_automatic(&identity.key, &identity.name, identity.category);
        }
    }

    pub(super) fn reset_observation(&mut self) {
        self.engine.suspend();
        let now = Instant::now();
        self.next_focus = now;
        self.next_processes = now;
        self.last_recording_boundary = None;
    }

    pub(super) fn reconcile_pause(&mut self) {
        if self.settings.enabled && !self.recording() {
            if self.pause_start.is_none() {
                let now = capture_clock();
                self.pause_start = self.last_recording_boundary.take().or(Some(now));
                self.next_pause_checkpoint = Instant::now() + PAUSE_CHECKPOINT;
                self.reset_observation();
            }
        } else if self.pause_start.is_some() {
            self.flush_pause(capture_clock());
            self.pause_start = None;
            self.reset_observation();
        }
    }

    pub(super) fn flush_pause(&mut self, now: (i64, i32)) {
        let Some((start_ms, offset_seconds)) = self.pause_start else {
            return;
        };
        let (end_ms, _) = now;
        if end_ms < start_ms {
            return;
        }
        if end_ms > start_ms
            && !self.write_intervals(&[Interval {
                start_ms,
                end_ms,
                offset_seconds,
                kind: IntervalKind::Paused,
            }])
        {
            return;
        }
        self.pause_start = Some(now);
    }

    /// Returns true for a successful write or an empty batch. A successful write clears the error.
    fn write_intervals(&mut self, intervals: &[Interval]) -> bool {
        if intervals.is_empty() {
            return true;
        }
        match self.store.record(intervals) {
            Ok(()) => {
                self.recording_problem = None;
                true
            }
            Err(error) => {
                self.recording_problem = Some(write_problem(&error).into());
                false
            }
        }
    }
}

fn refresh_executable(store: &Store, key: &str, identity: &AppIdentity) -> Result<()> {
    if key == identity.key {
        let _ = store.refresh_automatic(key, &identity.name, identity.category);
    } else if AppKey::parse(&identity.key).is_steam() {
        store.migrate_steam_alias(key, identity)?;
    }
    Ok(())
}

fn group_historical_executables(
    store: &Store,
    mut fingerprint: impl FnMut(&str) -> Option<String>,
) -> Result<HashSet<String>> {
    let mut fingerprinted = HashSet::new();
    for key in store.executable_keys_oldest_first()? {
        if store.auto_group(&key, fingerprint(&key).as_deref()).is_ok() {
            fingerprinted.insert(key);
        }
    }
    Ok(fingerprinted)
}

/// Resolves every exclusion before mutating the batch. Excluded focus keeps its time but loses
/// its owner, while excluded presence is removed altogether.
fn filter_group_exclusions(
    store: &Store,
    settings: &Settings,
    intervals: &mut Vec<Interval>,
    mut fingerprint: impl FnMut(&str) -> Option<String>,
) -> storage::Result<()> {
    if settings.excluded.is_empty() {
        return Ok(());
    }
    let mut excluded = HashSet::new();
    for key in intervals
        .iter()
        .filter_map(interval_app_key)
        .collect::<HashSet<_>>()
    {
        let fingerprint = fingerprint(key);
        if store.group_is_excluded(key, fingerprint.as_deref(), settings)? {
            excluded.insert(key.to_owned());
        }
    }
    intervals.retain_mut(|interval| match &interval.kind {
        IntervalKind::Focus(app) if excluded.contains(&app.key) => {
            interval.kind = IntervalKind::Unknown;
            true
        }
        IntervalKind::Presence(app) => !excluded.contains(&app.key),
        _ => true,
    });
    Ok(())
}

fn interval_app_key(interval: &Interval) -> Option<&str> {
    match &interval.kind {
        IntervalKind::Presence(app) | IntervalKind::Focus(app) => Some(&app.key),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempora::model::{CaptureMode, Category};

    #[test]
    fn startup_steam_alias_preserves_capture_settings_and_history() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let old = AppIdentity {
            key: "exe:c:\\games\\game.exe".into(),
            name: "game".into(),
            category: Category::Other,
        };
        let steam = AppIdentity {
            key: "steam:123".into(),
            name: "Game".into(),
            category: Category::Games,
        };
        store.record(&[Interval {
            start_ms: 10,
            end_ms: 20,
            offset_seconds: 0,
            kind: IntervalKind::Focus(old.clone()),
        }])?;
        let mut settings = Settings {
            enabled: true,
            mode: CaptureMode::Selected,
            ..Settings::default()
        };
        settings.selected.insert(old.key.clone());
        settings.excluded.insert(old.key.clone());
        store.save_settings(&settings)?;

        refresh_executable(&store, &old.key, &steam)?;
        refresh_executable(&store, &old.key, &steam)?;
        let settings = store.settings()?;
        assert!(settings.selected.contains(&steam.key));
        assert!(settings.excluded.contains(&steam.key));
        assert!(settings.excluded.contains(&old.key));
        assert!(!settings.allows(&steam.key));
        assert_eq!(store.apps()?, std::slice::from_ref(&steam));
        assert_eq!(store.source_keys(&steam.key)?, [old.key, steam.key.clone()]);
        assert_eq!(
            store
                .preview(&steam.key)?
                .expect("legacy history")
                .totals
                .focused_ms,
            10
        );
        Ok(())
    }

    #[test]
    fn steam_alias_carries_a_standalone_user_edit_and_its_undo_to_the_new_key() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let old = AppIdentity {
            key: "exe:c:\\games\\game.exe".into(),
            name: "game".into(),
            category: Category::Other,
        };
        let steam = AppIdentity {
            key: "steam:123".into(),
            name: "Game".into(),
            category: Category::Games,
        };
        store.record(&[Interval {
            start_ms: 10,
            end_ms: 20,
            offset_seconds: 0,
            kind: IntervalKind::Focus(old.clone()),
        }])?;
        store.edit_app(&old.key, "My game", Category::Productivity)?;
        refresh_executable(&store, &old.key, &steam)?;
        assert_eq!(
            store.apps()?,
            [AppIdentity {
                name: "My game".into(),
                category: Category::Productivity,
                ..steam.clone()
            }]
        );
        assert!(store.undo_correction()?);
        assert_eq!(store.apps()?, [steam]);
        Ok(())
    }

    #[test]
    fn new_install_in_an_excluded_group_leaves_no_identified_history() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let original = AppIdentity {
            key: "exe:c:\\old\\game.exe".into(),
            name: "Game".into(),
            category: Category::Games,
        };
        let new = AppIdentity {
            key: "exe:c:\\new\\game.exe".into(),
            ..original.clone()
        };
        let other = AppIdentity {
            key: "exe:c:\\other\\tool.exe".into(),
            name: "Tool".into(),
            category: Category::Other,
        };
        let interval = |kind| Interval {
            start_ms: 10,
            end_ms: 20,
            offset_seconds: 0,
            kind,
        };
        store.record(&[interval(IntervalKind::Focus(original.clone()))])?;
        store.auto_group(&original.key, Some("title:7"))?;
        let mut settings = Settings::default();
        settings.excluded.insert(original.key.clone());
        let mut new_intervals = vec![
            interval(IntervalKind::Focus(new.clone())),
            interval(IntervalKind::Presence(new.clone())),
            interval(IntervalKind::Presence(other.clone())),
            interval(IntervalKind::Idle),
        ];
        filter_group_exclusions(&store, &settings, &mut new_intervals, |key| {
            (key == new.key).then(|| "title:7".into())
        })?;
        store.record(&new_intervals)?;

        assert_eq!(
            new_intervals,
            [
                interval(IntervalKind::Unknown),
                interval(IntervalKind::Presence(other.clone())),
                interval(IntervalKind::Idle),
            ]
        );
        assert_eq!(store.qualified_keys()?, [original.key, other.key]);
        Ok(())
    }

    #[test]
    fn historical_grouping_keeps_the_oldest_recorded_install() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let old = AppIdentity {
            key: "exe:z/game.exe".into(),
            name: "Original".into(),
            category: Category::Games,
        };
        let new = AppIdentity {
            key: "exe:a/game.exe".into(),
            name: "New install".into(),
            category: Category::Games,
        };
        for app in [&old, &new] {
            store.record(&[Interval {
                start_ms: 10,
                end_ms: 20,
                offset_seconds: 0,
                kind: IntervalKind::Presence(app.clone()),
            }])?;
        }
        group_historical_executables(&store, |_| Some("title:42".into()))?;
        assert_eq!(store.apps()?, std::slice::from_ref(&old));
        assert_eq!(store.source_keys(&old.key)?.len(), 2);
        Ok(())
    }
}
