use eframe::egui::{
    self, Align, Color32, Layout, Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder,
    WidgetInfo, WidgetType, pos2, vec2,
};

use super::{Tokens, focus_ring, right_aligned_cell, snap};

#[derive(Clone, Copy)]
pub(in crate::dashboard) enum Width {
    Fixed(f32),
    /// Share of the remaining width, with a minimum.
    Flex(f32, f32),
}

#[derive(Clone, Copy)]
pub(in crate::dashboard) struct Column<'a> {
    pub label: &'a str,
    pub width: Width,
    pub align: Align,
}

impl<'a> Column<'a> {
    pub const fn new(label: &'a str, width: Width, align: Align) -> Self {
        Self {
            label,
            width,
            align,
        }
    }
}

/// Resolved column positions for one table width.
pub(in crate::dashboard) struct Columns {
    pub spans: Vec<(f32, f32)>,
    pub aligns: Vec<Align>,
    pub width: f32,
}

pub(in crate::dashboard) const TABLE_GAP: f32 = 16.0;
const CELL_PADDING: f32 = 12.0;

/// Columns for a table whose rows sit in a vertical scroll area. The scroll bar takes a
/// gutter on the right; rows laid out at the full width would push the card past the page.
pub(in crate::dashboard) fn scrolled_columns(
    ui: &egui::Ui,
    width: f32,
    specs: &[Column<'_>],
) -> Columns {
    let scroll = &ui.spacing().scroll;
    let gutter = scroll.bar_width + scroll.bar_inner_margin + scroll.bar_outer_margin;
    columns(width - gutter, specs)
}

pub(in crate::dashboard) fn columns(width: f32, specs: &[Column<'_>]) -> Columns {
    let inner = (width - 2.0 * CELL_PADDING).max(0.0);
    let gaps = TABLE_GAP * specs.len().saturating_sub(1) as f32;
    let fixed: f32 = specs
        .iter()
        .map(|column| match column.width {
            Width::Fixed(width) => width,
            Width::Flex(_, min) => min,
        })
        .sum();
    let weights: f32 = specs
        .iter()
        .map(|column| match column.width {
            Width::Flex(weight, _) => weight,
            Width::Fixed(_) => 0.0,
        })
        .sum();
    let spare = (inner - gaps - fixed).max(0.0);
    let mut x = CELL_PADDING;
    let mut spans = Vec::with_capacity(specs.len());
    for column in specs {
        let width = match column.width {
            Width::Fixed(width) => width,
            Width::Flex(weight, min) => min + spare * weight / weights.max(f32::EPSILON),
        };
        spans.push((x, width));
        x += width + TABLE_GAP;
    }
    Columns {
        spans,
        aligns: specs.iter().map(|column| column.align).collect(),
        width,
    }
}

/// Header row: caption-size muted labels, a bottom border.
pub(in crate::dashboard) fn table_header(ui: &mut Ui, columns: &Columns, labels: &[&str]) {
    let colors = Tokens::get(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(columns.width, 30.0), Sense::hover());
    for ((x, width), (label, align)) in columns.spans.iter().zip(labels.iter().zip(&columns.aligns))
    {
        let cell = Rect::from_min_size(
            pos2(rect.left() + x, rect.top()),
            vec2(*width, rect.height()),
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(cell).layout(cell_layout(*align)));
        child.set_clip_rect(cell.intersect(ui.clip_rect()));
        let text = RichText::new(*label)
            .font(Tokens::medium(Tokens::CAPTION))
            .color(colors.fg_muted);
        if *align == Align::Max {
            right_aligned_cell(&mut child, *width, rect.height(), text);
        } else {
            child.add(egui::Label::new(text).truncate().selectable(false));
        }
    }
    ui.painter().hline(
        rect.x_range(),
        snap(ui, rect.bottom() - 0.5),
        Stroke::new(1.0, colors.border),
    );
}

/// Header whose sortable labels are buttons; the active one is fg with a direction caret.
/// Returns the clicked column.
pub(in crate::dashboard) fn sortable_header(
    ui: &mut Ui,
    columns: &Columns,
    labels: &[&str],
    sortable: &[bool],
    active: usize,
    descending: bool,
) -> Option<usize> {
    let colors = Tokens::get(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(columns.width, 34.0), Sense::hover());
    let mut clicked = None;
    for (index, ((x, width), label)) in columns.spans.iter().zip(labels).enumerate() {
        let cell = Rect::from_min_size(
            pos2(rect.left() + x, rect.top()),
            vec2(*width, rect.height()),
        );
        let align = columns.aligns[index];
        let mut child = ui.new_child(UiBuilder::new().max_rect(cell).layout(cell_layout(align)));
        child.set_clip_rect(cell.intersect(ui.clip_rect()));
        let is_active = sortable.get(index).copied().unwrap_or(false) && index == active;
        let color = if is_active {
            colors.fg
        } else {
            colors.fg_muted
        };
        let text = (*label).to_owned();
        let rich = RichText::new(text)
            .font(Tokens::medium(Tokens::CAPTION))
            .color(color);
        if sortable.get(index).copied().unwrap_or(false) {
            let response = child.add(
                egui::Label::new(rich)
                    .sense(Sense::click())
                    .selectable(false),
            );
            let name = format!(
                "Sort by {label}{}",
                if is_active {
                    if descending {
                        ", descending"
                    } else {
                        ", ascending"
                    }
                } else {
                    ""
                }
            );
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
            focus_ring(&child, &response, 4);
            if is_active {
                let icon = if descending {
                    egui_phosphor::regular::ARROW_DOWN
                } else {
                    egui_phosphor::regular::ARROW_UP
                };
                let at = if align == Align::Max {
                    pos2(response.rect.left() - 8.0, response.rect.center().y)
                } else {
                    pos2(response.rect.right() + 8.0, response.rect.center().y)
                };
                child.painter().text(
                    at,
                    egui::Align2::CENTER_CENTER,
                    icon,
                    Tokens::icon(Tokens::CAPTION),
                    colors.fg,
                );
            }
            if response.clicked() {
                clicked = Some(index);
            }
        } else {
            child.add(egui::Label::new(rich).truncate().selectable(false));
        }
    }
    ui.painter().hline(
        rect.x_range(),
        snap(ui, rect.bottom() - 0.5),
        Stroke::new(1.0, colors.border),
    );
    clicked
}

fn cell_layout(align: Align) -> Layout {
    if align == Align::Max {
        Layout::right_to_left(Align::Center)
    } else {
        Layout::left_to_right(Align::Center)
    }
}

pub(in crate::dashboard) struct RowStyle {
    pub height: f32,
    pub clickable: bool,
    pub separator: bool,
    pub selected: bool,
    pub indent: f32,
}

impl Default for RowStyle {
    fn default() -> Self {
        Self {
            height: 48.0,
            clickable: true,
            separator: false,
            selected: false,
            indent: 0.0,
        }
    }
}

/// Cells of one row; each closure gets a child `Ui` clipped to its column.
pub(in crate::dashboard) struct RowCells<'a> {
    ui: &'a mut Ui,
    rect: Rect,
    columns: &'a Columns,
    indent: f32,
}

impl RowCells<'_> {
    pub fn cell<R>(&mut self, index: usize, content: impl FnOnce(&mut Ui) -> R) -> R {
        let (x, width) = self.columns.spans[index];
        let (x, width) = if index == 0 {
            (x + self.indent, (width - self.indent).max(0.0))
        } else {
            (x, width)
        };
        let cell = Rect::from_min_size(
            pos2(self.rect.left() + x, self.rect.top()),
            vec2(width, self.rect.height()),
        );
        let mut child = self.ui.new_child(
            UiBuilder::new()
                .max_rect(cell)
                .layout(cell_layout(self.columns.aligns[index])),
        );
        child.set_clip_rect(cell.intersect(self.ui.clip_rect()));
        child.style_mut().interaction.selectable_labels = false;
        child.spacing_mut().item_spacing.x = Tokens::CONTROL_GAP;
        content(&mut child)
    }

    pub fn width(&self, index: usize) -> f32 {
        self.columns.spans[index].1
    }

    /// Right-aligned mono text, the format for durations and counts.
    pub fn number(&mut self, index: usize, text: &str, color: Color32) {
        let width = self.width(index);
        let height = self.rect.height();
        self.cell(index, |ui| {
            right_aligned_cell(
                ui,
                width,
                height,
                RichText::new(text)
                    .font(Tokens::mono(Tokens::SECONDARY))
                    .color(color),
            );
        });
    }
}

