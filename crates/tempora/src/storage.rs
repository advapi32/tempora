mod corrections;
mod export;
mod report;
mod schema;

pub use corrections::CorrectionEntry;
use corrections::*;
use schema::*;

use std::collections::{HashMap, HashSet};
use std::fs::{self};
use std::path::Path;
use std::time::Duration;

use chrono::NaiveDate;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::model::{AppIdentity, Category, Interval, IntervalKind, Settings};

/// Why a storage operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Csv(#[from] csv::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The app (or the named kind of app) isn't in history.
    #[error("{0} was not found")]
    NotFound(&'static str),
    /// The request can't be carried out as asked; the message says why.
    #[error("{0}")]
    Rejected(&'static str),
    /// A date or time outside what the reports can represent.
    #[error("{0}")]
    OutOfRange(&'static str),
    /// Stored data this version can't read.
    #[error("{0}")]
    Corrupt(&'static str),
}

pub type Result<T, E = StorageError> = std::result::Result<T, E>;

/// Fails with `message` unless `condition` holds.
fn ensure(condition: bool, message: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(StorageError::Rejected(message))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub focused_ms: i64,
    pub running_ms: i64,
    pub background_ms: i64,
    pub unknown_ms: i64,
    pub idle_ms: i64,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub start_ms: i64,
    pub end_ms: i64,
    pub totals: Totals,
}

#[derive(Clone, Debug)]
pub struct AppSummary {
    pub key: String,
    pub name: String,
    pub category: Category,
    pub totals: Totals,
    pub last_seen_ms: i64,
    pub days: Vec<(NaiveDate, Totals)>,
    pub sessions: Vec<Session>,
}

#[derive(Clone, Debug)]
pub struct CategorySummary {
    pub category: Category,
    pub totals: Totals,
}

#[derive(Clone, Debug)]
pub struct DaySummary {
    pub date: NaiveDate,
    pub categories: Vec<CategorySummary>,
    pub covered_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineKind {
    Focus(String),
    Desktop,
    Unknown,
    Idle,
    Paused,
    Gap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineEntry {
    pub start_ms: i64,
    pub end_ms: i64,
    pub kind: TimelineKind,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub apps: Vec<AppSummary>,
    pub categories: Vec<CategorySummary>,
    pub days: Vec<DaySummary>,
    pub timeline: Vec<TimelineEntry>,
    pub coverage: Option<(i64, i64)>,
}

const PRESENCE: i64 = 0;
const FOCUS: i64 = 1;
const DESKTOP: i64 = 2;
const UNKNOWN: i64 = 3;
const IDLE: i64 = 4;
const PAUSED: i64 = 5;
const GAP: i64 = 6;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open_writer(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA secure_delete = ON;",
        )?;
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "
             CREATE TABLE IF NOT EXISTS apps (
                 key TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 category TEXT NOT NULL,
                 group_key TEXT REFERENCES apps(key),
                 edited_name TEXT,
                 edited_category TEXT
             );
             CREATE TABLE IF NOT EXISTS intervals (
                 id INTEGER PRIMARY KEY,
                 app_key TEXT REFERENCES apps(key),
                 kind INTEGER NOT NULL,
                 start_ms INTEGER NOT NULL,
                 end_ms INTEGER NOT NULL,
                 offset_seconds INTEGER NOT NULL,
                 CHECK (start_ms < end_ms)
             );
             CREATE INDEX IF NOT EXISTS intervals_time ON intervals(end_ms, start_ms);
             CREATE INDEX IF NOT EXISTS intervals_app ON intervals(app_key, kind, end_ms);
             CREATE INDEX IF NOT EXISTS apps_group ON apps(group_key);
             CREATE TABLE IF NOT EXISTS config (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 settings TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS correction_events (id INTEGER PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS correction_undo (
                 event_id INTEGER NOT NULL REFERENCES correction_events(id) ON DELETE CASCADE,
                 app_key TEXT NOT NULL REFERENCES apps(key),
                 group_key TEXT REFERENCES apps(key),
                 edited_name TEXT,
                 edited_category TEXT,
                 PRIMARY KEY (event_id, app_key)
             );",
        )?;
        migrate_qualification(&tx)?;
        migrate_product_aliases(&tx)?;
        migrate_v3(&tx)?;
        migrate_v4(&tx)?;
        tx.commit()?;
        Ok(Self { conn })
    }

    pub fn open_reader(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    pub fn settings(&self) -> Result<Settings> {
        load_settings(&self.conn)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        write_settings(&self.conn, settings)
    }

    /// Writes a batch atomically. App intervals qualify their identities for reports, so callers
    /// must verify a real app window and apply capture exclusions before writing.
    /// Zero-length intervals are skipped. A reversed interval rolls back the full batch.
    pub fn record(&self, intervals: &[Interval]) -> Result<()> {
        if intervals.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut seen_apps = HashSet::new();
            let mut app_statement = tx.prepare(
                "INSERT INTO apps(key, name, category, qualified) VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(key) DO UPDATE SET
                     name = excluded.name, category = excluded.category, qualified = 1
                 WHERE qualified = 0 OR name <> excluded.name OR category <> excluded.category",
            )?;
            let mut extend = tx.prepare(
                "UPDATE intervals SET end_ms = ?4 WHERE id = (
                    SELECT id FROM intervals
                    WHERE app_key IS ?1 AND kind = ?2 AND end_ms = ?3 AND offset_seconds = ?5
                    LIMIT 1
                 )",
            )?;
            let mut insert = tx.prepare(
                "INSERT INTO intervals(app_key, kind, start_ms, end_ms, offset_seconds)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for interval in intervals {
                if interval.start_ms == interval.end_ms {
                    continue;
                }
                ensure(
                    interval.start_ms < interval.end_ms,
                    "Reversed observation interval",
                )?;
                let (kind, app): (i64, Option<&AppIdentity>) = match &interval.kind {
                    IntervalKind::Presence(app) => (PRESENCE, Some(app)),
                    IntervalKind::Focus(app) => (FOCUS, Some(app)),
                    IntervalKind::Desktop => (DESKTOP, None),
                    IntervalKind::Unknown => (UNKNOWN, None),
                    IntervalKind::Idle => (IDLE, None),
                    IntervalKind::Paused => (PAUSED, None),
                    IntervalKind::Gap => (GAP, None),
                };
                if let Some(app) = app.filter(|app| seen_apps.insert(app.key.as_str())) {
                    app_statement.execute(params![
                        app.key,
                        app.name,
                        serde_json::to_string(&app.category)?
                    ])?;
                }
                let values = params![
                    app.map(|app| app.key.as_str()),
                    kind,
                    interval.start_ms,
                    interval.end_ms,
                    interval.offset_seconds
                ];
                if extend.execute(values)? == 0 {
                    insert.execute(values)?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn apps(&self) -> Result<Vec<AppIdentity>> {
        let mut query = self.conn.prepare(
            "SELECT key, COALESCE(edited_name, name), COALESCE(edited_category, category)
             FROM apps WHERE key IN (SELECT key FROM qualified_apps WHERE key = effective_key)
             ORDER BY 2 COLLATE NOCASE, key",
        )?;
        let mut rows = query.query([])?;
        let mut apps = Vec::new();
        while let Some(row) = rows.next()? {
            apps.push(AppIdentity {
                key: row.get(0)?,
                name: row.get(1)?,
                category: serde_json::from_str(&row.get::<_, String>(2)?)?,
            });
        }
        Ok(apps)
    }

    pub fn qualified_keys(&self) -> Result<Vec<String>> {
        let mut query = self
            .conn
            .prepare("SELECT key FROM apps WHERE qualified = 1 ORDER BY key")?;
        let rows = query.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    #[cfg(windows)]
    /// Recorded `exe:` keys, oldest first, so automatic grouping keeps the first install as
    /// the group's leader.
    pub fn executable_keys_oldest_first(&self) -> Result<Vec<String>> {
        let mut query = self.conn.prepare(
            "SELECT key FROM apps WHERE qualified = 1 AND key GLOB 'exe:*' ORDER BY rowid",
        )?;
        let rows = query.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Records that this app's window has covered a whole monitor. Only updates an existing app,
    /// so an excluded app without a row leaves no trace. Returns true the first time only.
    pub fn mark_fullscreen(&self, key: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE apps SET fullscreen_seen = 1 WHERE key = ?1 AND fullscreen_seen = 0",
            [key],
        )?;
        Ok(changed > 0)
    }

    pub fn fullscreen_keys(&self) -> Result<Vec<String>> {
        let mut query = self
            .conn
            .prepare("SELECT key FROM apps WHERE fullscreen_seen = 1 ORDER BY key")?;
        let rows = query.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces an app's automatic name and category. Past intervals are reclassified, while user
    /// edits in `edited_*` still win. Returns whether a row changed.
    pub fn refresh_automatic(&self, key: &str, name: &str, category: Category) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE apps SET name = ?2, category = ?3
             WHERE key = ?1 AND (name <> ?2 OR category <> ?3)",
            params![key, name, serde_json::to_string(&category)?],
        )?;
        Ok(changed > 0)
    }

    pub fn source_keys(&self, key: &str) -> Result<Vec<String>> {
        let mut query = self.conn.prepare(
            "SELECT key FROM qualified_apps
             WHERE effective_key = (SELECT effective_key FROM qualified_apps WHERE key = ?1)
             ORDER BY key",
        )?;
        let rows = query.query_map([key], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn preview(&self, key: &str) -> Result<Option<AppSummary>> {
        let Some(key) = effective_key(&self.conn, key)? else {
            return Ok(None);
        };
        let (start, end) = self.conn.query_row(
            "SELECT MIN(i.start_ms), MAX(i.end_ms)
             FROM intervals i JOIN qualified_apps a ON a.key = i.app_key
             WHERE a.effective_key = ?1",
            [&key],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
        )?;
        let Some((start, end)) = start.zip(end) else {
            return Ok(None);
        };
        Ok(self
            .report_between(start, end, false)?
            .apps
            .into_iter()
            .find(|app| app.key == key))
    }

    pub fn delete_history(&self, key: Option<&str>) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let count = if let Some(key) = key {
            let Some(key) = effective_key(&tx, key)? else {
                return Ok(0);
            };
            // The time remains tracked, but the deleted owner becomes unknown.
            let changed = tx.execute(
                "UPDATE intervals SET kind = ?2, app_key = NULL WHERE kind = ?3
                 AND app_key IN (SELECT key FROM apps WHERE key = ?1 OR group_key = ?1)",
                params![key, UNKNOWN, FOCUS],
            )?;
            let removed = tx.execute(
                "DELETE FROM intervals WHERE app_key IN
                 (SELECT key FROM apps WHERE key = ?1 OR group_key = ?1)",
                [&key],
            )?;
            tx.execute(
                "DELETE FROM correction_events WHERE id IN (
                     SELECT event_id FROM correction_undo WHERE app_key IN (
                         SELECT key FROM apps WHERE key = ?1 OR group_key = ?1
                     ) OR group_key IN (
                         SELECT key FROM apps WHERE key = ?1 OR group_key = ?1
                     )
                 )",
                [&key],
            )?;
            tx.execute("DELETE FROM apps WHERE key = ?1 OR group_key = ?1", [&key])?;
            changed + removed
        } else {
            let removed = tx.execute("DELETE FROM intervals", [])?;
            tx.execute("DELETE FROM correction_events", [])?;
            tx.execute("DELETE FROM apps", [])?;
            removed
        };
        tx.commit()?;
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(count)
    }
}

/// A short, user-facing reason a history write failed, for the tray tooltip.
pub fn write_problem(error: &StorageError) -> &'static str {
    use rusqlite::ErrorCode;
    let code = match error {
        StorageError::Sqlite(error) => error.sqlite_error_code(),
        _ => None,
    };
    match code {
        Some(ErrorCode::DiskFull) => "Disk full: recording resumes when space is freed",
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            "History database is locked by another program"
        }
        Some(ErrorCode::ReadOnly | ErrorCode::CannotOpen | ErrorCode::PermissionDenied) => {
            "History database can't be written"
        }
        _ => "Could not write local history",
    }
}

/// Read-only projections for the dashboard (redesign scope B).
impl Store {
    /// Changes whenever another connection commits; cheap enough to poll.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "data_version", |row| row.get(0))?)
    }

    /// First and last recorded activity across all history. Both ends walk the
    /// `intervals_time` index from one side, so this stays fast on large histories.
    pub fn history_bounds(&self) -> Result<Option<(i64, i64)>> {
        let bound = |order: &str, column: &str| -> Result<Option<i64>> {
            Ok(self
                .conn
                .query_row(
                    &format!(
                        "SELECT {column} FROM intervals
                         WHERE kind IN (?1, ?2, ?3, ?4)
                         ORDER BY end_ms {order} LIMIT 1"
                    ),
                    params![PRESENCE, FOCUS, DESKTOP, UNKNOWN],
                    |row| row.get(0),
                )
                .optional()?)
        };
        Ok(bound("ASC", "start_ms")?.zip(bound("DESC", "end_ms")?))
    }

    /// Focused time between two instants, with the report's accounting rules.
    pub fn focused_between(&self, start: i64, end: i64, exclude_idle: bool) -> Result<i64> {
        if end <= start {
            return Ok(0);
        }
        let report = self.report_between(start, end, exclude_idle)?;
        Ok(report
            .categories
            .iter()
            .map(|row| row.totals.focused_ms)
            .sum())
    }

    /// Last recorded activity per effective app, over all history: one index seek per
    /// stored identity and kind on `intervals_app`.
    pub fn last_seen_all(&self) -> Result<HashMap<String, i64>> {
        let mut members = self
            .conn
            .prepare("SELECT key, effective_key FROM qualified_apps")?;
        let members = members
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut latest = self
            .conn
            .prepare("SELECT MAX(end_ms) FROM intervals WHERE app_key = ?1 AND kind = ?2")?;
        let mut seen = HashMap::<String, i64>::new();
        for (key, effective) in members {
            for kind in [PRESENCE, FOCUS] {
                if let Some(end) =
                    latest.query_row(params![key, kind], |row| row.get::<_, Option<i64>>(0))?
                {
                    let entry = seen.entry(effective.clone()).or_insert(end);
                    *entry = (*entry).max(end);
                }
            }
        }
        Ok(seen)
    }

    pub fn interval_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM intervals", [], |row| row.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DateRange;
    use chrono::TimeZone;
    use chrono::Utc;

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;

    fn at(month: u32, day: u32, hour: u32) -> i64 {
        Utc.with_ymd_and_hms(2024, month, day, hour, 0, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn dates(start_month: u32, start_day: u32, end_month: u32, end_day: u32) -> DateRange {
        DateRange {
            start: NaiveDate::from_ymd_opt(2024, start_month, start_day).unwrap(),
            end: NaiveDate::from_ymd_opt(2024, end_month, end_day).unwrap(),
        }
    }

    fn app(key: &str, category: Category) -> AppIdentity {
        AppIdentity {
            key: key.to_owned(),
            name: key.to_owned(),
            category,
        }
    }

    fn observation(start_ms: i64, end_ms: i64, kind: IntervalKind) -> Interval {
        Interval {
            start_ms,
            end_ms,
            offset_seconds: 0,
            kind,
        }
    }

    fn app_report<'a>(report: &'a Report, key: &str) -> &'a AppSummary {
        report.apps.iter().find(|entry| entry.key == key).unwrap()
    }

    #[test]
    fn a_reversed_observation_rolls_back_the_batch() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let existing = app("existing", Category::Other);
        let new = app("new", Category::Other);
        store.record(&[observation(10, 20, IntervalKind::Focus(existing.clone()))])?;

        let error = store
            .record(&[
                observation(20, 30, IntervalKind::Focus(existing.clone())),
                observation(20, 30, IntervalKind::Presence(new)),
                observation(30, 20, IntervalKind::Desktop),
            ])
            .unwrap_err();

        assert_eq!(error.to_string(), "Reversed observation interval");
        assert_eq!(store.interval_count()?, 1);
        assert_eq!(store.history_bounds()?, Some((10, 20)));
        assert_eq!(store.qualified_keys()?, [existing.key]);
        Ok(())
    }

    #[test]
    fn canonical_product_migration_unions_history_and_preserves_settings_and_edits() -> Result<()> {
        use crate::model::CaptureMode;

        let store = Store::open_writer(Path::new(":memory:"))?;
        let spotify_package = app(
            "package:spotifyab.spotifymusic_zpdnekdrzrea0!spotify",
            Category::Other,
        );
        let spotify_desktop = app(
            r"exe:C:\Users\Example\AppData\Roaming\Spotify\Spotify.exe",
            Category::Other,
        );
        let discord = app(
            r"exe:C:\Users\Example\AppData\Local\Discord\app-1.0.101\Discord.exe",
            Category::Other,
        );
        let t = at(8, 1, 15);
        let dates = dates(8, 1, 8, 1);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(spotify_package.clone())),
            observation(
                t,
                t + 2 * HOUR,
                IntervalKind::Presence(spotify_desktop.clone()),
            ),
            observation(
                t + 2 * HOUR,
                t + 3 * HOUR,
                IntervalKind::Focus(discord.clone()),
            ),
        ])?;
        store.edit_app(&discord.key, "My chat", Category::Communication)?;
        store.save_settings(&Settings {
            enabled: true,
            mode: CaptureMode::Selected,
            selected: [spotify_package.key.clone(), discord.key.clone()].into(),
            excluded: [discord.key.clone()].into(),
            ..Settings::default()
        })?;
        store.conn.pragma_update(None, "user_version", 1)?;
        let tx = store.conn.unchecked_transaction()?;
        migrate_product_aliases(&tx)?;
        tx.commit()?;

        let settings = store.settings()?;
        assert!(settings.allows("product:spotify"));
        assert!(settings.selected.contains("product:discord"));
        assert!(!settings.allows("product:discord"));
        let keys = store.qualified_keys()?;
        assert!(keys.iter().any(|key| key == "product:spotify"));
        assert!(!keys.iter().any(|key| key == "product:discord"));
        assert_eq!(store.source_keys("product:spotify")?.len(), 3);
        let report = store.report(dates, chrono_tz::UTC, false)?;
        assert_eq!(report.apps.len(), 2);
        let spotify = app_report(&report, "product:spotify");
        assert_eq!(spotify.name, "Spotify");
        assert_eq!(spotify.totals.running_ms, 2 * HOUR);
        assert_eq!(spotify.totals.focused_ms, HOUR);
        assert_eq!(app_report(&report, &discord.key).name, "My chat");
        assert!(store.undo_correction()?);
        let restored = store.report(dates, chrono_tz::UTC, false)?;
        assert_eq!(app_report(&restored, &discord.key).name, discord.name);
        assert_eq!(
            app_report(&restored, "product:spotify").totals.running_ms,
            2 * HOUR
        );
        Ok(())
    }

    #[test]
    fn legacy_background_rows_stay_hidden_until_the_app_is_qualified() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let foreground = app("game", Category::Games);
        let background = app("background", Category::Other);
        let t = at(4, 3, 18);
        let range = dates(4, 3, 4, 3);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(foreground.clone())),
            observation(t, t + 2 * HOUR, IntervalKind::Presence(background.clone())),
        ])?;
        store.group_apps(&foreground.key, &background.key)?;
        store.conn.execute_batch(
            "DROP VIEW qualified_apps;
             ALTER TABLE apps DROP COLUMN qualified;
             PRAGMA user_version = 0;",
        )?;
        let tx = store.conn.unchecked_transaction()?;
        migrate_qualification(&tx)?;
        tx.commit()?;

        assert_eq!(store.qualified_keys()?, [foreground.key.as_str()]);
        assert_eq!(store.apps()?, std::slice::from_ref(&foreground));
        assert!(store.preview(&background.key)?.is_none());
        assert!(store.source_keys(&background.key)?.is_empty());
        let legacy = store.report(range, chrono_tz::UTC, false)?;
        assert_eq!(legacy.apps.len(), 1);
        assert_eq!(legacy.apps[0].key, foreground.key);
        assert_eq!(legacy.apps[0].totals.running_ms, HOUR);
        assert_eq!(legacy.coverage, Some((t, t + HOUR)));

        store.record(&[observation(
            t + 2 * HOUR,
            t + 3 * HOUR,
            IntervalKind::Presence(background.clone()),
        )])?;
        let proven = store.report(range, chrono_tz::UTC, false)?;
        assert_eq!(
            store.qualified_keys()?,
            [background.key.as_str(), foreground.key.as_str()]
        );
        assert_eq!(store.apps()?, std::slice::from_ref(&background));
        assert_eq!(
            store.source_keys(&background.key)?,
            [background.key.as_str(), foreground.key.as_str()]
        );
        assert_eq!(
            app_report(&proven, &background.key).totals.running_ms,
            3 * HOUR
        );
        Ok(())
    }

    #[test]
    fn monthly_and_quarterly_reports_clip_and_union_observations() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let game = app("game", Category::Games);
        let discord = app("Discord", Category::Communication);
        let browser = app("Firefox", Category::Browsers);
        let other_browser = app("Edge", Category::Browsers);
        let spotify = app("Spotify", Category::Media);
        let t = at(4, 15, 18);
        store.record(&[
            observation(
                at(1, 1, 18),
                at(1, 1, 19),
                IntervalKind::Focus(game.clone()),
            ),
            observation(
                at(2, 7, 18),
                at(2, 7, 20),
                IntervalKind::Focus(game.clone()),
            ),
            observation(at(3, 2, 0), at(3, 2, 12), IntervalKind::Presence(spotify)),
            observation(t, t + 90 * MINUTE, IntervalKind::Presence(game.clone())),
            observation(
                t + 15 * MINUTE,
                t + HOUR,
                IntervalKind::Presence(game.clone()),
            ),
            observation(t, t + 45 * MINUTE, IntervalKind::Focus(game.clone())),
            observation(
                t + 45 * MINUTE,
                t + HOUR,
                IntervalKind::Focus(browser.clone()),
            ),
            observation(t + HOUR, t + 90 * MINUTE, IntervalKind::Focus(game)),
            observation(t + 65 * MINUTE, t + 75 * MINUTE, IntervalKind::Idle),
            observation(t, t + 2 * HOUR, IntervalKind::Presence(discord.clone())),
            observation(
                t + 90 * MINUTE,
                t + 95 * MINUTE,
                IntervalKind::Focus(discord),
            ),
            observation(
                t + 95 * MINUTE,
                t + 105 * MINUTE,
                IntervalKind::Focus(other_browser.clone()),
            ),
            observation(
                t + 30 * MINUTE,
                t + 105 * MINUTE,
                IntervalKind::Presence(other_browser),
            ),
            observation(
                t + 35 * MINUTE,
                t + 100 * MINUTE,
                IntervalKind::Presence(browser),
            ),
            observation(t + 105 * MINUTE, t + 115 * MINUTE, IntervalKind::Desktop),
            observation(t + 115 * MINUTE, t + 2 * HOUR, IntervalKind::Unknown),
        ])?;

        let april = dates(4, 1, 4, 30);
        let report = store.report(april, chrono_tz::UTC, false)?;
        let game = app_report(&report, "game");
        assert_eq!(game.sessions.len(), 1);
        assert_eq!(
            game.totals,
            Totals {
                focused_ms: 75 * MINUTE,
                running_ms: 90 * MINUTE,
                background_ms: 15 * MINUTE,
                idle_ms: 10 * MINUTE,
                unknown_ms: 0,
            }
        );
        let discord = app_report(&report, "Discord");
        assert_eq!(discord.totals.running_ms, 2 * HOUR);
        assert_eq!(discord.totals.focused_ms, 5 * MINUTE);
        assert_eq!(discord.totals.background_ms, 110 * MINUTE);
        assert_eq!(discord.totals.unknown_ms, 5 * MINUTE);
        let browsers = report
            .categories
            .iter()
            .find(|entry| entry.category == Category::Browsers)
            .unwrap();
        assert_eq!(browsers.totals.focused_ms, 25 * MINUTE);
        assert_eq!(browsers.totals.running_ms, 75 * MINUTE);
        let filtered = store.report(april, chrono_tz::UTC, true)?;
        let filtered_game = &app_report(&filtered, "game").totals;
        assert_eq!(filtered_game.focused_ms, 65 * MINUTE);
        assert_eq!(filtered_game.idle_ms, 10 * MINUTE);
        assert_eq!(filtered_game.background_ms, game.totals.background_ms);
        let quarter = store.report(dates(1, 1, 3, 31), chrono_tz::UTC, false)?;
        assert_eq!(app_report(&quarter, "game").totals.focused_ms, 3 * HOUR);
        let march = store.report(dates(3, 1, 3, 31), chrono_tz::UTC, false)?;
        assert_eq!(app_report(&march, "Spotify").totals.running_ms, 12 * HOUR);
        assert_eq!(app_report(&march, "Spotify").totals.unknown_ms, 12 * HOUR);
        Ok(())
    }

    #[test]
    fn edits_and_grouping_apply_to_history_and_new_observations_and_can_be_undone() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let a = app("game.exe", Category::Other);
        let b = app("other-install.exe", Category::Other);
        let t = at(4, 3, 18);
        let range = dates(4, 3, 4, 3);
        store.record(&[
            observation(t, t + 2 * HOUR, IntervalKind::Presence(a.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(a.clone())),
            observation(t + HOUR, t + 3 * HOUR, IntervalKind::Presence(b.clone())),
            observation(t + HOUR, t + 2 * HOUR, IntervalKind::Focus(b.clone())),
            observation(t + 2 * HOUR, t + 4 * HOUR, IntervalKind::Desktop),
        ])?;
        store.edit_app(&a.key, "The game", Category::Games)?;
        store.group_apps(&b.key, &a.key)?;
        assert_eq!(store.source_keys(&b.key)?, [a.key.as_str(), b.key.as_str()]);
        assert_eq!(store.apps()?.len(), 1);
        store.record(&[observation(
            t + 3 * HOUR,
            t + 4 * HOUR,
            IntervalKind::Presence(b.clone()),
        )])?;

        let merged = store.report(range, chrono_tz::UTC, false)?;
        assert_eq!(merged.apps.len(), 1);
        let entry = app_report(&merged, &a.key);
        assert_eq!(entry.name, "The game");
        assert_eq!(entry.category, Category::Games);
        assert_eq!(entry.totals.running_ms, 4 * HOUR);
        assert_eq!(entry.totals.focused_ms, 2 * HOUR);
        assert_eq!(entry.totals.background_ms, 2 * HOUR);
        assert_eq!(entry.sessions.len(), 1);
        assert_eq!(store.preview(&b.key)?.unwrap().totals, entry.totals);
        assert!(store.undo_correction()?);
        assert!(store.undo_correction()?);
        assert!(!store.undo_correction()?);
        let restored = store.report(range, chrono_tz::UTC, false)?;
        assert_eq!(app_report(&restored, &a.key).name, a.name);
        assert_eq!(app_report(&restored, &a.key).category, Category::Other);
        assert_eq!(app_report(&restored, &a.key).totals.background_ms, HOUR);
        assert_eq!(app_report(&restored, &b.key).totals.running_ms, 3 * HOUR);

        store.record(&[observation(
            t,
            t + 4 * HOUR,
            IntervalKind::Presence(app("tray", Category::Other)),
        )])?;
        store.group_apps(&b.key, &a.key)?;
        store.delete_history(Some(&b.key))?;
        assert_eq!(store.apps()?.len(), 1);
        assert!(!store.undo_correction()?);
        let deleted = store.report(range, chrono_tz::UTC, false)?;
        let tray = app_report(&deleted, "tray");
        assert_eq!(tray.totals.running_ms, 4 * HOUR);
        assert_eq!(tray.totals.background_ms, 2 * HOUR);
        assert_eq!(tray.totals.unknown_ms, 2 * HOUR);
        assert!(
            deleted
                .timeline
                .iter()
                .all(|entry| !matches!(entry.kind, TimelineKind::Focus(_)))
        );
        Ok(())
    }

    #[test]
    fn read_only_reporting_exports_and_backup_keep_consistent_snapshots() -> Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "tempora-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap(),
        ));
        fs::create_dir(&directory)?;
        let result = (|| -> Result<()> {
            let database = directory.join("history.sqlite3");
            let backup = directory.join("backup.sqlite3");
            let csv = directory.join("sessions.csv");
            let writer = Store::open_writer(&database)?;
            let reader = Store::open_reader(&database)?;
            assert!(reader.save_settings(&Settings::default()).is_err());
            let t = at(8, 1, 15);
            let range = dates(8, 1, 8, 31);
            let game = app("Game, The", Category::Games);
            for start in [t, t + HOUR] {
                writer.record(&[observation(
                    start,
                    start + HOUR,
                    IntervalKind::Focus(game.clone()),
                )])?;
            }
            let count: i64 =
                writer
                    .conn
                    .query_row("SELECT COUNT(*) FROM intervals", [], |row| row.get(0))?;
            assert_eq!(count, 1);
            reader.export_csv(&csv, range, chrono_tz::UTC, false)?;
            writer.backup(&backup)?;
            writer.delete_history(None)?;
            assert!(reader.report(range, chrono_tz::UTC, false)?.apps.is_empty());
            let snapshot = Store::open_reader(&backup)?.report(range, chrono_tz::UTC, false)?;
            assert_eq!(app_report(&snapshot, &game.key).totals.focused_ms, 2 * HOUR);
            let mut csv = csv::Reader::from_path(csv)?;
            let records: Vec<_> = csv.records().collect::<csv::Result<_>>()?;
            assert_eq!(records.len(), 1);
            assert_eq!(&records[0][0], game.name);
            assert_eq!(&records[0][4], (2 * HOUR).to_string());
            Ok(())
        })();
        let cleanup = fs::remove_dir_all(directory);
        result?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn local_day_duration_handles_dst_and_pauses_split_sessions() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let app = app("game", Category::Games);
        let spring = dates(3, 30, 4, 1);
        let berlin = chrono_tz::Europe::Berlin;
        let start = at(3, 30, 22);
        let end = at(3, 31, 23);
        let pause = at(3, 31, 8);
        store.record(&[
            observation(start, end, IntervalKind::Focus(app.clone())),
            observation(pause, pause + HOUR, IntervalKind::Paused),
        ])?;
        let report = store.report(spring, berlin, false)?;
        let entry = app_report(&report, "game");
        let durations: Vec<_> = entry
            .days
            .iter()
            .map(|(_, totals)| totals.running_ms)
            .collect();
        assert_eq!(durations, [HOUR, 22 * HOUR, HOUR]);
        assert_eq!(entry.sessions.len(), 2);
        assert_eq!(entry.sessions[0].end_ms, pause);
        assert_eq!(entry.sessions[1].start_ms, pause + HOUR);
        store.record(&[observation(
            at(10, 26, 22),
            at(10, 27, 23),
            IntervalKind::Presence(app),
        )])?;
        let fall = store.report(dates(10, 27, 10, 27), berlin, false)?;
        assert_eq!(fall.days[0].covered_ms, 25 * HOUR);
        Ok(())
    }

    #[test]
    fn the_timeline_distinguishes_paused_running_only_and_unobserved_time() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let game = app("game", Category::Games);
        let start = at(4, 1, 10);
        let interval = |from, to, kind| observation(start + from, start + to, kind);
        store.record(&[
            interval(0, 10, IntervalKind::Focus(game.clone())),
            interval(3, 7, IntervalKind::Idle),
            interval(5, 8, IntervalKind::Paused),
            interval(8, 12, IntervalKind::Presence(game.clone())),
            interval(16, 18, IntervalKind::Focus(game.clone())),
        ])?;

        let range = dates(4, 1, 4, 1);
        let report = store.report(range, chrono_tz::UTC, false)?;
        let timeline = report
            .timeline
            .iter()
            .map(|entry| {
                (
                    entry.start_ms - start,
                    entry.end_ms - start,
                    entry.kind.clone(),
                )
            })
            .collect::<Vec<_>>();
        let focus = || TimelineKind::Focus(game.key.clone());
        assert_eq!(
            timeline,
            [
                (0, 5, focus()),
                (3, 5, TimelineKind::Idle),
                (5, 8, TimelineKind::Paused),
                (8, 10, focus()),
                (10, 12, TimelineKind::Unknown),
                (12, 16, TimelineKind::Gap),
                (16, 18, focus()),
            ]
        );
        assert_eq!(report.coverage, Some((start, start + 18)));
        assert_eq!(report.days[0].covered_ms, 11);
        assert_eq!(
            store.report(range, chrono_tz::UTC, true)?.timeline,
            report.timeline
        );
        Ok(())
    }

    #[test]
    fn dashboard_projections_cover_bounds_flags_history_and_partial_periods() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let game = app(r"exe:C:\Games\Game\game.exe", Category::Games);
        let copy = app(r"exe:D:\Other\game.exe", Category::Other);
        let tool = app(r"exe:C:\Tools\tool.exe", Category::Other);
        let t = at(8, 1, 10);
        let version = store.data_version()?;
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(game.clone())),
            observation(t + HOUR, t + 2 * HOUR, IntervalKind::Focus(tool.clone())),
            observation(
                t + 2 * HOUR,
                t + 3 * HOUR,
                IntervalKind::Focus(copy.clone()),
            ),
            observation(t + 3 * HOUR, t + 4 * HOUR, IntervalKind::Paused),
        ])?;
        assert_eq!(
            store.data_version()?,
            version,
            "own commits don't change it"
        );
        assert_eq!(store.history_bounds()?, Some((t, t + 3 * HOUR)));
        assert_eq!(store.interval_count()?, 4);
        assert_eq!(
            store.focused_between(t, t + 90 * MINUTE, false)?,
            90 * MINUTE
        );
        assert_eq!(store.focused_between(t, t, false)?, 0);
        let seen = store.last_seen_all()?;
        assert_eq!(seen[&tool.key], t + 2 * HOUR);

        store.edit_app(&game.key, "My game", Category::Games)?;
        store.group_apps(&copy.key, &game.key)?;
        let corrections = store.corrections()?;
        assert_eq!(corrections.len(), 2);
        assert!(corrections[0].id > corrections[1].id);
        assert_eq!(
            corrections[0].apps,
            std::slice::from_ref(&copy.name),
            "grouping snapshots the source"
        );
        assert_eq!(corrections[1].apps, ["My game".to_owned()]);
        assert!(store.undo_correction()?);
        assert_eq!(store.corrections()?.len(), 1);

        let empty = Store::open_writer(Path::new(":memory:"))?;
        assert_eq!(empty.history_bounds()?, None);
        assert!(empty.corrections()?.is_empty());
        Ok(())
    }

    /// Run with `cargo test --release --lib -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark: one million intervals"]
    fn monthly_summary_with_a_million_intervals_stays_under_300_ms() -> Result<()> {
        let directory = std::env::temp_dir().join(format!("gta-bench-{}", std::process::id()));
        fs::create_dir_all(&directory)?;
        let path = directory.join("bench.sqlite3");
        let _ = fs::remove_file(&path);
        let writer = Store::open_writer(&path)?;
        let apps: Vec<AppIdentity> = (0..40)
            .map(|index| {
                app(
                    &format!(r"exe:C:\Apps\app{index}.exe"),
                    Category::ALL[index % 6],
                )
            })
            .collect();
        // One million intervals over two years of history; the summary covers one month.
        let start = at(1, 1, 0) - 365 * 24 * HOUR;
        let step = 2 * 365 * 24 * HOUR / 500_000;
        let mut batch = Vec::with_capacity(20_000);
        for index in 0..500_000i64 {
            let t = start + index * step;
            let focus = &apps[(index as usize / 7) % apps.len()];
            let background = &apps[(index as usize / 13 + 5) % apps.len()];
            batch.push(observation(
                t,
                t + step - 1,
                IntervalKind::Focus(focus.clone()),
            ));
            batch.push(observation(
                t,
                t + step - 1,
                IntervalKind::Presence(background.clone()),
            ));
            if batch.len() >= 20_000 {
                writer.record(&batch)?;
                batch.clear();
            }
        }
        writer.record(&batch)?;
        let reader = Store::open_reader(&path)?;
        assert!(reader.interval_count()? >= 1_000_000);
        let month = dates(6, 1, 6, 30);
        reader.report(month, chrono_tz::UTC, false)?;
        let started = std::time::Instant::now();
        reader.report(month, chrono_tz::UTC, false)?;
        let report = started.elapsed();
        let time = |label: &str, run: &dyn Fn() -> Result<()>| -> Result<std::time::Duration> {
            let started = std::time::Instant::now();
            run()?;
            let elapsed = started.elapsed();
            println!("  {label}: {elapsed:?}");
            Ok(elapsed)
        };
        let extras = [
            time("previous month", &|| {
                reader
                    .focused_between(at(5, 1, 0), at(5, 31, 0), false)
                    .map(drop)
            })?,
            time("week", &|| {
                reader
                    .report(dates(6, 24, 6, 30), chrono_tz::UTC, false)
                    .map(drop)
            })?,
            time("bounds", &|| reader.history_bounds().map(drop))?,
            time("last seen", &|| reader.last_seen_all().map(drop))?,
            time("corrections", &|| reader.corrections().map(drop))?,
            time("interval count", &|| reader.interval_count().map(drop))?,
        ]
        .into_iter()
        .sum::<std::time::Duration>();
        println!("monthly report {report:?}, extras {extras:?}");
        drop(reader);
        drop(writer);
        let _ = fs::remove_dir_all(&directory);
        assert!(report.as_millis() < 300, "monthly report took {report:?}");
        assert!(
            report + extras < std::time::Duration::from_millis(600),
            "report job {:?}",
            report + extras
        );
        Ok(())
    }

    #[test]
    fn reset_to_automatic_is_one_undoable_correction() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let game = app(r"exe:C:\\Games\\game.exe", Category::Other);
        let t = at(8, 1, 10);
        store.record(&[observation(t, t + HOUR, IntervalKind::Focus(game.clone()))])?;
        assert!(store.reset_app(&game.key).is_err(), "nothing to reset");
        store.edit_app(&game.key, "Renamed", Category::Games)?;
        assert_eq!(
            store.automatic_identity(&game.key)?,
            Some((game.name.clone(), Category::Other))
        );
        store.reset_app(&game.key)?;
        let name = |store: &Store| store.preview(&game.key).map(|app| app.map(|app| app.name));
        assert_eq!(name(&store)?, Some(game.name.clone()));
        assert!(store.undo_correction()?);
        assert_eq!(name(&store)?, Some("Renamed".to_owned()));
        Ok(())
    }

    fn grouping(store: &Store, key: &str) -> Result<Option<String>> {
        Ok(store
            .conn
            .query_row("SELECT grouping FROM apps WHERE key = ?1", [key], |row| {
                row.get(0)
            })?)
    }

    fn column_names(store: &Store, table: &str) -> Result<Vec<String>> {
        let mut query = store.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = query.query_map([], |row| row.get::<_, String>(1))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A database file in the temp directory, removed with its WAL files on drop.
    struct TempDatabase(std::path::PathBuf);

    impl TempDatabase {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "gta-storage-{}-{stamp}.sqlite3",
                std::process::id()
            )))
        }
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut name = self.0.clone().into_os_string();
                name.push(suffix);
                let _ = fs::remove_file(name);
            }
        }
    }

    #[test]
    fn a_fresh_database_opens_at_version_four_with_the_new_columns() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let version: u32 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(version, 4);
        let apps = column_names(&store, "apps")?;
        for column in ["fullscreen_seen", "fingerprint", "grouping"] {
            assert!(apps.iter().any(|name| name == column), "missing {column}");
        }
        let snapshots = column_names(&store, "correction_undo")?;
        for column in ["grouping", "show_in_history"] {
            assert!(
                snapshots.iter().any(|name| name == column),
                "missing {column}"
            );
        }
        Ok(())
    }

    #[test]
    fn version_three_corrections_stay_visible_and_undoable_after_upgrade() -> Result<()> {
        let database = TempDatabase::new();
        let game = app("exe:game.exe", Category::Other);
        {
            let store = Store::open_writer(&database.0)?;
            store.record(&[observation(10, 20, IntervalKind::Focus(game.clone()))])?;
            store.edit_app(&game.key, "My game", Category::Games)?;
            store.conn.execute_batch(
                "ALTER TABLE correction_undo DROP COLUMN show_in_history; PRAGMA user_version = 3;",
            )?;
        }
        let store = Store::open_writer(&database.0)?;
        assert_eq!(store.corrections()?[0].apps, ["My game"]);
        assert!(store.undo_correction()?);
        assert_eq!(store.apps()?, [game]);
        Ok(())
    }

    #[test]
    fn a_version_two_database_gains_the_v3_columns_and_keeps_groups_as_user_decisions() -> Result<()>
    {
        let database = TempDatabase::new();
        {
            // The version 2 shape: qualification and product aliases are applied, v3 is not.
            let conn = Connection::open(&database.0)?;
            conn.execute_batch(
                "CREATE TABLE apps (
                     key TEXT PRIMARY KEY,
                     name TEXT NOT NULL,
                     category TEXT NOT NULL,
                     group_key TEXT REFERENCES apps(key),
                     edited_name TEXT,
                     edited_category TEXT,
                     qualified INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE VIEW qualified_apps AS
                     SELECT s.key, COALESCE(g.key, s.key) AS effective_key
                     FROM apps s LEFT JOIN apps g ON g.key = s.group_key AND g.qualified = 1
                     WHERE s.qualified = 1;
                 PRAGMA user_version = 2;",
            )?;
            conn.execute(
                "INSERT INTO apps(key, name, category, qualified, group_key) VALUES
                     ('exe:a', 'A', ?1, 1, NULL), ('exe:b', 'B', ?1, 1, 'exe:a')",
                [serde_json::to_string(&Category::Other)?],
            )?;
        }

        let store = Store::open_writer(&database.0)?;
        let version: u32 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(version, 4);
        assert_eq!(grouping(&store, "exe:b")?.as_deref(), Some("user"));
        assert_eq!(grouping(&store, "exe:a")?, None);
        assert!(
            column_names(&store, "apps")?
                .iter()
                .any(|name| name == "fingerprint")
        );
        assert!(
            column_names(&store, "correction_undo")?
                .iter()
                .any(|name| name == "grouping")
        );
        let fullscreen: i64 = store.conn.query_row(
            "SELECT fullscreen_seen FROM apps WHERE key = 'exe:a'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(fullscreen, 0);
        store
            .conn
            .execute("UPDATE apps SET grouping = NULL WHERE key = 'exe:b'", [])?;
        drop(store);

        // Reopening a current database does not run the backfill again.
        let store = Store::open_writer(&database.0)?;
        assert_eq!(grouping(&store, "exe:b")?, None);
        Ok(())
    }

    #[test]
    fn mark_fullscreen_updates_only_existing_apps_and_reports_the_first_time() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        assert!(!store.mark_fullscreen(r"exe:c:\\games\\missing.exe")?);
        let apps: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM apps", [], |row| row.get(0))?;
        assert_eq!(apps, 0);

        let game = app(r"exe:c:\\games\\game.exe", Category::Other);
        let t = at(8, 1, 10);
        store.record(&[observation(t, t + HOUR, IntervalKind::Focus(game.clone()))])?;
        assert!(store.fullscreen_keys()?.is_empty());
        assert!(store.mark_fullscreen(&game.key)?);
        assert!(!store.mark_fullscreen(&game.key)?);
        assert_eq!(store.fullscreen_keys()?, [game.key.as_str()]);
        Ok(())
    }

    #[test]
    fn refresh_automatic_reclassifies_history_and_keeps_user_edits() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let game = app(r"exe:d:\\games\\rdr2.exe", Category::Other);
        let edited = app(r"exe:d:\\games\\launcher.exe", Category::Other);
        let t = at(4, 3, 18);
        let range = dates(4, 3, 4, 3);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(game.clone())),
            observation(t + HOUR, t + 2 * HOUR, IntervalKind::Focus(edited.clone())),
        ])?;
        store.edit_app(&edited.key, "Kept name", Category::Media)?;

        let before = store.report(range, chrono_tz::UTC, false)?;
        assert_eq!(app_report(&before, &game.key).category, Category::Other);

        assert!(!store.refresh_automatic(&game.key, &game.name, Category::Other)?);
        assert!(store.refresh_automatic(&game.key, "Red Dead Redemption 2", Category::Games)?);
        assert!(!store.refresh_automatic(&game.key, "Red Dead Redemption 2", Category::Games)?);
        assert!(!store.refresh_automatic("exe:missing.exe", "Missing", Category::Games)?);
        let after = store.report(range, chrono_tz::UTC, false)?;
        let entry = app_report(&after, &game.key);
        assert_eq!(entry.name, "Red Dead Redemption 2");
        assert_eq!(entry.category, Category::Games);
        assert_eq!(entry.totals.focused_ms, HOUR);

        assert!(store.refresh_automatic(&edited.key, "Automatic", Category::Games)?);
        let after = store.report(range, chrono_tz::UTC, false)?;
        let entry = app_report(&after, &edited.key);
        assert_eq!(entry.name, "Kept name");
        assert_eq!(entry.category, Category::Media);
        Ok(())
    }

    fn fingerprint_of(store: &Store, key: &str) -> Result<Option<String>> {
        Ok(store.conn.query_row(
            "SELECT fingerprint FROM apps WHERE key = ?1",
            [key],
            |row| row.get(0),
        )?)
    }

    fn correction_events(store: &Store) -> Result<i64> {
        Ok(store
            .conn
            .query_row("SELECT COUNT(*) FROM correction_events", [], |row| {
                row.get(0)
            })?)
    }

    #[test]
    fn auto_group_never_chains_under_a_member_of_a_hidden_group() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let member = app("exe:games/one/game.exe", Category::Games);
        let newer = app("exe:games/two/game.exe", Category::Games);
        let t = at(4, 3, 18);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Presence(member.clone())),
            observation(t, t + HOUR, IntervalKind::Presence(newer.clone())),
        ])?;
        // A legacy, unqualified leader: `member` resolves to itself yet has a group_key.
        store.conn.execute(
            "INSERT INTO apps(key, name, category, qualified) VALUES ('exe:legacy', 'Legacy', '\"Games\"', 0)",
            [],
        )?;
        store.conn.execute(
            "UPDATE apps SET group_key = 'exe:legacy' WHERE key = ?1",
            [&member.key],
        )?;
        store.auto_group(&member.key, Some("title:1"))?;
        assert!(!store.auto_group(&newer.key, Some("title:1"))?);
        let chained: bool = store.conn.query_row(
            "SELECT group_key IS NOT NULL FROM apps WHERE key = ?1",
            [&newer.key],
            |row| row.get(0),
        )?;
        assert!(!chained);
        Ok(())
    }

    #[test]
    fn auto_group_joins_the_oldest_app_with_the_same_fingerprint_and_unions_history() -> Result<()>
    {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let older = app("exe:games/one/game.exe", Category::Games);
        let newer = app("exe:games/two/game.exe", Category::Games);
        let t = at(4, 3, 18);
        let range = dates(4, 3, 4, 3);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Presence(older.clone())),
            observation(t, t + HOUR, IntervalKind::Presence(newer.clone())),
            observation(
                t + HOUR,
                t + 2 * HOUR,
                IntervalKind::Presence(newer.clone()),
            ),
        ])?;

        assert!(!store.auto_group(&older.key, Some("title:1"))?);
        assert!(store.auto_group(&newer.key, Some("title:1"))?);
        assert_eq!(
            store.source_keys(&newer.key)?,
            [older.key.as_str(), newer.key.as_str()]
        );
        assert_eq!(store.apps()?.len(), 1);
        assert_eq!(
            grouping(&store, &newer.key)?,
            None,
            "automatic, not a user decision"
        );
        assert_eq!(
            correction_events(&store)?,
            0,
            "automatic grouping leaves no undo event"
        );

        let merged = store.report(range, chrono_tz::UTC, false)?;
        let entry = app_report(&merged, &older.key);
        assert_eq!(
            entry.totals.running_ms,
            2 * HOUR,
            "overlapping time counts once"
        );
        Ok(())
    }

    #[test]
    fn auto_group_stores_the_fingerprint_even_when_it_does_not_group() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let game = app("exe:games/alone.exe", Category::Games);
        let t = at(4, 3, 18);
        store.record(&[observation(t, t + HOUR, IntervalKind::Focus(game.clone()))])?;

        assert!(!store.auto_group(&game.key, Some("title:9"))?);
        assert_eq!(
            fingerprint_of(&store, &game.key)?.as_deref(),
            Some("title:9")
        );

        assert!(!store.auto_group(&game.key, None)?);
        assert_eq!(fingerprint_of(&store, &game.key)?, None);
        assert!(!store.auto_group("exe:missing.exe", Some("title:9"))?);
        Ok(())
    }

    #[test]
    fn the_same_file_name_without_evidence_never_groups() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let first = app("exe:a/tool.exe", Category::Other);
        let second = app("exe:b/tool.exe", Category::Other);
        let third = app("exe:c/tool.exe", Category::Other);
        let t = at(4, 3, 18);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(first.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(second.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(third.clone())),
        ])?;
        assert!(!store.auto_group(&first.key, None)?);
        assert!(!store.auto_group(&second.key, None)?);
        // Same product, different executables: the file name is part of the fingerprint.
        assert!(!store.auto_group(&first.key, Some("product:acme|widget|tool.exe"))?);
        assert!(!store.auto_group(&second.key, Some("product:acme|widget|helper.exe"))?);
        // The same product and the same executable name is real evidence, so this one joins.
        assert!(store.auto_group(&third.key, Some("product:acme|widget|helper.exe"))?);
        // Three sources, two visible apps: the third is grouped with the second.
        assert_eq!(store.apps()?.len(), 2);
        assert_eq!(store.source_keys(&third.key)?.len(), 2);
        Ok(())
    }

    #[test]
    fn decided_and_representative_apps_are_never_auto_grouped() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let representative = app("exe:r/game.exe", Category::Games);
        let member = app("exe:m/game.exe", Category::Games);
        let edited = app("exe:e/game.exe", Category::Games);
        let manual = app("exe:u/game.exe", Category::Games);
        let other = app("exe:o/game.exe", Category::Games);
        let t = at(4, 3, 18);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(representative.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(member.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(edited.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(manual.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(other.clone())),
        ])?;
        let fingerprint = Some("title:7");
        assert!(!store.auto_group(&representative.key, fingerprint)?);
        assert!(store.auto_group(&member.key, fingerprint)?);
        // The representative now has a member of its own, so it can't join another group.
        assert!(!store.auto_group(&representative.key, Some("title:8"))?);
        assert_eq!(store.source_keys(&member.key)?.len(), 2);

        store.edit_app(&edited.key, "Mine", Category::Games)?;
        assert!(!store.auto_group(&edited.key, fingerprint)?);
        assert_eq!(store.source_keys(&edited.key)?, [edited.key.as_str()]);

        store.group_apps(&manual.key, &other.key)?;
        assert!(!store.auto_group(&manual.key, Some("title:8"))?);
        assert_eq!(
            store.source_keys(&manual.key)?,
            [other.key.as_str(), manual.key.as_str()]
        );
        Ok(())
    }

    #[test]
    fn manual_group_and_undo_restore_every_affected_grouping_flag() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let target = app("exe:t/game.exe", Category::Games);
        let source = app("exe:s/game.exe", Category::Games);
        let member = app("exe:p/game.exe", Category::Games);
        let t = at(4, 3, 18);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(target.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(source.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(member.clone())),
        ])?;
        assert!(!store.auto_group(&target.key, Some("title:3"))?);
        assert!(store.auto_group(&member.key, Some("title:3"))?);

        store.group_apps(&source.key, &target.key)?;
        assert_eq!(grouping(&store, &target.key)?.as_deref(), Some("user"));
        assert_eq!(grouping(&store, &source.key)?.as_deref(), Some("user"));
        assert_eq!(grouping(&store, &member.key)?.as_deref(), Some("user"));

        assert!(store.undo_correction()?);
        assert_eq!(grouping(&store, &target.key)?, None);
        assert_eq!(grouping(&store, &source.key)?, None);
        assert_eq!(grouping(&store, &member.key)?, None);
        assert_eq!(store.source_keys(&target.key)?.len(), 2);
        Ok(())
    }

    #[test]
    fn ungroup_removes_a_source_sticks_across_auto_grouping_and_can_be_undone() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let target = app("exe:a/game.exe", Category::Games);
        let source = app("exe:b/game.exe", Category::Games);
        let t = at(4, 3, 18);
        let range = dates(4, 3, 4, 3);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Presence(target.clone())),
            observation(
                t + HOUR,
                t + 2 * HOUR,
                IntervalKind::Presence(source.clone()),
            ),
        ])?;
        assert!(!store.auto_group(&target.key, Some("title:5"))?);
        assert!(store.auto_group(&source.key, Some("title:5"))?);

        assert!(
            store.ungroup_app(&target.key).is_err(),
            "a representative is not in a group"
        );
        assert!(store.ungroup_app("exe:missing.exe").is_err());
        assert_eq!(correction_events(&store)?, 0);

        store.ungroup_app(&source.key)?;
        assert_eq!(correction_events(&store)?, 1);
        assert_eq!(store.source_keys(&source.key)?, [source.key.as_str()]);
        assert_eq!(grouping(&store, &source.key)?.as_deref(), Some("user"));
        assert!(!store.auto_group(&source.key, Some("title:5"))?);
        assert_eq!(store.source_keys(&source.key)?, [source.key.as_str()]);

        let separate = store.report(range, chrono_tz::UTC, false)?;
        assert_eq!(separate.apps.len(), 2);
        assert_eq!(app_report(&separate, &source.key).totals.running_ms, HOUR);

        assert!(store.undo_correction()?);
        assert_eq!(
            store.source_keys(&source.key)?,
            [target.key.as_str(), source.key.as_str()]
        );
        assert_eq!(
            grouping(&store, &source.key)?,
            None,
            "undo restores the automatic state"
        );
        assert_eq!(store.apps()?.len(), 1);
        Ok(())
    }

    #[test]
    fn undo_of_a_user_ungroup_restores_the_user_group_and_its_decision() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let target = app("exe:a/game.exe", Category::Games);
        let source = app("exe:b/game.exe", Category::Games);
        let t = at(4, 3, 18);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Focus(target.clone())),
            observation(t, t + HOUR, IntervalKind::Focus(source.clone())),
        ])?;
        store.group_apps(&source.key, &target.key)?;
        store.ungroup_app(&source.key)?;
        assert_eq!(store.source_keys(&source.key)?, [source.key.as_str()]);

        assert!(store.undo_correction()?);
        assert_eq!(
            store.source_keys(&source.key)?,
            [target.key.as_str(), source.key.as_str()]
        );
        assert_eq!(grouping(&store, &source.key)?.as_deref(), Some("user"));
        Ok(())
    }

    #[test]
    fn undo_ungroup_keeps_new_automatic_members_at_the_restored_root() -> Result<()> {
        let store = Store::open_writer(Path::new(":memory:"))?;
        let root = app("exe:root/game.exe", Category::Games);
        let detached = app("exe:detached/game.exe", Category::Games);
        let new = app("exe:new/game.exe", Category::Games);
        let t = at(4, 3, 18);
        store.record(&[
            observation(t, t + HOUR, IntervalKind::Presence(root.clone())),
            observation(t, t + HOUR, IntervalKind::Presence(detached.clone())),
        ])?;
        store.auto_group(&detached.key, Some("title:42"))?;
        store.group_apps(&detached.key, &root.key)?;
        store.ungroup_app(&detached.key)?;
        store.record(&[observation(
            t,
            t + HOUR,
            IntervalKind::Presence(new.clone()),
        )])?;
        assert!(store.auto_group(&new.key, Some("title:42"))?);

        assert!(store.undo_correction()?);
        assert_eq!(
            effective_key(&store.conn, &new.key)?.as_deref(),
            Some(root.key.as_str())
        );
        assert_eq!(store.source_keys(&root.key)?.len(), 3);
        store.delete_history(Some(&root.key))?;
        assert!(store.apps()?.is_empty());
        Ok(())
    }

    #[test]
    fn write_problems_name_disk_full_and_locked_databases() {
        let sqlite = |code| {
            StorageError::from(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        assert!(write_problem(&sqlite(rusqlite::ffi::SQLITE_FULL)).starts_with("Disk full"));
        assert!(write_problem(&sqlite(rusqlite::ffi::SQLITE_BUSY)).contains("locked"));
        assert!(
            write_problem(&sqlite(rusqlite::ffi::SQLITE_READONLY)).contains("can't be written")
        );
        assert_eq!(
            write_problem(&StorageError::Rejected("other")),
            "Could not write local history"
        );
    }
}
