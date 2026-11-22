use crate::model::{AppIdentity, Interval, IntervalKind, Settings};
use chrono::Local;
use std::{collections::BTreeMap, time::Duration};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::WindowsObserver;

const MAX_SAMPLE_SPACING_MS: i64 = 6_000;
const MAX_CLOCK_DRIFT_MS: u64 = 1_000;

pub enum Focus {
    App(AppIdentity),
    Desktop,
    Unknown,
}

/// A point-in-time sample with both a monotonic clock and a wall clock.
pub struct Observation {
    pub monotonic: Duration,
    pub utc_ms: i64,
    pub local_offset_seconds: i32,
    pub focus: Focus,
    pub idle_age: Option<Duration>,
    /// Between continuous samples, `None` keeps the previous process list and `Some` replaces it.
    pub processes: Option<Vec<AppIdentity>>,
}

pub fn capture_clock() -> (i64, i32) {
    let now = Local::now();
    (now.timestamp_millis(), now.offset().local_minus_utc())
}

struct Previous {
    sample: Observation,
    projected_utc_ms: i64,
    idle_threshold: Duration,
}

impl Previous {
    /// Projects from the previous boundary when the two samples are close and their clocks agree.
    fn project_continuous_end(&self, sample: &Observation) -> Option<i64> {
        // Check the full durations before rounding to milliseconds.
        if sample.monotonic < self.sample.monotonic {
            return None;
        }
        let elapsed_ms = monotonic_ms(sample.monotonic) - monotonic_ms(self.sample.monotonic);
        if elapsed_ms > MAX_SAMPLE_SPACING_MS {
            return None;
        }
        let end_ms = self.projected_utc_ms.saturating_add(elapsed_ms);
        (end_ms.abs_diff(sample.utc_ms) <= MAX_CLOCK_DRIFT_MS).then_some(end_ms)
    }
}

/// Builds history intervals from successive observations.
#[derive(Default)]
pub struct Engine {
    previous: Option<Previous>,
    present: BTreeMap<String, AppIdentity>,
    latest_verified_utc_ms: Option<i64>,
}

impl Engine {
    /// Discards the previous sample and process list while retaining the UTC boundary.
    pub fn suspend(&mut self) {
        self.previous = None;
        self.present.clear();
    }

    /// Returns the latest accepted sample boundary, including one that produced no intervals.
    pub fn latest_verified_utc_ms(&self) -> Option<i64> {
        self.latest_verified_utc_ms
    }

    /// While recording, closes the interval following the previous sample and saves this sample
    /// for the next call. When recording is disabled or paused, suspends the engine.
    /// A forward discontinuity emits a gap. After a backwards jump, samples are ignored until
    /// wall time catches up to the last accepted boundary.
    pub fn observe(&mut self, mut sample: Observation, settings: &Settings) -> Vec<Interval> {
        if !settings.is_recording() {
            self.suspend();
            return Vec::new();
        }
        if self.previous.is_none()
            && self
                .latest_verified_utc_ms
                .is_some_and(|end| sample.utc_ms < end)
        {
            return Vec::new();
        }

        let mut projected_utc_ms = sample.utc_ms;
        let mut output = Vec::new();
        if let Some(previous) = self.previous.take() {
            let start_ms = previous.projected_utc_ms;
            if let Some(end_ms) = previous.project_continuous_end(&sample) {
                self.append_activity(&previous, &sample, end_ms, settings, &mut output);
                projected_utc_ms = end_ms;
            } else {
                self.present.clear();
                if sample.utc_ms < start_ms {
                    // Wait for civil time to catch up before accepting overlapping observations.
                    return output;
                }
                if start_ms < sample.utc_ms {
                    output.push(Interval {
                        start_ms,
                        end_ms: sample.utc_ms,
                        offset_seconds: previous.sample.local_offset_seconds,
                        kind: IntervalKind::Gap,
                    });
                }
            }
        }

        if let Some(processes) = sample.processes.take() {
            self.present = processes
                .into_iter()
                .filter(|identity| settings.allows(&identity.key))
                .map(|identity| (identity.key.clone(), identity))
                .collect();
        }
        self.latest_verified_utc_ms = Some(projected_utc_ms);
        self.previous = Some(Previous {
            sample,
            projected_utc_ms,
            idle_threshold: Duration::from_secs(u64::from(settings.idle_minutes) * 60),
        });
        output
    }

