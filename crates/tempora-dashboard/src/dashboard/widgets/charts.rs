use eframe::egui::{
    Align2, Color32, CornerRadius, Id, Key, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType, pos2, vec2,
};
use tempora::model::Category;

use super::motion::{self, Ease};
use super::{Tokens, chart_block, chart_label, focus_ring, snap};
use crate::dashboard::format;

const HOUR: i64 = 3_600_000;
const MINUTE: i64 = 60_000;
const SEGMENT_GAP: f32 = 2.0;
const DIMMED: f32 = 0.2;
/// A hovered chart column is washed with half the hover color, so the gridlines behind it
/// still show.
const COLUMN_WASH: f32 = 0.5;
/// A dimmed lane block keeps this share of its color.
const LANE_DIMMED: f32 = 0.3;
/// A hovered lane block is lightened by this much white.
const LANE_HOVER: f32 = 0.14;
/// The lane wipes in from the left this long.
const LANE_REVEAL: f32 = 0.5;
/// A drag shorter than this across the lane is not a pick.
const LANE_PICK_MIN: f32 = 6.0;

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
    /// The day shown next to the total in each slice's tooltip. Without it the bar uses
    /// the plain `description` tooltip.
    pub slice_detail: Option<String>,
}

/// A slice's tooltip: its swatch, series and duration, with the day and bar total underneath.
fn slice_tip(ui: &mut Ui, segment: &Segment, day: &str, total: i64) {
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
            eframe::egui::RichText::new(format::duration(segment.ms))
                .font(Tokens::mono_medium(Tokens::SECONDARY))
                .color(colors.fg),
        );
    });
    ui.label(
        eframe::egui::RichText::new(format!("{day} · {} total", format::duration(total)))
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

/// Five y gridlines with compact labels, the baseline in the border color.
fn y_grid(ui: &Ui, plot: Rect, step: i64, label_zero: bool) {
    let colors = Tokens::get(ui);
    for index in 0..=5 {
        let y = plot.bottom() - plot.height() * index as f32 / 5.0;
        if index > 0 || label_zero {
            let label = if index == 0 {
                "0".to_owned()
            } else {
                axis_label(step * index)
            };
            chart_label(ui, pos2(plot.left() - 8.0, y), Align2::RIGHT_CENTER, &label);
        }
        let line = if index == 0 {
            colors.border
        } else {
            colors.chart_grid
        };
        ui.painter()
            .hline(plot.x_range(), snap(ui, y), Stroke::new(1.0, line));
    }
}

/// A run of bars picked by dragging across a chart.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Picked {
    first: usize,
    last: usize,
    /// The bar the drag started on.
    anchor: usize,
    /// Bars in the chart when it was picked; another count means other data.
    count: usize,
    /// The frame the chart last drew it in.
    frame: u64,
}

/// The slot under `x` when `area` holds `count` equal slots.
fn slot_at(area: Rect, count: usize, x: f32) -> usize {
    let slot = ((x - area.left()) / area.width() * count as f32).floor();
    (slot.max(0.0) as usize).min(count.saturating_sub(1))
}

/// Lets a drag across `area` pick a run of its `count` slots. The run stays picked after the
/// release until the next click or Escape, or until the chart shows other data. Returns the
/// picked run, first and last inclusive, and whether the drag is still going.
fn pick_bars(ui: &mut Ui, id: Id, area: Rect, count: usize) -> Option<(usize, usize, bool)> {
    let response = ui.interact(area, id.with("pick"), Sense::drag());
    let key = id.with("picked");
    let frame = ui.ctx().cumulative_frame_nr();
    let mut picked = ui
        .data(|data| data.get_temp::<Picked>(key))
        .filter(|picked| picked.count == count && picked.frame + 1 >= frame);
    let dragging = response.dragged();
    let escape = ui.input(|input| input.key_pressed(Key::Escape));
    if dragging || response.drag_stopped() {
        // The release frame has no press position, so the drag's anchor is kept; its end is
        // where the pointer is now, which on the release frame is where the button came up.
        let anchor = ui
            .input(|input| input.pointer.press_origin())
            .map(|origin| slot_at(area, count, origin.x))
            .or(picked.map(|picked| picked.anchor));
        let end = ui
            .input(|input| input.pointer.latest_pos())
            .map(|pointer| slot_at(area, count, pointer.x));
        if let (Some(anchor), Some(end)) = (anchor, end) {
            picked = Some(Picked {
                first: anchor.min(end),
                last: anchor.max(end),
                anchor,
                count,
                frame,
            });
        }
        if escape {
            picked = None;
        }
    } else if escape || ui.input(|input| input.pointer.any_click()) {
        picked = None;
    }
    ui.data_mut(|data| match &mut picked {
        Some(picked) => {
            picked.frame = frame;
            data.insert_temp(key, *picked);
        }
        None => data.remove::<Picked>(key),
    });
    picked
        .filter(|picked| picked.last > picked.first)
        .map(|picked| (picked.first, picked.last, dragging))
}

