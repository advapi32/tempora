use eframe::egui::{
    self, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText,
    Stroke, Ui, emath::GuiRounding, epaint::FontColorTransferFunction, style::ScrollStyle,
};
use tempora::model::Category;

const fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

pub(super) struct Tokens {
    pub bg: Color32,
    pub card: Color32,
    pub popover: Color32,
    pub border: Color32,
    pub muted: Color32,
    pub hover: Color32,
    /// Fill of outlined controls (secondary buttons, inputs). In dark mode a faint white
    /// overlay, so a control reads slightly lighter than the surface it sits on.
    pub control: Color32,
    pub control_hover: Color32,
    pub fg: Color32,
    pub fg_muted: Color32,
    pub fg_subtle: Color32,
    pub primary: Color32,
    pub primary_fg: Color32,
    pub focus: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub chart_grid: Color32,
    pub gap_hatch: Color32,
    pub scroll_thumb: Color32,
    pub scroll_thumb_hover: Color32,
    pub shadow: egui::Shadow,
}

const DARK: Tokens = Tokens {
    bg: rgb(0x09090b),
    card: rgb(0x0f0f11),
    popover: rgb(0x141417),
    border: rgb(0x27272a),
    muted: rgb(0x1a1a1d),
    hover: rgb(0x1c1c20),
    control: Color32::from_rgba_premultiplied(12, 12, 12, 12),
    control_hover: Color32::from_rgba_premultiplied(20, 20, 20, 20),
    fg: rgb(0xfafafa),
    fg_muted: rgb(0xa1a1aa),
    fg_subtle: rgb(0x8d8d96),
    primary: rgb(0xfafafa),
    primary_fg: rgb(0x18181b),
    focus: rgb(0x71717a),
    ok: rgb(0x22c55e),
    warn: rgb(0xf59e0b),
    bad: rgb(0xf87171),
    chart_grid: rgb(0x1d1d21),
    gap_hatch: rgb(0x45454d),
    scroll_thumb: rgb(0x3f3f46),
    scroll_thumb_hover: rgb(0x52525b),
    shadow: shadow(128),
};

const LIGHT: Tokens = Tokens {
    bg: rgb(0xfafafa),
    card: rgb(0xffffff),
    popover: rgb(0xffffff),
    border: rgb(0xe4e4e7),
    muted: rgb(0xf4f4f5),
    hover: rgb(0xf4f4f5),
    control: rgb(0xffffff),
    control_hover: rgb(0xf4f4f5),
    fg: rgb(0x09090b),
    fg_muted: rgb(0x52525b),
    fg_subtle: rgb(0x6b6b74),
    primary: rgb(0x18181b),
    primary_fg: rgb(0xfafafa),
    focus: rgb(0xa1a1aa),
    ok: rgb(0x16a34a),
    warn: rgb(0xb45309),
    bad: rgb(0xdc2626),
    chart_grid: rgb(0xefeff1),
    gap_hatch: rgb(0xd4d4d8),
    scroll_thumb: rgb(0xd4d4d8),
    scroll_thumb_hover: rgb(0xa1a1aa),
    shadow: shadow(31),
};

const fn shadow(alpha: u8) -> egui::Shadow {
    egui::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    }
}

impl Tokens {
    pub const WINDOW_MIN: [f32; 2] = [1100.0, 720.0];
    pub const CONTENT_MAX: f32 = 1320.0;
    pub const WIDE_CONTENT: f32 = 1280.0;
    pub const PAGE_PADDING: i8 = 32;
    pub const CARD_PADDING: i8 = 16;
    pub const GRID_GAP: f32 = 16.0;
    pub const CONTROL_GAP: f32 = 8.0;
    pub const CONTROL_HEIGHT: f32 = 36.0;
    pub const SMALL_HEIGHT: f32 = 32.0;
    pub const CONTROL_PADDING: f32 = 14.0;
    pub const CONTROL_RADIUS: u8 = 8;
    pub const CARD_RADIUS: u8 = 12;
    pub const DIALOG_RADIUS: u8 = 14;
    pub const POPOVER_RADIUS: u8 = 10;
    pub const TAG_RADIUS: u8 = 6;
    pub const SEGMENT_RADIUS: u8 = 7;
    pub const ICON_SIZE: f32 = 16.0;
    pub const BODY: f32 = 14.0;
    pub const SECONDARY: f32 = 13.0;
    pub const CAPTION: f32 = 12.0;
    pub const MICRO: f32 = 11.0;
    pub const SECTION: f32 = 15.0;
    pub const DIALOG_TITLE: f32 = 17.0;
    pub const PAGE_TITLE: f32 = 26.0;
    pub const KPI: f32 = 28.0;
    pub const DANGER: Color32 = rgb(0xdc2626);
    pub const DANGER_HOVER: Color32 = rgb(0xb91c1c);
    /// The Day page's colors for the five apps with the most focused time, by rank.
    pub const APP_PALETTE: [Color32; 5] = [
        rgb(0x3b82f6),
        rgb(0xf97316),
        rgb(0x8b5cf6),
        rgb(0xec4899),
        rgb(0x14b8a6),
    ];
    /// Every other app on the Day page.
    pub const APP_OTHER: Color32 = rgb(0x71717a);

