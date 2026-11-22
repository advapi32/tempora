//! Card surfaces: plain and table cards, KPI tiles, stat tiles and the split card.

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Layout, Rect, RichText, Sense, Stroke, Ui,
    UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};

use super::marks::{BadgeKind, badge};
use super::share_bar;
use super::{Tokens, snap};

pub(super) fn surface(fill: Color32, border: Color32, radius: u8, padding: i8) -> Frame {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(radius)
        .inner_margin(padding)
}

pub(in crate::dashboard) fn card<R>(
    ui: &mut Ui,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let colors = Tokens::get(ui);
    // A title line carries about 3 px of leading above its capitals, so the top inset is
    // trimmed by that much to look the same as the sides.
    let padding = Tokens::CARD_PADDING;
    surface(colors.card, colors.border, Tokens::CARD_RADIUS, padding)
        .inner_margin(egui::Margin {
            left: padding,
            right: padding,
            top: padding - 3,
            bottom: padding,
        })
        .show(ui, content)
}

/// Bottom padding of a titled table card (Top apps, sessions, This day).
pub(in crate::dashboard) const TITLED_TABLE_BOTTOM: f32 = 10.0;

/// Bottom padding of a header-only table card (Library).
pub(in crate::dashboard) const PLAIN_TABLE_BOTTOM: f32 = 8.0;

/// Card for a table: tighter than `card` because table cells carry their own 12 px
/// padding. Titled tables get room for their heading; header-only tables sit close.
pub(in crate::dashboard) fn table_card<R>(
    ui: &mut Ui,
    titled: bool,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let colors = Tokens::get(ui);
    let margin = if titled {
        egui::Margin {
            left: 12,
            right: 12,
            top: Tokens::CARD_PADDING - 3,
            bottom: TITLED_TABLE_BOTTOM as i8,
        }
    } else {
        egui::Margin {
            left: 8,
            right: 8,
            top: 6,
            bottom: PLAIN_TABLE_BOTTOM as i8,
        }
    };
    surface(colors.card, colors.border, Tokens::CARD_RADIUS, 0)
        .inner_margin(margin)
        .show(ui, content)
}

pub(in crate::dashboard) struct Kpi<'a> {
    pub label: &'a str,
    pub icon: &'static str,
    pub value: String,
    pub value_size: f32,
    pub delta: Option<String>,
    pub caption: Option<String>,
    /// Share bar before the caption: fraction and color.
    pub share: Option<(f32, Color32)>,
}

impl<'a> Kpi<'a> {
    pub fn new(label: &'a str, icon: &'static str, value: String) -> Self {
        Self {
            label,
            icon,
            value,
            value_size: Tokens::KPI,
            delta: None,
            caption: None,
            share: None,
        }
    }
}

/// KPI cards in equal columns and equal heights.
pub(in crate::dashboard) fn kpi_grid(ui: &mut Ui, kpis: &[Kpi<'_>], columns: usize) {
    let columns = columns.max(1);
    let gap = Tokens::GRID_GAP;
    let width = ui.available_width();
    let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).floor();
    let caption_row = kpis
        .iter()
        .any(|kpi| kpi.delta.is_some() || kpi.caption.is_some() || kpi.share.is_some());
    for row in kpis.chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for kpi in row {
                ui.allocate_ui_with_layout(
                    vec2(card_width, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(card_width);
                        kpi_body(ui, kpi, caption_row);
                    },
                );
            }
        });
    }
}

#[cfg_attr(not(debug_assertions), allow(dead_code))] // Shown in the debug-only gallery.
pub(in crate::dashboard) fn kpi_card(
    ui: &mut Ui,
    label: &str,
    icon: &'static str,
    value: &str,
    delta: Option<&str>,
    caption: &str,
) {
    let kpi = Kpi {
        delta: delta.map(str::to_owned),
        caption: (!caption.is_empty()).then(|| caption.to_owned()),
        ..Kpi::new(label, icon, value.to_owned())
    };
    let reserve = kpi.delta.is_some() || kpi.caption.is_some();
    kpi_body(ui, &kpi, reserve);
}

