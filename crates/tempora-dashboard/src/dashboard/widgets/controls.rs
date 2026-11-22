use eframe::egui::{
    self, Align2, Color32, FontId, Frame, Key, Margin, Modifiers, Rect, Response, Sense, Stroke,
    StrokeKind, TextEdit, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icons;

use super::{Tokens, focus_ring, glyph};

#[derive(Clone, Copy, PartialEq)]
pub(in crate::dashboard) enum ButtonKind {
    Secondary,
    Ghost,
    Primary,
    Danger,
    Icon,
    GhostIcon,
}

impl ButtonKind {
    fn icon_only(self) -> bool {
        matches!(self, Self::Icon | Self::GhostIcon)
    }
}

#[derive(Clone, Copy)]
pub(in crate::dashboard) enum ButtonSize {
    Regular,
    Small,
    /// Inside a `button_group`, sized like a segmented control's options.
    Segment,
}

impl ButtonSize {
    pub(super) fn height(self) -> f32 {
        match self {
            Self::Regular => Tokens::CONTROL_HEIGHT,
            Self::Small => Tokens::SMALL_HEIGHT,
            Self::Segment => SEGMENT_HEIGHT,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::dashboard) struct Button<'a> {
    pub label: &'a str,
    pub kind: ButtonKind,
    pub size: ButtonSize,
    pub icon: Option<&'static str>,
    pub trailing_icon: Option<&'static str>,
    pub min_width: f32,
    /// Left-aligns the contents; a trailing icon then sits at the right edge.
    pub align_left: bool,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str, kind: ButtonKind) -> Self {
        Self {
            label,
            kind,
            size: ButtonSize::Regular,
            icon: None,
            trailing_icon: None,
            min_width: 0.0,
            align_left: false,
        }
    }

    pub fn icon(label: &'a str, icon: &'static str) -> Self {
        Self {
            icon: Some(icon),
            ..Self::new(label, ButtonKind::Icon)
        }
    }

    pub fn small(self) -> Self {
        Self {
            size: ButtonSize::Small,
            ..self
        }
    }

    /// Sized and styled for a `button_group`.
    pub fn segment(self) -> Self {
        Self {
            size: ButtonSize::Segment,
            kind: ButtonKind::Ghost,
            ..self
        }
    }

    pub fn with_icon(self, icon: &'static str) -> Self {
        Self {
            icon: Some(icon),
            ..self
        }
    }

    pub fn trailing(self, icon: &'static str) -> Self {
        Self {
            trailing_icon: Some(icon),
            ..self
        }
    }

    pub fn width(&self, ui: &Ui) -> f32 {
        if self.kind.icon_only() {
            self.size.height()
        } else {
            let text = control_text(ui, self.label);
            (text.size().x
                + icon_width(self.icon)
                + icon_width(self.trailing_icon)
                + 2.0 * Tokens::CONTROL_PADDING)
                .max(self.min_width)
        }
    }
}

impl egui::Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        control(
            ui,
            self.label,
            Control {
                kind: self.kind,
                icon: self.icon,
                trailing_icon: self.trailing_icon,
                height: self.size.height(),
                min_width: self.min_width,
                align_left: self.align_left,
                ..Control::default()
            },
        )
    }
}

struct Control {
    kind: ButtonKind,
    height: f32,
    min_width: f32,
    padding: f32,
    radius: u8,
    icon: Option<&'static str>,
    trailing_icon: Option<&'static str>,
    shortcut: Option<&'static str>,
    selected: Option<bool>,
    selected_shadow: bool,
    outlined: bool,
    text_color: Option<Color32>,
    align_left: bool,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            kind: ButtonKind::Ghost,
            height: Tokens::CONTROL_HEIGHT,
            min_width: 0.0,
            padding: Tokens::CONTROL_PADDING,
            radius: Tokens::CONTROL_RADIUS,
            icon: None,
            trailing_icon: None,
            shortcut: None,
            selected: None,
            selected_shadow: false,
            outlined: false,
            text_color: None,
            align_left: false,
        }
    }
}

