use eframe::egui::{
    Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, Ui, WidgetInfo, WidgetType,
    pos2, vec2,
};
use tempora::model::Category;

use super::{Tokens, chart_block, chart_label, focus_ring, snap};

const HOUR: i64 = 3_600_000;
const MINUTE: i64 = 60_000;
const SEGMENT_GAP: f32 = 2.0;
const DIMMED: f32 = 0.2;
/// A hovered chart column is washed with half the hover color, so the gridlines behind it
/// still show.
const COLUMN_WASH: f32 = 0.5;
/// A dimmed lane block keeps this share of its color.
const LANE_DIMMED: f32 = 0.3;

pub(in crate::dashboard) struct Segment {
    pub color: Color32,
    pub ms: i64,
    pub dimmed: bool,
    /// The series it belongs to, named in the slice's tooltip.
    pub name: &'static str,
}

/// Slices thinner than this still answer to the pointer within half of it.
const SLICE_REACH: f32 = 6.0;

pub(in crate::dashboard) struct StackedBar {
    /// Axis label under the bar.
    pub label: String,
    /// Tooltip and accessible name.
    pub description: String,
    /// Total shown above the bar in week view.
    pub total: Option<String>,
    pub segments: Vec<Segment>,
    pub today: bool,
    /// Days after today draw an empty placeholder and are not clickable.
    pub future: bool,
    /// The second line of a slice's tooltip, naming where the slice sits (the day). Without
    /// it the bar keeps the plain `description` tooltip.
    pub slice_detail: Option<String>,
}

/// A slice's tooltip: its swatch, series and duration, with the day underneath.
fn slice_tip(ui: &mut Ui, segment: &Segment, detail: &str) {
    let colors = Tokens::get(ui);
    // Tooltip rows size to their text, not to the app's 36 px control height.
    ui.spacing_mut().interact_size.y = 0.0;
    ui.spacing_mut().item_spacing = vec2(8.0, 2.0);
    ui.horizontal(|ui| {
        super::swatch(ui, segment.color, 8.0);
        ui.label(
            eframe::egui::RichText::new(segment.name)
                .font(Tokens::medium(Tokens::SECONDARY))
                .color(colors.fg),
        );
        ui.add_space(4.0);
        ui.label(
            eframe::egui::RichText::new(crate::dashboard::format::duration(segment.ms))
                .font(Tokens::mono_medium(Tokens::SECONDARY))
                .color(colors.fg),
        );
    });
    ui.label(
        eframe::egui::RichText::new(detail)
            .size(Tokens::CAPTION)
            .color(colors.fg_subtle),
    );
}

/// Picks a step so five gridlines cover `max_ms`, down to five minutes, so a light week isn't
/// drawn as slivers under an hours-high axis. With nothing to show the axis stays in hours.
pub(in crate::dashboard) fn axis_step(max_ms: i64) -> i64 {
    if max_ms <= 0 {
        return HOUR;
    }
    [5, 10, 15, 20, 30, 60, 120, 180, 240, 360, 720, 1440]
        .into_iter()
        .map(|minutes| minutes * MINUTE)
        .find(|step| step * 5 >= max_ms)
        .unwrap_or(24 * HOUR)
}

/// Compact axis labels: `15m`, `1h`, `1h30`.
pub(in crate::dashboard) fn axis_label(ms: i64) -> String {
    let (hours, minutes) = (ms / HOUR, ms % HOUR / MINUTE);
    match (hours, minutes) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h{minutes:02}"),
    }
}

