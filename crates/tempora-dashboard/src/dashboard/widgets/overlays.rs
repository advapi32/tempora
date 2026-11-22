//! Modal dialogs and their parts, popovers, menus and bordered lists.

use eframe::egui::{self, Align, Color32, Context, Frame, Layout, RichText, Ui, vec2};
use egui_phosphor::regular as icons;

use super::cards::surface;
use super::{Button, ButtonKind, ButtonSize, Tokens, table_cell};

const DIALOG_WIDTH: f32 = 460.0;

const DIALOG_PADDING: i8 = 16;

const DIALOG_INSET: f32 = 2.0 * (DIALOG_PADDING as f32 + 1.0);

fn dialog_frame(colors: &Tokens) -> Frame {
    surface(
        colors.popover,
        colors.border,
        Tokens::DIALOG_RADIUS,
        DIALOG_PADDING,
    )
    .shadow(colors.shadow)
}

pub(in crate::dashboard) fn modal(
    ctx: &Context,
    id: egui::Id,
    title: &str,
    content: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    let colors = Tokens::for_dark(ctx.global_style().visuals.dark_mode);
    let width = DIALOG_WIDTH.min(ctx.content_rect().width() - 48.0);
    let modal = egui::Modal::new(id)
        .frame(dialog_frame(colors))
        .show(ctx, |ui| {
            let close = dialog_header(ui, title, width - DIALOG_INSET);
            content(ui) || close
        });
    modal.inner || modal.should_close()
}

#[cfg_attr(not(debug_assertions), allow(dead_code))] // Shown in the debug-only gallery.
pub(in crate::dashboard) fn dialog_preview(
    ui: &mut Ui,
    title: &str,
    content: impl FnOnce(&mut Ui),
) {
    let frame = dialog_frame(Tokens::get(ui));
    let width = ui.available_width().min(DIALOG_WIDTH);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 0.0),
        Layout::top_down(Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                frame.show(ui, |ui| {
                    dialog_header(ui, title, width - DIALOG_INSET);
                    content(ui);
                });
            });
        },
    );
}

fn dialog_header(ui: &mut Ui, title: &str, width: f32) -> bool {
    ui.set_width(width);
    let close = Button {
        icon: Some(icons::X),
        size: ButtonSize::Small,
        ..Button::new("Close dialog", ButtonKind::GhostIcon)
    };
    let height = close.size.height();
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = Tokens::CONTROL_GAP;
            let title_width = (width - close.width(ui) - Tokens::CONTROL_GAP).max(0.0);
            table_cell(ui, title_width, height, Align::Min, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(title).font(Tokens::semibold(Tokens::DIALOG_TITLE)),
                    )
                    .truncate(),
                )
                .on_hover_text(title);
            });
            ui.add(close).clicked()
        },
    )
    .inner
}

pub(in crate::dashboard) enum DialogAction {
    Cancel,
    Confirm,
}

pub(in crate::dashboard) fn dialog_footer(
    ui: &mut Ui,
    confirm: Button<'_>,
    enabled: bool,
    caption: &str,
) -> Option<DialogAction> {
    footer(ui, Some("Cancel"), confirm, enabled, caption)
}

/// Step footer: the same layout with Back in place of Cancel; no Back on the first step
/// when `back` is empty.
pub(in crate::dashboard) fn wizard_footer(
    ui: &mut Ui,
    back: &str,
    confirm: Button<'_>,
    enabled: bool,
    caption: &str,
) -> Option<DialogAction> {
    footer(
        ui,
        (!back.is_empty()).then_some(back),
        confirm,
        enabled,
        caption,
    )
}

fn footer(
    ui: &mut Ui,
    cancel: Option<&str>,
    confirm: Button<'_>,
    enabled: bool,
    caption: &str,
) -> Option<DialogAction> {
    let cancel = cancel.map(|label| Button {
        min_width: 92.0,
        ..Button::new(label, ButtonKind::Secondary)
    });
    let confirm = Button {
        min_width: 92.0,
        ..confirm
    };
    let width = ui.available_width();
    let height = confirm.size.height();
    let cancel_width = cancel.map_or(0.0, |cancel| cancel.width(ui) + Tokens::CONTROL_GAP);
    let left_width = (width - cancel_width - confirm.width(ui) - Tokens::CONTROL_GAP).max(0.0);
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = Tokens::CONTROL_GAP;
            table_cell(ui, left_width, height, Align::Min, |ui| {
                if !caption.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(caption)
                                .size(Tokens::CAPTION)
                                .color(Tokens::get(ui).fg_muted),
                        )
                        .truncate(),
                    )
                    .on_hover_text(caption);
                }
            });
            let cancelled = cancel.is_some_and(|cancel| ui.add(cancel).clicked());
            let confirmed = ui.add_enabled(enabled, confirm).clicked();
            if cancelled {
                Some(DialogAction::Cancel)
            } else if confirmed {
                Some(DialogAction::Confirm)
            } else {
                None
            }
        },
    )
    .inner
}

/// Bordered, scrolling list container (radius 10).
pub(in crate::dashboard) fn bordered_list(ui: &mut Ui, height: f32, content: impl FnOnce(&mut Ui)) {
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    surface(
        Color32::TRANSPARENT,
        colors.border,
        Tokens::POPOVER_RADIUS,
        0,
    )
    .show(ui, |ui| {
        ui.set_width(width - 2.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        super::scroll(
            ui,
            egui::ScrollArea::vertical()
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false]),
            content,
        );
    });
}

pub(in crate::dashboard) fn popover<R>(
    ui: &mut Ui,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let colors = Tokens::get(ui);
    surface(colors.popover, colors.border, Tokens::POPOVER_RADIUS, 8)
        .shadow(colors.shadow)
        .show(ui, |ui| {
            ui.with_layout(Layout::top_down(Align::Min), content).inner
        })
}

pub(in crate::dashboard) fn menu<R>(
    ui: &mut Ui,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    popover(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.set_min_width(170.0);
        content(ui)
    })
}
