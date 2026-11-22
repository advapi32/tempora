//! Small inline marks: swatches, share bars, tags, app icon tiles and cells, category labels and
//! the recorder status pill.

use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, Color32, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType, pos2, vec2,
};
use tempora::model::Category;

use super::hatch_gap;
use super::{Tokens, glyph, snap};

/// Thin progress bar: muted track, colored share.
pub(in crate::dashboard) fn share_bar(ui: &mut Ui, fraction: f32, color: Color32, width: f32) {
    let colors = Tokens::get(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    let fraction = super::motion::tween(
        ui.ctx(),
        response.id,
        fraction.clamp(0.0, 1.0),
        super::motion::Ease::GROW,
    );
    let rect = snap(ui, rect);
    ui.painter().rect_filled(rect, 3, colors.muted);
    let filled = rect.width() * fraction;
    if filled > 0.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(filled.max(2.0), rect.height())),
            3,
            color,
        );
    }
}

/// Small rounded color square for categories and legends.
pub(in crate::dashboard) fn swatch(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter()
        .rect_filled(snap(ui, rect), (size / 4.0) as u8, color);
}

/// 6 px green dot marking an app that is running now.
#[cfg(debug_assertions)]
pub(in crate::dashboard) fn live_dot(ui: &mut Ui) {
    let (rect, response) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 3.0, Tokens::get(ui).ok);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Running now"));
    response.on_hover_text("Running now");
}

/// Legend entry: a swatch (or hatched box) and a caption label.
pub(in crate::dashboard) fn legend_item(
    ui: &mut Ui,
    color: Option<Color32>,
    label: &str,
) -> Response {
    let colors = Tokens::get(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 10.0), Sense::hover());
        match color {
            Some(color) => {
                ui.painter().rect_filled(snap(ui, rect), 3, color);
            }
            None => hatch_gap(ui, rect),
        }
        ui.label(
            RichText::new(label)
                .size(Tokens::CAPTION)
                .color(colors.fg_muted),
        );
    })
    .response
}

#[cfg_attr(not(debug_assertions), allow(dead_code))] // Shown in the debug-only gallery.
pub(in crate::dashboard) fn tag(ui: &mut Ui, label: &str) -> Response {
    badge(ui, label, BadgeKind::Tag)
}

pub(super) enum BadgeKind {
    #[cfg_attr(not(debug_assertions), allow(dead_code))] // Shown in the debug-only gallery.
    Tag,
    Delta,
}

pub(super) fn badge(ui: &mut Ui, label: &str, kind: BadgeKind) -> Response {
    let colors = Tokens::get(ui);
    let (font, color, height, padding, border) = match kind {
        BadgeKind::Tag => (
            Tokens::medium(Tokens::MICRO),
            colors.fg_muted,
            20.0,
            7.0,
            Stroke::new(1.0, colors.border),
        ),
        BadgeKind::Delta => (
            Tokens::mono_medium(Tokens::MICRO),
            colors.fg,
            18.0,
            6.0,
            Stroke::NONE,
        ),
    };
    let text = ui.painter().layout_no_wrap(label.into(), font, color);
    let (rect, response) =
        ui.allocate_exact_size(vec2(text.size().x + 2.0 * padding, height), Sense::hover());
    ui.painter().rect(
        snap(ui, rect),
        Tokens::TAG_RADIUS,
        colors.muted,
        border,
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(snap(ui, rect.center() - text.size() / 2.0), text, color);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), label));
    response
}

/// App tile: the app's icon when one was extracted, otherwise its initial on the
/// category color at 14%. `key` is `None` for synthetic rows such as groups.
pub(in crate::dashboard) fn icon_tile(
    ui: &mut Ui,
    key: Option<&str>,
    name: &str,
    category: Category,
    size: f32,
) {
    let texture = key.and_then(|key| crate::dashboard::icons::lookup(ui.ctx(), key));
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if let Some(texture) = texture {
        let inset = snap(ui, rect.shrink((size * 0.06).round()));
        ui.painter().image(
            texture.id(),
            inset,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        return;
    }
    let color = Tokens::get(ui).category(category);
    let letter = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    ui.painter()
        .rect_filled(rect, Tokens::CONTROL_RADIUS, color.gamma_multiply(0.14));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter,
        Tokens::semibold(size * 0.38),
        color,
    );
}