fn control_text(ui: &Ui, label: &str) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(
        label.into(),
        Tokens::medium(Tokens::SECONDARY),
        Color32::PLACEHOLDER,
    )
}

fn icon_width(icon: Option<&str>) -> f32 {
    icon.map_or(0.0, |_| Tokens::ICON_SIZE + Tokens::CONTROL_GAP)
}

fn icon_width_trailing(icon: Option<&str>) -> f32 {
    icon.map_or(0.0, |_| Tokens::CONTROL_GAP + Tokens::ICON_SIZE)
}

fn control(ui: &mut Ui, label: &str, spec: Control) -> Response {
    let colors = Tokens::get(ui);
    let icon_only = spec.kind.icon_only();
    let text = (!icon_only).then(|| control_text(ui, label));
    let text_width = text.as_ref().map_or(0.0, |text| text.size().x);
    let shortcut = spec.shortcut.map(|text| InlineKey::new(ui, text));
    let shortcut_width = shortcut
        .as_ref()
        .map_or(0.0, |key| key.size.x + Tokens::CONTROL_GAP);
    let icon_width = icon_width(spec.icon);
    let trailing_width = icon_width_trailing(spec.trailing_icon);
    let contents = text_width + icon_width + shortcut_width + trailing_width;
    let width = if icon_only {
        spec.height
    } else {
        (contents + spec.padding * 2.0).max(spec.min_width)
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, spec.height), Sense::click());
    // Whole physical pixels on every edge, so fills and borders anti-alias alike and
    // neighbouring controls look the same height at fractional display scales.
    let rect = super::snap(ui, rect);
    response.widget_info(|| match spec.selected {
        Some(selected) => WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        ),
        None => WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label),
    });

    let active = response.hovered() || response.is_pointer_button_down_on();
    let mut foreground = spec.text_color.unwrap_or(colors.fg);
    let (fill, border) = match spec.kind {
        ButtonKind::Primary => {
            foreground = colors.primary_fg;
            let fill = if active {
                colors.primary.gamma_multiply(0.88)
            } else {
                colors.primary
            };
            (fill, fill)
        }
        ButtonKind::Danger => {
            foreground = Color32::WHITE;
            let fill = if active {
                Tokens::DANGER_HOVER
            } else {
                Tokens::DANGER
            };
            (fill, fill)
        }
        _ if spec.selected == Some(true) && spec.selected_shadow => {
            let border = if spec.outlined {
                colors.border
            } else {
                Color32::TRANSPARENT
            };
            (colors.card, border)
        }
        _ if spec.selected == Some(true) => (colors.muted, colors.fg_subtle),
        // An unselected option inside a muted group: `hover` would match the group's fill.
        _ if spec.selected_shadow && active => {
            foreground = colors.fg;
            (colors.group_hover, Color32::TRANSPARENT)
        }
        ButtonKind::Secondary | ButtonKind::Icon => (
            if active {
                colors.control_hover
            } else {
                colors.control
            },
            colors.border,
        ),
        ButtonKind::Ghost | ButtonKind::GhostIcon => {
            let fill = if active {
                colors.hover
            } else {
                Color32::TRANSPARENT
            };
            let border = if spec.outlined {
                colors.border
            } else {
                Color32::TRANSPARENT
            };
            (fill, border)
        }
    };

    let painter = ui.painter();
    if spec.selected_shadow && spec.selected == Some(true) {
        painter.add(colors.raised.as_shape(rect, spec.radius));
    }
    painter.rect(
        rect,
        spec.radius,
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let mut x = if spec.align_left {
        rect.left() + spec.padding
    } else {
        rect.center().x - contents / 2.0
    };
    if let Some(icon) = spec.icon {
        let center_x = if icon_only {
            rect.center().x
        } else {
            x + Tokens::ICON_SIZE / 2.0
        };
        painter.text(
            pos2(center_x, rect.center().y),
            Align2::CENTER_CENTER,
            icon,
            Tokens::icon(Tokens::ICON_SIZE),
            foreground,
        );
        x += icon_width;
    }
    if let Some(text) = text {
        painter.galley(
            pos2(x, rect.center().y - text.size().y / 2.0),
            text,
            foreground,
        );
    }
    if let Some(key) = shortcut {
        // Left-aligned contents put the shortcut at the right edge, like a search field.
        let key_x = if spec.align_left {
            rect.right() - spec.padding - key.size.x
        } else {
            x + text_width + Tokens::CONTROL_GAP
        };
        let key_rect =
            Rect::from_min_size(pos2(key_x, rect.center().y - key.size.y / 2.0), key.size);
        key.paint(ui, key_rect);
    }
    if let Some(icon) = spec.trailing_icon {
        let center_x = if spec.align_left {
            rect.right() - spec.padding - Tokens::ICON_SIZE / 2.0
        } else {
            x + text_width + shortcut_width + trailing_width - Tokens::ICON_SIZE / 2.0
        };
        painter.text(
            pos2(center_x, rect.center().y),
            Align2::CENTER_CENTER,
            icon,
            Tokens::icon(Tokens::ICON_SIZE - 2.0),
            foreground,
        );
    }
    if spec.selected_shadow {
        // Packed 2 px from its neighbours and 4 px from the track's edge: a ring outside
        // would run under the next option's hover fill, so it goes inside the chip.
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                spec.radius,
                Stroke::new(2.0, colors.focus),
                StrokeKind::Inside,
            );
        }
    } else {
        focus_ring(ui, &response, spec.radius);
    }
    if icon_only {
        response.on_hover_text(label)
    } else {
        response
    }
}

