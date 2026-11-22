use chrono::{Datelike, Days, NaiveDate, Weekday};
use eframe::egui::{
    self, Align, Align2, Key, Layout, Modifiers, Rect, Response, RichText, Sense, Stroke, Ui,
    WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icons;

use super::{
    Button, ButtonKind, DialogAction, Tokens, dialog_footer, list_item, paint_focus_ring, snap,
};

/// Secondary button showing the selected range, with a calendar icon and a caret.
/// The date range picker's button. `grouped` styles it as a segment of a `button_group`,
/// sized to its label.
pub(in crate::dashboard) fn range_button(ui: &mut Ui, label: &str, grouped: bool) -> Response {
    let button = Button::new(label, ButtonKind::Secondary)
        .with_icon(icons::CALENDAR_BLANK)
        .trailing(icons::CARET_DOWN);
    if grouped {
        ui.add(button.segment())
    } else {
        ui.add(Button {
            min_width: 210.0,
            align_left: true,
            ..button
        })
    }
}

/// Draft selection inside the range picker. Nothing applies until Apply.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::dashboard) struct Calendar {
    pub month: NaiveDate,
    pub start: NaiveDate,
    /// `None` while the second end is being picked.
    pub end: Option<NaiveDate>,
    /// A day to focus once its month is drawn (keyboard navigation across months).
    pub focus: Option<NaiveDate>,
}

impl Calendar {
    pub fn new(start: NaiveDate, end: NaiveDate) -> Self {
        Self {
            month: end.with_day(1).unwrap_or(end),
            start,
            end: Some(end),
            focus: None,
        }
    }

    pub fn selection(&self) -> (NaiveDate, NaiveDate) {
        (self.start, self.end.unwrap_or(self.start))
    }

    fn pick(&mut self, date: NaiveDate) {
        match self.end {
            None if date >= self.start => self.end = Some(date),
            _ => {
                self.start = date;
                self.end = None;
            }
        }
    }
}

pub(in crate::dashboard) enum PickerAction {
    Preset(usize),
    Apply,
    Cancel,
}

pub(in crate::dashboard) struct PickerInput<'a> {
    pub presets: &'a [(&'a str, bool)],
    /// "History available …" line; hidden when coverage is unknown.
    pub history: Option<&'a str>,
    pub today: NaiveDate,
    pub week_start: Weekday,
    /// Days with recorded history, when known. Unknown days stay selectable.
    pub covered: Option<&'a dyn Fn(NaiveDate) -> bool>,
}

/// Presets column plus a month calendar with Cancel / Apply.
pub(in crate::dashboard) fn range_picker(
    ui: &mut Ui,
    calendar: &mut Calendar,
    input: &PickerInput<'_>,
) -> Option<PickerAction> {
    let colors = Tokens::get(ui);
    let mut action = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        ui.allocate_ui_with_layout(vec2(156.0, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(156.0);
            for (index, (label, selected)) in input.presets.iter().enumerate() {
                if list_item(ui, label, None, *selected).clicked() {
                    action = Some(PickerAction::Preset(index));
                }
            }
        });
        let separator = ui.cursor().left();
        ui.add_space(16.0);
        ui.allocate_ui_with_layout(vec2(268.0, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(268.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            month_header(ui, calendar, input.today);
            month_grid(ui, calendar, input);
            if let Some(history) = input.history {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = Tokens::CONTROL_GAP;
                    let (dot, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, colors.ok);
                    ui.label(
                        RichText::new(history)
                            .size(Tokens::CAPTION)
                            .color(colors.fg_subtle),
                    );
                });
            }
            match dialog_footer(ui, Button::new("Apply", ButtonKind::Primary), true, "") {
                Some(DialogAction::Confirm) => action = Some(PickerAction::Apply),
                Some(DialogAction::Cancel) => action = Some(PickerAction::Cancel),
                None => {}
            }
        });
        let rect = ui.min_rect();
        ui.painter().vline(
            separator + 8.0,
            rect.y_range(),
            Stroke::new(1.0, colors.border),
        );
    });
    action
}

fn month_header(ui: &mut Ui, calendar: &mut Calendar, today: NaiveDate) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, 30.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let previous = Button {
                min_width: 30.0,
                ..Button::new("Previous month", ButtonKind::GhostIcon)
                    .small()
                    .with_icon(icons::CARET_LEFT)
            };
            if ui.add(previous).clicked() {
                calendar.month = shift(calendar.month, -1);
            }
            let title_width = width - 2.0 * Tokens::SMALL_HEIGHT;
            ui.allocate_ui_with_layout(
                vec2(title_width, 30.0),
                Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    ui.label(
                        RichText::new(calendar.month.format("%B %Y").to_string())
                            .font(Tokens::semibold(Tokens::BODY)),
                    );
                },
            );
            let next_month = shift(calendar.month, 1);
            let next = Button::new("Next month", ButtonKind::GhostIcon)
                .small()
                .with_icon(icons::CARET_RIGHT);
            if ui.add_enabled(next_month <= today, next).clicked() {
                calendar.month = next_month;
            }
        },
    );
}