    pub fn get(ui: &Ui) -> &'static Self {
        Self::for_dark(ui.visuals().dark_mode)
    }

    pub fn for_dark(dark: bool) -> &'static Self {
        if dark { &DARK } else { &LIGHT }
    }

    pub fn category(&self, category: Category) -> Color32 {
        match category {
            Category::Games => rgb(0x3b82f6),
            Category::Browsers => rgb(0xf97316),
            Category::Productivity => rgb(0x8b5cf6),
            Category::Communication => rgb(0xec4899),
            Category::Media => rgb(0x14b8a6),
            Category::Other => rgb(0x71717a),
        }
    }

    pub fn medium(size: f32) -> FontId {
        FontId::new(size, named("Geist Medium"))
    }

    pub fn semibold(size: f32) -> FontId {
        FontId::new(size, named("Geist Semibold"))
    }

    pub fn mono(size: f32) -> FontId {
        FontId::monospace(size)
    }

    pub fn mono_medium(size: f32) -> FontId {
        FontId::new(size, named("Geist Mono Medium"))
    }

    pub fn icon(size: f32) -> FontId {
        FontId::new(size, named("Phosphor"))
    }

    pub fn filled_icon(size: f32) -> FontId {
        FontId::new(size, named("Phosphor Fill"))
    }
}

