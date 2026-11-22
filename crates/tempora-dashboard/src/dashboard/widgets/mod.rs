mod cards;
mod chart_parts;
mod charts;
mod controls;
mod date;
mod feedback;
mod inputs;
mod layout;
mod marks;
mod overlays;
mod popup;
mod table;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, WidgetType, vec2,
};

use super::theme::{Tokens, snap};

pub(in crate::dashboard) use cards::*;
pub(in crate::dashboard) use chart_parts::*;
pub(in crate::dashboard) use charts::*;
pub(in crate::dashboard) use controls::*;
pub(in crate::dashboard) use date::*;
pub(in crate::dashboard) use feedback::*;
pub(in crate::dashboard) use inputs::*;
pub(in crate::dashboard) use layout::*;
pub(in crate::dashboard) use marks::*;
pub(in crate::dashboard) use overlays::*;
pub(in crate::dashboard) use popup::*;
pub(in crate::dashboard) use table::*;

pub(super) fn focus_ring(ui: &Ui, response: &Response, radius: u8) {
    if response.has_focus() {
        paint_focus_ring(ui, response.rect, radius);
    }
}

pub(super) fn paint_focus_ring(ui: &Ui, rect: Rect, radius: u8) {
    ui.painter().rect_stroke(
        snap(ui, rect.expand(2.0)),
        CornerRadius::same(radius.saturating_add(2)),
        Stroke::new(2.0, Tokens::get(ui).focus),
        StrokeKind::Outside,
    );
}

fn glyph(ui: &mut Ui, icon: &str, size: f32, color: Color32) {
    glyph_with_font(ui, icon, Tokens::icon(size), color);
}

fn glyph_with_font(ui: &mut Ui, icon: &str, font: FontId, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(font.size, font.size), Sense::hover());
    ui.painter().text(
        snap(ui, rect.center()),
        Align2::CENTER_CENTER,
        icon,
        font,
        color,
    );
}

pub(super) fn scroll<R>(
    ui: &mut Ui,
    area: egui::ScrollArea,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let content_style = ui.style().clone();
    ui.scope(|ui| {
        let colors = Tokens::get(ui);
        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = colors.muted;
        visuals.widgets.inactive.bg_fill = colors.scroll_thumb;
        visuals.widgets.hovered.bg_fill = colors.scroll_thumb_hover;
        visuals.widgets.active.bg_fill = colors.scroll_thumb_hover;
        area.show(ui, |ui| {
            ui.set_style(content_style);
            content(ui)
        })
    })
    .inner
}

pub(super) fn table_cell(
    ui: &mut Ui,
    width: f32,
    height: f32,
    align: Align,
    content: impl FnOnce(&mut Ui),
) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let layout = if align == Align::Max {
        Layout::right_to_left(Align::Center)
    } else {
        Layout::left_to_right(Align::Center)
    };
    let mut cell = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    cell.set_clip_rect(rect.intersect(ui.clip_rect()));
    content(&mut cell);
}

pub(super) fn right_aligned_cell(ui: &mut Ui, width: f32, height: f32, label: RichText) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let text = egui::WidgetText::from(label);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), text.text()));
    let galley = text.into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width,
        egui::TextStyle::Body,
    );
    let origin = rect.right_center() - vec2(galley.size().x, galley.size().y / 2.0);
    ui.painter()
        .with_clip_rect(rect)
        .galley(origin, galley, Tokens::get(ui).fg);
}

pub(super) fn wrapped_text(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
    rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.into(), font, color, width);
    job.wrap.max_rows = rows;
    ui.painter().layout_job(job)
}

/// Single-line label that ends in an ellipsis when it doesn't fit and shows the full text
/// on hover.
pub(in crate::dashboard) fn truncated(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    ui.add(egui::Label::new(text).truncate().selectable(false))
}

/// Text in the shared type scale and a token color.
pub(in crate::dashboard) fn text(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text).size(size).color(color)
}

/// Mono text for durations, times and counts in tables, lists and axes.
pub(in crate::dashboard) fn mono(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(Tokens::mono(size))
}