/// The columns of a picked run, washed in the focus color.
fn paint_pick(ui: &Ui, band: Rect) {
    let colors = Tokens::get(ui);
    ui.painter().rect(
        snap(ui, band),
        6,
        colors.focus.gamma_multiply(0.16),
        Stroke::new(1.0, colors.focus.gamma_multiply(0.8)),
        StrokeKind::Inside,
    );
}

/// A small label on a chart, centered above `anchor` and kept inside `bounds` sideways.
fn chart_pill(ui: &Ui, anchor: Pos2, bounds: Rect, text: &str) {
    let colors = Tokens::get(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        Tokens::mono_medium(Tokens::MICRO),
        colors.fg,
    );
    let size = galley.size() + vec2(14.0, 8.0);
    let rect = Rect::from_center_size(pos2(anchor.x, anchor.y - size.y / 2.0), size);
    let shift = (bounds.left() - rect.left()).max(0.0) + (bounds.right() - rect.right()).min(0.0);
    let rect = snap(ui, rect.translate(vec2(shift, 0.0)));
    ui.painter().rect(
        rect,
        6,
        colors.popover,
        Stroke::new(1.0, colors.border),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        snap(ui, rect.center() - galley.size() / 2.0),
        galley,
        colors.fg,
    );
}

/// "3 days" or "1 day".
fn days_label(days: usize) -> String {
    if days == 1 {
        "1 day".to_owned()
    } else {
        format!("{days} days")
    }
}

/// What a picked run of stacked bars adds up to, counting only the series not filtered out.
fn stacked_pick_summary(bars: &[StackedBar]) -> String {
    let recorded: Vec<i64> = bars
        .iter()
        .filter(|bar| !bar.future)
        .map(|bar| {
            bar.segments
                .iter()
                .filter(|segment| !segment.dimmed)
                .map(|segment| segment.ms)
                .sum()
        })
        .collect();
    if recorded.is_empty() {
        return "No data yet".to_owned();
    }
    let total: i64 = recorded.iter().sum();
    format!(
        "{} · {} · {} avg",
        days_label(recorded.len()),
        format::duration(total),
        format::duration(total / recorded.len() as i64)
    )
}