pub(in crate::dashboard) fn settings_button(ui: &mut Ui, active: bool) -> Response {
    control(
        ui,
        "Settings",
        Control {
            kind: ButtonKind::Icon,
            icon: Some(icons::SLIDERS_HORIZONTAL),
            height: Tokens::SMALL_HEIGHT,
            radius: (Tokens::SMALL_HEIGHT / 2.0) as u8,
            selected: Some(active),
            ..Control::default()
        },
    )
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::dashboard) enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Caption button size: as tall as the other title bar controls.
pub(in crate::dashboard) const WINDOW_BUTTON: Vec2 = vec2(40.0, Tokens::SMALL_HEIGHT);
/// Space between caption buttons.
pub(in crate::dashboard) const WINDOW_BUTTON_GAP: f32 = 4.0;

/// A caption button of the custom title bar, styled like the bar's other icon buttons:
/// a Phosphor icon, a rounded hover background, red for close. `native` is the hover and
/// pressed state when Windows tracks the pointer instead of egui (the maximize button).
pub(in crate::dashboard) fn window_button(
    ui: &mut Ui,
    kind: WindowButton,
    native: Option<(bool, bool)>,
) -> Response {
    let colors = Tokens::get(ui);
    let (label, icon) = match kind {
        WindowButton::Minimize => ("Minimize", icons::MINUS),
        WindowButton::Maximize => ("Maximize", icons::SQUARE),
        WindowButton::Restore => ("Restore", icons::COPY),
        WindowButton::Close => ("Close", icons::X),
    };
    let (rect, response) = ui.allocate_exact_size(WINDOW_BUTTON, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    let close = kind == WindowButton::Close;
    let (hovered, pressed) = native.unwrap_or_else(|| {
        let pressed = response.is_pointer_button_down_on();
        (response.hovered() || pressed, pressed)
    });
    let radius = (WINDOW_BUTTON.y / 2.0) as u8;
    if hovered {
        let fill = match (close, pressed) {
            (true, false) => Tokens::DANGER,
            (true, true) => Tokens::DANGER_HOVER,
            (false, _) => colors.hover,
        };
        ui.painter().rect_filled(rect, radius, fill);
    }
    let color = match (hovered, close) {
        (true, true) => Color32::WHITE,
        (true, false) => colors.fg,
        (false, _) => colors.fg_muted,
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        Tokens::icon(Tokens::ICON_SIZE),
        color,
    );
    focus_ring(ui, &response, radius);
    if native.is_some() {
        response
    } else {
        response.on_hover_text(label)
    }
}

/// Width of the top bar's search pill, typing or not.
pub(in crate::dashboard) const SEARCH_WIDTH: f32 = 200.0;

pub(in crate::dashboard) fn search_pill(ui: &mut Ui) -> Response {
    control(
        ui,
        "Search",
        Control {
            kind: ButtonKind::Secondary,
            icon: Some(icons::MAGNIFYING_GLASS),
            height: Tokens::SMALL_HEIGHT,
            padding: 10.0,
            radius: (Tokens::SMALL_HEIGHT / 2.0) as u8,
            shortcut: Some("Ctrl K"),
            text_color: Some(Tokens::get(ui).fg_subtle),
            // Reads as a field rather than a button.
            min_width: SEARCH_WIDTH,
            align_left: true,
            ..Control::default()
        },
    )
}

pub(in crate::dashboard) fn nav(
    ui: &mut Ui,
    items: &[(&str, &'static str, bool)],
) -> Option<usize> {
    let colors = Tokens::get(ui);
    let radius = (Tokens::SMALL_HEIGHT / 2.0) as u8;
    group_frame(colors, 4, radius + 5)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().interact_size.y = Tokens::SMALL_HEIGHT;
                let mut clicked = None;
                for (index, &(label, icon, active)) in items.iter().enumerate() {
                    let response = control(
                        ui,
                        label,
                        Control {
                            icon: Some(icon),
                            height: Tokens::SMALL_HEIGHT,
                            radius,
                            selected: Some(active),
                            selected_shadow: true,
                            // Light mode lifts the selected tab with its shadow alone.
                            outlined: active && ui.visuals().dark_mode,
                            text_color: Some(if active { colors.fg } else { colors.fg_muted }),
                            ..Control::default()
                        },
                    );
                    if response.clicked() {
                        clicked = Some(index);
                    }
                }
                clicked
            })
            .inner
        })
        .inner
}

