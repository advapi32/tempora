use eframe::egui::{self, Align, Align2, Context, Layout, Rect, RichText, Sense, Theme, Ui, vec2};
use egui_phosphor::regular as icons;
use tempora::model::Category;

use super::theme::{self, Tokens};
use super::widgets::{
    self, BannerKind, Button, ButtonKind, ButtonSize, StackedBar as ChartBar, StatusKind,
};

pub(crate) struct Gallery {
    samples: [Samples; 2],
}

struct Samples {
    nav: usize,
    period: usize,
    games: bool,
    apps: bool,
    idle_off: bool,
    idle_on: bool,
    search: String,
    checked: bool,
    mode: usize,
    name: String,
    chip: usize,
}

impl Default for Samples {
    fn default() -> Self {
        Self {
            nav: 0,
            period: 0,
            games: true,
            apps: false,
            idle_off: false,
            idle_on: true,
            search: String::new(),
            checked: true,
            mode: 0,
            name: String::new(),
            chip: 0,
        }
    }
}

impl Gallery {
    pub fn new(ctx: &Context) -> Self {
        theme::setup(ctx);
        Self {
            samples: std::array::from_fn(|_| Samples::default()),
        }
    }

    fn draw(&mut self, root: &mut Ui) {
        egui::CentralPanel::default().show(root, |ui| {
            theme::heading(ui, "Component gallery");
            let area = egui::ScrollArea::vertical().auto_shrink([false, false]);
            widgets::scroll(ui, area, |ui| {
                let themes = [Theme::Light, Theme::Dark];
                if ui.available_width() >= 980.0 {
                    ui.spacing_mut().item_spacing.x = Tokens::GRID_GAP;
                    ui.columns(2, |columns| {
                        for (index, theme) in themes.into_iter().enumerate() {
                            self.show_theme(&mut columns[index], theme, index);
                        }
                    });
                } else {
                    for (index, theme) in themes.into_iter().enumerate() {
                        self.show_theme(ui, theme, index);
                    }
                }
            });
        });
    }

    fn show_theme(&mut self, ui: &mut Ui, theme: Theme, index: usize) {
        let style = ui.ctx().style_of(theme);
        ui.push_id(theme, |ui| {
            ui.set_style(style);
            let colors = Tokens::get(ui);
            egui::Frame::new()
                .fill(colors.bg)
                .inner_margin(Tokens::CARD_PADDING)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    theme::section(
                        ui,
                        if theme == Theme::Dark {
                            "Dark"
                        } else {
                            "Light"
                        },
                    );
                    self.samples[index].show(ui);
                });
        });
    }
}

impl eframe::App for Gallery {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl Samples {
    fn show(&mut self, ui: &mut Ui) {
        self.controls(ui);
        ui.columns(2, |columns| {
            widgets::kpi_card(
                &mut columns[0],
                "Focused time",
                icons::CLOCK,
                "4h 36m",
                Some("+12%"),
                "vs. last week",
            );
            widgets::kpi_card(
                &mut columns[1],
                "Gaming",
                icons::GAME_CONTROLLER,
                "2h 15m",
                None,
                "49% of focused time",
            );
        });
        self.feedback(ui);
        widgets::toast_body(ui, "App updated", false, Some("Undo"));
        self.open_surfaces(ui);
        examples_table(ui);
        chart_examples(ui, self.period != 1);
        self.screen_widgets(ui);
    }