/// Stacked daily bars with y gridlines. Bars grow in when the chart appears and move to new
/// values when the data changes. Dragging across bars totals the days picked. Returns the
/// index of a clicked or activated bar.
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
    let ctx = ui.ctx().clone();
    let (outer, chart) = ui.allocate_exact_size(
        vec2(ui.available_width(), height + TOP + BOTTOM),
        Sense::hover(),
    );
    let id = chart.id;
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
    let axis = (step * 5) as f32;
    y_grid(ui, plot, step, true);
    if bars.is_empty() {
        return None;
    }
    let slot = plot.width() / bars.len() as f32;
    let bar_width = if week {
        46.0f32.min(slot * 0.8)
    } else {
        slot * 0.72
    };
    let columns = plot.with_min_y(plot.top() - TOP);
    // Registered before the columns, so a click still reaches the column on top of it.
    let picked = pick_bars(ui, id, columns, bars.len());
    if let Some((first, last, _)) = picked {
        paint_pick(
            ui,
            Rect::from_x_y_ranges(
                (plot.left() + first as f32 * slot + 1.0)
                    ..=(plot.left() + (last + 1) as f32 * slot - 1.0),
                columns.y_range(),
            ),
        );
    }
    let mut clicked = None;
    for (index, bar) in bars.iter().enumerate() {
        let bar_id = id.with(index);
        let x = plot.left() + (index as f32 + 0.5) * slot;
        let column = Rect::from_center_size(
            pos2(x, plot.center().y),
            vec2((slot - 2.0).max(1.0), plot.height() + TOP),
        )
        .translate(vec2(0.0, -TOP / 2.0));
        let total: i64 = bar.segments.iter().map(|segment| segment.ms).sum();
        // Each series' height as a share of the axis, eased toward the data.
        let ease = Ease::GROW.staggered(index);
        let shares: Vec<(usize, f32)> = bar
            .segments
            .iter()
            .enumerate()
            .map(|(position, segment)| {
                let target = if bar.future {
                    0.0
                } else {
                    segment.ms as f32 / axis
                };
                (
                    position,
                    motion::tween(&ctx, bar_id.with(segment.name), target, ease),
                )
            })
            .collect();
        let shown: f32 = shares.iter().map(|(_, share)| share).sum();
        let rect = Rect::from_min_max(
            pos2(x - bar_width / 2.0, plot.bottom() - shown * plot.height()),
            pos2(x + bar_width / 2.0, plot.bottom()),
        );
        let slices = segment_rects(rect, &shares);
        let response = (!bar.future).then(|| {
            let response = ui.interact(column, ui.next_auto_id(), Sense::click());
            ui.skip_ahead_auto_ids(1);
            response.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &bar.description)
            });
            response
        });
        let hovered = response
            .as_ref()
            .filter(|response| response.hovered() && picked.is_none_or(|(.., dragging)| !dragging));
        let slice = hovered
            .and(bar.slice_detail.as_ref())
            .and_then(|_| ui.ctx().pointer_hover_pos())
            .and_then(|pointer| slice_at(&slices, pointer));
        let wash = motion::fade(&ctx, bar_id.with("wash"), hovered.is_some(), motion::HOVER);
        if wash > 0.0 {
            ui.painter()
                .rect_filled(column, 6, colors.hover.gamma_multiply(COLUMN_WASH * wash));
        }
        // A day still to come keeps its slot so the week doesn't shift, but draws no bar: a stub
        // there would read as a little recorded time.
        if !bar.future && rect.height() > 0.0 {
            paint_segments(ui, bar_id, &bar.segments, &slices, slice);
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
                // Show the tooltip immediately so readers can move across the slices.
                (Some(detail), Some(slice)) => {
                    eframe::egui::Tooltip::always_open(
                        ui.ctx().clone(),
                        ui.layer_id(),
                        response.id,
                        eframe::egui::PopupAnchor::Pointer,
                    )
                    .gap(14.0)
                    .show(|ui| slice_tip(ui, &bar.segments[slice], detail, total));
                }
                (Some(_), None) => {}
                (None, _) if picked.is_none() => {
                    response = response.on_hover_text(&bar.description)
                }
                (None, _) => {}
            }
            focus_ring(ui, &response, 6);
            if response.clicked() {
                clicked = Some(index);
            }
        }
    }
    if let Some((first, last, _)) = picked {
        let center = plot.left() + (first + last + 1) as f32 * slot / 2.0;
        chart_pill(
            ui,
            pos2(center, columns.top() - 4.0),
            outer,
            &stacked_pick_summary(&bars[first..=last]),
        );
    }
    clicked
}

fn bar_response(ui: &mut Ui, rect: Rect, description: &str) -> Response {
    let response = ui.interact(rect, ui.next_auto_id(), Sense::click());
    ui.skip_ahead_auto_ids(1);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), description));
    response.on_hover_text(description)
}