pub(in crate::dashboard) fn nav_width(ui: &Ui, items: &[(&str, &'static str, bool)]) -> f32 {
    let content_width = items
        .iter()
        .map(|(label, _, _)| {
            let text = ui.painter().layout_no_wrap(
                (*label).into(),
                Tokens::medium(Tokens::SECONDARY),
                Color32::PLACEHOLDER,
            );
            text.size().x + Tokens::ICON_SIZE + Tokens::CONTROL_GAP + 2.0 * Tokens::CONTROL_PADDING
        })
        .sum::<f32>();
    (content_width + 10.0 + 2.0 * items.len().saturating_sub(1) as f32).ceil()
}

pub(in crate::dashboard) fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let colors = Tokens::get(ui);
    let mut changed = false;
    // Options always read left to right, even inside a right-aligned parent, and the
    // control takes only the width its options need.
    let width = options
        .iter()
        .map(|(_, label)| control_text(ui, label).size().x + 24.0)
        .sum::<f32>()
        + 2.0 * options.len().saturating_sub(1) as f32;
    group_frame(colors, 3, Tokens::POPOVER_RADIUS).show(ui, |ui| {
        ui.allocate_ui_with_layout(
            vec2(width.ceil(), 28.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().interact_size.y = 28.0;
                let mut responses = Vec::with_capacity(options.len());
                let mut next = None;
                // Only arrow keys move focus along: a click selects without leaving a
                // keyboard focus ring behind.
                let mut by_key = false;
                for (index, &(option, label)) in options.iter().enumerate() {
                    let selected = *value == option;
                    let response = control(
                        ui,
                        label,
                        Control {
                            height: 28.0,
                            padding: 12.0,
                            radius: Tokens::SEGMENT_RADIUS,
                            selected: Some(selected),
                            selected_shadow: true,
                            text_color: Some(if selected { colors.fg } else { colors.fg_muted }),
                            ..Control::default()
                        },
                    );
                    if response.clicked() {
                        next = Some(index);
                    } else if response.has_focus() {
                        next = ui.input_mut(|input| {
                            if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                                Some((index + options.len() - 1) % options.len())
                            } else if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
                                Some((index + 1) % options.len())
                            } else {
                                None
                            }
                        });
                        by_key = next.is_some();
                    }
                    responses.push(response);
                }
                if let Some(index) = next {
                    let selected = options[index].0;
                    changed = *value != selected;
                    *value = selected;
                    if by_key {
                        responses[index].request_focus();
                    }
                    if changed {
                        responses[index].mark_changed();
                    }
                }
            },
        );
    });
    changed
}

