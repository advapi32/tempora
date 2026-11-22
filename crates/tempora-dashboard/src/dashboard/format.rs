//! Text adapters shared by every screen. Pure functions over existing records.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use tempora::model::{AppKey, DateRange};

use super::nav::Period;

pub(super) fn duration(ms: i64) -> String {
    if ms <= 0 {
        return "0m".into();
    }
    let minutes = ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "<1m".into(),
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes:02}m"),
    }
}

pub(super) fn local(ms: i64, zone: Tz, format: &str) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|value| value.with_timezone(&zone).format(format).to_string())
        .unwrap_or_else(|| "–".into())
}

pub(super) fn local_date(ms: i64, zone: Tz) -> Option<NaiveDate> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(|value| value.with_timezone(&zone).date_naive())
}

/// Local midnight of `date` in milliseconds, skipping a DST gap the way reports do.
pub(super) fn day_start(date: NaiveDate, zone: Tz) -> i64 {
    let mut local = date.and_hms_opt(0, 0, 0).expect("midnight exists");
    for _ in 0..7200 {
        match zone.from_local_datetime(&local) {
            LocalResult::Single(time) => return time.timestamp_millis(),
            LocalResult::Ambiguous(first, second) => return first.min(second).timestamp_millis(),
            LocalResult::None => local += chrono::TimeDelta::seconds(1),
        }
    }
    local.and_utc().timestamp_millis()
}

/// The instant the clock in `zone` reads `clock_ms` after midnight on `date`; 24:00 is the
/// next midnight. A time skipped when clocks spring forward maps to where the clock resumes,
/// and a time that repeats when they fall back maps to its first occurrence.
pub(super) fn clock_instant(date: NaiveDate, clock_ms: i64, zone: Tz) -> i64 {
    const DAY_MS: i64 = 24 * 3_600_000;
    if clock_ms >= DAY_MS {
        return day_start(date + chrono::Days::new(1), zone);
    }
    let mut local = date.and_hms_opt(0, 0, 0).expect("midnight exists")
        + chrono::TimeDelta::milliseconds(clock_ms.max(0));
    // Clock changes skip at most a few hours, so a gap ends within this many minutes.
    for _ in 0..240 {
        match zone.from_local_datetime(&local) {
            LocalResult::Single(time) => return time.timestamp_millis(),
            LocalResult::Ambiguous(first, second) => return first.min(second).timestamp_millis(),
            LocalResult::None => local += chrono::TimeDelta::minutes(1),
        }
    }
    day_start(date, zone) + clock_ms
}

/// What the clock in `zone` reads at `instant`, in milliseconds after midnight.
pub(super) fn clock_ms(instant: i64, zone: Tz) -> i64 {
    use chrono::Timelike;
    DateTime::<Utc>::from_timestamp_millis(instant).map_or(0, |time| {
        let time = time.with_timezone(&zone).time();
        i64::from(time.num_seconds_from_midnight()) * 1_000
            + i64::from(time.nanosecond() / 1_000_000)
    })
}

pub(super) fn percent(part: i64, whole: i64) -> Option<u32> {
    (whole > 0).then(|| ((part.max(0) as f64 / whole as f64) * 100.0).round() as u32)
}

/// "Nov 16 – 22, 2026", "November 2026" or "Nov 20, 2026".
pub(super) fn range_label(range: DateRange, period: Period) -> String {
    let (start, end) = (range.start, range.end);
    let whole_month = start.day() == 1
        && end.succ_opt().is_some_and(|next| next.day() == 1)
        && start.month() == end.month()
        && start.year() == end.year();
    if period == Period::Month || whole_month {
        start.format("%B %Y").to_string()
    } else if start == end {
        start.format("%b %-d, %Y").to_string()
    } else if start.year() != end.year() {
        format!(
            "{} – {}",
            start.format("%b %-d, %Y"),
            end.format("%b %-d, %Y")
        )
    } else if start.month() != end.month() {
        format!("{} – {}", start.format("%b %-d"), end.format("%b %-d, %Y"))
    } else {
        format!("{} – {}", start.format("%b %-d"), end.format("%-d, %Y"))
    }
}

pub(super) fn dates_span(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Option<String> {
    match (start, end) {
        (Some(start), Some(end)) if start != end => {
            Some(range_label(DateRange { start, end }, Period::Custom))
        }
        (Some(date), _) | (_, Some(date)) => Some(date.format("%b %-d, %Y").to_string()),
        _ => None,
    }
}

/// Where an identity came from, decoded from the evidence its stored key carries.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Source {
    /// Short sub-line, for example "Steam · App 1086940" or "Code.exe".
    pub line: String,
    /// The recognition source, for example "Steam" or "Executable".
    pub kind: &'static str,
    /// Steam App ID, Riot product, package family or product name.
    pub id: Option<String>,
    /// The recorded executable path, when the key is path-based.
    pub path: Option<String>,
}

pub(super) fn source(key: &str) -> Source {
    let file_name = |path: &str| path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned();
    match AppKey::parse(key) {
        AppKey::Steam(id) => Source {
            line: format!("Steam · App {id}"),
            kind: "Steam",
            id: Some(id.into()),
            path: None,
        },
        AppKey::Riot(id) => Source {
            line: format!("Riot · {id}"),
            kind: "Riot",
            id: Some(id.into()),
            path: None,
        },
        AppKey::Package(package) => {
            let family = package.split('!').next().unwrap_or(package);
            Source {
                line: format!("Package · {family}"),
                kind: "Package",
                id: Some(package.into()),
                path: None,
            }
        }
        AppKey::Product(product) => Source {
            line: "Recognized product".into(),
            kind: "Product",
            id: Some(product.into()),
            path: None,
        },
        AppKey::Exe(path) => Source {
            line: file_name(path),
            kind: "Executable",
            id: None,
            path: Some(path.into()),
        },
        AppKey::Unknown(key) => Source {
            line: file_name(key),
            kind: "Executable",
            id: None,
            path: None,
        },
    }
}

