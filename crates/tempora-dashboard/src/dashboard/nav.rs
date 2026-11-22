//! The single owner of what the dashboard is showing: page, shared date range, period,
//! filters and selection. Drill-downs push the originating view so Back restores it exactly.

use chrono::{Datelike, Days, NaiveDate, Weekday};
use tempora::model::{Category, DateRange};

#[derive(Clone, Debug, Hash, PartialEq)]
pub(super) enum Page {
    Overview,
    Library,
    App(String),
    Day(NaiveDate),
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Period {
    Week,
    Month,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum LibraryFilter {
    #[default]
    All,
    Games,
    Apps,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum LibrarySort {
    #[default]
    Focused,
    Running,
    Name,
    LastSeen,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum DayZoom {
    /// Whole hours around the day's recorded activity.
    #[default]
    Active,
    FullDay,
    Evening,
    /// A span picked on the lane or reached by zooming and panning it, as local clock times
    /// in milliseconds after midnight, so stepping to another day keeps the same hours, even
    /// across a daylight saving change.
    Custom {
        from: i64,
        to: i64,
    },
}

/// Everything a Back press restores.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct View {
    pub page: Page,
    pub range: DateRange,
    pub period: Period,
    pub exclude_idle: bool,
    /// Category filter shared by the Overview chips and the Library category menu.
    pub category: Option<Category>,
    pub library: LibraryFilter,
    pub sort: LibrarySort,
    pub search: String,
    /// Overview and App detail Chart/Table toggles.
    pub overview_table: bool,
    pub trend_table: bool,
    pub zoom: DayZoom,
    /// The preset a custom zoom goes back to when it is reset.
    pub zoom_base: DayZoom,
    /// App highlighted on the Day page.
    pub lane_app: Option<String>,
}

impl View {
    /// Shows a new lane zoom. Leaving a preset for a custom span remembers the preset.
    pub fn set_zoom(&mut self, zoom: DayZoom) {
        if !matches!(self.zoom, DayZoom::Custom { .. }) {
            self.zoom_base = self.zoom;
        }
        self.zoom = zoom;
    }

    /// Whether the lane shows a custom span rather than a preset.
    pub fn zoomed(&self) -> bool {
        matches!(self.zoom, DayZoom::Custom { .. })
    }

    /// Goes back from a custom span to the preset it was zoomed from.
    pub fn reset_zoom(&mut self) {
        if self.zoomed() {
            self.zoom = self.zoom_base;
        }
    }
}

/// Most page visits kept for each of the back and forward history stacks.
const HISTORY_LIMIT: usize = 50;

pub(super) struct Nav {
    pub view: View,
    /// Drill-down trail behind the breadcrumbs.
    back: Vec<View>,
    /// Browser-style page history, oldest first: each entry is the view a page was left in.
    history: Vec<View>,
    /// Entries stepped back over, most recent last.
    forward: Vec<View>,
    /// The view as of the last `observe`; a page change leaves this behind as a history entry.
    observed: View,
}

impl Nav {
    pub fn new(today: NaiveDate, week_start: Weekday) -> Self {
        let view = View {
            page: Page::Overview,
            range: period_range(Period::Week, today, week_start),
            period: Period::Week,
            exclude_idle: false,
            category: None,
            library: LibraryFilter::All,
            sort: LibrarySort::Focused,
            search: String::new(),
            overview_table: false,
            trend_table: false,
            zoom: DayZoom::Active,
            zoom_base: DayZoom::Active,
            lane_app: None,
        };
        Self {
            observed: view.clone(),
            view,
            back: Vec::new(),
            history: Vec::new(),
            forward: Vec::new(),
        }
    }

    /// Records a history entry when the page changed since the last call, keeping the view
    /// the old page was left in. Any page change, however it was made, clears the forward
    /// stack. Call once per frame before input is handled.
    pub fn observe(&mut self) {
        if self.view.page == self.observed.page {
            self.observed.clone_from(&self.view);
            return;
        }
        let left = std::mem::replace(&mut self.observed, self.view.clone());
        push_capped(&mut self.history, left);
        self.forward.clear();
    }

    /// Steps back to the previous page visit. Returns false when there is none.
    pub fn history_back(&mut self) -> bool {
        self.observe();
        let Some(entry) = self.history.pop() else {
            return false;
        };
        let leaving = std::mem::replace(&mut self.view, entry);
        push_capped(&mut self.forward, leaving);
        self.restored();
        true
    }

    /// Steps forward again after `history_back`. Returns false when there is nothing to redo.
    pub fn history_forward(&mut self) -> bool {
        self.observe();
        let Some(entry) = self.forward.pop() else {
            return false;
        };
        let leaving = std::mem::replace(&mut self.view, entry);
        push_capped(&mut self.history, leaving);
        self.restored();
        true
    }

    /// A history step replaces the drill trail, which described how the old page was reached.
    fn restored(&mut self) {
        self.observed.clone_from(&self.view);
        self.back.clear();
    }

    pub fn page(&self) -> &Page {
        &self.view.page
    }

    /// Top-level navigation keeps the shared range and filters but starts a new trail.
    pub fn section(&mut self, page: Page) {
        if self.view.page != page {
            self.back.clear();
            self.view.page = page;
        }
    }

    /// Opens a page from inside another one and remembers where it came from.
    pub fn drill(&mut self, page: Page) {
        self.drill_with(page, |_| {});
    }

    pub fn drill_with(&mut self, page: Page, adjust: impl FnOnce(&mut View)) {
        let origin = self.view.clone();
        self.view.page = page;
        adjust(&mut self.view);
        if self.view != origin {
            self.back.push(origin);
        }
    }

    pub fn origin(&self) -> Option<&View> {
        self.back.last()
    }

    pub fn go_back(&mut self) -> bool {
        match self.back.pop() {
            Some(view) => {
                self.view = view;
                true
            }
            None => false,
        }
    }

    pub fn set_range(&mut self, period: Period, range: DateRange) {
        self.view.period = period;
        self.view.range = range;
    }

    /// The adjacent period, or `None` past the current one.
    pub fn adjacent(&self, next: bool, week_start: Weekday, today: NaiveDate) -> Option<DateRange> {
        let range = self.view.range;
        let shifted = match self.view.period {
            Period::Week => {
                let date = if next {
                    range.start.checked_add_days(Days::new(7))
                } else {
                    range.start.checked_sub_days(Days::new(7))
                };
                date.map(|date| period_range(Period::Week, date, week_start))
            }
            Period::Month => shift_month(range.start, if next { 1 } else { -1 })
                .map(|date| period_range(Period::Month, date, week_start)),
            Period::Custom => {
                let span = (range.end - range.start).num_days() + 1;
                let shift = chrono::Duration::days(if next { span } else { -span });
                range
                    .start
                    .checked_add_signed(shift)
                    .zip(range.end.checked_add_signed(shift))
                    .map(|(start, end)| DateRange { start, end })
            }
        }?;
        (!next || shifted.start <= today).then_some(shifted)
    }
}

fn push_capped(stack: &mut Vec<View>, view: View) {
    stack.push(view);
    if stack.len() > HISTORY_LIMIT {
        stack.remove(0);
    }
}

pub(super) fn shift_month(date: NaiveDate, by: i32) -> Option<NaiveDate> {
    let month = date.year() * 12 + date.month0() as i32 + by;
    NaiveDate::from_ymd_opt(month.div_euclid(12), month.rem_euclid(12) as u32 + 1, 1)
}

pub(super) fn week_start_of(date: NaiveDate, week_start: Weekday) -> NaiveDate {
    date - Days::new(date.weekday().days_since(week_start).into())
}

pub(super) fn period_range(period: Period, date: NaiveDate, week_start: Weekday) -> DateRange {
    match period {
        Period::Week => {
            let start = week_start_of(date, week_start);
            DateRange {
                start,
                end: start + Days::new(6),
            }
        }
        Period::Month => DateRange {
            start: date.with_day(1).unwrap_or(date),
            end: shift_month(date, 1)
                .and_then(|next| next.pred_opt())
                .unwrap_or(date),
        },
        Period::Custom => DateRange {
            start: date,
            end: date,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Preset {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    Last30Days,
    LastQuarter,
}

impl Preset {
    pub const ALL: [Self; 8] = [
        Self::Today,
        Self::Yesterday,
        Self::ThisWeek,
        Self::LastWeek,
        Self::ThisMonth,
        Self::LastMonth,
        Self::Last30Days,
        Self::LastQuarter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::ThisWeek => "This week",
            Self::LastWeek => "Last week",
            Self::ThisMonth => "This month",
            Self::LastMonth => "Last month",
            Self::Last30Days => "Last 30 days",
            Self::LastQuarter => "Last quarter",
        }
    }

    pub fn resolve(self, today: NaiveDate, week_start: Weekday) -> (Period, DateRange) {
        let day = |date: NaiveDate| DateRange {
            start: date,
            end: date,
        };
        let yesterday = today.pred_opt().unwrap_or(today);
        match self {
            Self::Today => (Period::Custom, day(today)),
            Self::Yesterday => (Period::Custom, day(yesterday)),
            Self::ThisWeek => (Period::Week, period_range(Period::Week, today, week_start)),
            Self::LastWeek => (
                Period::Week,
                period_range(Period::Week, today - Days::new(7), week_start),
            ),
            Self::ThisMonth => (
                Period::Month,
                period_range(Period::Month, today, week_start),
            ),
            Self::LastMonth => (
                Period::Month,
                period_range(
                    Period::Month,
                    shift_month(today, -1).unwrap_or(today),
                    week_start,
                ),
            ),
            Self::Last30Days => (
                Period::Custom,
                DateRange {
                    start: today - Days::new(29),
                    end: today,
                },
            ),
            Self::LastQuarter => {
                let quarter_start =
                    NaiveDate::from_ymd_opt(today.year(), today.month0() / 3 * 3 + 1, 1)
                        .unwrap_or(today);
                let start = shift_month(quarter_start, -3).unwrap_or(quarter_start);
                let end = quarter_start.pred_opt().unwrap_or(quarter_start);
                (Period::Custom, DateRange { start, end })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn back_restores_the_originating_range_filters_and_selection() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        nav.set_range(
            Period::Month,
            period_range(Period::Month, date(10, 4), Weekday::Mon),
        );
        nav.view.category = Some(Category::Games);
        nav.view.overview_table = true;
        let origin = nav.view.clone();
        nav.drill(Page::Day(date(10, 12)));
        nav.view.zoom = DayZoom::Evening;
        nav.drill_with(Page::App("steam:1".into()), |view| view.trend_table = true);
        assert!(nav.go_back());
        assert_eq!(nav.view.page, Page::Day(date(10, 12)));
        assert_eq!(nav.view.zoom, DayZoom::Evening);
        assert!(nav.go_back());
        assert_eq!(nav.view, origin);
        assert!(!nav.go_back());
    }

    #[test]
    fn resetting_a_custom_zoom_returns_to_the_preset_it_left() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        nav.view.set_zoom(DayZoom::Evening);
        nav.view.set_zoom(DayZoom::Custom { from: 1, to: 2 });
        nav.view.set_zoom(DayZoom::Custom { from: 1, to: 3 });
        assert!(nav.view.zoomed());
        nav.view.reset_zoom();
        assert_eq!(
            nav.view.zoom,
            DayZoom::Evening,
            "back past every nested zoom"
        );
        nav.view.reset_zoom();
        assert_eq!(nav.view.zoom, DayZoom::Evening, "a preset stays put");
    }

    #[test]
    fn sections_keep_the_shared_range_and_clear_the_trail() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        let range = nav.view.range;
        nav.drill(Page::Day(date(11, 18)));
        nav.section(Page::Library);
        assert!(nav.origin().is_none());
        assert_eq!(nav.view.range, range);
    }

    #[test]
    fn forward_navigation_stops_at_the_current_period() {
        let today = date(11, 20);
        let mut nav = Nav::new(today, Weekday::Mon);
        assert_eq!(nav.adjacent(true, Weekday::Mon, today), None);
        let previous = nav.adjacent(false, Weekday::Mon, today).unwrap();
        assert_eq!(previous.start, date(11, 9));
        nav.set_range(Period::Week, previous);
        assert_eq!(
            nav.adjacent(true, Weekday::Mon, today).unwrap().start,
            date(11, 16)
        );
    }

    /// One frame of a click: the frame observes what the previous one left, then the page changes.
    fn go(nav: &mut Nav, page: Page) {
        nav.observe();
        nav.view.page = page;
        nav.observe();
    }

    #[test]
    fn history_steps_back_and_forward_in_order_restoring_each_view() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        nav.view.category = Some(Category::Games);
        let overview = nav.view.clone();
        go(&mut nav, Page::Library);
        go(&mut nav, Page::Day(date(11, 18)));
        let day = nav.view.clone();

        assert!(nav.history_back());
        assert_eq!(nav.view.page, Page::Library);
        assert!(nav.history_back());
        assert_eq!(nav.view, overview);
        assert!(!nav.history_back());

        assert!(nav.history_forward());
        assert_eq!(nav.view.page, Page::Library);
        assert!(nav.history_forward());
        assert_eq!(nav.view, day);
        assert!(!nav.history_forward());
    }

    #[test]
    fn restored_entries_keep_the_range_they_were_left_with() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        let range = period_range(Period::Month, date(10, 4), Weekday::Mon);
        nav.set_range(Period::Month, range);
        go(&mut nav, Page::Library);
        nav.set_range(
            Period::Week,
            period_range(Period::Week, date(11, 2), Weekday::Mon),
        );
        assert!(nav.history_back());
        assert_eq!(nav.view.range, range);
        assert_eq!(nav.view.period, Period::Month);
    }

    #[test]
    fn a_new_navigation_clears_the_forward_stack() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        go(&mut nav, Page::Library);
        go(&mut nav, Page::Settings);
        assert!(nav.history_back());
        assert_eq!(nav.view.page, Page::Library);
        go(&mut nav, Page::App("steam:1".into()));
        assert!(!nav.history_forward());
        assert_eq!(nav.view.page, Page::App("steam:1".into()));
        assert!(nav.history_back());
        assert_eq!(nav.view.page, Page::Library);
    }

    #[test]
    fn same_page_navigation_records_nothing() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        go(&mut nav, Page::Overview);
        nav.section(Page::Overview);
        nav.set_range(
            Period::Month,
            period_range(Period::Month, date(10, 4), Weekday::Mon),
        );
        go(&mut nav, Page::Overview);
        assert!(!nav.history_back());
        go(&mut nav, Page::Library);
        go(&mut nav, Page::Library);
        assert!(nav.history_back());
        assert!(!nav.history_back());
    }

    #[test]
    fn history_is_capped_and_drops_the_oldest_visits() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        for day in 0..(HISTORY_LIMIT as u64 + 10) {
            go(&mut nav, Page::Day(date(1, 1) + Days::new(day)));
        }
        let mut steps = 0;
        while nav.history_back() {
            steps += 1;
        }
        assert_eq!(steps, HISTORY_LIMIT);
        assert_eq!(nav.view.page, Page::Day(date(1, 1) + Days::new(9)));
    }