    fn controls(&mut self, ui: &mut Ui) {
        let items = [
            ("Overview", icons::CHART_BAR, self.nav == 0),
            ("Library", icons::SQUARES_FOUR, self.nav == 1),
            ("Timeline", icons::ROWS, self.nav == 2),
        ];
        if let Some(index) = widgets::nav(ui, &items) {
            self.nav = index;
        }
        ui.horizontal_wrapped(|ui| {
            for (kind, label) in [
                (ButtonKind::Secondary, "Secondary"),
                (ButtonKind::Ghost, "Ghost"),
                (ButtonKind::Primary, "Primary"),
                (ButtonKind::Danger, "Danger"),
            ] {
                ui.add(Button::new(label, kind));
            }
            ui.add(Button {
                size: ButtonSize::Small,
                icon: Some(icons::SLIDERS_HORIZONTAL),
                ..Button::new("Settings", ButtonKind::Icon)
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled(false, Button::new("Disabled", ButtonKind::Secondary));
            let focus = ui.add(Button::new("Keyboard focus", ButtonKind::Secondary));
            widgets::paint_focus_ring(ui, focus.rect, Tokens::CONTROL_RADIUS);
            widgets::kbd(ui, "Ctrl K");
        });
        ui.horizontal_wrapped(|ui| {
            widgets::segmented(
                ui,
                &mut self.period,
                &[(0, "Week"), (1, "Month"), (2, "Custom")],
            );
            if widgets::chip(ui, "Games", self.games).clicked() {
                self.games = !self.games;
            }
            if widgets::chip(ui, "Apps", self.apps).clicked() {
                self.apps = !self.apps;
            }
        });
        ui.horizontal_wrapped(|ui| {
            widgets::switch(ui, "Exclude estimated idle", &mut self.idle_off);
            widgets::switch(ui, "Exclude estimated idle", &mut self.idle_on);
        });
        widgets::search_field(
            ui,
            ui.id().with("search"),
            &mut self.search,
            "Search apps",
            210.0,
        );
    }

    fn feedback(&self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            widgets::status_pill(ui, StatusKind::Recording, Some("· 4h 36m today"));
            widgets::status_pill(ui, StatusKind::Paused, None);
            widgets::status_pill(ui, StatusKind::Unreachable, None);
        });
        widgets::banner(
            ui,
            BannerKind::Info,
            "History available",
            "May 1 – May 31",
            &[],
        );
        widgets::banner(
            ui,
            BannerKind::Paused,
            "Recording paused",
            "Nothing is captured until you resume.",
            &[(Button::new("Resume", ButtonKind::Primary), true)],
        );
        widgets::banner(
            ui,
            BannerKind::Error,
            "Can't reach the recorder.",
            "History is available through 14:22. Settings changes are disabled until the recorder connects.",
            &[(Button::new("Retry", ButtonKind::Secondary), true)],
        );
    }

    fn open_surfaces(&self, ui: &mut Ui) {
        // Render each layer in the page so both themes can show their open states at once.
        ui.horizontal_top(|ui| {
            widgets::popover(ui, calendar);
            ui.vertical(|ui| {
                ui.set_width(176.0);
                widgets::menu(ui, |ui| {
                    widgets::menu_item(ui, "Edit app…", icons::PENCIL_SIMPLE, false);
                    widgets::menu_item(ui, "Open sessions", icons::ROWS, false);
                    ui.separator();
                    widgets::menu_item(ui, "Delete history…", icons::TRASH, true);
                });
            });
        });
        widgets::dialog_preview(ui, "Edit app", |ui| {
            ui.label("Rename the app or change its category.");
            widgets::dialog_footer(
                ui,
                Button::new("Save changes", ButtonKind::Primary),
                true,
                "Changes can be undone.",
            );
        });
    }