fn named(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

pub(super) fn snap<T: GuiRounding>(ui: &Ui, geometry: T) -> T {
    geometry.round_to_pixels(ui.ctx().pixels_per_point())
}

pub(super) fn setup(ctx: &Context) {
    // Set the startup options before the first layout so restored keyboard zoom cannot
    // rescale the first frame's viewport rectangle.
    ctx.options_mut(|options| {
        options.zoom_factor = 1.0;
        options.zoom_with_keyboard = false;
    });
    ctx.tessellation_options_mut(|options| {
        options.round_text_to_pixels = true;
        options.round_rects_to_pixels = true;
    });
    let mut fonts = FontDefinitions::default();
    for (name, family, fallback, bytes) in [
        (
            "Geist",
            FontFamily::Proportional,
            FontFamily::Proportional,
            include_bytes!("fonts/Geist-Regular.ttf").as_slice(),
        ),
        (
            "Geist Mono",
            FontFamily::Monospace,
            FontFamily::Monospace,
            include_bytes!("fonts/GeistMono-Regular.ttf").as_slice(),
        ),
        (
            "Geist Medium",
            named("Geist Medium"),
            FontFamily::Proportional,
            include_bytes!("fonts/Geist-Medium.ttf").as_slice(),
        ),
        (
            "Geist Semibold",
            named("Geist Semibold"),
            FontFamily::Proportional,
            include_bytes!("fonts/Geist-SemiBold.ttf").as_slice(),
        ),
        (
            "Geist Mono Medium",
            named("Geist Mono Medium"),
            FontFamily::Monospace,
            include_bytes!("fonts/GeistMono-Medium.ttf").as_slice(),
        ),
        (
            "Geist Mono Semibold",
            named("Geist Mono Semibold"),
            FontFamily::Monospace,
            include_bytes!("fonts/GeistMono-SemiBold.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(bytes).into());
        let fallbacks = fonts.families[&fallback].iter().cloned();
        let family_fonts = std::iter::once(name.into()).chain(fallbacks).collect();
        fonts.families.insert(family, family_fonts);
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    fonts
        .families
        .insert(named("Phosphor"), vec!["phosphor".into()]);
    fonts.font_data.insert(
        "Phosphor Fill".into(),
        egui_phosphor::Variant::Fill.font_data().into(),
    );
    fonts
        .families
        .insert(named("Phosphor Fill"), vec!["Phosphor Fill".into()]);
    ctx.set_fonts(fonts);

    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let colors = Tokens::for_dark(theme == egui::Theme::Dark);
        ctx.style_mut_of(theme, |style| {
            style.text_styles = [
                (egui::TextStyle::Heading, Tokens::semibold(Tokens::SECTION)),
                (egui::TextStyle::Body, FontId::proportional(Tokens::BODY)),
                (egui::TextStyle::Button, Tokens::medium(Tokens::SECONDARY)),
                (
                    egui::TextStyle::Small,
                    FontId::proportional(Tokens::CAPTION),
                ),
                (egui::TextStyle::Monospace, Tokens::mono(Tokens::SECONDARY)),
            ]
            .into();
            style.spacing.item_spacing = egui::vec2(Tokens::CONTROL_GAP, 12.0);
            style.spacing.interact_size =
                egui::vec2(Tokens::CONTROL_HEIGHT, Tokens::CONTROL_HEIGHT);
            style.spacing.button_padding = egui::vec2(Tokens::CONTROL_PADDING, 0.0);
            style.spacing.scroll = ScrollStyle {
                bar_width: 6.0,
                bar_inner_margin: 8.0,
                handle_min_length: 24.0,
                ..ScrollStyle::solid()
            };
            style.visuals = visuals(theme, colors);
        });
    }
}

fn visuals(theme: egui::Theme, colors: &Tokens) -> egui::Visuals {
    let mut visuals = theme.default_visuals();
    // Hinted glyphs on whole pixels. Sub-pixel binning evens out kerning but blurs stems,
    // so it stays off. The transfer curves are egui's per-theme defaults.
    visuals.text_options.font_hinting = true;
    visuals.text_options.subpixel_binning = false;
    visuals.text_options.color_transfer_function = match theme {
        egui::Theme::Light => FontColorTransferFunction::LIGHT_MODE_DEFAULT,
        egui::Theme::Dark => FontColorTransferFunction::DARK_MODE_DEFAULT,
    };
    visuals.panel_fill = colors.bg;
    visuals.window_fill = colors.popover;
    visuals.window_stroke = Stroke::new(1.0, colors.border);
    visuals.window_corner_radius = Tokens::DIALOG_RADIUS.into();
    visuals.menu_corner_radius = Tokens::POPOVER_RADIUS.into();
    visuals.window_shadow = colors.shadow;
    visuals.popup_shadow = colors.shadow;
    visuals.extreme_bg_color = colors.muted;
    visuals.faint_bg_color = colors.muted;
    visuals.code_bg_color = colors.muted;
    visuals.warn_fg_color = colors.warn;
    visuals.error_fg_color = colors.bad;
    visuals.selection.bg_fill = colors.muted;
    visuals.selection.stroke = Stroke::new(1.0, colors.focus);
    visuals.widgets.noninteractive.fg_stroke.color = colors.fg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.border);
    for (state, fill, border) in [
        (&mut visuals.widgets.inactive, colors.card, colors.border),
        (&mut visuals.widgets.hovered, colors.hover, colors.border),
        (&mut visuals.widgets.active, colors.muted, colors.focus),
        (&mut visuals.widgets.open, colors.muted, colors.focus),
    ] {
        state.corner_radius = CornerRadius::same(Tokens::CONTROL_RADIUS);
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, border);
        state.fg_stroke.color = colors.fg;
        state.expansion = 0.0;
    }
    visuals
}

pub(super) fn display(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(Tokens::semibold(size))
}

pub(super) fn heading(ui: &mut Ui, title: &str) {
    ui.add(egui::Label::new(display(title, Tokens::PAGE_TITLE)).truncate())
        .on_hover_text(title);
}

pub(super) fn section(ui: &mut Ui, title: &str) {
    ui.label(display(title, Tokens::SECTION));
}