/// Each visible series' index and rectangle, bottom up, with a small gap between them.
/// `shares` are the series' heights in any one unit; together they fill `rect`.
fn segment_rects(rect: Rect, shares: &[(usize, f32)]) -> Vec<(usize, Rect)> {
    let visible: Vec<_> = shares
        .iter()
        .copied()
        .filter(|(_, share)| *share > 0.0)
        .collect();
    let total: f32 = visible.iter().map(|(_, share)| share).sum();
    if total <= 0.0 || rect.height() <= 0.0 {
        return Vec::new();
    }
    let count = visible.len();
    let gap = SEGMENT_GAP.min(rect.height() / count.max(1) as f32 / 2.0);
    let usable = rect.height() - gap * count.saturating_sub(1) as f32;
    let mut bottom = rect.bottom();
    visible
        .into_iter()
        .map(|(index, share)| {
            let top = bottom - usable * share / total;
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
/// Filtered-out series fade the same way.
fn paint_segments(
    ui: &Ui,
    bar_id: Id,
    segments: &[Segment],
    slices: &[(usize, Rect)],
    hovered: Option<usize>,
) {
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
        let dim = motion::fade(
            ui.ctx(),
            bar_id.with((segment.name, "dim")),
            faded,
            motion::HOVER,
        );
        let color = segment.color.gamma_multiply(motion::lerp(1.0, DIMMED, dim));
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

/// What a picked run of paired bars adds up to.
fn paired_pick_summary(bars: &[PairedBar]) -> String {
    let front: i64 = bars.iter().map(|bar| bar.front_ms).sum();
    let back: i64 = bars.iter().map(|bar| bar.back_ms).sum();
    format!(
        "{} · {} focused · {} running",
        days_label(bars.len()),
        format::duration(front),
        format::duration(back)
    )
}

/// Daily trend: a tinted running bar behind a solid focused bar. Bars grow in and follow the
/// data like `stacked_bars`, and dragging across them totals the days picked. Returns the
/// clicked bar.
pub(in crate::dashboard) fn paired_bars(
    ui: &mut Ui,
    bars: &[PairedBar],
    color: Color32,
    height: f32,
) -> Option<usize> {
    const AXIS_WIDTH: f32 = 42.0;
    const BOTTOM: f32 = 20.0;
    /// Room above the plot for the summary of a picked run.
    const TOP: f32 = 22.0;
    let colors = Tokens::get(ui);
    let ctx = ui.ctx().clone();
    let (outer, chart) = ui.allocate_exact_size(
        vec2(ui.available_width(), height + TOP + BOTTOM),
        Sense::hover(),
    );
    let id = chart.id;
    let plot = Rect::from_min_max(
        outer.min + vec2(AXIS_WIDTH, TOP),
        outer.max - vec2(0.0, BOTTOM),
    );
    let max = bars
        .iter()
        .map(|bar| bar.back_ms.max(bar.front_ms))
        .max()
        .unwrap_or(0);
    let step = axis_step(max);
    let axis = (step * 5) as f32;
    y_grid(ui, plot, step, false);
    if bars.is_empty() {
        return None;
    }
    let slot = plot.width() / bars.len() as f32;
    let width = (slot * 0.7).clamp(1.0, 28.0);
    let label_every = (bars.len() / 12).max(1);
    let picked = pick_bars(ui, id, plot, bars.len());
    if let Some((first, last, _)) = picked {
        paint_pick(
            ui,
            Rect::from_x_y_ranges(
                (plot.left() + first as f32 * slot + 0.5)
                    ..=(plot.left() + (last + 1) as f32 * slot - 0.5),
                plot.y_range(),
            ),
        );
    }
    let mut clicked = None;
    for (index, bar) in bars.iter().enumerate() {
        let bar_id = id.with(index);
        let x = plot.left() + (index as f32 + 0.5) * slot;
        let column = Rect::from_center_size(
            pos2(x, plot.center().y),
            vec2((slot - 1.0).max(1.0), plot.height()),
        );
        let response = bar_response(ui, column, &bar.description);
        let hovered = response.hovered() && picked.is_none_or(|(.., dragging)| !dragging);
        let wash = motion::fade(&ctx, bar_id.with("wash"), hovered, motion::HOVER);
        if wash > 0.0 {
            ui.painter()
                .rect_filled(column, 4, colors.hover.gamma_multiply(COLUMN_WASH * wash));
        }
        let radius = CornerRadius {
            nw: 3,
            ne: 3,
            sw: 0,
            se: 0,
        };
        let ease = Ease::GROW.staggered(index);
        for (ms, fill, part) in [
            (bar.back_ms, color.gamma_multiply(0.28), "back"),
            (bar.front_ms, color, "front"),
        ] {
            let share = motion::tween(&ctx, bar_id.with(part), ms as f32 / axis, ease);
            if ms > 0 && share > 0.0 {
                let top = plot.bottom() - (share * plot.height()).max(1.0);
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
    if let Some((first, last, _)) = picked {
        let center = plot.left() + (first + last + 1) as f32 * slot / 2.0;
        chart_pill(
            ui,
            pos2(center, plot.top() - 2.0),
            outer,
            &paired_pick_summary(&bars[first..=last]),
        );
    }
    clicked
}

/// 24 hourly bars of focused time with 00/06/12/18/24 labels. The hour under the pointer is
/// drawn in full color with its tooltip shown at once; `summary` is the chart's accessible name.
pub(in crate::dashboard) fn hour_histogram(
    ui: &mut Ui,
    hours: &[i64; 24],
    color: Color32,
    summary: &str,
) {
    const HEIGHT: f32 = 64.0;
    const GAP: f32 = 2.0;
    let colors = Tokens::get(ui);
    let ctx = ui.ctx().clone();
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT + 16.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Image, true, summary));
    let id = response.id;
    let max = hours.iter().copied().max().unwrap_or(0).max(1);
    let slot = (rect.width() + GAP) / 24.0;
    let plot = Rect::from_min_size(rect.min, vec2(rect.width(), HEIGHT));
    let hovered_hour = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pointer| response.hovered() && plot.contains(*pointer))
        .map(|pointer| (((pointer.x - rect.left()) / slot).floor().max(0.0) as usize).min(23));
    for (hour, ms) in hours.iter().enumerate() {
        let level = *ms as f32 / max as f32;
        let shown = motion::tween(&ctx, id.with(hour), level, Ease::GROW.staggered(hour));
        let height = (HEIGHT * shown).max(2.0);
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
        let lit = motion::fade(
            &ctx,
            id.with(("lit", hour)),
            hovered_hour == Some(hour),
            motion::HOVER,
        );
        let fill = if *ms == 0 {
            colors.border
        } else {
            color.gamma_multiply(motion::lerp(0.35 + 0.65 * level, 1.0, lit))
        };
        ui.painter().rect_filled(snap(ui, bar), 2, fill);
    }
    if let Some(hour) = hovered_hour {
        let ms = hours[hour];
        eframe::egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            id,
            eframe::egui::PopupAnchor::Pointer,
        )
        .gap(14.0)
        .show(|ui| {
            ui.label(format!(
                "{hour:02}:00 – {:02}:00, {} min focused",
                hour + 1,
                ms / 60_000
            ));
        });
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
        // 100 px tall: a 1-minute sliver on top of 99 minutes, and an empty series skipped.
        let shares = [(0, 99.0), (1, 0.0), (2, 1.0)];
        let bar = Rect::from_min_max(pos2(0.0, 0.0), pos2(40.0, 100.0));
        let slices = segment_rects(bar, &shares);
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
        assert!(
            segment_rects(
                Rect::from_min_max(pos2(0.0, 100.0), pos2(40.0, 100.0)),
                &shares
            )
            .is_empty(),
            "a bar that hasn't grown yet has no slices"
        );
    }

    #[test]
    fn a_drag_picks_whole_slots_inside_the_chart() {
        let area = Rect::from_min_max(pos2(100.0, 0.0), pos2(800.0, 50.0));
        assert_eq!(slot_at(area, 7, 100.0), 0);
        assert_eq!(slot_at(area, 7, 299.0), 1);
        assert_eq!(slot_at(area, 7, 50.0), 0, "left of the chart");
        assert_eq!(slot_at(area, 7, 900.0), 6, "right of the chart");
    }

    #[test]
    fn a_picked_run_totals_recorded_days_and_skips_filtered_series() {
        let bar = |games: i64, browsers: i64, future: bool| StackedBar {
            label: String::new(),
            description: String::new(),
            total: None,
            segments: vec![
                Segment {
                    color: Color32::WHITE,
                    ms: games,
                    dimmed: false,
                    name: "Games",
                },
                Segment {
                    color: Color32::WHITE,
                    ms: browsers,
                    dimmed: true,
                    name: "Browsers",
                },
            ],
            today: false,
            future,
            slice_detail: None,
        };
        let bars = [
            bar(2 * HOUR, HOUR, false),
            bar(HOUR, HOUR, false),
            bar(0, 0, true),
        ];
        assert_eq!(stacked_pick_summary(&bars), "2 days · 3h · 1h 30m avg");
        assert_eq!(stacked_pick_summary(&bars[2..]), "No data yet");
        let paired = [
            PairedBar {
                label: String::new(),
                description: String::new(),
                front_ms: HOUR,
                back_ms: 2 * HOUR,
                today: false,
            },
            PairedBar {
                label: String::new(),
                description: String::new(),
                front_ms: 30 * MINUTE,
                back_ms: HOUR,
                today: false,
            },
        ];
        assert_eq!(
            paired_pick_summary(&paired),
            "2 days · 1h 30m focused · 3h running"
        );
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
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    let share = motion::tween(ui.ctx(), response.id, share.clamp(0.0, 1.0), Ease::GROW);
    let rect = snap(ui, rect);
    ui.painter().rect_filled(rect, 3, color.gamma_multiply(0.3));
    let front = Rect::from_min_size(rect.min, vec2(rect.width() * share, HEIGHT));
    if front.width() > 0.0 {
        ui.painter().rect_filled(snap(ui, front), 3, color);
    }
    chart_block(ui, rect, label, false);
}

/// Tick labels above a lane: (position 0..1, label). Ticks outside the lane are skipped and
/// labels at its edges stay inside it.
pub(in crate::dashboard) fn lane_axis(ui: &mut Ui, ticks: &[(f32, String)]) {
    let colors = Tokens::get(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    for (at, label) in ticks {
        if !(-0.001..=1.001).contains(at) {
            continue;
        }
        let galley = ui.painter().layout_no_wrap(
            label.clone(),
            eframe::egui::FontId::monospace(Tokens::MICRO),
            colors.fg_subtle,
        );
        let width = galley.size().x;
        let x = (rect.left() + rect.width() * at - width / 2.0)
            .clamp(rect.left(), (rect.right() - width).max(rect.left()));
        ui.painter().galley(
            snap(ui, pos2(x, rect.center().y - galley.size().y / 2.0)),
            galley,
            colors.fg_subtle,
        );
    }
}

/// Turns lane positions into the labels the scrubber and a pick show.
pub(in crate::dashboard) struct LaneScale<'a> {
    /// The time at a lane position, 0..1.
    pub time: &'a dyn Fn(f32) -> String,
    /// The span between two lane positions, in order.
    pub span: &'a dyn Fn(f32, f32) -> String,
}

/// What a lane reports this frame.
#[derive(Default)]
pub(in crate::dashboard) struct LaneResponse {
    /// The block clicked, by index.
    pub clicked: Option<usize>,
    /// A secondary (right) click on a block: its index and where the pointer was.
    pub context: Option<(usize, Pos2)>,
    /// A span dragged across and released, as lane positions in order.
    pub picked: Option<(f32, f32)>,
    /// Ctrl+wheel or a pinch over the lane: the pointer's lane position and the zoom factor,
    /// above 1 to zoom in.
    pub zoom: Option<(f32, f32)>,
    /// Shift+wheel or sideways scrolling over the lane: how far to move, as a share of the
    /// lane, positive toward later times.
    pub pan: Option<f32>,
    /// A double-click on the lane, which resets its zoom.
    pub reset: bool,
    /// A span is being dragged across the lane.
    pub dragging: bool,
}

/// One painted lane: blocks on a muted track with tick gridlines. Blocks are focusable,
/// carry their description as tooltip and accessible name, and report clicks and right-clicks.
/// `outline` is one frame around a span (window positions 0..1), such as the selected session.
/// The blocks wipe in from the left when the lane appears.
///
/// With a `scale`, the lane also follows the pointer with a time readout, picks a span
/// dragged across it, and reports wheel zooming and panning for the caller to apply.
pub(in crate::dashboard) fn lane(
    ui: &mut Ui,
    height: f32,
    ticks: &[f32],
    blocks: &[LaneBlock],
    outline: Option<(f32, f32)>,
    scale: Option<&LaneScale<'_>>,
) -> LaneResponse {
    let colors = Tokens::get(ui);
    let ctx = ui.ctx().clone();
    let (rect, track) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let id = track.id;
    let rect = snap(ui, rect);
    let at_x = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    ui.painter().rect_filled(rect, 6, colors.muted);
    for at in ticks.iter().filter(|at| (0.0..=1.0).contains(*at)) {
        let x = snap(ui, rect.left() + rect.width() * at);
        ui.painter().vline(
            x,
            (rect.top() - 4.0)..=(rect.bottom() + 4.0),
            Stroke::new(1.0, colors.chart_grid),
        );
    }
    let reveal = motion::tween(
        &ctx,
        id.with("reveal"),
        1.0,
        Ease {
            enter: LANE_REVEAL,
            ..Ease::GROW
        },
    );
    let revealed = rect
        .with_max_x(rect.left() + rect.width() * reveal)
        .expand2(vec2(1.0, 2.0));
    // Registered before the blocks, so their clicks stay on top of it.
    let scrub = scale.map(|_| ui.interact(rect, id.with("scrub"), Sense::drag()));
    let dragging = scrub.as_ref().is_some_and(Response::dragged);
    let mut response = LaneResponse::default();
    for (index, block) in blocks.iter().enumerate() {
        if block.end <= 0.0 || block.start >= 1.0 {
            continue;
        }
        let left = rect.left() + rect.width() * block.start.clamp(0.0, 1.0);
        let right = (rect.left() + rect.width() * block.end.clamp(0.0, 1.0)).max(left + 1.0);
        let area = snap(
            ui,
            Rect::from_min_max(
                pos2(left, rect.top() + 2.0),
                pos2(right, rect.bottom() - 2.0),
            ),
        );
        let block_response = match &block.tooltip {
            Some(tip) => {
                let block_response = ui.interact(area, ui.next_auto_id(), Sense::click());
                ui.skip_ahead_auto_ids(1);
                block_response.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &block.description)
                });
                // Shown at once while hovered: the lane is scanned by moving across it.
                if block_response.hovered() && !dragging {
                    eframe::egui::Tooltip::always_open(
                        ui.ctx().clone(),
                        ui.layer_id(),
                        block_response.id,
                        eframe::egui::PopupAnchor::Pointer,
                    )
                    .gap(14.0)
                    .show(|ui| lane_tip(ui, tip));
                }
                block_response
            }
            None => bar_response(ui, area, &block.description),
        };
        let painter = ui
            .painter()
            .with_clip_rect(area.expand(1.0).intersect(revealed));
        let dim = motion::fade(&ctx, id.with(("dim", index)), block.dimmed, motion::HOVER);
        let lit = motion::fade(
            &ctx,
            id.with(("lit", index)),
            block_response.hovered() && !dragging,
            motion::HOVER,
        );
        let label_color = match block.style {
            LaneStyle::Solid(color) => {
                painter.rect_filled(
                    area,
                    3,
                    color.gamma_multiply(motion::lerp(1.0, LANE_DIMMED, dim)),
                );
                if lit > 0.0 {
                    painter.rect_filled(area, 3, Color32::WHITE.gamma_multiply(LANE_HOVER * lit));
                }
                Color32::WHITE
            }
            LaneStyle::Neutral => {
                painter.rect(
                    area,
                    3,
                    colors.card,
                    Stroke::new(1.0, colors.border),
                    StrokeKind::Inside,
                );
                colors.fg_muted
            }
            LaneStyle::Gap => {
                super::hatch_gap(ui, area.intersect(revealed));
                colors.fg_muted
            }
        };
        if let Some(label) = block.label.as_ref().filter(|_| dim < 0.5) {
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
        focus_ring(ui, &block_response, 3);
        if block_response.clicked() {
            response.clicked = Some(index);
        }
        if block_response.secondary_clicked()
            && let Some(pos) = block_response.interact_pointer_pos()
        {
            response.context = Some((index, pos));
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
    let (Some(scale), Some(scrub)) = (scale, scrub) else {
        return response;
    };
    // Where the drag started, kept so the release frame (which has no press position) can
    // finish it, and whether Escape cancelled it. egui ends a drag itself when Escape is
    // pressed, so the frame that sees Escape may be the one the drag stops in.
    let pick_origin = id.with("pick-origin");
    let pick_cancelled = id.with("pick-cancelled");
    let escape = ui.input(|input| input.key_pressed(Key::Escape));
    if scrub.drag_started() {
        ui.data_mut(|data| data.remove_temp::<bool>(pick_cancelled));
    }
    if dragging && let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
        ui.data_mut(|data| data.insert_temp(pick_origin, origin.x));
    }
    if dragging && escape {
        ui.data_mut(|data| data.insert_temp(pick_cancelled, true));
    }
    // The Escape that cancels a drag is used up here, so the page doesn't also read it as
    // "zoom back out".
    if escape && (dragging || scrub.drag_stopped()) {
        ui.input_mut(|input| input.consume_key(eframe::egui::Modifiers::NONE, Key::Escape));
    }
    let cancelled =
        escape || ui.data(|data| data.get_temp::<bool>(pick_cancelled).unwrap_or(false));
    // The span from the drag's start to the pointer, when it is wide enough to pick.
    let span = (dragging || scrub.drag_stopped())
        .then(|| {
            let origin = ui.data(|data| data.get_temp::<f32>(pick_origin))?;
            let pointer = ui.input(|input| input.pointer.latest_pos())?.x;
            let (a, b) = (at_x(origin.min(pointer)), at_x(origin.max(pointer)));
            ((b - a) * rect.width() >= LANE_PICK_MIN && !cancelled).then_some((a, b))
        })
        .flatten();
    if dragging {
        if let Some((a, b)) = span {
            let band = Rect::from_x_y_ranges(
                (rect.left() + rect.width() * a)..=(rect.left() + rect.width() * b),
                rect.y_range(),
            );
            paint_pick(ui, band);
            chart_pill(
                ui,
                pos2(band.center().x, rect.top() - 4.0),
                rect,
                &format!("{} · release to zoom", (scale.span)(a, b)),
            );
        }
    } else if let Some(pointer) = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|_| ui.rect_contains_pointer(rect))
    {
        // The scrubber: a hairline and the time under the pointer.
        let x = snap(ui, pointer.x);
        ui.painter().vline(
            x,
            (rect.top() - 4.0)..=(rect.bottom() + 4.0),
            Stroke::new(1.0, colors.fg_muted),
        );
        chart_pill(
            ui,
            pos2(x, rect.top() - 4.0),
            rect,
            &(scale.time)(at_x(pointer.x)),
        );
        let zoom = ui.input(|input| input.zoom_delta());
        if (zoom - 1.0).abs() > 1e-4 {
            response.zoom = Some((at_x(pointer.x), zoom));
        }
        let sideways = ui.input(|input| input.smooth_scroll_delta.x);
        if sideways != 0.0 {
            ui.input_mut(|input| input.smooth_scroll_delta.x = 0.0);
            response.pan = Some(-sideways / rect.width());
        }
    }
    response.dragging = dragging;
    response.reset = ui.rect_contains_pointer(rect)
        && ui.input(|input| {
            input
                .pointer
                .button_double_clicked(eframe::egui::PointerButton::Primary)
        });
    if scrub.drag_stopped() {
        response.picked = span;
        ui.data_mut(|data| {
            data.remove_temp::<f32>(pick_origin);
            data.remove_temp::<bool>(pick_cancelled);
        });
    }
    response
}

/// Daily activity heat strip: weeks as columns, weekdays as rows, the most recent weeks
/// that fit. The weeks fade in from the left; each day shows its date and time on hover.
/// The summary is its accessible name.
pub(in crate::dashboard) fn heat_strip(
    ui: &mut Ui,
    days: &[(chrono::NaiveDate, i64)],
    summary: &str,
) {
    use chrono::Datelike;
    const GAP: f32 = 2.0;
    let colors = Tokens::get(ui);
    let ctx = ui.ctx().clone();
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
    let id = response.id;
    let pointer = ui.ctx().pointer_hover_pos().filter(|_| response.hovered());
    let mut hovered = None;
    let mut month = None;
    for (date, ms) in days.iter().filter(|(date, _)| *date >= first_shown) {
        let column = ((*date - first_shown).num_days() / 7) as usize;
        let row = date.weekday().num_days_from_monday() as f32;
        let cell_rect = Rect::from_min_size(
            rect.min + vec2(column as f32 * (cell + GAP), row * (cell + GAP)),
            vec2(cell, cell),
        );
        let shown = motion::tween(
            &ctx,
            id.with(("week", column)),
            1.0,
            Ease::GROW.staggered(column),
        );
        let fill = if *ms <= 0 {
            colors.muted
        } else {
            let level = (*ms as f32 / max as f32).clamp(0.15, 1.0);
            colors.fg_muted.gamma_multiply(0.25 + 0.75 * level)
        };
        ui.painter()
            .rect_filled(snap(ui, cell_rect), 2, fill.gamma_multiply(shown));
        if pointer.is_some_and(|pointer| cell_rect.expand(GAP / 2.0).contains(pointer)) {
            hovered = Some((cell_rect, *date, *ms));
        }
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
    match hovered {
        Some((cell_rect, date, ms)) => {
            ui.painter().rect_stroke(
                snap(ui, cell_rect.expand(1.0)),
                3,
                Stroke::new(1.0, colors.fg),
                StrokeKind::Outside,
            );
            eframe::egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                id,
                eframe::egui::PopupAnchor::Pointer,
            )
            .gap(14.0)
            .show(|ui| {
                let activity = if ms > 0 {
                    format!("{} focused", format::duration(ms))
                } else {
                    "No activity".to_owned()
                };
                ui.label(format!("{} · {activity}", date.format("%a, %b %-d")));
            });
        }
        None => {
            response.on_hover_text(summary);
        }
    }
}