    /// Specimens of the widgets the screens added, with sample values.
    fn screen_widgets(&mut self, ui: &mut Ui) {
        let colors = Tokens::get(ui);
        theme::section(ui, "Screen widgets");
        widgets::breadcrumb(ui, &["Overview · Week 47", "Thursday"]);
        widgets::kpi_grid(
            ui,
            &[
                widgets::Kpi {
                    caption: Some("vs. same point last week".into()),
                    delta: Some("-8%".into()),
                    ..widgets::Kpi::new("Total focus", icons::TIMER, "6h 12m".into())
                },
                widgets::Kpi {
                    share: Some((0.4, colors.category(Category::Games))),
                    caption: Some("40% of focus".into()),
                    ..widgets::Kpi::new("Play", icons::GAME_CONTROLLER, "2h 30m".into())
                },
            ],
            2,
        );
        ui.horizontal_wrapped(|ui| {
            for (index, category) in [Category::Games, Category::Media].into_iter().enumerate() {
                if widgets::category_chip(
                    ui,
                    category.label(),
                    colors.category(category),
                    Some("1h 05m"),
                    self.chip == index,
                    true,
                )
                .clicked()
                {
                    self.chip = index;
                }
            }
            widgets::filter_chip(ui, "Media");
            widgets::category_tag(ui, Category::Productivity);
            widgets::tone_tag(ui, "Ended by pause", colors.warn);
            widgets::tone_tag(ui, "Edited", colors.fg_muted);
            widgets::live_dot(ui);
        });
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 12.0) / 2.0;
            for (index, title) in ["Record all", "Record selected"].iter().enumerate() {
                if widgets::radio_card(
                    ui,
                    self.mode == index,
                    title,
                    "Sample description of the option.",
                    width,
                )
                .clicked()
                {
                    self.mode = index;
                }
            }
        });
        widgets::checkbox(
            ui,
            &mut self.checked,
            "Sample checkbox",
            Some("Muted second line"),
        );
        let width = ui.available_width();
        widgets::text_field(
            ui,
            ui.id().with("name"),
            &mut self.name,
            "Display name",
            width,
        );
        widgets::setting_row(
            ui,
            "Setting row",
            "Detail text wraps under the title.",
            |ui| {
                ui.add(Button::new("Action", ButtonKind::Secondary));
            },
        );
        widgets::stat_tiles(
            ui,
            &[
                ("Running time", "46m".into()),
                ("Focus", "38m".into()),
                ("Idle", "0m".into()),
                ("Background", "8m".into()),
            ],
            4,
        );
        widgets::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            widgets::section_header(ui, "Section header", Some("Muted subtitle"), |ui| {
                ui.add(Button::new("Ghost", ButtonKind::Ghost).small());
            });
            let mut hours = [0i64; 24];
            for (hour, value) in hours.iter_mut().enumerate().skip(18) {
                *value = (hour as i64 - 17) * 600_000;
            }
            widgets::hour_histogram(ui, &hours, colors.category(Category::Games), "Evening play");
            let bars: Vec<_> = (0..14)
                .map(|day| widgets::PairedBar {
                    label: (day + 1).to_string(),
                    description: format!("Day {}", day + 1),
                    front_ms: (day % 4) * 1_800_000,
                    back_ms: (day % 4) * 2_400_000,
                    today: day == 13,
                })
                .collect();
            widgets::paired_bars(ui, &bars, colors.category(Category::Games), 120.0);
            let blocks = [
                widgets::LaneBlock {
                    start: 0.05,
                    end: 0.35,
                    style: widgets::LaneStyle::Solid(colors.category(Category::Games)),
                    label: Some("Sample game".into()),
                    description: "Focused".into(),
                    tooltip: None,
                    dimmed: false,
                },
                widgets::LaneBlock {
                    start: 0.5,
                    end: 0.7,
                    style: widgets::LaneStyle::Gap,
                    label: Some("Paused".into()),
                    description: "Paused".into(),
                    tooltip: None,
                    dimmed: false,
                },
            ];
            widgets::lane_axis(
                ui,
                &[(0.0, "18".into()), (0.5, "21".into()), (1.0, "24".into())],
            );
            widgets::lane(ui, 40.0, &[0.5], &blocks, Some((0.05, 0.35)), None);
            let start = chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
            let days: Vec<_> = (0..90)
                .map(|offset| {
                    (
                        start + chrono::Days::new(offset),
                        (offset as i64 % 5) * 900_000,
                    )
                })
                .collect();
            widgets::heat_strip(ui, &days, "Sample activity");
        });
        widgets::step_item(ui, 1, "Done step", widgets::StepState::Done);
        widgets::step_item(ui, 2, "Current step", widgets::StepState::Current);
        widgets::step_item(ui, 3, "Next step", widgets::StepState::Upcoming);
        widgets::icon_line(
            ui,
            icons::LOCK_SIMPLE,
            "Icon line with wrapped secondary text.",
        );
        widgets::wizard_footer(
            ui,
            "Back",
            Button::new("Continue", ButtonKind::Primary),
            true,
            "Wizard footer caption",
        );
        widgets::empty_state(
            ui,
            "Empty state",
            "Explains why there's nothing here.",
            &[Button::new("Retry", ButtonKind::Secondary)],
        );
    }
}