const SEGMENT_HEIGHT: f32 = 28.0;

/// Related actions joined in one frame, like a segmented control: give each button
/// `Button::segment`. Its height matches a regular button.
pub(in crate::dashboard) fn button_group<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let colors = Tokens::get(ui);
    group_frame(colors, 3, Tokens::POPOVER_RADIUS)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().interact_size.y = SEGMENT_HEIGHT;
            content(ui)
        })
        .inner
}

/// A muted track without a border, like shadcn's tabs list. The extra pixel of margin
/// stands in for the border it used to have, so group sizes stay the same.
fn group_frame(colors: &Tokens, padding: i8, radius: u8) -> Frame {
    Frame::new()
        .inner_margin(padding + 1)
        .corner_radius(radius)
        .fill(colors.muted)
}

#[cfg_attr(not(debug_assertions), allow(dead_code))] // Shown in the debug-only gallery.
pub(in crate::dashboard) fn chip(ui: &mut Ui, label: &str, selected: bool) -> Response {
    control(
        ui,
        label,
        Control {
            height: Tokens::SMALL_HEIGHT,
            radius: (Tokens::SMALL_HEIGHT / 2.0) as u8,
            outlined: true,
            selected: Some(selected),
            text_color: Some(Tokens::get(ui).fg_muted),
            ..Control::default()
        },
    )
}

pub(in crate::dashboard) fn switch(ui: &mut Ui, label: &str, value: &mut bool) -> Response {
    let colors = Tokens::get(ui);
    let text =
        ui.painter()
            .layout_no_wrap(label.into(), FontId::proportional(Tokens::BODY), colors.fg);
    let (rect, mut response) = ui.allocate_exact_size(
        vec2(36.0 + Tokens::CONTROL_GAP + text.size().x, 20.0),
        Sense::click(),
    );
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *value, label));
    let track = Rect::from_min_size(rect.min, vec2(36.0, 20.0));
    let t = ui.ctx().animate_bool(response.id, *value);
    let fill = colors.muted.lerp_to_gamma(colors.primary, t);
    let knob = colors.fg_subtle.lerp_to_gamma(colors.primary_fg, t);
    ui.painter().rect(
        track,
        10,
        fill,
        Stroke::new(1.0, colors.border),
        StrokeKind::Inside,
    );
    ui.painter().circle_filled(
        pos2(track.left() + 10.0 + 16.0 * t, track.center().y),
        8.0,
        knob,
    );
    ui.painter().galley(
        pos2(
            track.right() + Tokens::CONTROL_GAP,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        colors.fg,
    );
    focus_ring(ui, &response, 10);
    response
}