/// Stacked daily bars with y gridlines. Returns the index of a clicked or activated bar.
pub(in crate::dashboard) fn stacked_bars(
    ui: &mut Ui,
    bars: &[StackedBar],
    week: bool,
    height: f32,
) -> Option<usize> {
    const AXIS_WIDTH: f32 = 42.0;
    const TOP: f32 = 18.0;
    const BOTTOM: f32 = 24.0;
    let colors = Tokens::get(ui);
    let (outer, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), height + TOP + BOTTOM),
        Sense::hover(),
    );
    let plot = Rect::from_min_max(
        outer.min + vec2(AXIS_WIDTH, TOP),
        outer.max - vec2(0.0, BOTTOM),
    );
    let max = bars
        .iter()
        .map(|bar| bar.segments.iter().map(|segment| segment.ms).sum::<i64>())
        .max()
        .unwrap_or(0);
    let step = axis_step(max);
    let scale = plot.height() / (step * 5) as f32;
    for index in 0..=5 {
        let y = plot.bottom() - plot.height() * index as f32 / 5.0;
        let label = if index == 0 {
            "0".to_owned()
        } else {
            axis_label(step * index)
        };
        chart_label(ui, pos2(plot.left() - 8.0, y), Align2::RIGHT_CENTER, &label);
        let line = if index == 0 {
            colors.border
        } else {
            colors.chart_grid
        };
        ui.painter()
            .hline(plot.x_range(), snap(ui, y), Stroke::new(1.0, line));
    }
    if bars.is_empty() {
        return None;
    }
    let slot = plot.width() / bars.len() as f32;
    let bar_width = if week {
        46.0f32.min(slot * 0.8)
    } else {
        slot * 0.72
    };
    let mut clicked = None;
    for (index, bar) in bars.iter().enumerate() {
        let x = plot.left() + (index as f32 + 0.5) * slot;
        let column = Rect::from_center_size(
            pos2(x, plot.center().y),
            vec2((slot - 2.0).max(1.0), plot.height() + TOP),
        )
        .translate(vec2(0.0, -TOP / 2.0));
        let total: i64 = bar.segments.iter().map(|segment| segment.ms).sum();
        let rect = Rect::from_min_max(
            pos2(x - bar_width / 2.0, plot.bottom() - total as f32 * scale),
            pos2(x + bar_width / 2.0, plot.bottom()),
        );
        let slices = if total > 0 {
            segment_rects(rect, &bar.segments, total)
        } else {
            Vec::new()
        };
        let response = (!bar.future).then(|| {
            let response = ui.interact(column, ui.next_auto_id(), Sense::click());
            ui.skip_ahead_auto_ids(1);
            response.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &bar.description)
            });
            response
        });
        let hovered = response.as_ref().filter(|response| response.hovered());
        let slice = hovered
            .and(bar.slice_detail.as_ref())
            .and_then(|_| ui.ctx().pointer_hover_pos())
            .and_then(|pointer| slice_at(&slices, pointer));
        if hovered.is_some() {
            ui.painter()
                .rect_filled(column, 6, colors.hover.gamma_multiply(COLUMN_WASH));
        }
        // A day still to come keeps its slot so the week doesn't shift, but draws no bar: a stub
        // there would read as a little recorded time.
        if !bar.future && total > 0 {
            paint_segments(ui, &bar.segments, &slices, slice);
            if week && let Some(total) = &bar.total {
                chart_label(ui, pos2(x, rect.top() - 4.0), Align2::CENTER_BOTTOM, total);
            }
        }
        let show_label = week || bars.len() <= 16 || index % 2 == 0 || bar.today;
        if show_label {
            ui.painter().text(
                snap(ui, pos2(x, plot.bottom() + 6.0)),
                Align2::CENTER_TOP,
                &bar.label,
                if bar.today {
                    Tokens::mono_medium(if week { Tokens::CAPTION } else { 10.0 })
                } else {
                    Tokens::mono(if week { Tokens::CAPTION } else { 10.0 })
                },
                if bar.today {
                    colors.fg
                } else if bar.future {
                    colors.fg_subtle.gamma_multiply(0.6)
                } else {
                    colors.fg_muted
                },
            );
        }
        if let Some(mut response) = response {
            match (&bar.slice_detail, slice) {
                // Shown at once: the chart is read by moving across its slices. Between
                // slices there is no tooltip: the total above the bar already says it.
                (Some(detail), Some(slice)) => {
                    eframe::egui::Tooltip::always_open(
                        ui.ctx().clone(),
                        ui.layer_id(),
                        response.id,
                        eframe::egui::PopupAnchor::Pointer,
                    )
                    .gap(14.0)
                    .show(|ui| slice_tip(ui, &bar.segments[slice], detail));
                }
                (Some(_), None) => {}
                (None, _) => response = response.on_hover_text(&bar.description),
            }
            focus_ring(ui, &response, 6);
            if response.clicked() {
                clicked = Some(index);
            }
        }
    }
    clicked
}

fn bar_response(ui: &mut Ui, rect: Rect, description: &str) -> Response {
    let response = ui.interact(rect, ui.next_auto_id(), Sense::click());
    ui.skip_ahead_auto_ids(1);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), description));
    response.on_hover_text(description)
}

