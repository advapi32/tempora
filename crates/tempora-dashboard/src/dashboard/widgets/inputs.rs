use eframe::egui::{
    self, Align2, Color32, FontId, Frame, Margin, Rect, Response, Sense, Stroke, StrokeKind,
    TextEdit, Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icons;

use super::{Tokens, focus_ring, paint_focus_ring, wrapped_text};

/// Category filter chip: swatch, label and an optional mono duration. Chips that are not
/// selected while another one is dim, matching the chart.
pub(in crate::dashboard) fn category_chip(
    ui: &mut Ui,
    label: &str,
    color: Color32,
    detail: Option<&str>,
    selected: bool,
    dimmed: bool,
) -> Response {
    let colors = Tokens::get(ui);
    let height = Tokens::SMALL_HEIGHT;
    let text = ui.painter().layout_no_wrap(
        label.into(),
        FontId::proportional(Tokens::SECONDARY),
        colors.fg,
    );
    let detail_text = detail.map(|detail| {
        ui.painter().layout_no_wrap(
            detail.into(),
            Tokens::mono(Tokens::SECONDARY),
            colors.fg_muted,
        )
    });
    let gap = Tokens::CONTROL_GAP;
    let width = 12.0
        + 10.0
        + gap
        + text.size().x
        + detail_text.as_ref().map_or(0.0, |text| gap + text.size().x)
        + 12.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width.ceil(), height), Sense::click());
    response.widget_info(|| {
        let label = match detail {
            Some(detail) => format!("{label}, {detail}"),
            None => label.to_owned(),
        };
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let opacity = if dimmed && !selected { 0.6 } else { 1.0 };
    let (fill, border) = if selected {
        (colors.muted, colors.fg_subtle)
    } else if response.hovered() {
        (colors.hover, colors.border)
    } else {
        (Color32::TRANSPARENT, colors.border)
    };
    let radius = (height / 2.0) as u8;
    let painter = ui.painter();
    painter.rect(
        rect,
        radius,
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let swatch =
        Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(10.0, 10.0));
    painter.rect_filled(swatch, 3, color.gamma_multiply(opacity));
    let x = swatch.right() + gap;
    let text_width = text.size().x;
    painter.galley(
        pos2(x, rect.center().y - text.size().y / 2.0),
        text,
        colors.fg.gamma_multiply(opacity),
    );
    if let Some(detail_text) = detail_text {
        painter.galley(
            pos2(
                x + text_width + gap,
                rect.center().y - detail_text.size().y / 2.0,
            ),
            detail_text,
            colors.fg_muted.gamma_multiply(opacity),
        );
    }
    focus_ring(ui, &response, radius);
    response
}

fn check_mark(ui: &Ui, rect: Rect, checked: bool, round: bool) {
    let colors = Tokens::get(ui);
    let radius = if round { 8 } else { 4 };
    if checked {
        ui.painter().rect_filled(rect, radius, colors.primary);
        if round {
            ui.painter()
                .circle_filled(rect.center(), 3.0, colors.primary_fg);
        } else {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                icons::CHECK,
                Tokens::icon(12.0),
                colors.primary_fg,
            );
        }
    } else {
        ui.painter().rect(
            rect,
            radius,
            colors.card,
            Stroke::new(1.0, colors.fg_subtle),
            StrokeKind::Inside,
        );
    }
}

/// Checkbox with a label and an optional muted second line, as wide as the parent.
pub(in crate::dashboard) fn checkbox(
    ui: &mut Ui,
    checked: &mut bool,
    label: &str,
    detail: Option<&str>,
) -> Response {
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    let text_width = (width - 16.0 - Tokens::CONTROL_GAP).max(0.0);
    let title = wrapped_text(
        ui,
        label,
        FontId::proportional(Tokens::BODY),
        colors.fg,
        text_width,
        2,
    );
    let sub = detail.map(|detail| {
        wrapped_text(
            ui,
            detail,
            FontId::proportional(Tokens::CAPTION),
            colors.fg_subtle,
            text_width,
            3,
        )
    });
    let height = (title.size().y + sub.as_ref().map_or(0.0, |sub| sub.size().y + 2.0)).max(20.0);
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *checked, label)
    });
    let opacity = if ui.is_enabled() { 1.0 } else { 0.4 };
    let box_rect = Rect::from_min_size(rect.min + vec2(0.0, 1.0), vec2(16.0, 16.0));
    check_mark(ui, box_rect, *checked, false);
    let x = box_rect.right() + Tokens::CONTROL_GAP;
    let title_height = title.size().y;
    ui.painter().galley(
        pos2(x, rect.top()),
        title,
        colors.fg.gamma_multiply(opacity),
    );
    if let Some(sub) = sub {
        ui.painter().galley(
            pos2(x, rect.top() + title_height + 2.0),
            sub,
            colors.fg_subtle.gamma_multiply(opacity),
        );
    }
    if response.has_focus() {
        paint_focus_ring(ui, box_rect, 4);
    }
    response
}