    fn append_activity(
        &self,
        previous: &Previous,
        sample: &Observation,
        end_ms: i64,
        settings: &Settings,
        output: &mut Vec<Interval>,
    ) {
        let start_ms = previous.projected_utc_ms;
        if end_ms <= start_ms {
            return;
        }
        let offset_seconds = previous.sample.local_offset_seconds;
        let mut push = |kind| {
            output.push(Interval {
                start_ms,
                end_ms,
                offset_seconds,
                kind,
            })
        };
        let focused = match &previous.sample.focus {
            Focus::App(app) if settings.allows(&app.key) => {
                push(IntervalKind::Focus(app.clone()));
                Some(app)
            }
            Focus::Desktop => {
                push(IntervalKind::Desktop);
                None
            }
            Focus::App(_) | Focus::Unknown => {
                push(IntervalKind::Unknown);
                None
            }
        };
        for app in self
            .present
            .values()
            .filter(|app| settings.allows(&app.key))
        {
            push(IntervalKind::Presence(app.clone()));
        }
        if let Some(app) = focused.filter(|app| !self.present.contains_key(&app.key)) {
            push(IntervalKind::Presence(app.clone()));
        }
        if let Some(age) = previous.sample.idle_age {
            let idle_start =
                start_ms.saturating_add(monotonic_ms(previous.idle_threshold.saturating_sub(age)));
            let mut idle_end = end_ms;
            if let Some(current_age) = sample.idle_age {
                let since_input = monotonic_ms(current_age);
                if current_age < age && since_input < end_ms - start_ms {
                    idle_end = end_ms - since_input;
                }
            }
            if idle_start < idle_end {
                output.push(Interval {
                    start_ms: idle_start,
                    end_ms: idle_end,
                    offset_seconds,
                    kind: IntervalKind::Idle,
                });
            }
        }
    }
}