fn calendar(ui: &mut Ui) {
    const CELL: f32 = 30.0;
    const GAP: f32 = 2.0;
    let colors = Tokens::get(ui);
    let width = 7.0 * CELL + 6.0 * GAP;
    ui.set_width(width);
    ui.spacing_mut().item_spacing = vec2(GAP, GAP);
    ui.allocate_ui_with_layout(
        vec2(width, Tokens::SMALL_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add(Button {
                size: ButtonSize::Small,
                icon: Some(icons::CARET_LEFT),
                ..Button::new("Previous month", ButtonKind::GhostIcon)
            });
            let title_width = width - 2.0 * (Tokens::SMALL_HEIGHT + GAP);
            ui.allocate_ui_with_layout(
                vec2(title_width, Tokens::SMALL_HEIGHT),
                Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    ui.label(RichText::new("November 2026").font(Tokens::semibold(Tokens::BODY)));
                },
            );
            ui.add_enabled(
                false,
                Button {
                    size: ButtonSize::Small,
                    icon: Some(icons::CARET_RIGHT),
                    ..Button::new("Next month", ButtonKind::GhostIcon)
                },
            );
        },
    );
    egui::Grid::new(ui.id().with("november-2026"))
        .spacing(vec2(GAP, GAP))
        .show(ui, |ui| {
            for day in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                ui.allocate_ui_with_layout(
                    vec2(CELL, 18.0),
                    Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        ui.label(
                            RichText::new(day)
                                .size(Tokens::MICRO)
                                .color(colors.fg_subtle),
                        );
                    },
                );
            }
            ui.end_row();
        });

    let start = 13;
    let end = 20;
    for week in 0..6 {
        let first = week * 7 - 5;
        let last = first + 6;
        let (row, _) = ui.allocate_exact_size(vec2(width, CELL), Sense::hover());
        let day_rect = |day| {
            Rect::from_min_size(
                row.min + vec2((day - first) as f32 * (CELL + GAP), 0.0),
                vec2(CELL, CELL),
            )
        };
        let band_start = start.max(first);
        let band_end = end.min(last);
        let painter = ui.painter();
        if band_start <= band_end {
            let band = day_rect(band_start).union(day_rect(band_end));
            painter.rect_filled(theme::snap(ui, band), Tokens::TAG_RADIUS, colors.muted);
        }
        for number in first.max(1)..=last.min(30) {
            let in_range = (start..=end).contains(&number);
            let available = number <= 20;
            let rect = theme::snap(ui, day_rect(number));
            let foreground = if number == start || number == end {
                painter.rect_filled(rect, Tokens::TAG_RADIUS, colors.primary);
                colors.primary_fg
            } else if available {
                colors.fg
            } else {
                colors.fg_subtle.gamma_multiply(0.65)
            };
            painter.text(
                theme::snap(ui, rect.center()),
                Align2::CENTER_CENTER,
                number,
                Tokens::mono(Tokens::CAPTION),
                foreground,
            );
            let response = ui.interact(rect, ui.id().with(("november", number)), Sense::hover());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Label,
                    true,
                    format!(
                        "November {number}{}",
                        if in_range {
                            ", in range"
                        } else if !available {
                            ", unavailable"
                        } else {
                            ""
                        },
                    ),
                )
            });
        }
    }
}