/// Icon tile, name and an optional mono sub-line, both truncating with a tooltip.
pub(in crate::dashboard) fn app_cell(
    ui: &mut Ui,
    key: Option<&str>,
    name: &str,
    sub: Option<&str>,
    category: Category,
    tile: f32,
) {
    app_cell_with(ui, key, name, sub, category, tile, |_| {});
}

/// `app_cell` with extra markers (live dot, tags) after the name. The text block is
/// measured first and given a fixed height so the row centers it on the tile. Returns the
/// name's response, so a caller can hang a tooltip on it.
pub(in crate::dashboard) fn app_cell_with(
    ui: &mut Ui,
    key: Option<&str>,
    name: &str,
    sub: Option<&str>,
    category: Category,
    tile: f32,
    after_name: impl FnOnce(&mut Ui),
) -> egui::Response {
    let colors = Tokens::get(ui);
    let name_font = Tokens::medium(Tokens::BODY);
    let sub_font = Tokens::mono(Tokens::MICRO);
    let line = |text: &str, font: egui::FontId| {
        ui.painter()
            .layout_no_wrap(text.into(), font, colors.fg)
            .size()
            .y
    };
    let name_height = line(name, name_font.clone());
    let gap = 1.0;
    let height = name_height + sub.map_or(0.0, |sub| gap + line(sub, sub_font.clone()));
    ui.spacing_mut().item_spacing.x = 12.0;
    icon_tile(ui, key, name, category, tile);
    let width = ui.available_width();
    ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = gap;
        let name_response = ui
            .allocate_ui_with_layout(
                vec2(width, name_height),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let response = ui.add(
                        egui::Label::new(RichText::new(name).font(name_font))
                            .truncate()
                            .selectable(false),
                    );
                    after_name(ui);
                    response
                },
            )
            .inner;
        if let Some(sub) = sub {
            ui.add(
                egui::Label::new(RichText::new(sub).font(sub_font).color(colors.fg_subtle))
                    .truncate()
                    .selectable(false),
            );
        }
        name_response
    })
    .inner
}

/// Category swatch and name.
pub(in crate::dashboard) fn category_label(ui: &mut Ui, category: Category) {
    let colors = Tokens::get(ui);
    ui.spacing_mut().item_spacing.x = 6.0;
    swatch(ui, colors.category(category), 8.0);
    ui.add(
        egui::Label::new(
            RichText::new(category.label())
                .size(Tokens::SECONDARY)
                .color(colors.fg_muted),
        )
        .truncate()
        .selectable(false),
    );
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::dashboard) enum StatusKind {
    Recording,
    Paused,
    Unreachable,
    Attention,
    Setup,
    Connecting,
}

impl StatusKind {
    fn color(self, colors: &Tokens) -> Color32 {
        match self {
            Self::Recording => colors.ok,
            Self::Paused => colors.warn,
            Self::Unreachable | Self::Attention => colors.bad,
            Self::Setup | Self::Connecting => colors.fg_subtle,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Recording => "Recording",
            Self::Paused => "Paused",
            Self::Unreachable => "Unreachable",
            Self::Attention => "Needs attention",
            Self::Setup => "Setup required",
            Self::Connecting => "Connecting",
        }
    }
}