/// Each visible segment's index and rectangle, bottom up, with a small gap between them.
fn segment_rects(rect: Rect, segments: &[Segment], total: i64) -> Vec<(usize, Rect)> {
    let visible: Vec<_> = segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| segment.ms > 0)
        .collect();
    let count = visible.len();
    let gap = SEGMENT_GAP.min(rect.height() / count.max(1) as f32 / 2.0);
    let usable = rect.height() - gap * count.saturating_sub(1) as f32;
    let mut bottom = rect.bottom();
    visible
        .into_iter()
        .map(|(index, segment)| {
            let top = bottom - usable * segment.ms as f32 / total as f32;
            let slice = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom));
            bottom = top - gap;
            (index, slice)
        })
        .collect()
}

/// The segment under `pointer`: the nearest slice vertically, so thin ones stay reachable.
fn slice_at(slices: &[(usize, Rect)], pointer: Pos2) -> Option<usize> {
    slices
        .iter()
        .filter(|(_, rect)| rect.x_range().contains(pointer.x))
        .map(|(index, rect)| {
            let distance = if rect.y_range().contains(pointer.y) {
                0.0
            } else {
                (rect.top() - pointer.y)
                    .abs()
                    .min((rect.bottom() - pointer.y).abs())
            };
            (*index, distance)
        })
        .filter(|(_, distance)| *distance <= SLICE_REACH / 2.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// Paints the slices; while one is hovered the others in its bar fade so it stands out.
fn paint_segments(ui: &Ui, segments: &[Segment], slices: &[(usize, Rect)], hovered: Option<usize>) {
    let count = slices.len();
    for (position, (index, slice)) in slices.iter().enumerate() {
        let segment = &segments[*index];
        let top_radius = if position + 1 == count { 4 } else { 1 };
        let radius = CornerRadius {
            nw: top_radius,
            ne: top_radius,
            sw: 1,
            se: 1,
        };
        let faded = segment.dimmed || hovered.is_some_and(|hovered| hovered != *index);
        let color = if faded {
            segment.color.gamma_multiply(DIMMED)
        } else {
            segment.color
        };
        ui.painter().rect_filled(snap(ui, *slice), radius, color);
    }
}

pub(in crate::dashboard) fn sparkline(ui: &mut Ui, category: Category, activity: [u8; 7]) {
    let colors = Tokens::get(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(53.0, 24.0), Sense::hover());
    let max = f32::from(activity.iter().copied().max().unwrap_or(1).max(1));
    for (index, amount) in activity.into_iter().enumerate() {
        let height = (rect.height() * f32::from(amount) / max).max(2.0);
        let bar = Rect::from_min_size(
            pos2(rect.left() + index as f32 * 8.0, rect.bottom() - height),
            vec2(5.0, height),
        );
        let fill = if amount == 0 {
            colors.border
        } else {
            colors.category(category)
        };
        ui.painter().rect_filled(snap(ui, bar), 1, fill);
    }
    chart_block(
        ui,
        rect,
        &format!("Seven days of focused time: {activity:?}"),
        false,
    );
}

pub(in crate::dashboard) struct PairedBar {
    pub label: String,
    pub description: String,
    /// Solid front bar (focused).
    pub front_ms: i64,
    /// Tinted bar behind it (running).
    pub back_ms: i64,
    pub today: bool,
}

/// Daily trend: a tinted running bar behind a solid focused bar. Returns the clicked bar.
pub(in crate::dashboard) fn paired_bars(
    ui: &mut Ui,
    bars: &[PairedBar],
    color: Color32,
    height: f32,
) -> Option<usize> {
    const AXIS_WIDTH: f32 = 42.0;
    const BOTTOM: f32 = 20.0;
    let colors = Tokens::get(ui);
    let (outer, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), height + BOTTOM), Sense::hover());
    let plot = Rect::from_min_max(
        outer.min + vec2(AXIS_WIDTH, 0.0),
        outer.max - vec2(0.0, BOTTOM),
    );
    let max = bars
        .iter()
        .map(|bar| bar.back_ms.max(bar.front_ms))
        .max()
        .unwrap_or(0);
    let step = axis_step(max);
    let scale = plot.height() / (step * 5) as f32;
    for index in 0..=5 {
        let y = plot.bottom() - plot.height() * index as f32 / 5.0;
        if index > 0 {
            chart_label(
                ui,
                pos2(plot.left() - 8.0, y),
                Align2::RIGHT_CENTER,
                &axis_label(step * index),
            );
        }
        let line = if index == 0 {
            colors.border
        } else {
            colors.chart_grid
        };
        ui.painter()
            .hline(plot.x_range(), snap(ui, y), Stroke::new(1.0, line));
    }
    if bars.is_empty() {
        return None;
    }
    let slot = plot.width() / bars.len() as f32;
    let width = (slot * 0.7).clamp(1.0, 28.0);
    let label_every = (bars.len() / 12).max(1);
    let mut clicked = None;
    for (index, bar) in bars.iter().enumerate() {
        let x = plot.left() + (index as f32 + 0.5) * slot;
        let column = Rect::from_center_size(
            pos2(x, plot.center().y),
            vec2((slot - 1.0).max(1.0), plot.height()),
        );
        let response = bar_response(ui, column, &bar.description);
        if response.hovered() {
            ui.painter()
                .rect_filled(column, 4, colors.hover.gamma_multiply(COLUMN_WASH));
        }
        let radius = CornerRadius {
            nw: 3,
            ne: 3,
            sw: 0,
            se: 0,
        };
        for (ms, fill) in [
            (bar.back_ms, color.gamma_multiply(0.28)),
            (bar.front_ms, color),
        ] {
            if ms > 0 {
                let top = plot.bottom() - (ms as f32 * scale).max(1.0);
                ui.painter().rect_filled(
                    snap(
                        ui,
                        Rect::from_min_max(
                            pos2(x - width / 2.0, top),
                            pos2(x + width / 2.0, plot.bottom()),
                        ),
                    ),
                    radius,
                    fill,
                );
            }
        }
        if index % label_every == 0 || bar.today {
            ui.painter().text(
                snap(ui, pos2(x, plot.bottom() + 5.0)),
                Align2::CENTER_TOP,
                &bar.label,
                if bar.today {
                    Tokens::mono_medium(10.0)
                } else {
                    Tokens::mono(10.0)
                },
                if bar.today {
                    colors.fg
                } else {
                    colors.fg_subtle
                },
            );
        }
        focus_ring(ui, &response, 4);
        if response.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}

/// 24 hourly bars of focused time with 00/06/12/18/24 labels. Each bar has a tooltip;
/// `summary` is the chart's accessible name.
pub(in crate::dashboard) fn hour_histogram(
    ui: &mut Ui,
    hours: &[i64; 24],
    color: Color32,
    summary: &str,
) {
    const HEIGHT: f32 = 64.0;
    const GAP: f32 = 2.0;
    let colors = Tokens::get(ui);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT + 16.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Image, true, summary));
    let max = hours.iter().copied().max().unwrap_or(0).max(1);
    let slot = (rect.width() + GAP) / 24.0;
    for (hour, ms) in hours.iter().enumerate() {
        let height = (HEIGHT * *ms as f32 / max as f32).max(2.0);
        let bar = Rect::from_min_max(
            pos2(
                rect.left() + hour as f32 * slot,
                rect.top() + HEIGHT - height,
            ),
            pos2(
                rect.left() + hour as f32 * slot + slot - GAP,
                rect.top() + HEIGHT,
            ),
        );
        let fill = if *ms == 0 {
            colors.border
        } else {
            color.gamma_multiply(0.35 + 0.65 * *ms as f32 / max as f32)
        };
        ui.painter().rect_filled(snap(ui, bar), 2, fill);
        let hit = Rect::from_min_max(
            pos2(bar.left(), rect.top()),
            pos2(bar.right(), rect.top() + HEIGHT),
        );
        let label = format!(
            "{hour:02}:00 – {:02}:00, {} min focused",
            hour + 1,
            ms / 60_000
        );
        ui.interact(hit, ui.id().with(("hour", hour)), Sense::hover())
            .on_hover_text(label);
    }
    for (index, label) in ["00", "06", "12", "18", "24"].iter().enumerate() {
        let align = match index {
            0 => Align2::LEFT_BOTTOM,
            4 => Align2::RIGHT_BOTTOM,
            _ => Align2::CENTER_BOTTOM,
        };
        chart_label(
            ui,
            pos2(
                rect.left() + rect.width() * index as f32 / 4.0,
                rect.bottom(),
            ),
            align,
            label,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_finds_its_slice_and_thin_slices_stay_reachable() {
        let segment = |ms, name| Segment {
            color: Color32::WHITE,
            ms,
            dimmed: false,
            name,
        };
        // 100 px tall: a 1-minute sliver on top of 99 minutes, and an empty series skipped.
        let segments = [
            segment(99 * MINUTE, "Games"),
            segment(0, "Media"),
            segment(MINUTE, "Other"),
        ];
        let bar = Rect::from_min_max(pos2(0.0, 0.0), pos2(40.0, 100.0));
        let slices = segment_rects(bar, &segments, 100 * MINUTE);
        assert_eq!(
            slices.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            [0, 2],
            "empty series draw no slice"
        );
        let sliver = slices[1].1;
        assert!(sliver.height() < 2.0, "{sliver:?}");
        assert_eq!(slice_at(&slices, pos2(20.0, 60.0)), Some(0));
        assert_eq!(
            slice_at(&slices, pos2(20.0, sliver.top() - 2.0)),
            Some(2),
            "just above the sliver still reaches it"
        );
        assert_eq!(slice_at(&slices, pos2(20.0, sliver.top() - 10.0)), None);
        assert_eq!(slice_at(&slices, pos2(50.0, 60.0)), None, "beside the bar");
    }

    #[test]
    fn axis_covers_the_tallest_bar_in_five_steps() {
        assert_eq!(axis_step(0), HOUR, "an empty chart stays in hours");
        assert_eq!(
            axis_step(11 * MINUTE),
            5 * MINUTE,
            "an 11-minute day gets 0-25m"
        );
        assert_eq!(axis_step(70 * MINUTE), 15 * MINUTE);
        assert_eq!(axis_step(2 * HOUR), 30 * MINUTE);
        assert_eq!(axis_step(5 * HOUR), HOUR);
        assert_eq!(axis_step(385 * 60_000), 2 * HOUR);
        assert_eq!(axis_step(11 * HOUR), 3 * HOUR);
        assert_eq!(axis_step(30 * HOUR), 6 * HOUR);
        assert_eq!(axis_step(31 * HOUR), 12 * HOUR);
    }

    #[test]
    fn the_in_front_share_is_focused_over_running() {
        assert_eq!(in_front_share(HOUR, 4 * HOUR), 0.25);
        assert_eq!(
            in_front_share(0, HOUR),
            0.0,
            "open the whole time, never in front"
        );
        assert_eq!(
            in_front_share(HOUR, 0),
            0.0,
            "no running time, nothing to show"
        );
        assert_eq!(
            in_front_share(2 * HOUR, HOUR),
            1.0,
            "capped at the running time"
        );
    }

    #[test]
    fn axis_labels_are_compact() {
        assert_eq!(axis_label(15 * MINUTE), "15m");
        assert_eq!(axis_label(HOUR), "1h");
        assert_eq!(axis_label(90 * MINUTE), "1h30");
        assert_eq!(axis_label(5 * HOUR), "5h");
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::dashboard) enum LaneStyle {
    /// Focused time in a color.
    Solid(Color32),
    /// Desktop or an excluded app.
    Neutral,
    /// Not recording: dashed outline and hatch.
    Gap,
}

pub(in crate::dashboard) struct LaneBlock {
    /// Position inside the lane window, 0..1.
    pub start: f32,
    pub end: f32,
    pub style: LaneStyle,
    /// Painted inside the block when it fits.
    pub label: Option<String>,
    /// Tooltip and accessible name.
    pub description: String,
    /// A richer tooltip shown instead of `description`, without the hover delay.
    pub tooltip: Option<LaneTip>,
    /// A solid block drawn at reduced emphasis, without its label.
    pub dimmed: bool,
}

/// A lane block's tooltip: a title, detail lines, and a quiet hint about what clicking does.
pub(in crate::dashboard) struct LaneTip {
    pub title: String,
    pub lines: Vec<String>,
    pub hint: Option<String>,
}

fn lane_tip(ui: &mut Ui, tip: &LaneTip) {
    let colors = Tokens::get(ui);
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(
        eframe::egui::RichText::new(&tip.title)
            .font(Tokens::medium(Tokens::SECONDARY))
            .color(colors.fg),
    );
    for line in &tip.lines {
        ui.label(
            eframe::egui::RichText::new(line)
                .font(Tokens::mono(Tokens::CAPTION))
                .color(colors.fg_muted),
        );
    }
    if let Some(hint) = &tip.hint {
        ui.add_space(4.0);
        ui.label(
            eframe::egui::RichText::new(hint)
                .size(Tokens::CAPTION)
                .color(colors.fg_subtle),
        );
    }
}

/// The share of a session's running time that was in front, from 0 to 1.
pub(in crate::dashboard) fn in_front_share(focused_ms: i64, running_ms: i64) -> f32 {
    if running_ms <= 0 {
        return 0.0;
    }
    (focused_ms as f32 / running_ms as f32).clamp(0.0, 1.0)
}

/// A session's open-versus-in-front bar: the in-front share in solid, and the rest of the
/// running time at 30%. `label` is the tooltip and accessible name.
/// Fills its cell's width, so a wide column gives the share more room to read.
pub(in crate::dashboard) fn open_bar(ui: &mut Ui, color: Color32, share: f32, label: &str) {
    const HEIGHT: f32 = 8.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    let rect = snap(ui, rect);
    ui.painter().rect_filled(rect, 3, color.gamma_multiply(0.3));
    let front = Rect::from_min_size(rect.min, vec2(rect.width() * share.clamp(0.0, 1.0), HEIGHT));
    if front.width() > 0.0 {
        ui.painter().rect_filled(snap(ui, front), 3, color);
    }
    chart_block(ui, rect, label, false);
}

/// Tick labels above a lane: (position 0..1, label).
pub(in crate::dashboard) fn lane_axis(ui: &mut Ui, ticks: &[(f32, String)]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let last = ticks.len().saturating_sub(1);
    for (index, (at, label)) in ticks.iter().enumerate() {
        let align = if index == 0 {
            Align2::LEFT_CENTER
        } else if index == last {
            Align2::RIGHT_CENTER
        } else {
            Align2::CENTER_CENTER
        };
        chart_label(
            ui,
            pos2(rect.left() + rect.width() * at, rect.center().y),
            align,
            label,
        );
    }
}

/// What a lane reports this frame.
#[derive(Default)]
pub(in crate::dashboard) struct LaneResponse {
    /// The block clicked, by index.
    pub clicked: Option<usize>,
    /// A secondary (right) click on a block: its index and where the pointer was.
    pub context: Option<(usize, Pos2)>,
}

/// One painted lane: blocks on a muted track with tick gridlines. Blocks are focusable,
/// carry their description as tooltip and accessible name, and report clicks and right-clicks.
/// `outline` is one frame around a span (window positions 0..1), such as the selected session.
pub(in crate::dashboard) fn lane(
    ui: &mut Ui,
    height: f32,
    ticks: &[f32],
    blocks: &[LaneBlock],
    outline: Option<(f32, f32)>,
) -> LaneResponse {
    let colors = Tokens::get(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let rect = snap(ui, rect);
    ui.painter().rect_filled(rect, 6, colors.muted);
    for at in ticks {
        let x = snap(ui, rect.left() + rect.width() * at);
        ui.painter().vline(
            x,
            (rect.top() - 4.0)..=(rect.bottom() + 4.0),
            Stroke::new(1.0, colors.chart_grid),
        );
    }
    let mut clicked = None;
    let mut context = None;
    for (index, block) in blocks.iter().enumerate() {
        let left = rect.left() + rect.width() * block.start.clamp(0.0, 1.0);
        let right = (rect.left() + rect.width() * block.end.clamp(0.0, 1.0)).max(left + 1.0);
        let area = snap(
            ui,
            Rect::from_min_max(
                pos2(left, rect.top() + 2.0),
                pos2(right, rect.bottom() - 2.0),
            ),
        );
        let painter = ui.painter().with_clip_rect(area.expand(1.0));
        let label_color = match block.style {
            LaneStyle::Solid(color) => {
                let fill = if block.dimmed {
                    color.gamma_multiply(LANE_DIMMED)
                } else {
                    color
                };
                painter.rect_filled(area, 3, fill);
                Color32::WHITE
            }
            LaneStyle::Neutral => {
                painter.rect(
                    area,
                    3,
                    colors.card,
                    Stroke::new(1.0, colors.border),
                    eframe::egui::StrokeKind::Inside,
                );
                colors.fg_muted
            }
            LaneStyle::Gap => {
                super::hatch_gap(ui, area);
                colors.fg_muted
            }
        };
        if let Some(label) = block.label.as_ref().filter(|_| !block.dimmed) {
            let text = ui.painter().layout_no_wrap(
                label.clone(),
                Tokens::medium(Tokens::MICRO),
                label_color,
            );
            if text.size().x + 10.0 <= area.width() {
                painter.galley(
                    snap(
                        ui,
                        pos2(area.left() + 5.0, area.center().y - text.size().y / 2.0),
                    ),
                    text,
                    label_color,
                );
            }
        }
        let response = match &block.tooltip {
            Some(tip) => {
                let response = ui.interact(area, ui.next_auto_id(), Sense::click());
                ui.skip_ahead_auto_ids(1);
                response.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &block.description)
                });
                // Shown at once while hovered: the lane is scanned by moving across it.
                if response.hovered() {
                    eframe::egui::Tooltip::always_open(
                        ui.ctx().clone(),
                        ui.layer_id(),
                        response.id,
                        eframe::egui::PopupAnchor::Pointer,
                    )
                    .gap(14.0)
                    .show(|ui| lane_tip(ui, tip));
                }
                response
            }
            None => bar_response(ui, area, &block.description),
        };
        focus_ring(ui, &response, 3);
        if response.clicked() {
            clicked = Some(index);
        }
        if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            context = Some((index, pos));
        }
    }
    if let Some((start, end)) = outline {
        let left = rect.left() + rect.width() * start.clamp(0.0, 1.0);
        let right = rect.left() + rect.width() * end.clamp(0.0, 1.0);
        if right > left {
            let area = snap(
                ui,
                Rect::from_min_max(
                    pos2(left, rect.top() + 2.0),
                    pos2(right, rect.bottom() - 2.0),
                ),
            );
            ui.painter().rect_stroke(
                area.expand(2.0),
                4,
                Stroke::new(1.5, colors.fg),
                eframe::egui::StrokeKind::Outside,
            );
        }
    }
    LaneResponse { clicked, context }
}