fn examples_table(ui: &mut Ui) {
    const DURATION_WIDTH: f32 = 64.0;
    const SPARKLINE_WIDTH: f32 = 78.0;
    const GAP: f32 = 12.0;
    let colors = Tokens::get(ui);
    widgets::card(ui, |ui| {
        let width = ui.available_width();
        ui.set_min_width(width);
        theme::section(ui, "Top apps");
        let name_width = (width - 2.0 * DURATION_WIDTH - SPARKLINE_WIDTH - 3.0 * GAP).max(0.0);
        let columns = [
            ("App", name_width, Align::Min),
            ("Focused", DURATION_WIDTH, Align::Max),
            ("Running", DURATION_WIDTH, Align::Max),
            ("Last 7 days", SPARKLINE_WIDTH, Align::Max),
        ];
        ui.allocate_ui_with_layout(
            vec2(width, 24.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for (label, width, align) in columns {
                    let text = RichText::new(label)
                        .size(Tokens::CAPTION)
                        .color(colors.fg_muted);
                    if align == Align::Max {
                        widgets::right_aligned_cell(ui, width, 24.0, text);
                    } else {
                        widgets::table_cell(ui, width, 24.0, align, |ui| {
                            ui.label(text);
                        });
                    }
                }
            },
        );
        for (name, category, focus, running, activity) in [
            (
                "Hades II",
                Category::Games,
                "2h 15m",
                "3h 20m",
                [1, 0, 3, 2, 5, 2, 4],
            ),
            (
                "Firefox",
                Category::Browsers,
                "1h 45m",
                "5h 12m",
                [2, 3, 2, 4, 2, 3, 2],
            ),
            (
                "Discord",
                Category::Communication,
                "36m",
                "6h 4m",
                [0, 1, 0, 2, 1, 0, 1],
            ),
        ] {
            ui.separator();
            ui.allocate_ui_with_layout(
                vec2(width, 36.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    widgets::table_cell(ui, name_width, 36.0, Align::Min, |ui| {
                        widgets::icon_tile(ui, None, name, category, 30.0);
                        ui.add(egui::Label::new(name).truncate())
                            .on_hover_text(name);
                        if category == Category::Games {
                            widgets::tag(ui, "Edited");
                        }
                    });
                    for duration in [focus, running] {
                        widgets::right_aligned_cell(
                            ui,
                            DURATION_WIDTH,
                            36.0,
                            RichText::new(duration).font(Tokens::mono(Tokens::SECONDARY)),
                        );
                    }
                    widgets::table_cell(ui, SPARKLINE_WIDTH, 36.0, Align::Max, |ui| {
                        widgets::sparkline(ui, category, activity);
                    });
                },
            );
        }
    });
}