pub(in crate::dashboard) fn status_pill(
    ui: &mut Ui,
    kind: StatusKind,
    detail: Option<&str>,
) -> Response {
    let colors = Tokens::get(ui);
    let color = kind.color(colors);
    // Recording is the normal state: a quiet dot, details on hover. Every other state needs
    // attention and gets the labeled pill.
    let quiet = kind == StatusKind::Recording;
    let label = ui.painter().layout_no_wrap(
        kind.label().into(),
        Tokens::medium(Tokens::SECONDARY),
        colors.fg,
    );
    let detail_text = detail.map(|detail| {
        ui.painter()
            .layout_no_wrap(detail.into(), Tokens::mono(Tokens::MICRO), colors.fg_subtle)
    });
    let detail_width = detail_text
        .as_ref()
        .map_or(0.0, |text| text.size().x + Tokens::CONTROL_GAP);
    let height = Tokens::SMALL_HEIGHT;
    let padding = 11.0;
    let dot = 8.0;
    // The quiet dot sits right after whatever it follows.
    let width = if quiet {
        dot + 4.0
    } else {
        (padding * 2.0 + dot + Tokens::CONTROL_GAP + label.size().x + detail_width).ceil()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let radius = (height / 2.0) as u8;
    if !quiet {
        ui.painter().rect(
            rect,
            radius,
            colors.card,
            Stroke::new(1.0, colors.border),
            StrokeKind::Inside,
        );
    }
    let center = if quiet {
        rect.center()
    } else {
        pos2(rect.left() + padding + dot / 2.0, rect.center().y)
    };
    let is_pulsing = kind == StatusKind::Recording && ui.input(|i| i.focused);
    let animation_id = response.id.with("recording-pulse");
    let now = ui.input(|i| i.time);
    let started = ui.data_mut(|data| {
        if is_pulsing {
            let started = data.get_temp::<f64>(animation_id).unwrap_or(now);
            data.insert_temp(animation_id, started);
            Some(started)
        } else {
            data.remove::<f64>(animation_id);
            None
        }
    });
    if let Some(elapsed) = started
        .map(|start| now - start)
        .filter(|elapsed| *elapsed < 0.65)
    {
        let t = (elapsed / 0.65) as f32;
        ui.painter().circle_stroke(
            center,
            dot / 2.0 + 6.0 * t,
            Stroke::new(2.0, color.gamma_multiply(0.55 * (1.0 - t))),
        );
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
    ui.painter().circle_filled(center, dot / 2.0, color);
    let summary = match detail {
        Some(detail) => format!("{} {detail}", kind.label()),
        None => kind.label().to_owned(),
    };
    if quiet {
        response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Label,
                ui.is_enabled(),
                format!("Recorder: {summary}"),
            )
        });
        return response.on_hover_text(summary);
    }
    let x = center.x + dot / 2.0 + Tokens::CONTROL_GAP;
    let next_x = x + label.size().x + Tokens::CONTROL_GAP;
    ui.painter().galley(
        pos2(x, rect.center().y - label.size().y / 2.0),
        label,
        colors.fg,
    );
    if let Some(text) = detail_text {
        ui.painter().galley(
            pos2(next_x, rect.center().y - text.size().y / 2.0),
            text,
            colors.fg_subtle,
        );
    }
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Label,
            ui.is_enabled(),
            format!("Recorder: {summary}"),
        )
    });
    response
}

/// Icon followed by wrapped secondary text.
pub(in crate::dashboard) fn icon_line(ui: &mut Ui, icon: &'static str, text: &str) {
    let colors = Tokens::get(ui);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        glyph(ui, icon, Tokens::ICON_SIZE, colors.fg_muted);
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .size(Tokens::SECONDARY)
                    .color(colors.fg_muted),
            )
            .wrap()
            .selectable(false),
        );
    });
}

/// Bordered tag with a category swatch.
pub(in crate::dashboard) fn category_tag(ui: &mut Ui, category: Category) {
    let colors = Tokens::get(ui);
    let label = category.label();
    let text =
        ui.painter()
            .layout_no_wrap(label.into(), Tokens::medium(Tokens::MICRO), colors.fg_muted);
    let (rect, response) = ui.allocate_exact_size(
        vec2(text.size().x + 7.0 + 8.0 + 6.0 + 7.0, 20.0),
        Sense::hover(),
    );
    ui.painter().rect(
        snap(ui, rect),
        Tokens::TAG_RADIUS,
        Color32::TRANSPARENT,
        Stroke::new(1.0, colors.border),
        StrokeKind::Inside,
    );
    let swatch = Rect::from_min_size(
        pos2(rect.left() + 7.0, rect.center().y - 4.0),
        vec2(8.0, 8.0),
    );
    ui.painter()
        .rect_filled(snap(ui, swatch), 2, colors.category(category));
    ui.painter().galley(
        snap(
            ui,
            pos2(swatch.right() + 6.0, rect.center().y - text.size().y / 2.0),
        ),
        text,
        colors.fg_muted,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
}

/// Bordered tag in a tone color, for session notes such as "Ended by pause".
pub(in crate::dashboard) fn tone_tag(ui: &mut Ui, label: &str, color: Color32) {
    let colors = Tokens::get(ui);
    let text = ui
        .painter()
        .layout_no_wrap(label.into(), Tokens::medium(Tokens::MICRO), color);
    let (rect, response) = ui.allocate_exact_size(vec2(text.size().x + 14.0, 20.0), Sense::hover());
    let border = if color == colors.fg_muted {
        colors.border
    } else {
        color.gamma_multiply(0.45)
    };
    ui.painter().rect(
        snap(ui, rect),
        Tokens::TAG_RADIUS,
        Color32::TRANSPARENT,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(snap(ui, rect.center() - text.size() / 2.0), text, color);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
}
