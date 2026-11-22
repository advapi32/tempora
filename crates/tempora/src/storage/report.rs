//! Report accounting for recorded activity over UTC spans and local calendar days.

use super::{
    AppSummary, CategorySummary, DESKTOP, DaySummary, FOCUS, GAP, IDLE, PAUSED, PRESENCE, Report,
    Session, Store, TimelineEntry, TimelineKind, Totals, UNKNOWN,
};
use super::{Result, StorageError, ensure};
use crate::model::{Category, DateRange};
use chrono::{LocalResult, NaiveDate, TimeZone};
use chrono_tz::Tz;
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, btree_map::Entry};

impl Store {
    /// Reports activity in the inclusive range of local calendar dates in `zone`.
    /// `exclude_idle` subtracts estimated idle time from focused totals.
    pub fn report(&self, range: DateRange, zone: Tz, exclude_idle: bool) -> Result<Report> {
        let buckets = date_buckets(range, zone)?;
        let span = Span {
            start: buckets
                .first()
                .ok_or(StorageError::OutOfRange("Empty date range"))?
                .1
                .start,
            end: buckets
                .last()
                .ok_or(StorageError::OutOfRange("Empty date range"))?
                .1
                .end,
        };
        self.report_for_span(span, &buckets, exclude_idle)
    }

    pub(super) fn report_between(
        &self,
        start: i64,
        end: i64,
        exclude_idle: bool,
    ) -> Result<Report> {
        self.report_for_span(Span { start, end }, &[], exclude_idle)
    }

    fn report_for_span(
        &self,
        span: Span,
        buckets: &[(NaiveDate, Span)],
        exclude_idle: bool,
    ) -> Result<Report> {
        let intervals = ReportIntervals::load(&self.conn, span)?;
        Ok(intervals
            .resolve(span)
            .summarize(span, buckets, exclude_idle))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    start: i64,
    end: i64,
}

impl Span {
    fn intersection(self, other: Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        (start < end).then_some(Self { start, end })
    }

