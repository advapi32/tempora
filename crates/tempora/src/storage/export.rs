//! Copies out of the database: backups and CSV exports.

use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rusqlite::Connection;

use crate::model::DateRange;

use super::*;

impl Store {
    pub fn backup(&self, path: &Path) -> Result<()> {
        let file = create_output(path)?;
        drop(file);
        let result = (|| -> Result<()> {
            let mut destination = Connection::open(path)?;
            let backup = rusqlite::backup::Backup::new(&self.conn, &mut destination)?;
            backup.run_to_completion(128, Duration::from_millis(10), None)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }

    pub fn export_csv(
        &self,
        path: &Path,
        range: DateRange,
        zone: Tz,
        exclude_idle: bool,
    ) -> Result<()> {
        let report = self.report(range, zone, exclude_idle)?;
        let mut csv = csv::Writer::from_writer(create_output(path)?);
        csv.write_record([
            "app",
            "category",
            "start",
            "end",
            "running_ms",
            "focused_ms",
            "background_ms",
            "unknown_ms",
            "estimated_idle_ms",
        ])?;
        for app in report.apps {
            for session in app.sessions {
                csv.write_record([
                    app.name.clone(),
                    app.category.label().to_owned(),
                    timestamp(session.start_ms, zone)?,
                    timestamp(session.end_ms, zone)?,
                    session.totals.running_ms.to_string(),
                    session.totals.focused_ms.to_string(),
                    session.totals.background_ms.to_string(),
                    session.totals.unknown_ms.to_string(),
                    session.totals.idle_ms.to_string(),
                ])?;
            }
        }
        csv.flush()?;
        Ok(())
    }
}

pub(super) fn create_output(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}

pub(super) fn timestamp(millis: i64, zone: Tz) -> Result<String> {
    Ok(DateTime::<Utc>::from_timestamp_millis(millis)
        .ok_or(StorageError::OutOfRange(
            "Timestamp outside the supported range",
        ))?
        .with_timezone(&zone)
        .to_rfc3339())
}