impl Source {
    /// The whole source, for a tooltip: the executable path or the package ID, else the line.
    pub(super) fn full(&self) -> String {
        if let Some(path) = &self.path {
            return path.clone();
        }
        if self.kind == "Package"
            && let Some(id) = &self.id
        {
            return id.clone();
        }
        self.line.clone()
    }
}

/// The second line for each entry whose name is shared with another entry in the same list,
/// keyed by app key. Entries with a unique name get nothing. Names compare case-insensitively.
/// Each row is `(key, name)`.
pub(super) fn disambiguation(rows: &[(&str, &str)]) -> HashMap<String, String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, name) in rows {
        *counts.entry(name.to_lowercase()).or_default() += 1;
    }
    rows.iter()
        .filter(|(_, name)| counts[&name.to_lowercase()] > 1)
        .map(|(key, _)| ((*key).to_owned(), distinguishing_line(key)))
        .collect()
}

/// What tells a source apart from a same-named one: its recognition source, or for an
/// executable the folder it runs from.
fn distinguishing_line(key: &str) -> String {
    let info = source(key);
    match info.kind {
        "Steam" | "Riot" => info.kind.into(),
        "Package" => "Microsoft Store".into(),
        _ => info
            .path
            .as_deref()
            .and_then(|path| path.rsplit(['\\', '/']).nth(1))
            // A drive root such as `c:` says nothing about the app.
            .filter(|folder| !folder.is_empty() && !folder.ends_with(':'))
            .map_or(info.line, str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn range_labels_compress_shared_parts() {
        let range = |start, end| DateRange { start, end };
        assert_eq!(
            range_label(range(date(11, 16), date(11, 22)), Period::Week),
            "Nov 16 – 22, 2026"
        );
        assert_eq!(
            range_label(range(date(10, 26), date(11, 1)), Period::Week),
            "Oct 26 – Nov 1, 2026"
        );
        assert_eq!(
            range_label(range(date(11, 1), date(11, 30)), Period::Custom),
            "November 2026"
        );
        assert_eq!(
            range_label(range(date(11, 20), date(11, 20)), Period::Custom),
            "Nov 20, 2026"
        );
    }

    #[test]
    fn sources_decode_only_the_evidence_in_the_key() {
        assert_eq!(source("steam:1086940").line, "Steam · App 1086940");
        assert_eq!(source("riot:valorant").kind, "Riot");
        let exe = source(r"exe:c:\program files\app\code.exe");
        assert_eq!(exe.line, "code.exe");
        assert_eq!(exe.path.as_deref(), Some(r"c:\program files\app\code.exe"));
        assert_eq!(
            source("package:spotifyab.spotifymusic_x!spotify").line,
            "Package · spotifyab.spotifymusic_x"
        );
        assert_eq!(source("observed").path, None);
    }

    #[test]
    fn full_sources_are_the_path_or_the_package_id() {
        assert_eq!(
            source(r"exe:c:\program files\app\code.exe").full(),
            r"c:\program files\app\code.exe"
        );
        assert_eq!(
            source("package:spotifyab.spotifymusic_x!spotify").full(),
            "spotifyab.spotifymusic_x!spotify"
        );
        assert_eq!(source("steam:1086940").full(), "Steam · App 1086940");
    }

    #[test]
    fn a_unique_name_gets_no_second_line() {
        let rows = [("steam:1", "Hades"), (r"exe:c:\games\tool.exe", "Tool")];
        assert!(disambiguation(&rows).is_empty());
    }

    #[test]
    fn same_named_entries_are_told_apart_by_source_or_folder() {
        let rows = [
            ("steam:1145360", "Hades"),
            (r"exe:d:\epic\hades\hades.exe", "Hades"),
            ("package:contoso.tool_x!tool", "Tool"),
            (r"exe:c:\tools\tool.exe", "tool"),
        ];
        let lines = disambiguation(&rows);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines["steam:1145360"], "Steam");
        assert_eq!(lines[r"exe:d:\epic\hades\hades.exe"], "hades");
        assert_eq!(lines["package:contoso.tool_x!tool"], "Microsoft Store");
        assert_eq!(lines[r"exe:c:\tools\tool.exe"], "tools");
    }

    #[test]
    fn name_matching_ignores_case() {
        let rows = [("riot:valorant", "VALORANT"), ("riot:client", "valorant")];
        let lines = disambiguation(&rows);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines["riot:valorant"], "Riot");
    }

    #[test]
    fn an_executable_at_the_root_of_a_drive_names_its_file() {
        let rows = [(r"exe:c:\tool.exe", "Tool"), (r"exe:d:\tool.exe", "Tool")];
        let lines = disambiguation(&rows);
        assert_eq!(lines[r"exe:c:\tool.exe"], "tool.exe");
    }

    #[test]
    fn durations_round_down_to_minutes() {
        assert_eq!(duration(0), "0m");
        assert_eq!(duration(59_000), "<1m");
        assert_eq!(duration(65 * 60_000), "1h 05m");
        assert_eq!(duration(120 * 60_000), "2h");
        assert_eq!(percent(1, 3), Some(33));
        assert_eq!(percent(1, 0), None);
    }
}
