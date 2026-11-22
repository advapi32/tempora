//! Activity-list rows derived from the day report's timeline.

use super::{MINUTE, clip};
use std::collections::HashMap;
use tempora::storage::{TimelineEntry, TimelineKind};

/// Focus stretches shorter than this fold into runs of quick switches.
const QUICK_SWITCH_MS: i64 = MINUTE;

/// A focus stretch of the Activity list: an app's, or the desktop's when `key` is `None`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stretch {
    pub start_ms: i64,
    pub end_ms: i64,
    pub key: Option<String>,
    pub name: String,
}

/// One row of the Activity list, in time order.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ActivityRow {
    Focus(Stretch),
    /// Stretches under a minute in a row, folded. `folded` keeps them in time order.
    QuickSwitches {
        start_ms: i64,
        end_ms: i64,
        total_ms: i64,
        /// Up to `QUICK_NAMES_SHOWN` app names by time, then `more` for the rest.
        names: Vec<String>,
        more: usize,
        folded: Vec<Stretch>,
    },
    Pause {
        start_ms: i64,
        end_ms: i64,
    },
    Gap {
        start_ms: i64,
        end_ms: i64,
    },
}

/// Apps named in a quick-switch row before "+N more".
const QUICK_NAMES_SHOWN: usize = 3;

/// The Activity list: the day's focus stretches in time order, clipped to `window`. Unknown and
/// idle entries are dropped and consecutive focus entries of one app become one stretch. A
/// stretch under a minute folds with the stretches in its run into one quick-switch row; a
/// pause or gap is a row of its own and ends a run.
pub(super) fn activity_rows(
    timeline: &[TimelineEntry],
    window: (i64, i64),
    names: &HashMap<&str, &str>,
) -> Vec<ActivityRow> {
    let mut rows: Vec<ActivityRow> = Vec::new();
    for entry in timeline {
        let Some((start_ms, end_ms)) = clip(entry.start_ms, entry.end_ms, window) else {
            continue;
        };
        let (key, name) = match &entry.kind {
            TimelineKind::Focus(key) => {
                let name = names.get(key.as_str()).copied().unwrap_or("App");
                (Some(key.as_str()), name)
            }
            TimelineKind::Desktop => (None, "Desktop"),
            TimelineKind::Paused => {
                rows.push(ActivityRow::Pause { start_ms, end_ms });
                continue;
            }
            TimelineKind::Gap => {
                rows.push(ActivityRow::Gap { start_ms, end_ms });
                continue;
            }
            TimelineKind::Unknown | TimelineKind::Idle => continue,
        };
        match rows.last_mut() {
            Some(ActivityRow::Focus(last)) if last.key.as_deref() == key => last.end_ms = end_ms,
            _ => rows.push(ActivityRow::Focus(Stretch {
                start_ms,
                end_ms,
                key: key.map(str::to_owned),
                name: name.to_owned(),
            })),
        }
    }
    let mut folded_rows = Vec::new();
    let mut run: Vec<Stretch> = Vec::new();
    for row in rows {
        match row {
            ActivityRow::Focus(stretch) if stretch.end_ms - stretch.start_ms < QUICK_SWITCH_MS => {
                run.push(stretch);
            }
            row => {
                fold(std::mem::take(&mut run), &mut folded_rows);
                folded_rows.push(row);
            }
        }
    }
    fold(run, &mut folded_rows);
    folded_rows
}

/// Folds a run of quick switches into one row. A lone quick switch stays an ordinary row:
/// folding it would only hide it behind "1 quick switch".
fn fold(mut run: Vec<Stretch>, rows: &mut Vec<ActivityRow>) {
    match run.len() {
        0 => {}
        1 => rows.extend(run.pop().map(ActivityRow::Focus)),
        _ => rows.push(quick_switches(run)),
    }
}