    #[test]
    fn history_back_does_not_disturb_the_drill_trail_once_it_is_replaced() {
        let mut nav = Nav::new(date(11, 20), Weekday::Mon);
        go(&mut nav, Page::Library);
        nav.drill(Page::App("steam:1".into()));
        assert!(nav.origin().is_some());
        assert!(nav.history_back());
        assert!(nav.origin().is_none());
        assert!(nav.history_forward());
        assert_eq!(nav.view.page, Page::App("steam:1".into()));
    }

    #[test]
    fn presets_follow_the_week_start_and_calendar_quarters() {
        let today = date(11, 20);
        let (_, week) = Preset::ThisWeek.resolve(today, Weekday::Sun);
        assert_eq!((week.start, week.end), (date(11, 15), date(11, 21)));
        let (_, quarter) = Preset::LastQuarter.resolve(today, Weekday::Mon);
        assert_eq!((quarter.start, quarter.end), (date(7, 1), date(9, 30)));
        let (_, month) = Preset::LastMonth.resolve(today, Weekday::Mon);
        assert_eq!((month.start, month.end), (date(10, 1), date(10, 31)));
        let (_, days) = Preset::Last30Days.resolve(today, Weekday::Mon);
        assert_eq!((days.end - days.start).num_days(), 29);
    }
}
