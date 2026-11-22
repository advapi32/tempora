//! Pieces shared by the charts: the chart block, axis labels and the gap hatch.

use eframe::egui::{
    self, Align2, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType,
    pos2,
};

use super::{Tokens, focus_ring, snap};

pub(in crate::dashboard) fn chart_block(
    ui: &mut Ui,
    rect: Rect,
    label: &str,
    clickable: bool,
) -> Response {
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::focusable_noninteractive()
    };
    let response = ui.interact(rect, ui.next_auto_id(), sense);
    ui.skip_ahead_auto_ids(1);
    response.widget_info(|| {
        WidgetInfo::labeled(
            if clickable {
                WidgetType::Button
            } else {
                WidgetType::Label
            },
            ui.is_enabled(),
            label,
        )
    });
    focus_ring(ui, &response, 4);
    response.on_hover_text(label)
}

pub(in crate::dashboard) fn chart_label(ui: &Ui, at: egui::Pos2, align: Align2, label: &str) {
    ui.painter().text(
        snap(ui, at),
        align,
        label,
        FontId::monospace(Tokens::MICRO),
        Tokens::get(ui).fg_subtle,
    );
}

pub(in crate::dashboard) fn hatch_gap(ui: &Ui, rect: Rect) {
    let colors = Tokens::get(ui);
    let rect = snap(ui, rect);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4, colors.muted);
    painter.rect_stroke(rect, 4, Stroke::new(1.0, colors.border), StrokeKind::Inside);
    let height = rect.height();
    let mut x = rect.left() - height;
    while x < rect.right() {
        painter.line_segment(
            [pos2(x, rect.bottom()), pos2(x + height, rect.top())],
            Stroke::new(1.0, colors.gap_hatch),
        );
        x += 5.0;
    }
}
