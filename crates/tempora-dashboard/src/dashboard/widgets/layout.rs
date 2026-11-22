//! Page structure: section headers, breadcrumbs, setting rows and setup steps.

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Ui, WidgetInfo,
    WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icons;

use super::{Tokens, focus_ring, glyph, snap};

/// Card heading: section title with an optional muted subtitle, and controls on the right.
pub(in crate::dashboard) fn section_header<R>(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    right: impl FnOnce(&mut Ui) -> R,
) -> R {
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::left_to_right(Align::Min), |ui| {
        let right_ui = |ui: &mut Ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), right)
                .inner
        };
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.set_max_width((width * 0.6).max(200.0));
            ui.add(
                egui::Label::new(RichText::new(title).font(Tokens::semibold(Tokens::SECTION)))
                    .truncate()
                    .selectable(false),
            );
            if let Some(subtitle) = subtitle {
                ui.add(
                    egui::Label::new(
                        RichText::new(subtitle)
                            .size(Tokens::SECONDARY)
                            .color(colors.fg_subtle),
                    )
                    .truncate()
                    .selectable(false),
                );
            }
        });
        right_ui(ui)
    })
    .inner
}

/// "Overview · Nov 16 – 22 › Thursday": every part but the last is a link.
pub(in crate::dashboard) fn breadcrumb(ui: &mut Ui, parts: &[&str]) -> Option<usize> {
    let colors = Tokens::get(ui);
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (index, part) in parts.iter().enumerate() {
            let last = index + 1 == parts.len();
            if last {
                ui.add(
                    egui::Label::new(
                        RichText::new(*part)
                            .size(Tokens::SECONDARY)
                            .color(colors.fg),
                    )
                    .truncate()
                    .selectable(false),
                );
            } else {
                let link = ui.add(
                    egui::Label::new(
                        RichText::new(*part)
                            .size(Tokens::SECONDARY)
                            .color(colors.fg_muted),
                    )
                    .sense(Sense::click())
                    .selectable(false),
                );
                let link = link.on_hover_cursor(egui::CursorIcon::PointingHand);
                link.widget_info(|| WidgetInfo::labeled(WidgetType::Link, true, *part));
                if link.hovered() {
                    ui.painter().hline(
                        link.rect.x_range(),
                        link.rect.bottom(),
                        Stroke::new(1.0, colors.fg_muted),
                    );
                }
                focus_ring(ui, &link, 4);
                if link.clicked() {
                    clicked = Some(index);
                }
                glyph(ui, icons::CARET_RIGHT, Tokens::CAPTION, colors.fg_subtle);
            }
        }
    });
    clicked
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::dashboard) enum StepState {
    Done,
    Current,
    Upcoming,
}

/// Numbered setup step: done shows a check in the ok color; current is highlighted.
pub(in crate::dashboard) fn step_item(ui: &mut Ui, number: usize, label: &str, state: StepState) {
    let colors = Tokens::get(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    response.widget_info(|| {
        let suffix = match state {
            StepState::Done => ", done",
            StepState::Current => ", current step",
            StepState::Upcoming => "",
        };
        WidgetInfo::labeled(
            WidgetType::Label,
            true,
            format!("Step {number}: {label}{suffix}"),
        )
    });
    if state == StepState::Current {
        ui.painter()
            .rect_filled(rect, Tokens::CONTROL_RADIUS, colors.muted);
    }
    let badge = Rect::from_center_size(pos2(rect.left() + 21.0, rect.center().y), vec2(22.0, 22.0));
    match state {
        StepState::Done => {
            ui.painter().circle_filled(badge.center(), 11.0, colors.ok);
            ui.painter().text(
                badge.center(),
                Align2::CENTER_CENTER,
                icons::CHECK,
                Tokens::icon(Tokens::CAPTION),
                Color32::from_rgb(0x05, 0x2e, 0x12),
            );
        }
        StepState::Current | StepState::Upcoming => {
            let stroke = if state == StepState::Current {
                colors.fg
            } else {
                colors.border
            };
            ui.painter()
                .circle_stroke(badge.center(), 10.25, Stroke::new(1.5, stroke));
            ui.painter().text(
                badge.center(),
                Align2::CENTER_CENTER,
                number.to_string(),
                Tokens::mono(Tokens::CAPTION),
                if state == StepState::Current {
                    colors.fg
                } else {
                    colors.fg_subtle
                },
            );
        }
    }
    let (font, color) = match state {
        StepState::Done => (FontId::proportional(Tokens::BODY), colors.fg_muted),
        StepState::Current => (Tokens::medium(Tokens::BODY), colors.fg),
        StepState::Upcoming => (FontId::proportional(Tokens::BODY), colors.fg_subtle),
    };
    ui.painter().text(
        pos2(badge.right() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
}

/// Settings row: title and wrapped detail on the left, the control on the right, and a
/// top border between rows.
pub(in crate::dashboard) fn setting_row<R>(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    let top = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        snap(ui, top - 0.5),
        Stroke::new(1.0, colors.border),
    );
    // The same gap below the separator as above it, so each row's text sits centered.
    ui.add_space(ui.spacing().item_spacing.y);
    let text_width = (width * 0.55).max(240.0);
    // Top-aligned: the control is centered on the text block below, not on a fixed row height.
    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
        let text =
            ui.allocate_ui_with_layout(vec2(text_width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(text_width);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(title).font(Tokens::medium(Tokens::BODY)));
                if !detail.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(detail)
                                .size(Tokens::CAPTION)
                                .color(colors.fg_subtle),
                        )
                        .wrap()
                        .selectable(false),
                    );
                }
            });
        // Centered on the text block, however many lines it wrapped to.
        let height = text.response.rect.height().max(Tokens::CONTROL_HEIGHT);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), height),
            Layout::right_to_left(Align::Center),
            control,
        )
        .inner
    })
    .inner
}