fn kpi_body(ui: &mut Ui, kpi: &Kpi<'_>, caption_row: bool) {
    let colors = Tokens::get(ui);
    let (label, icon, value) = (kpi.label, kpi.icon, kpi.value.as_str());
    card(ui, |ui| {
        let width = ui.available_width();
        ui.set_min_width(width);
        ui.spacing_mut().item_spacing.y = 6.0;
        let label_width = (width - Tokens::ICON_SIZE - Tokens::CONTROL_GAP).max(0.0);
        let (header, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
        let text_rect = Rect::from_min_size(header.min, vec2(label_width, header.height()));
        let mut left = ui.new_child(
            UiBuilder::new()
                .max_rect(text_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        left.add(
            egui::Label::new(
                RichText::new(label)
                    .size(Tokens::SECONDARY)
                    .color(colors.fg_muted),
            )
            .truncate(),
        )
        .on_hover_text(label);
        ui.painter().text(
            snap(ui, header.right_center()),
            Align2::RIGHT_CENTER,
            icon,
            Tokens::icon(Tokens::ICON_SIZE),
            colors.fg_muted,
        );
        ui.add(
            egui::Label::new(RichText::new(value).font(Tokens::semibold(kpi.value_size)))
                .truncate()
                .selectable(false),
        );
        if caption_row {
            ui.allocate_ui_with_layout(
                vec2(width, 20.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    // Keeps equal card heights when a card has no caption.
                    ui.set_min_height(20.0);
                    if let Some(delta) = &kpi.delta {
                        badge(ui, delta, BadgeKind::Delta);
                    }
                    if let Some((fraction, color)) = kpi.share {
                        share_bar(ui, fraction, color, 64.0);
                    }
                    if let Some(caption) = &kpi.caption {
                        ui.add(
                            egui::Label::new(
                                RichText::new(caption)
                                    .size(Tokens::CAPTION)
                                    .color(colors.fg_subtle),
                            )
                            .truncate()
                            .selectable(false),
                        );
                    }
                },
            );
        }
    });
}

/// Horizontal inset of the text inside stat tiles; event logs under them share it.
pub(in crate::dashboard) const STAT_TEXT_INSET: f32 = 14.0;

/// Equal-height muted tiles (caption label, large value) in a grid of `columns`.
#[cfg_attr(not(debug_assertions), allow(dead_code))] // Shown in the debug-only gallery.
pub(in crate::dashboard) fn stat_tiles(ui: &mut Ui, tiles: &[(&str, String)], columns: usize) {
    const HEIGHT: f32 = 62.0;
    const GAP: f32 = 10.0;
    let colors = Tokens::get(ui);
    let columns = columns.max(1);
    let width = ui.available_width();
    let tile_width = ((width - GAP * (columns - 1) as f32) / columns as f32).floor();
    let rows = tiles.len().div_ceil(columns);
    let (area, _) = ui.allocate_exact_size(
        vec2(
            width,
            rows as f32 * HEIGHT + rows.saturating_sub(1) as f32 * GAP,
        ),
        Sense::hover(),
    );
    for (index, (label, value)) in tiles.iter().enumerate() {
        let (row, column) = (index / columns, index % columns);
        let rect = snap(
            ui,
            Rect::from_min_size(
                area.min
                    + vec2(
                        column as f32 * (tile_width + GAP),
                        row as f32 * (HEIGHT + GAP),
                    ),
                vec2(tile_width, HEIGHT),
            ),
        );
        ui.painter()
            .rect_filled(rect, Tokens::POPOVER_RADIUS, colors.muted);
        let painter = ui.painter().with_clip_rect(rect.shrink(4.0));
        painter.text(
            snap(ui, rect.left_top() + vec2(STAT_TEXT_INSET, 12.0)),
            Align2::LEFT_TOP,
            *label,
            FontId::proportional(Tokens::CAPTION),
            colors.fg_muted,
        );
        painter.text(
            snap(ui, rect.left_bottom() + vec2(STAT_TEXT_INSET, -10.0)),
            Align2::LEFT_BOTTOM,
            value,
            Tokens::semibold(20.0),
            colors.fg,
        );
        let response = ui.interact(rect, ui.id().with(("stat-tile", index)), Sense::hover());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Label, true, format!("{label}: {value}"))
        });
    }
}

/// Vertical margins of the side pane (32 top and bottom) and the main pane (36 + 28).
const SPLIT_MARGINS: f32 = 64.0;

/// Card split into a muted side pane and a main pane, as in the first-run window. The card is
/// only as tall as its content: the taller pane sets the height. The side pane is given the
/// main pane's inner height from the previous frame (`fill`), so anything it pins to its
/// bottom lines up with the main pane's last line; the height is stored and repainted when it
/// changes.
pub(in crate::dashboard) fn split_card(
    ui: &mut Ui,
    side_width: f32,
    side: impl FnOnce(&mut Ui, f32),
    main: impl FnOnce(&mut Ui),
) {
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    let used_id = ui.id().with("split-card-main-height");
    let used = ui.data(|data| data.get_temp::<f32>(used_id)).unwrap_or(0.0);
    let frame = surface(colors.card, colors.border, Tokens::CARD_RADIUS, 0).show(ui, |ui| {
        ui.set_width(width - 2.0);
        let available = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.allocate_ui_with_layout(
                vec2(side_width, available),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(28, 32))
                        .show(ui, |ui| {
                            ui.set_width(side_width - 56.0);
                            side(ui, (used - SPLIT_MARGINS).max(0.0));
                        });
                },
            );
            let main_width = ui.available_width();
            let main_used = ui
                .allocate_ui_with_layout(
                    vec2(main_width, available),
                    Layout::top_down(Align::Min),
                    |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 40,
                                right: 40,
                                top: 36,
                                bottom: 28,
                            })
                            .show(ui, |ui| {
                                ui.set_width(main_width - 80.0);
                                main(ui);
                            });
                    },
                )
                .response
                .rect
                .height();
            if (main_used - used).abs() > 0.5 {
                ui.data_mut(|data| data.insert_temp(used_id, main_used));
                ui.ctx().request_repaint();
            }
        });
    });
    let rect = frame.response.rect;
    let side_rect = Rect::from_min_max(rect.min, pos2(rect.left() + side_width, rect.bottom()));
    let side_rect = side_rect.shrink(1.0);
    ui.painter().rect_filled(
        side_rect,
        egui::CornerRadius {
            nw: Tokens::CARD_RADIUS - 1,
            sw: Tokens::CARD_RADIUS - 1,
            ne: 0,
            se: 0,
        },
        colors.muted.gamma_multiply(0.5),
    );
    ui.painter().vline(
        snap(ui, side_rect.right() + 0.5),
        rect.y_range(),
        Stroke::new(1.0, colors.border),
    );
}