/// A table row. Clickable rows are focusable, activate with Enter or Space and carry
/// `label` as their accessible name.
pub(in crate::dashboard) fn table_row(
    ui: &mut Ui,
    id: egui::Id,
    columns: &Columns,
    style: RowStyle,
    label: &str,
    cells: impl FnOnce(&mut RowCells<'_>),
) -> Response {
    let colors = Tokens::get(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(columns.width, style.height), Sense::hover());
    let sense = if style.clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| {
        WidgetInfo::labeled(
            if style.clickable {
                WidgetType::Button
            } else {
                WidgetType::Label
            },
            ui.is_enabled(),
            label,
        )
    });
    // Highlights float 3 px inside the row, so their rounded corners never butt against
    // the header line or the neighbouring row.
    let highlight = snap(ui, rect.shrink2(vec2(0.0, 3.0)));
    if style.selected {
        ui.painter()
            .rect_filled(highlight, Tokens::CONTROL_RADIUS, colors.muted);
    } else if style.clickable && response.hovered() {
        ui.painter()
            .rect_filled(highlight, Tokens::CONTROL_RADIUS, colors.hover);
    }
    if style.separator {
        ui.painter().hline(
            rect.x_range(),
            snap(ui, rect.bottom() - 0.5),
            Stroke::new(1.0, colors.border),
        );
    }
    let mut row = RowCells {
        ui,
        rect,
        columns,
        indent: style.indent,
    };
    cells(&mut row);
    if response.has_focus() {
        super::paint_focus_ring(row.ui, highlight, Tokens::CONTROL_RADIUS);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flex_columns_share_the_remaining_width() {
        let specs = [
            Column::new("App", Width::Flex(2.0, 100.0), Align::Min),
            Column::new("Category", Width::Fixed(130.0), Align::Min),
            Column::new("Bar", Width::Flex(1.0, 100.0), Align::Min),
        ];
        let resolved = columns(600.0, &specs);
        let (last_x, last_width) = resolved.spans[2];
        assert!((last_x + last_width - (600.0 - CELL_PADDING)).abs() < 0.01);
        assert!((resolved.spans[0].1 - 100.0 - 2.0 * resolved.spans[2].1 + 200.0).abs() < 0.01);
    }
}