/// Daily activity heat strip: weeks as columns, weekdays as rows, the most recent weeks
/// that fit. Returns nothing; the summary is its accessible name.
pub(in crate::dashboard) fn heat_strip(
    ui: &mut Ui,
    days: &[(chrono::NaiveDate, i64)],
    summary: &str,
) {
    use chrono::Datelike;
    const GAP: f32 = 2.0;
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    let cell = 10.0f32;
    let columns = ((width + GAP) / (cell + GAP)).floor().max(1.0) as usize;
    let Some(last) = days.last().map(|(date, _)| *date) else {
        return;
    };
    let max = days.iter().map(|(_, ms)| *ms).max().unwrap_or(0).max(1);
    let last_week_start =
        last - chrono::Days::new(u64::from(last.weekday().num_days_from_monday()));
    let earliest =
        days[0].0 - chrono::Days::new(u64::from(days[0].0.weekday().num_days_from_monday()));
    // Left-aligned from the first week when everything fits, else the most recent weeks.
    let first_shown = earliest.max(last_week_start - chrono::Days::new(7 * (columns as u64 - 1)));
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, 7.0 * cell + 6.0 * GAP + 16.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Image, true, summary));
    let response = response.on_hover_text(summary);
    let _ = response;
    let mut month = None;
    for (date, ms) in days.iter().filter(|(date, _)| *date >= first_shown) {
        let column = ((*date - first_shown).num_days() / 7) as f32;
        let row = date.weekday().num_days_from_monday() as f32;
        let cell_rect = Rect::from_min_size(
            rect.min + vec2(column * (cell + GAP), row * (cell + GAP)),
            vec2(cell, cell),
        );
        let fill = if *ms <= 0 {
            colors.muted
        } else {
            let level = (*ms as f32 / max as f32).clamp(0.15, 1.0);
            colors.fg_muted.gamma_multiply(0.25 + 0.75 * level)
        };
        ui.painter().rect_filled(snap(ui, cell_rect), 2, fill);
        if month != Some(date.month()) && date.day() <= 7 {
            month = Some(date.month());
            chart_label(
                ui,
                pos2(cell_rect.left(), rect.bottom() - 6.0),
                Align2::LEFT_CENTER,
                &date.format("%b").to_string(),
            );
        }
    }
}
