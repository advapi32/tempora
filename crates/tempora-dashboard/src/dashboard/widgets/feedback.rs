//! Messages about state: empty states, banners and toasts.

use eframe::egui::{self, Align, Align2, Color32, Context, FontId, Layout, RichText, Ui, vec2};
use egui_phosphor::regular as icons;

use super::cards::{card, surface};
use super::{Button, ButtonKind, ButtonSize, Tokens, glyph, glyph_with_font};

/// Centered empty state inside a card, with up to two actions.
pub(in crate::dashboard) fn empty_state(
    ui: &mut Ui,
    title: &str,
    message: &str,
    actions: &[Button<'_>],
) -> Option<usize> {
    let colors = Tokens::get(ui);
    let mut clicked = None;
    card(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space(24.0);
            glyph(ui, icons::CHART_BAR, 28.0, colors.fg_subtle);
            ui.label(RichText::new(title).font(Tokens::semibold(Tokens::SECTION)));
            ui.label(
                RichText::new(message)
                    .size(Tokens::SECONDARY)
                    .color(colors.fg_muted),
            );
            if !actions.is_empty() {
                let gap = Tokens::CONTROL_GAP;
                let width = actions.iter().map(|button| button.width(ui)).sum::<f32>()
                    + gap * (actions.len() - 1) as f32;
                ui.allocate_ui_with_layout(
                    vec2(width, Tokens::CONTROL_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for (index, button) in actions.iter().enumerate() {
                            if ui.add(*button).clicked() {
                                clicked = Some(index);
                            }
                        }
                    },
                );
            }
            ui.add_space(24.0);
        });
    });
    clicked
}

#[derive(Clone, Copy)]
pub(in crate::dashboard) enum BannerKind {
    Info,
    Paused,
    Error,
}

pub(in crate::dashboard) fn banner(
    ui: &mut Ui,
    kind: BannerKind,
    title: &str,
    detail: &str,
    actions: &[(Button<'_>, bool)],
) -> Option<usize> {
    const ICON: f32 = 18.0;
    const TEXT_GAP: f32 = 2.0;
    let colors = Tokens::get(ui);
    let (icon, font, color, border) = match kind {
        BannerKind::Info => (
            icons::INFO,
            Tokens::icon(ICON),
            colors.fg_muted,
            colors.border,
        ),
        BannerKind::Paused => (
            egui_phosphor::fill::PAUSE,
            Tokens::filled_icon(ICON),
            colors.warn,
            colors.border,
        ),
        BannerKind::Error => (icons::WARNING, Tokens::icon(ICON), colors.bad, colors.bad),
    };
    surface(colors.card, border, Tokens::POPOVER_RADIUS, 12)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            let gap = Tokens::CONTROL_GAP;
            let actions_width = actions
                .iter()
                .map(|(button, _)| button.width(ui) + gap)
                .sum::<f32>();
            // A little more room after the icon than between the buttons.
            let icon_gap = 12.0;
            let text_width = (width - ICON - icon_gap - actions_width).max(0.0);
            let title_text = wrapped(
                ui,
                title,
                Tokens::semibold(Tokens::BODY),
                colors.fg,
                text_width,
                1,
            );
            let detail_text = (!detail.is_empty()).then(|| {
                wrapped(
                    ui,
                    detail,
                    FontId::proportional(Tokens::SECONDARY),
                    colors.fg_muted,
                    text_width,
                    2,
                )
            });
            let text_height = title_text.size().y
                + detail_text
                    .as_ref()
                    .map_or(0.0, |text| text.size().y + TEXT_GAP);
            let height = actions
                .iter()
                .fold(ICON.max(text_height), |height, (button, _)| {
                    height.max(button.size.height())
                });
            ui.allocate_ui_with_layout(
                vec2(width, height),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    glyph_with_font(ui, icon, font, color);
                    ui.add_space(icon_gap - gap);
                    ui.allocate_ui_with_layout(
                        vec2(text_width, text_height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            // Take the whole width so the actions sit at the right edge.
                            ui.set_min_width(text_width);
                            ui.spacing_mut().item_spacing.y = TEXT_GAP;
                            ui.label(title_text).on_hover_text(title);
                            if let Some(detail_text) = detail_text {
                                ui.label(detail_text).on_hover_text(detail);
                            }
                        },
                    );
                    let mut clicked = None;
                    for (index, (button, enabled)) in actions.iter().enumerate() {
                        if ui.add_enabled(*enabled, *button).clicked() {
                            clicked = Some(index);
                        }
                    }
                    clicked
                },
            )
            .inner
        })
        .inner
}

fn wrapped(
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

#[derive(PartialEq)]
pub(in crate::dashboard) enum ToastAction {
    Dismiss,
    Undo,
}

pub(in crate::dashboard) fn toast(
    ctx: &Context,
    id: egui::Id,
    message: &str,
    error: bool,
    undo: Option<&str>,
) -> Option<ToastAction> {
    const INSET: f32 = 20.0;
    let mut result = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-INSET, -INSET))
        .show(ctx, |ui| {
            result = toast_body(ui, message, error, undo);
        });
    result
}

pub(in crate::dashboard) fn toast_body(
    ui: &mut Ui,
    message: &str,
    error: bool,
    undo: Option<&str>,
) -> Option<ToastAction> {
    let colors = Tokens::get(ui);
    let (icon, color) = if error {
        (icons::WARNING, colors.bad)
    } else {
        (icons::CHECK_CIRCLE, colors.ok)
    };
    let undo_button = undo.map(|label| Button {
        size: ButtonSize::Small,
        ..Button::new(label, ButtonKind::Secondary)
    });
    let close_button = Button {
        size: ButtonSize::Small,
        icon: Some(icons::X),
        ..Button::new("Dismiss", ButtonKind::GhostIcon)
    };
    surface(colors.popover, colors.border, Tokens::POPOVER_RADIUS, 12)
        .inner_margin(egui::Margin {
            left: 16,
            right: 12,
            top: 12,
            bottom: 12,
        })
        .shadow(colors.shadow)
        .show(ui, |ui| {
            // The mockup's toast spaces icon, message and actions 14 apart.
            let gap = 14.0;
            let actions_width = close_button.width(ui)
                + gap
                + undo_button
                    .as_ref()
                    .map_or(0.0, |button| button.width(ui) + gap);
            let icon_width = Tokens::ICON_SIZE + gap;
            let max_text_width =
                (ui.available_width() - icon_width - actions_width).clamp(0.0, 250.0);
            let text = wrapped(
                ui,
                message,
                FontId::proportional(Tokens::SECONDARY),
                colors.fg,
                max_text_width,
                3,
            );
            let height = close_button.size.height().max(text.size().y);
            let width = icon_width + text.size().x + actions_width;
            ui.allocate_ui_with_layout(
                vec2(width, height),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    glyph(ui, icon, Tokens::ICON_SIZE, color);
                    ui.label(text).on_hover_text(message);
                    let undo = undo_button.is_some_and(|button| ui.add(button).clicked());
                    let close = ui.add(close_button).clicked();
                    if undo {
                        Some(ToastAction::Undo)
                    } else if close {
                        Some(ToastAction::Dismiss)
                    } else {
                        None
                    }
                },
            )
            .inner
        })
        .inner
}