fn chart_examples(ui: &mut Ui, week: bool) {
    let categories = [Category::Games, Category::Browsers, Category::Communication];
    let minutes = [
        [180, 60, 30],
        [210, 75, 35],
        [105, 60, 25],
        [170, 45, 20],
        [250, 60, 20],
        [310, 60, 15],
        [235, 105, 30],
    ];
    let stacks =
        minutes.map(|amounts| std::array::from_fn::<_, 3, _>(|i| (categories[i], amounts[i])));
    let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let colors = Tokens::get(ui);
    let bars: Vec<_> = (0..if week { weekdays.len() } else { 30 })
        .map(|index| {
            let stack = &stacks[index % stacks.len()];
            let total: i64 = stack
                .iter()
                .map(|(_, minutes)| i64::from(*minutes) * 60_000)
                .sum();
            ChartBar {
                label: if week {
                    weekdays[index].into()
                } else {
                    (index + 1).to_string()
                },
                description: format!("{} focused", super::format::duration(total)),
                total: Some(format_minutes(total / 60_000)),
                segments: stack
                    .iter()
                    .map(|(category, minutes)| widgets::Segment {
                        color: colors.category(*category),
                        ms: i64::from(*minutes) * 60_000,
                        dimmed: false,
                        name: category.label(),
                    })
                    .collect(),
                today: false,
                future: false,
                slice_detail: None,
            }
        })
        .collect();
    widgets::card(ui, |ui| {
        theme::section(ui, "Focused time by day");
        widgets::stacked_bars(ui, &bars, week, 260.0);
        theme::section(ui, "Timeline");
        let (lane, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let gap = Rect::from_min_size(lane.min, vec2(lane.width() * 0.6, lane.height()));
        widgets::hatch_gap(ui, gap);
        widgets::chart_block(ui, gap, "10:00 to 11:00, collection gap", false);
    });
}

fn format_minutes(total: i64) -> String {
    match (total / 60, total % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    #[test]
    fn both_themes_draw_the_open_specimens_at_each_scale() {
        for scale in [1.0, 1.5, 2.0] {
            let ctx = Context::default();
            ctx.options_mut(|options| options.zoom_factor = 1.25);
            let mut gallery = Gallery::new(&ctx);
            // Specimens are checked as drawn once settled.
            widgets::motion::set_enabled(&ctx, false);
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1100.0, 2000.0))),
                ..egui::RawInput::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            let mut output = ctx.run_ui(input, |ui| gallery.draw(ui));
            output.textures_delta.clear();
            assert_eq!(output.pixels_per_point, scale);
            assert_eq!(ctx.zoom_factor(), 1.0);
            for label in [
                "Keyboard focus",
                "Disabled",
                "+12%",
                "November 2026",
                "Delete history…",
                "Save changes",
                "Hades II",
                "Firefox",
                "Discord",
                "10h",
                "4h 30m",
            ] {
                let matches = output
                    .shapes
                    .iter()
                    .filter(|paint| {
                        matches!(&paint.shape, egui::Shape::Text(text)
                        if text.galley.text() == label
                        && paint.clip_rect.contains_rect(text.visual_bounding_rect()))
                    })
                    .count();
                let bounds: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|paint| match &paint.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some((text.visual_bounding_rect(), paint.clip_rect))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    matches, 2,
                    "expected {label:?} in each theme at {scale}×: {bounds:?}"
                );
            }

            let label_rights = |label: &str| {
                output
                    .shapes
                    .iter()
                    .filter_map(|paint| match &paint.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(text.pos.x + text.galley.size().x)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            for (header, cell) in [("Focused", "1h 45m"), ("Running", "5h 12m")] {
                let headers = label_rights(header);
                let cells = label_rights(cell);
                assert_eq!(headers.len(), 2);
                assert_eq!(cells.len(), 2);
                for (header, cell) in headers.into_iter().zip(cells) {
                    assert!((header - cell).abs() <= 0.5 / scale, "{header} vs {cell}");
                }
            }

            for dark in [false, true] {
                let colors = Tokens::for_dark(dark);
                for width in [94.0, 158.0] {
                    assert!(output.shapes.iter().any(
                        |paint| matches!(&paint.shape, egui::Shape::Rect(rect)
                        if rect.fill == colors.muted && rect.corner_radius.nw == Tokens::TAG_RADIUS
                        && (rect.rect.width() - width).abs() <= 1.0
                        && (rect.rect.height() - 30.0).abs() <= 1.0)
                    ));
                }
                assert!(output.shapes.iter().any(
                    |paint| matches!(&paint.shape, egui::Shape::Rect(rect)
                    if rect.fill == colors.border && (rect.rect.height() - 2.0).abs() <= 0.1
                    && (rect.rect.width() - 5.0).abs() <= 1.0)
                ));
            }
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .filter(|paint| matches!(&paint.shape, egui::Shape::Rect(rect)
                        if rect.fill == Tokens::for_dark(true).category(Category::Communication)
                        && rect.corner_radius.nw == 4 && (rect.rect.width() - 46.0).abs() <= 1.0
                    ))
                    .count(),
                14
            );
            let text_centers = |label: &str| {
                output
                    .shapes
                    .iter()
                    .filter_map(|paint| match &paint.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(text.pos.y + text.galley.size().y / 2.0)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            for (left, right) in [
                (icons::PAUSE, "Resume"),
                ("Focused time", icons::CLOCK),
                ("App updated", "Undo"),
                (icons::TRASH, "Delete history…"),
            ] {
                let left_centers = text_centers(left);
                let right_centers = text_centers(right);
                assert_eq!(left_centers.len(), 2, "{left:?}");
                assert_eq!(right_centers.len(), 2, "{right:?}");
                for (a, b) in left_centers.into_iter().zip(right_centers) {
                    assert!(
                        (a - b).abs() <= 1.0,
                        "{left:?} and {right:?} at {scale}×: {a}, {b}"
                    );
                }
            }
        }
    }

    #[test]
    fn shared_dialog_centers_a_460_point_modal() {
        let ctx = Context::default();
        theme::setup(&ctx);
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1100.0, 720.0))),
            time: Some(0.0),
            system_theme: Some(Theme::Dark),
            ..egui::RawInput::default()
        };
        let draw = |ui: &mut Ui| {
            widgets::modal(ui.ctx(), egui::Id::new("modal-test"), "Edit app", |ui| {
                ui.label("Rename the app or change its category.");
                widgets::dialog_footer(
                    ui,
                    Button::new("Save changes", ButtonKind::Primary),
                    true,
                    "Changes can be undone.",
                )
                .is_some()
            });
        };
        ctx.run_ui(input.clone(), draw).textures_delta.clear();
        input.time = Some(0.25);
        ctx.run_ui(input.clone(), draw).textures_delta.clear();
        input.time = Some(0.5);
        let mut output = ctx.run_ui(input, draw);
        output.textures_delta.clear();
        let colors = Tokens::for_dark(true);
        let frame = output
            .shapes
            .iter()
            .filter_map(|paint| match &paint.shape {
                egui::Shape::Vec(shapes) => Some(shapes.iter()),
                _ => None,
            })
            .flatten()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.fill == colors.popover
                        && rect.corner_radius.nw == Tokens::DIALOG_RADIUS =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("dialog frame");
        assert_eq!(frame.width(), 460.0);
        assert!(
            (frame.center() - pos2(550.0, 360.0)).length() <= 1.0,
            "{frame:?}"
        );
    }

    #[test]
    fn footer_buttons_share_whole_pixel_edges_at_each_scale() {
        for scale in [1.0f32, 1.25, 1.5] {
            let ctx = Context::default();
            let mut gallery = Gallery::new(&ctx);
            // Specimens are checked as drawn once settled.
            widgets::motion::set_enabled(&ctx, false);
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1100.0, 2000.0))),
                ..egui::RawInput::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            ctx.run_ui(input.clone(), |ui| gallery.draw(ui))
                .textures_delta
                .clear();
            let mut output = ctx.run_ui(input, |ui| gallery.draw(ui));
            output.textures_delta.clear();
            let mut rects = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<Rect>) {
                match shape {
                    egui::Shape::Rect(rect) => out.push(rect.rect),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut rects);
            }
            let button = |label: &str| {
                let at = output
                    .shapes
                    .iter()
                    .find_map(|clipped| match &clipped.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(text.visual_bounding_rect().center())
                        }
                        _ => None,
                    })
                    .unwrap();
                rects
                    .iter()
                    .filter(|rect| rect.contains(at) && (30.0..40.0).contains(&rect.height()))
                    .min_by(|a, b| a.area().total_cmp(&b.area()))
                    .copied()
                    .unwrap()
            };
            let (cancel, confirm) = (button("Cancel"), button("Save changes"));
            let whole = |value: f32| ((value * scale).round() - value * scale).abs() < 0.01;
            for edge in [
                cancel.top(),
                cancel.bottom(),
                cancel.left(),
                confirm.top(),
                confirm.bottom(),
                confirm.right(),
            ] {
                assert!(whole(edge), "edge {edge} not on a pixel at {scale}x");
            }
            assert_eq!(
                (cancel.top(), cancel.bottom()),
                (confirm.top(), confirm.bottom())
            );
        }
    }
}