/// The top bar's search pill while typing. It draws exactly like `search_pill` (size, icon,
/// placeholder and font), so opening search only adds a caret and a brighter border.
pub(in crate::dashboard) fn pill_search_field(
    ui: &mut Ui,
    edit_id: egui::Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    let colors = Tokens::get(ui);
    let height = Tokens::SMALL_HEIGHT;
    let padding = 10.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let rect = super::snap(ui, rect);
    ui.painter().rect(
        rect,
        (height / 2.0) as u8,
        colors.control,
        Stroke::new(1.0, colors.fg_subtle),
        StrokeKind::Inside,
    );
    ui.painter().text(
        pos2(
            rect.left() + padding + Tokens::ICON_SIZE / 2.0,
            rect.center().y,
        ),
        Align2::CENTER_CENTER,
        icons::MAGNIFYING_GLASS,
        Tokens::icon(Tokens::ICON_SIZE),
        colors.fg_subtle,
    );
    let font = Tokens::medium(Tokens::SECONDARY);
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let left = rect.left() + padding + icon_width(Some(icons::MAGNIFYING_GLASS));
    let field = Rect::from_min_max(
        pos2(left, rect.center().y - line / 2.0),
        pos2(rect.right() - padding, rect.center().y + line / 2.0),
    );
    let mut edit = ui.put(
        field,
        TextEdit::singleline(text)
            .id(edit_id)
            .font(font.clone())
            .text_color(colors.fg)
            // The hint takes its own font, or egui draws it in the larger body style.
            .hint_text(
                egui::RichText::new(hint)
                    .font(font.clone())
                    .color(colors.fg_subtle),
            )
            .margin(Margin::ZERO)
            .frame(egui::Frame::NONE)
            .desired_width(field.width()),
    );
    // Callers anchor the results to the whole pill.
    edit.rect = rect;
    edit
}

pub(in crate::dashboard) fn search_field(
    ui: &mut Ui,
    edit_id: egui::Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    let colors = Tokens::get(ui);
    let frame = Frame::new()
        .inner_margin(Margin::symmetric(10, 0))
        .stroke(Stroke::new(1.0, colors.border))
        .corner_radius(Tokens::CONTROL_RADIUS)
        .fill(colors.control)
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                vec2(width, Tokens::CONTROL_HEIGHT - 2.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    glyph(
                        ui,
                        icons::MAGNIFYING_GLASS,
                        Tokens::ICON_SIZE,
                        colors.fg_subtle,
                    );
                    ui.add(
                        TextEdit::singleline(text)
                            .id(edit_id)
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
        let mut field = edit.clone();
        field.rect = frame.response.rect;
        focus_ring(ui, &field, Tokens::CONTROL_RADIUS);
    }
    edit
}

struct InlineKey {
    text: std::sync::Arc<egui::Galley>,
    size: Vec2,
}

impl InlineKey {
    fn new(ui: &Ui, label: &str) -> Self {
        let text = ui.painter().layout_no_wrap(
            label.into(),
            Tokens::mono(Tokens::MICRO),
            Color32::PLACEHOLDER,
        );
        let size = text.size() + vec2(10.0, 4.0);
        Self { text, size }
    }

    fn paint(&self, ui: &Ui, rect: Rect) {
        let colors = Tokens::get(ui);
        ui.painter()
            .rect_filled(rect, Tokens::TAG_RADIUS, colors.muted);
        ui.painter().galley(
            rect.center() - self.text.size() / 2.0,
            self.text.clone(),
            colors.fg_subtle,
        );
    }
}

#[cfg_attr(not(debug_assertions), allow(dead_code))] // Shown in the debug-only gallery.
pub(in crate::dashboard) fn kbd(ui: &mut Ui, label: &str) {
    let key = InlineKey::new(ui, label);
    let (rect, response) = ui.allocate_exact_size(key.size, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), label));
    key.paint(ui, rect);
}