    fn len(self) -> i64 {
        self.end - self.start
    }
}

#[derive(Default)]
struct Activity {
    running: Vec<Span>,
    focused: Vec<Span>,
}

struct AppActivity {
    name: String,
    category: Category,
    activity: Activity,
}

#[derive(Default)]
struct ReportIntervals {
    apps: BTreeMap<String, AppActivity>,
    foreground: Vec<(Span, TimelineKind)>,
    interruptions: Vec<(Span, TimelineKind)>,
    idle: Vec<Span>,
}

struct ResolvedActivity {
    apps: BTreeMap<String, AppActivity>,
    stopped: Vec<Span>,
    known_focus: Vec<Span>,
    idle: Vec<Span>,
    tracked: Vec<Span>,
    timeline: Vec<TimelineEntry>,
    coverage: Option<(i64, i64)>,
}

impl ReportIntervals {
    fn load(conn: &Connection, span: Span) -> Result<Self> {
        let mut intervals = Self::default();
        let mut query = conn.prepare(
            "SELECT i.kind, i.start_ms, i.end_ms, a.key, a.edited_name, a.name,
                    a.edited_category, a.category
             FROM intervals i
             LEFT JOIN qualified_apps s ON s.key = i.app_key
             LEFT JOIN apps a ON a.key = s.effective_key
             WHERE i.end_ms > ?1 AND i.start_ms < ?2
               AND (i.app_key IS NULL OR s.key IS NOT NULL)",
        )?;
        let mut rows = query.query(params![span.start, span.end])?;
        while let Some(row) = rows.next()? {
            let kind: i64 = row.get(0)?;
            let Some(clipped) = span.intersection(Span {
                start: row.get(1)?,
                end: row.get(2)?,
            }) else {
                continue;
            };
            match kind {
                PRESENCE | FOCUS => {
                    let key: String = row.get(3)?;
                    let app = match intervals.apps.entry(key.clone()) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            let name = row
                                .get::<_, Option<String>>(4)?
                                .map_or_else(|| row.get(5), Ok)?;
                            let category = row
                                .get::<_, Option<String>>(6)?
                                .map_or_else(|| row.get(7), Ok)?;
                            entry.insert(AppActivity {
                                name,
                                category: serde_json::from_str(&category)?,
                                activity: Activity::default(),
                            })
                        }
                    };
                    app.activity.running.push(clipped);
                    if kind == FOCUS {
                        intervals
                            .foreground
                            .push((clipped, TimelineKind::Focus(key)));
                    }
                }
                DESKTOP => intervals.foreground.push((clipped, TimelineKind::Desktop)),
                UNKNOWN => intervals.foreground.push((clipped, TimelineKind::Unknown)),
                IDLE => intervals.idle.push(clipped),
                PAUSED => intervals
                    .interruptions
                    .push((clipped, TimelineKind::Paused)),
                GAP => intervals.interruptions.push((clipped, TimelineKind::Gap)),
                _ => return Err(StorageError::Corrupt("Unrecognized interval kind")),
            }
        }
        Ok(intervals)
    }

    fn resolve(self, span: Span) -> ResolvedActivity {
        let Self {
            mut apps,
            mut foreground,
            mut interruptions,
            idle,
        } = self;
        foreground.sort_by_key(|(span, _)| *span);
        interruptions.sort_by_key(|(span, _)| *span);
        let stopped = union(interruptions.iter().map(|(span, _)| *span).collect());
        // Once a foreground slice claims a period, later overlaps cannot claim it again.
        // Explicit pauses and gaps mask both foreground and running time.
        let mut known_focus = Vec::new();
        let mut app_focus = Vec::new();
        let mut timeline = Vec::<TimelineEntry>::new();
        let mut observed_end = span.start;
        for (foreground_span, kind) in foreground {
            let candidate = Span {
                start: foreground_span.start.max(observed_end),
                end: foreground_span.end,
            };
            for part in candidate.subtract(&stopped) {
                if let TimelineKind::Focus(key) = &kind {
                    apps.get_mut(key)
                        .expect("Focus owner has an app")
                        .activity
                        .focused
                        .push(part);
                    app_focus.push(part);
                }
                if kind != TimelineKind::Unknown {
                    known_focus.push(part);
                }
                push_timeline(&mut timeline, part, kind.clone());
                observed_end = part.end;
            }
        }
        let idle = union(idle);
        let known_focus = union(known_focus);
        let foreground_spans = timeline
            .iter()
            .map(|entry| Span {
                start: entry.start_ms,
                end: entry.end_ms,
            })
            .collect::<Vec<_>>();
        let mut tracked = foreground_spans.clone();
        for app in apps.values() {
            tracked.extend_from_slice(&app.activity.running);
        }
        let tracked: Vec<_> = union(tracked)
            .into_iter()
            .flat_map(|part| part.subtract(&stopped))
            .collect();
        let coverage = tracked
            .first()
            .zip(tracked.last())
            .map(|(first, last)| (first.start, last.end));
        for part in intersections(&idle, &app_focus) {
            push_timeline(&mut timeline, part, TimelineKind::Idle);
        }
        for (part, kind) in interruptions {
            push_timeline(&mut timeline, part, kind);
        }
        // Between the first and last tracked activity, running-only time has unknown focus.
        // Periods with no activity or explicit interruption become inferred gaps.
        if let Some((start, end)) = coverage {
            let mut available = foreground_spans;
            available.extend_from_slice(&stopped);
            for part in (Span { start, end }).subtract(&union(available)) {
                for unknown in part.overlapping(&tracked) {
                    push_timeline(&mut timeline, unknown, TimelineKind::Unknown);
                }
                for gap in part.subtract(&tracked) {
                    push_timeline(&mut timeline, gap, TimelineKind::Gap);
                }
            }
        }
        timeline.sort_by_key(|entry| (entry.start_ms, entry.end_ms));

        ResolvedActivity {
            apps,
            stopped,
            known_focus,
            idle,
            tracked,
            timeline,
            coverage,
        }
    }
}

impl ResolvedActivity {
    fn summarize(self, span: Span, buckets: &[(NaiveDate, Span)], exclude_idle: bool) -> Report {
        let Self {
            apps: recorded_apps,
            stopped,
            known_focus,
            idle,
            tracked,
            timeline,
            coverage,
        } = self;
        let mut apps = Vec::new();
        let mut categories = BTreeMap::<Category, Activity>::new();
        for (key, app) in recorded_apps {
            let activity = Activity {
                running: union(app.activity.running)
                    .into_iter()
                    .flat_map(|part| part.subtract(&stopped))
                    .collect(),
                focused: union(app.activity.focused),
            };
            let Some(last_running) = activity.running.last() else {
                continue;
            };
            let category = categories.entry(app.category).or_default();
            category.running.extend_from_slice(&activity.running);
            category.focused.extend_from_slice(&activity.focused);
            let totals = |span| activity.totals(span, &known_focus, &idle, exclude_idle);
            apps.push(AppSummary {
                key,
                name: app.name,
                category: app.category,
                totals: totals(span),
                last_seen_ms: last_running.end,
                days: buckets
                    .iter()
                    .map(|(date, bucket)| (*date, totals(*bucket)))
                    .collect(),
                sessions: activity
                    .running
                    .iter()
                    .map(|part| Session {
                        start_ms: part.start,
                        end_ms: part.end,
                        totals: totals(*part),
                    })
                    .collect(),
            });
        }
        apps.sort_by(|a, b| {
            b.totals
                .focused_ms
                .cmp(&a.totals.focused_ms)
                .then_with(|| b.totals.running_ms.cmp(&a.totals.running_ms))
                .then_with(|| a.name.cmp(&b.name))
        });
        for activity in categories.values_mut() {
            activity.running = union(std::mem::take(&mut activity.running));
            activity.focused = union(std::mem::take(&mut activity.focused));
        }
        let category_totals = |span| {
            categories
                .iter()
                .map(|(&category, activity)| CategorySummary {
                    category,
                    totals: activity.totals(span, &known_focus, &idle, exclude_idle),
                })
                .collect()
        };
        Report {
            apps,
            categories: category_totals(span),
            days: buckets
                .iter()
                .map(|(date, bucket)| DaySummary {
                    date: *date,
                    categories: category_totals(*bucket),
                    covered_ms: bucket.overlapping(&tracked).map(Span::len).sum(),
                })
                .collect(),
            timeline,
            coverage,
        }
    }
}