fn monotonic_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Category;

    fn app(name: &str) -> AppIdentity {
        AppIdentity {
            key: name.into(),
            name: name.into(),
            category: Category::Other,
        }
    }

    fn enabled() -> Settings {
        Settings {
            enabled: true,
            ..Settings::default()
        }
    }

    fn sample(second: u64, focus: Focus) -> Observation {
        Observation {
            monotonic: Duration::from_secs(second),
            utc_ms: second as i64 * 1_000,
            local_offset_seconds: 3_600,
            focus,
            idle_age: None,
            processes: None,
        }
    }

    #[test]
    fn focused_sightings_establish_presence_and_duplicates_share_one_running_interval() {
        let settings = enabled();
        let game = app("Game");
        let browser = app("Browser");
        let mut engine = Engine::default();
        let mut game_focus_ms = 0;
        let mut game_presence_ms = 0;
        let mut browser_presence_ms = 0;
        for second in (0..=90 * 60).step_by(2) {
            let focus = match second {
                0..=1799 | 2700..=5399 => Focus::App(game.clone()),
                1800..=2699 => Focus::App(browser.clone()),
                _ => Focus::Unknown,
            };
            let mut observation = sample(second, focus);
            if second % 10 == 0 {
                observation.processes = Some(vec![game.clone(), game.clone()]);
            }
            for interval in engine.observe(observation, &settings) {
                let duration = interval.end_ms - interval.start_ms;
                match interval.kind {
                    IntervalKind::Focus(app) if app == game => game_focus_ms += duration,
                    IntervalKind::Presence(app) if app == game => game_presence_ms += duration,
                    IntervalKind::Presence(app) if app == browser => {
                        browser_presence_ms += duration
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(game_focus_ms, 75 * 60_000);
        assert_eq!(game_presence_ms, 90 * 60_000);
        assert_eq!(browser_presence_ms, 15 * 60_000);
    }

    #[test]
    fn clock_jump_and_suspend_clear_stale_presence() {
        let settings = enabled();
        let mut engine = Engine::default();
        let mut first = sample(0, Focus::Unknown);
        first.processes = Some(vec![app("Tray app")]);
        assert!(engine.observe(first, &settings).is_empty());
        let mut jumped = sample(2, Focus::Unknown);
        jumped.utc_ms += 10_000;
        let gap = engine.observe(jumped, &settings);
        assert_eq!(gap.len(), 1);
        assert_eq!(gap[0].kind, IntervalKind::Gap);
        assert_eq!((gap[0].start_ms, gap[0].end_ms), (0, 12_000));
        let mut next = sample(4, Focus::Unknown);
        next.utc_ms += 10_000;
        let unknown = engine.observe(next, &settings);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].kind, IntervalKind::Unknown);
        engine.suspend();
        assert!(
            engine
                .observe(sample(8, Focus::Desktop), &settings)
                .is_empty()
        );
    }

    #[test]
    fn the_spacing_and_drift_limits_are_inclusive() {
        let settings = enabled();
        let mut engine = Engine::default();
        assert!(
            engine
                .observe(sample(0, Focus::Desktop), &settings)
                .is_empty()
        );

        let mut at_limits = sample(6, Focus::Desktop);
        at_limits.utc_ms = 7_000;
        at_limits.local_offset_seconds = 7_200;
        assert_eq!(
            engine.observe(at_limits, &settings),
            [Interval {
                start_ms: 0,
                end_ms: 6_000,
                offset_seconds: 3_600,
                kind: IntervalKind::Desktop,
            }]
        );

        let mut too_much_drift = sample(12, Focus::Desktop);
        too_much_drift.utc_ms = 13_001;
        assert_eq!(
            engine.observe(too_much_drift, &settings),
            [Interval {
                start_ms: 6_000,
                end_ms: 13_001,
                offset_seconds: 7_200,
                kind: IntervalKind::Gap,
            }]
        );

        let mut too_far_apart = sample(18, Focus::Desktop);
        too_far_apart.monotonic += Duration::from_millis(1);
        too_far_apart.utc_ms = 19_002;
        assert_eq!(
            engine.observe(too_far_apart, &settings),
            [Interval {
                start_ms: 13_001,
                end_ms: 19_002,
                offset_seconds: 3_600,
                kind: IntervalKind::Gap,
            }]
        );
    }

    #[test]
    fn backwards_clock_jump_waits_for_last_verified_time() {
        let settings = enabled();
        let mut engine = Engine::default();
        let mut first = sample(0, Focus::Desktop);
        first.utc_ms = 10_000;
        engine.observe(first, &settings);
        let mut second = sample(2, Focus::Desktop);
        second.utc_ms = 12_000;
        assert_eq!(engine.observe(second, &settings)[0].end_ms, 12_000);
        assert!(
            engine
                .observe(sample(4, Focus::Desktop), &settings)
                .is_empty()
        );
        engine.suspend();
        assert!(
            engine
                .observe(sample(6, Focus::Desktop), &settings)
                .is_empty()
        );
        assert_eq!(engine.latest_verified_utc_ms(), Some(12_000));
        assert!(
            engine
                .observe(sample(12, Focus::Desktop), &settings)
                .is_empty()
        );
        let resumed = engine.observe(sample(14, Focus::Desktop), &settings);
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].kind, IntervalKind::Desktop);
        assert_eq!((resumed[0].start_ms, resumed[0].end_ms), (12_000, 14_000));
    }

    #[test]
    fn idle_starts_at_threshold_and_exclusion_keeps_focus_anonymous() {
        let mut settings = enabled();
        let game = app("Game");
        settings.excluded.insert(game.key.clone());
        let mut engine = Engine::default();
        let mut first = sample(0, Focus::App(game.clone()));
        first.idle_age = Some(Duration::from_secs(299));
        first.processes = Some(vec![game]);
        engine.observe(first, &settings);
        settings.idle_minutes = 10;
        let mut next = sample(2, Focus::Desktop);
        next.idle_age = Some(Duration::from_secs(301));
        let intervals = engine.observe(next, &settings);
        assert_eq!(intervals.len(), 2);
        assert_eq!(intervals[0].kind, IntervalKind::Unknown);
        assert_eq!(intervals[1].kind, IntervalKind::Idle);
        assert_eq!((intervals[1].start_ms, intervals[1].end_ms), (1_000, 2_000));
        assert_eq!(intervals[1].offset_seconds, 3_600);
    }
}