/// Radio card: a bordered option with a title and description.
pub(in crate::dashboard) fn radio_card(
    ui: &mut Ui,
    selected: bool,
    title: &str,
    detail: &str,
    width: f32,
) -> Response {
    let colors = Tokens::get(ui);
    let padding = 14.0;
    let text_width = (width - 2.0 * padding - 16.0 - 10.0).max(0.0);
    let title_text = wrapped_text(
        ui,
        title,
        Tokens::medium(Tokens::BODY),
        colors.fg,
        text_width,
        2,
    );
    let detail_text = wrapped_text(
        ui,
        detail,
        FontId::proportional(Tokens::CAPTION),
        colors.fg_muted,
        text_width,
        4,
    );
    let title_height = title_text.size().y;
    let height = 2.0 * padding + title_height + 3.0 + detail_text.size().y;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), selected, title)
    });
    let (fill, border) = if selected {
        // The filled dot and muted fill mark the choice; the border stays quiet.
        (colors.muted, colors.border)
    } else if response.hovered() {
        (colors.hover, colors.border)
    } else {
        (colors.card, colors.border)
    };
    let opacity = if ui.is_enabled() { 1.0 } else { 0.5 };
    ui.painter().rect(
        rect,
        Tokens::POPOVER_RADIUS,
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let dot = Rect::from_min_size(rect.min + vec2(padding, padding + 1.0), vec2(16.0, 16.0));
    check_mark(ui, dot, selected, true);
    let x = dot.right() + 10.0;
    let y = rect.top() + padding;
    ui.painter()
        .galley(pos2(x, y), title_text, colors.fg.gamma_multiply(opacity));
    ui.painter().galley(
        pos2(x, y + title_height + 3.0),
        detail_text,
        colors.fg_muted.gamma_multiply(opacity),
    );
    focus_ring(ui, &response, Tokens::POPOVER_RADIUS);
    response
}

/// Radio chip with a color swatch, used for category choices.
pub(in crate::dashboard) fn radio_chip(
    ui: &mut Ui,
    label: &str,
    color: Color32,
    selected: bool,
) -> Response {
    let response = category_chip(ui, label, color, None, selected, false);
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), selected, label)
    });
    response
}

/// Bordered single-line text input, 36 high.
pub(in crate::dashboard) fn text_field(
    ui: &mut Ui,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    let colors = Tokens::get(ui);
    let frame = Frame::new()
        .inner_margin(Margin::symmetric(12, 0))
        .stroke(Stroke::new(1.0, colors.border))
        .corner_radius(Tokens::CONTROL_RADIUS)
        .fill(colors.control)
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                vec2((width - 26.0).max(0.0), Tokens::CONTROL_HEIGHT - 2.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(
                        TextEdit::singleline(text)
                            .id(id)
                            .hint_text(hint)
                            .margin(Margin::ZERO)
                            .frame(egui::Frame::NONE)
                            .desired_width(ui.available_width()),
                    )
                },
            )
            .inner
        });
    let edit = frame.inner;
    if edit.has_focus() {
        paint_focus_ring(ui, frame.response.rect, Tokens::CONTROL_RADIUS);
    }
    edit
}

/// Checklist row: checkbox, icon tile, name and a mono sub-line. Toggles on click.
pub(in crate::dashboard) fn app_check_row(
    ui: &mut Ui,
    checked: &mut bool,
    key: &str,
    name: &str,
    sub: &str,
    category: tempora::model::Category,
) -> Response {
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, 48.0), Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *checked, name)
    });
    if response.hovered() {
        ui.painter().rect_filled(rect, 0, colors.hover);
    }
    let box_rect =
        Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(16.0, 16.0));
    check_mark(ui, box_rect, *checked, false);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(box_rect.right() + 12.0, rect.top()),
                rect.max - vec2(14.0, 0.0),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.style_mut().interaction.selectable_labels = false;
    super::app_cell(&mut child, Some(key), name, Some(sub), category, 28.0);
    ui.painter().hline(
        rect.x_range(),
        super::snap(ui, rect.bottom() - 0.5),
        Stroke::new(1.0, colors.border),
    );
    focus_ring(ui, &response, 0);
    response
}