/// Small removable filter chip ("Games ×").
pub(in crate::dashboard) fn filter_chip(ui: &mut Ui, label: &str) -> Response {
    control(
        ui,
        label,
        Control {
            height: FILTER_CHIP_HEIGHT,
            padding: 10.0,
            radius: 13,
            outlined: true,
            trailing_icon: Some(icons::X),
            ..Control::default()
        },
    )
    .on_hover_text(format!("Clear the {label} filter"))
}

/// Chip naming the highlighted app; clicking it stops the highlight.
pub(in crate::dashboard) fn highlight_chip(ui: &mut Ui, name: &str) -> Response {
    control(
        ui,
        name,
        Control {
            height: FILTER_CHIP_HEIGHT,
            padding: 10.0,
            radius: 13,
            outlined: true,
            trailing_icon: Some(icons::X),
            ..Control::default()
        },
    )
    .on_hover_text(format!("Stop highlighting {name}"))
}

/// Height of filter and highlight chips; headers reserve it so a chip never shifts layout.
pub(in crate::dashboard) const FILTER_CHIP_HEIGHT: f32 = 26.0;

/// Full-width selectable row used for presets and section navigation.
pub(in crate::dashboard) fn list_item(
    ui: &mut Ui,
    label: &str,
    icon: Option<&'static str>,
    selected: bool,
) -> Response {
    let colors = Tokens::get(ui);
    control(
        ui,
        label,
        Control {
            height: Tokens::SMALL_HEIGHT,
            min_width: ui.available_width(),
            padding: 10.0,
            radius: Tokens::TAG_RADIUS,
            align_left: true,
            icon,
            selected: Some(selected),
            text_color: Some(if selected { colors.fg } else { colors.fg_muted }),
            ..Control::default()
        },
    )
}

/// Menu item with a keyboard shortcut hint on the right.
pub(in crate::dashboard) fn menu_item_with_key(
    ui: &mut Ui,
    label: &str,
    icon: &'static str,
    key: &'static str,
) -> Response {
    let colors = Tokens::get(ui);
    control(
        ui,
        label,
        Control {
            height: 34.0,
            min_width: ui.available_width(),
            padding: Tokens::CONTROL_GAP,
            align_left: true,
            icon: Some(icon),
            shortcut: Some(key),
            text_color: Some(colors.fg),
            ..Control::default()
        },
    )
}

/// Selectable menu row with a category color swatch in the icon slot.
pub(in crate::dashboard) fn swatch_item(
    ui: &mut Ui,
    label: &str,
    color: Color32,
    selected: bool,
) -> Response {
    let colors = Tokens::get(ui);
    let response = control(
        ui,
        label,
        Control {
            height: Tokens::SMALL_HEIGHT,
            min_width: ui.available_width(),
            padding: 10.0,
            radius: Tokens::TAG_RADIUS,
            align_left: true,
            // Reserves the icon slot; the swatch is painted into it.
            icon: Some(""),
            selected: Some(selected),
            text_color: Some(if selected { colors.fg } else { colors.fg_muted }),
            ..Control::default()
        },
    );
    let swatch = Rect::from_center_size(
        pos2(
            response.rect.left() + 10.0 + Tokens::ICON_SIZE / 2.0,
            response.rect.center().y,
        ),
        vec2(10.0, 10.0),
    );
    ui.painter().rect_filled(super::snap(ui, swatch), 3, color);
    response
}

pub(in crate::dashboard) fn menu_item(
    ui: &mut Ui,
    label: &str,
    icon: &'static str,
    destructive: bool,
) -> Response {
    let colors = Tokens::get(ui);
    control(
        ui,
        label,
        Control {
            height: 34.0,
            min_width: ui.available_width(),
            padding: Tokens::CONTROL_GAP,
            align_left: true,
            icon: Some(icon),
            text_color: Some(if destructive { colors.bad } else { colors.fg }),
            ..Control::default()
        },
    )
}