/// One quick-switch row for a non-empty run: the apps by time spent, and the run's total.
fn quick_switches(folded: Vec<Stretch>) -> ActivityRow {
    let mut by_name: Vec<(String, i64)> = Vec::new();
    for stretch in &folded {
        let ms = stretch.end_ms - stretch.start_ms;
        match by_name.iter_mut().find(|(name, _)| *name == stretch.name) {
            Some(app) => app.1 += ms,
            None => by_name.push((stretch.name.clone(), ms)),
        }
    }
    // Stable, so a tie keeps the app that appeared first.
    by_name.sort_by_key(|(_, ms)| std::cmp::Reverse(*ms));
    let more = by_name.len().saturating_sub(QUICK_NAMES_SHOWN);
    let names = by_name
        .iter()
        .take(QUICK_NAMES_SHOWN)
        .map(|(name, _)| name.clone())
        .collect();
    ActivityRow::QuickSwitches {
        start_ms: folded[0].start_ms,
        end_ms: folded[folded.len() - 1].end_ms,
        total_ms: folded.iter().map(|s| s.end_ms - s.start_ms).sum(),
        names,
        more,
        folded,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{HOUR, example_app_names};
    use super::*;

    const SECOND: i64 = 1_000;

    fn timeline(entries: Vec<(i64, i64, TimelineKind)>) -> Vec<TimelineEntry> {
        entries
            .into_iter()
            .map(|(start_ms, end_ms, kind)| TimelineEntry {
                start_ms,
                end_ms,
                kind,
            })
            .collect()
    }

    fn focus(key: &str) -> TimelineKind {
        TimelineKind::Focus(key.into())
    }

    /// The Activity rows for a timeline over the first hour.
    fn rows(entries: Vec<(i64, i64, TimelineKind)>) -> Vec<ActivityRow> {
        activity_rows(&timeline(entries), (0, HOUR), &example_app_names())
    }

    /// Each row as a short label: app name, desktop, quick-switch count, or the break's name.
    fn kinds(rows: &[ActivityRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                ActivityRow::Focus(stretch) => stretch.name.clone(),
                ActivityRow::QuickSwitches { folded, .. } => format!("{} quick", folded.len()),
                ActivityRow::Pause { .. } => "Paused".into(),
                ActivityRow::Gap { .. } => "Gap".into(),
            })
            .collect()
    }

    #[test]
    fn same_app_merges_across_an_unknown_blip() {
        let rows = rows(vec![
            (0, 10 * MINUTE, focus("brave")),
            (
                10 * MINUTE,
                10 * MINUTE + 30 * SECOND,
                TimelineKind::Unknown,
            ),
            (10 * MINUTE + 30 * SECOND, 20 * MINUTE, focus("brave")),
        ]);
        assert_eq!(
            rows,
            [ActivityRow::Focus(Stretch {
                start_ms: 0,
                end_ms: 20 * MINUTE,
                key: Some("brave".into()),
                name: "brave".into(),
            })]
        );
        assert!(self::rows(vec![(0, 10 * MINUTE, TimelineKind::Unknown)]).is_empty());
    }

    #[test]
    fn five_short_switches_fold_into_one_row_and_keep_their_order() {
        let rows = rows(vec![
            (0, 20 * SECOND, focus("brave")),
            (20 * SECOND, 40 * SECOND, focus("vesktop")),
            (40 * SECOND, 60 * SECOND, focus("spotify")),
            (60 * SECOND, 80 * SECOND, focus("brave")),
            (80 * SECOND, 100 * SECOND, focus("steam")),
        ]);
        let [
            ActivityRow::QuickSwitches {
                start_ms,
                end_ms,
                total_ms,
                names,
                more,
                folded,
            },
        ] = rows.as_slice()
        else {
            panic!("five short stretches make one quick-switch row: {rows:?}");
        };
        assert_eq!((*start_ms, *end_ms), (0, 100 * SECOND));
        assert_eq!(*total_ms, 100 * SECOND);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(names, ["brave", "vesktop", "Spotify"]);
        assert_eq!(*more, 1, "Steam is the fourth app");
        let starts: Vec<i64> = folded.iter().map(|stretch| stretch.start_ms).collect();
        assert_eq!(
            starts,
            [0, 20 * SECOND, 40 * SECOND, 60 * SECOND, 80 * SECOND]
        );
        let names: Vec<&str> = folded.iter().map(|stretch| stretch.name.as_str()).collect();
        assert_eq!(names, ["brave", "vesktop", "Spotify", "brave", "Steam"]);
    }

    #[test]
    fn a_pause_splits_a_run_and_each_part_is_its_own_row() {
        let rows = rows(vec![
            (0, 20 * SECOND, focus("brave")),
            (20 * SECOND, 40 * SECOND, focus("vesktop")),
            (40 * SECOND, 50 * SECOND, TimelineKind::Paused),
            (50 * SECOND, 70 * SECOND, focus("vesktop")),
            (70 * SECOND, 90 * SECOND, focus("spotify")),
        ]);
        assert_eq!(kinds(&rows), ["2 quick", "Paused", "2 quick"]);
    }

    #[test]
    fn stretches_of_a_minute_stand_alone_and_rows_are_in_time_order() {
        let rows = rows(vec![
            (0, 10 * MINUTE, focus("brave")),
            (10 * MINUTE, 12 * MINUTE, TimelineKind::Desktop),
            (12 * MINUTE, 12 * MINUTE + 30 * SECOND, focus("vesktop")),
            (13 * MINUTE, 14 * MINUTE, TimelineKind::Gap),
            (14 * MINUTE, 20 * MINUTE, focus("steam")),
        ]);
        // A lone quick switch isn't folded behind "1 quick switch"; it stays its own row.
        assert_eq!(
            kinds(&rows),
            ["brave", "Desktop", "vesktop", "Gap", "Steam"]
        );
        let starts: Vec<i64> = rows
            .iter()
            .map(|row| match row {
                ActivityRow::Focus(stretch) => stretch.start_ms,
                ActivityRow::QuickSwitches { start_ms, .. } => *start_ms,
                ActivityRow::Pause { start_ms, .. } | ActivityRow::Gap { start_ms, .. } => {
                    *start_ms
                }
            })
            .collect();
        assert!(
            starts.windows(2).all(|pair| pair[0] < pair[1]),
            "{starts:?}"
        );
    }

    #[test]
    fn only_stretches_inside_the_window_appear_clipped_to_it() {
        let window = (10 * MINUTE, 20 * MINUTE);
        let rows = activity_rows(
            &timeline(vec![
                (0, 5 * MINUTE, focus("brave")),
                (5 * MINUTE, 15 * MINUTE, focus("vesktop")),
                (25 * MINUTE, 30 * MINUTE, focus("steam")),
            ]),
            window,
            &example_app_names(),
        );
        assert_eq!(
            rows,
            [ActivityRow::Focus(Stretch {
                start_ms: 10 * MINUTE,
                end_ms: 15 * MINUTE,
                key: Some("vesktop".into()),
                name: "vesktop".into(),
            })]
        );
    }

    #[test]
    fn a_quick_switch_row_keeps_every_folded_stretch_in_order() {
        let rows = rows(vec![
            (0, 30 * SECOND, focus("steam")),
            (30 * SECOND, 45 * SECOND, TimelineKind::Desktop),
            (45 * SECOND, 50 * SECOND, focus("brave")),
        ]);
        let [ActivityRow::QuickSwitches { folded, .. }] = rows.as_slice() else {
            panic!("three short stretches make one row: {rows:?}");
        };
        let order: Vec<(&str, i64)> = folded
            .iter()
            .map(|stretch| (stretch.name.as_str(), stretch.start_ms))
            .collect();
        assert_eq!(
            order,
            [
                ("Steam", 0),
                ("Desktop", 30 * SECOND),
                ("brave", 45 * SECOND)
            ]
        );
        assert_eq!(folded[1].key, None, "desktop has no app");
    }
}