fn shift(month: NaiveDate, by: i32) -> NaiveDate {
    let index = month.year() * 12 + month.month0() as i32 + by;
    NaiveDate::from_ymd_opt(index.div_euclid(12), index.rem_euclid(12) as u32 + 1, 1)
        .unwrap_or(month)
}

fn month_grid(ui: &mut Ui, calendar: &mut Calendar, input: &PickerInput<'_>) {
    const ROW: f32 = 34.0;
    let colors = Tokens::get(ui);
    let width = ui.available_width();
    let cell = width / 7.0;
    let (header, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    for index in 0..7u8 {
        let day = weekday_after(input.week_start, index);
        let name = &format!("{day}")[..2];
        ui.painter().text(
            pos2(
                header.left() + (f32::from(index) + 0.5) * cell,
                header.center().y,
            ),
            Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(Tokens::MICRO),
            colors.fg_subtle,
        );
    }
    let first = calendar.month;
    let lead = first.weekday().days_since(input.week_start) as usize;
    let days = shift(first, 1).signed_duration_since(first).num_days() as usize;
    let rows = (lead + days).div_ceil(7);
    let (grid, _) = ui.allocate_exact_size(vec2(width, rows as f32 * (ROW + 2.0)), Sense::hover());
    let (start, end) = calendar.selection();
    let cell_rect = |slot: usize| {
        Rect::from_min_size(
            grid.min + vec2((slot % 7) as f32 * cell, (slot / 7) as f32 * (ROW + 2.0)),
            vec2(cell, ROW),
        )
    };
    let mut picked = None;
    let mut moved = None;
    for offset in 0..days {
        let date = first + Days::new(offset as u64);
        let rect = snap(ui, cell_rect(lead + offset));
        let covered = input.covered.is_none_or(|covered| covered(date));
        let enabled = date <= input.today && covered;
        let in_range = calendar.end.is_some() && date >= start && date <= end;
        let endpoint = date == start || (calendar.end.is_some() && date == end);
        let id = ui.id().with(("calendar-day", date));
        let response = ui.interact(
            rect,
            id,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if calendar.focus == Some(date) {
            response.request_focus();
            calendar.focus = None;
        }
        let painter = ui.painter();
        if in_range && !endpoint {
            painter.rect_filled(rect, 0, colors.muted);
        }
        let foreground = if endpoint {
            painter.rect_filled(rect, Tokens::TAG_RADIUS, colors.primary);
            colors.primary_fg
        } else if enabled {
            if response.hovered() {
                painter.rect_filled(rect, Tokens::TAG_RADIUS, colors.hover);
            }
            colors.fg
        } else {
            colors.fg_subtle.gamma_multiply(0.5)
        };
        let font = if date == input.today {
            Tokens::mono_medium(Tokens::SECONDARY)
        } else {
            Tokens::mono(Tokens::SECONDARY)
        };
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            date.day().to_string(),
            font,
            foreground,
        );
        response.widget_info(|| {
            let mut label = date.format("%A, %B %-d, %Y").to_string();
            if date == input.today {
                label.push_str(", today");
            }
            if endpoint || in_range {
                label.push_str(", selected");
            }
            if !enabled {
                label.push_str(if date > input.today {
                    ", in the future"
                } else {
                    ", no history"
                });
            }
            WidgetInfo::selected(WidgetType::Button, enabled, endpoint || in_range, label)
        });
        if response.has_focus() {
            paint_focus_ring(ui, rect, Tokens::TAG_RADIUS);
            let step = ui.input_mut(|input| {
                [
                    (Key::ArrowLeft, -1),
                    (Key::ArrowRight, 1),
                    (Key::ArrowUp, -7),
                    (Key::ArrowDown, 7),
                ]
                .into_iter()
                .find(|(key, _)| input.consume_key(Modifiers::NONE, *key))
                .map(|(_, step)| step)
            });
            if let Some(step) = step {
                moved = date.checked_add_signed(chrono::Duration::days(step));
            }
        }
        if response.clicked() {
            picked = Some(date);
        }
    }
    if let Some(date) = picked {
        calendar.pick(date);
    }
    if let Some(date) = moved.filter(|date| *date <= input.today) {
        let month = date.with_day(1).unwrap_or(date);
        if month == calendar.month {
            ui.memory_mut(|memory| memory.request_focus(ui.id().with(("calendar-day", date))));
        } else {
            calendar.month = month;
            calendar.focus = Some(date);
        }
    }
}

fn weekday_after(start: Weekday, offset: u8) -> Weekday {
    (0..offset).fold(start, |day, _| day.succ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_two_days_selects_a_span_and_a_third_restarts() {
        let date = |day| NaiveDate::from_ymd_opt(2026, 11, day).unwrap();
        let mut calendar = Calendar::new(date(1), date(7));
        calendar.pick(date(10));
        assert_eq!(calendar.selection(), (date(10), date(10)));
        calendar.pick(date(14));
        assert_eq!(calendar.selection(), (date(10), date(14)));
        calendar.pick(date(3));
        assert_eq!(calendar.end, None);
        assert_eq!(weekday_after(Weekday::Sun, 1), Weekday::Mon);
    }
}