impl Activity {
    fn totals(
        &self,
        span: Span,
        known_focus: &[Span],
        idle: &[Span],
        exclude_idle: bool,
    ) -> Totals {
        let (running, known) = overlap_totals(&self.running, known_focus, span);
        let (focused, idle) = overlap_totals(&self.focused, idle, span);
        Totals {
            running_ms: running,
            focused_ms: focused - if exclude_idle { idle } else { 0 },
            background_ms: known - focused,
            unknown_ms: running - known,
            idle_ms: idle,
        }
    }
}

impl Span {
    fn overlapping(self, sorted: &[Span]) -> impl Iterator<Item = Span> + '_ {
        sorted[sorted.partition_point(|other| other.end <= self.start)..]
            .iter()
            .take_while(move |other| other.start < self.end)
            .filter_map(move |other| self.intersection(*other))
    }

    fn subtract(self, masks: &[Span]) -> Vec<Span> {
        let mut remaining = Vec::new();
        let mut start = self.start;
        for part in self.overlapping(masks) {
            if start < part.start {
                remaining.push(Span {
                    start,
                    end: part.start,
                });
            }
            start = part.end;
        }
        if start < self.end {
            remaining.push(Span {
                start,
                end: self.end,
            });
        }
        remaining
    }
}

fn union(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_unstable();
    let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(last) = merged.last_mut().filter(|last| span.start <= last.end) {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    merged
}

fn intersections<'a>(left: &'a [Span], right: &'a [Span]) -> impl Iterator<Item = Span> + 'a {
    left.iter().flat_map(|span| span.overlapping(right))
}

fn overlap_totals(spans: &[Span], masks: &[Span], range: Span) -> (i64, i64) {
    let mut total = 0;
    let mut overlap = 0;
    for span in range.overlapping(spans) {
        total += span.len();
        overlap += span.overlapping(masks).map(Span::len).sum::<i64>();
    }
    (total, overlap)
}

fn push_timeline(entries: &mut Vec<TimelineEntry>, part: Span, kind: TimelineKind) {
    if let Some(last) = entries
        .last_mut()
        .filter(|last| last.end_ms == part.start && last.kind == kind)
    {
        last.end_ms = part.end;
    } else {
        entries.push(TimelineEntry {
            start_ms: part.start,
            end_ms: part.end,
            kind,
        });
    }
}

fn date_buckets(range: DateRange, zone: Tz) -> Result<Vec<(NaiveDate, Span)>> {
    ensure(range.start <= range.end, "Date range ends before it begins")?;
    let mut buckets = Vec::new();
    let mut date = range.start;
    let mut start = local_day_start(date, zone)?;
    while date <= range.end {
        let next = date
            .succ_opt()
            .ok_or(StorageError::OutOfRange("Date outside the supported range"))?;
        let end = local_day_start(next, zone)?;
        buckets.push((date, Span { start, end }));
        date = next;
        start = end;
    }
    Ok(buckets)
}

fn local_day_start(date: NaiveDate, zone: Tz) -> Result<i64> {
    let mut local = date
        .and_hms_opt(0, 0, 0)
        .ok_or(StorageError::OutOfRange("Invalid local date"))?;
    loop {
        match zone.from_local_datetime(&local) {
            LocalResult::Single(time) => return Ok(time.timestamp_millis()),
            LocalResult::Ambiguous(first, second) => {
                return Ok(first.min(second).timestamp_millis());
            }
            LocalResult::None => {
                local = local
                    .checked_add_signed(chrono::TimeDelta::seconds(1))
                    .ok_or(StorageError::OutOfRange("Date outside the supported range"))?;
            }
        }
    }
}
