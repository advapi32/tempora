use std::process::Command;
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{
    self, Align, Context, Layout, Rect, RichText, Sense, Ui, UiBuilder, WidgetInfo, WidgetType,
    pos2, vec2,
};
use egui_phosphor::regular as icons;
use tempora::model::ControlRequest;

use super::nav::Page;
use super::screens::{Popup, overview_subtitle};
use super::theme::{self, Tokens};
use super::widgets::{self, BannerKind, Button, ButtonKind, StatusKind, ToastAction};
use super::{APP_NAME, Dashboard, format};

const START_TIMEOUT: Duration = Duration::from_secs(8);
const START_POLL: Duration = Duration::from_millis(500);
const FIRST_STATUS_SEEN: &str = "recorder-first-status-seen";
const SEARCH_FIELD: &str = "global-search-field";
const SEARCH_RESULTS: usize = 8;

pub(super) struct RecorderStartup {
    deadline: Instant,
    next_check: Instant,
    quiet: bool,
}

enum SearchResult {
    App(String, String, tempora::model::Category),
    Day(NaiveDate),
}

impl Dashboard {
    pub(super) fn onboarding(&self) -> bool {
        self.recorder
            .as_ref()
            .is_some_and(|status| !status.settings.enabled)
    }

    pub(super) fn draw_shell(&mut self, root: &mut Ui) {
        let ctx = &root.ctx().clone();
        self.start_recorder_if_missing(ctx);
        self.check_recorder_start(ctx);
        let colors = Tokens::get(root);
        let onboarding = self.onboarding();
        if !onboarding
            && self.dialog.is_none()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::K))
        {
            self.open_search(ctx);
        }
        egui::Panel::top("dashboard-top-bar")
            .frame(egui::Frame::new().fill(colors.bg))
            .show(root, |ui| self.top_bar(ui, onboarding));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(colors.bg)
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 18,
                        bottom: 0,
                    }),
            )
            .show(root, |ui| {
                // The scroll area spans the window so its bar sits at the window edge.
                let area = egui::ScrollArea::vertical()
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .auto_shrink([false, false])
                    .id_salt(self.nav.page());
                // The bar keeps clear of the window's resize edge.
                ui.spacing_mut().scroll.bar_outer_margin = RESIZE_EDGE;
                widgets::scroll(ui, area, |ui| {
                    ui.spacing_mut().scroll.bar_outer_margin = 0.0;
                    let full = ui.available_width();
                    let padding = f32::from(Tokens::PAGE_PADDING);
                    let width = (full - 2.0 * padding).min(Tokens::CONTENT_MAX);
                    let wide = width >= Tokens::WIDE_CONTENT;
                    let inset = ((full - width) / 2.0).floor();
                    let height = ui.available_height();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(inset);
                        ui.allocate_ui_with_layout(
                            vec2(width, height),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(width);
                                self.viewport_bottom = ui.max_rect().top() + height;
                                ui.spacing_mut().item_spacing =
                                    vec2(Tokens::CONTROL_GAP, Tokens::GRID_GAP);
                                if onboarding {
                                    // Fills the viewport exactly; no trailing space, no scrolling.
                                    self.recorder_banner(ui);
                                    self.first_run_page(ui, wide);
                                } else {
                                    // Each page, and each day on the Day page, fades in.
                                    let fade = widgets::motion::tween(
                                        ctx,
                                        egui::Id::new(("page-fade", self.nav.page())),
                                        1.0,
                                        widgets::motion::Ease {
                                            enter: widgets::motion::PAGE,
                                            ..widgets::motion::Ease::GROW
                                        },
                                    );
                                    ui.multiply_opacity(fade);
                                    self.page_heading(ui);
                                    self.recorder_banner(ui);
                                    self.content(ui, wide);
                                    // One 24 px bottom margin; the item gap above already counts.
                                    ui.add_space(24.0 - Tokens::GRID_GAP);
                                }
                            },
                        );
                    });
                });
            });
        // Any page can open the app menu; drawn once, above the page.
        self.app_menu(ctx);
        resize_edges(ctx);
    }

    pub(super) fn draw_toast(&mut self, ctx: &Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        let elapsed = toast.shown.elapsed();
        if elapsed >= toast.lifetime() {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(toast.lifetime() - elapsed);
        let action = widgets::toast(
            ctx,
            egui::Id::new("dashboard-toast"),
            &toast.message,
            toast.error,
            toast
                .undo
                .filter(|_| self.recorder_online() && !self.pending),
        );
        match action {
            Some(ToastAction::Dismiss) => self.toast = None,
            Some(ToastAction::Undo) => self.undo(),
            None => {}
        }
    }

    fn top_bar(&mut self, ui: &mut Ui, onboarding: bool) {
        const HEIGHT: f32 = 70.0;
        // The bar is the window's title bar: its empty parts move the window.
        let (bar, caption) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::click_and_drag());
        let page = self.nav.page().clone();
        let items = [
            ("Overview", icons::CHART_BAR, page == Page::Overview),
            (
                "Library",
                icons::SQUARES_FOUR,
                matches!(page, Page::Library | Page::App(_)),
            ),
            ("Timeline", icons::ROWS, matches!(page, Page::Day(_))),
        ];
        let dock_width = widgets::nav_width(ui, &items);
        let narrow = bar.width() - f32::from(Tokens::PAGE_PADDING) * 2.0 < Tokens::WIDE_CONTENT;
        let side_padding = if narrow { 12.0 } else { 24.0 };
        let bounds = bar.shrink2(vec2(side_padding, 14.0));
        let controls = Rect::from_min_max(
            pos2(bounds.right() - WINDOW_CONTROLS_WIDTH, bounds.top()),
            bounds.max,
        );
        // The nav stays centered on the window unless the actions to its right need the room;
        // their width is measured on the previous frame.
        let actions_id = ui.id().with("main-actions-width");
        let actions_width = ui
            .data(|data| data.get_temp::<f32>(actions_id))
            .unwrap_or(0.0);
        let gap = Tokens::CONTROL_GAP;
        let center_x = bar
            .center()
            .x
            .min(controls.left() - 2.0 * gap - actions_width - dock_width / 2.0)
            .max(bounds.left() + dock_width / 2.0);
        let dock = Rect::from_center_size(
            pos2(center_x, bar.center().y),
            vec2(dock_width, bounds.height()),
        );
        let left = Rect::from_min_max(bounds.min, pos2(dock.left() - gap, bounds.bottom()));
        let right = Rect::from_min_max(
            pos2(dock.right() + gap, bounds.top()),
            pos2(controls.left() - gap, bounds.bottom()),
        );
        ui.scope_builder(
            UiBuilder::new()
                .id_salt("brand")
                .max_rect(left)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                if brand(ui, onboarding).clicked() {
                    self.nav.section(Page::Overview);
                }
                // The recorder's state sits with the app's name: a quiet dot while
                // recording, a labeled pill when something needs attention.
                ui.spacing_mut().item_spacing.x = 4.0;
                let kind = self.status_kind();
                let detail = self
                    .today_focus
                    .filter(|(date, _)| {
                        *date == Local::now().date_naive() && kind == StatusKind::Recording
                    })
                    .map(|(_, total)| format!("· {} today", format::duration(total)))
                    .or_else(|| {
                        let last = self.extras()?.bounds?.1;
                        (kind == StatusKind::Unreachable).then(|| {
                            let today = format::local_date(last, self.zone)
                                == Some(Local::now().date_naive());
                            let pattern = if today { "%H:%M" } else { "%b %-d, %H:%M" };
                            format!("· data to {}", format::local(last, self.zone, pattern))
                        })
                    });
                if kind != StatusKind::Recording {
                    ui.add_space(6.0);
                }
                widgets::status_pill(ui, kind, detail.as_deref());
            },
        );
        ui.scope_builder(
            UiBuilder::new()
                .id_salt("main-nav")
                .max_rect(dock)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                let selected = ui
                    .add_enabled_ui(!onboarding, |ui| widgets::nav(ui, &items))
                    .inner;
                if let Some(index) = selected {
                    let page = match index {
                        0 => Page::Overview,
                        1 => Page::Library,
                        _ => self.timeline_page(),
                    };
                    self.nav.section(page);
                }
            },
        );
        ui.scope_builder(
            UiBuilder::new()
                .id_salt("main-actions")
                .max_rect(right)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = if narrow { 6.0 } else { Tokens::CONTROL_GAP };
                let settings = ui
                    .add_enabled_ui(!onboarding, |ui| {
                        widgets::settings_button(ui, page == Page::Settings)
                    })
                    .inner;
                if settings.clicked() {
                    self.nav.section(Page::Settings);
                }
                // Searching types into the pill itself; the results open below it.
                if let Some(Popup::Search(query)) = &mut self.popup {
                    let field = widgets::pill_search_field(
                        ui,
                        egui::Id::new(SEARCH_FIELD),
                        query,
                        "Search",
                        widgets::SEARCH_WIDTH,
                    );
                    self.search_results(ui.ctx(), &field);
                } else {
                    let search = ui.add_enabled_ui(!onboarding, widgets::search_pill).inner;
                    if search.clicked() {
                        self.open_search(ui.ctx());
                    }
                }
                let used = right.right() - ui.min_rect().left();
                ui.data_mut(|data| data.insert_temp(actions_id, used));
            },
        );
        ui.scope_builder(
            UiBuilder::new()
                .id_salt("window-controls")
                .max_rect(controls)
                .layout(Layout::right_to_left(Align::Center)),
            window_controls,
        );
        let ctx = ui.ctx();
        // egui hands a drag that starts on a click-only control (a nav tab, the search pill)
        // to the bar beneath it, so remember where the press landed: only a press on empty
        // bar space moves the window, as with a native title bar.
        let press_on_bar = caption.id.with("press-on-bar");
        let press = ctx.input(|input| {
            input.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
        });
        if let Some(press) = press {
            let on_bar = caption.interact_rect.contains(press)
                && ctx.layer_id_at(press) == Some(caption.layer_id)
                && ctx.viewport(|viewport| {
                    !viewport
                        .prev_pass
                        .widgets
                        .get_layer(caption.layer_id)
                        .any(|widget| {
                            widget.id != caption.id
                                && widget.enabled
                                && widget.sense.interactive()
                                && widget.interact_rect.contains(press)
                        })
                });
            ctx.data_mut(|data| data.insert_temp(press_on_bar, on_bar));
        }
        if ctx
            .data(|data| data.get_temp(press_on_bar))
            .unwrap_or(false)
        {
            if caption.double_clicked() {
                toggle_maximized(ctx);
            } else if caption.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
        // With the window hook, Windows takes presses on the bar's empty space itself (see
        // `caption`), and the drag handling above only runs without it. Nothing is claimed
        // while a popup or dialog is open, so a click on the bar still dismisses it.
        if super::caption::installed() {
            let open =
                self.popup.is_some() || self.dialog.is_some() || egui::Popup::is_any_open(ctx);
            publish_caption(ctx, &caption, !open);
        }
    }

    fn open_search(&mut self, ctx: &Context) {
        if matches!(self.popup, Some(Popup::Search(_))) {
            return;
        }
        self.popup = Some(Popup::Search(String::new()));
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(SEARCH_FIELD)));
    }

    /// Ctrl+K: jump to an app or a date. The results under the search pill, with the
    /// keyboard: Up and Down move the selection, Enter opens it, Esc closes.
    fn search_results(&mut self, ctx: &Context, field: &egui::Response) {
        let Some(Popup::Search(query)) = &self.popup else {
            return;
        };
        let today = Local::now().date_naive();
        let results = search(query, &self.apps, today);
        let empty = query.trim().is_empty();
        let selected_id = egui::Id::new("global-search-selected");
        let mut selected = ctx
            .data(|data| data.get_temp::<usize>(selected_id))
            .unwrap_or(0)
            .min(results.len().saturating_sub(1));
        let (down, up, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.key_pressed(egui::Key::Enter),
            )
        });
        if down && selected + 1 < results.len() {
            selected += 1;
        }
        if up {
            selected = selected.saturating_sub(1);
        }
        let mut chosen = enter.then_some(selected).filter(|_| !results.is_empty());
        let output = widgets::anchored_popover(
            ctx,
            egui::Id::new("global-search"),
            field.rect,
            Align::Max,
            |ui| {
                ui.set_width(320.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                let colors = Tokens::get(ui);
                if empty {
                    ui.label(widgets::text(
                        "Type an app name or a date, like \"yesterday\".",
                        Tokens::SECONDARY,
                        colors.fg_muted,
                    ));
                } else if results.is_empty() {
                    ui.label(widgets::text(
                        "No apps or dates match.",
                        Tokens::SECONDARY,
                        colors.fg_muted,
                    ));
                }
                for (index, result) in results.iter().enumerate() {
                    if search_row(ui, result, index == selected).clicked() {
                        chosen = Some(index);
                    }
                }
            },
        );
        ctx.data_mut(|data| data.insert_temp(selected_id, selected));
        if let Some(result) = chosen.and_then(|index| results.into_iter().nth(index)) {
            self.popup = None;
            match result {
                SearchResult::App(key, ..) => self.nav.drill(Page::App(key)),
                SearchResult::Day(date) => self.nav.drill(Page::Day(date)),
            }
        } else if output.dismissed {
            self.popup = None;
        }
        if self.popup.is_none() {
            ctx.data_mut(|data| data.remove::<usize>(selected_id));
        }
    }

    fn timeline_page(&self) -> Page {
        if let Page::Day(date) = self.nav.page() {
            return Page::Day(*date);
        }
        let today = Local::now().date_naive();
        let range = self.nav.view.range;
        // Only this range's report: the previous one, still drawn while this one loads,
        // could pick a day outside the range.
        let date = self
            .report
            .get(&self.report_key())
            .and_then(|report| {
                report
                    .days
                    .iter()
                    .rev()
                    .find(|day| day.date <= today && day.covered_ms > 0)
                    .map(|day| day.date)
            })
            .unwrap_or_else(|| today.clamp(range.start, range.end));
        Page::Day(date)
    }

    fn page_heading(&mut self, ui: &mut Ui) {
        let (title, subtitle) = match self.nav.page() {
            Page::Overview => (
                "Overview".to_owned(),
                Some(overview_subtitle(&self.nav.view, self.today())),
            ),
            Page::Library => ("Library".to_owned(), self.library_subtitle()),
            Page::Settings => (
                "Settings".to_owned(),
                Some("Changes save to the recorder immediately.".to_owned()),
            ),
            // These pages draw a breadcrumb heading of their own.
            Page::App(_) | Page::Day(_) => return,
        };
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            theme::heading(ui, &title);
            if let Some(subtitle) = subtitle {
                ui.label(RichText::new(subtitle).color(Tokens::get(ui).fg_muted));
            }
        });
    }

    fn library_subtitle(&self) -> Option<String> {
        let count = self.apps.len();
        (count > 0).then(|| match count {
            1 => "1 app".to_owned(),
            count => format!("{count} apps"),
        })
    }

    pub(super) fn status_kind(&self) -> StatusKind {
        match &self.recorder {
            Some(status) if status.problem.is_some() => StatusKind::Attention,
            Some(status) if !status.settings.enabled => StatusKind::Setup,
            Some(status) if !status.recording => StatusKind::Paused,
            Some(_) => StatusKind::Recording,
            None if self.recorder_startup.is_some() => StatusKind::Connecting,
            None if self.recorder_error.is_some() => StatusKind::Unreachable,
            None => StatusKind::Connecting,
        }
    }

    fn recorder_banner(&mut self, ui: &mut Ui) {
        if self.recorder_error.is_some() {
            let enabled = self.recorder_startup.is_none();
            let clicked = widgets::banner(
                ui,
                BannerKind::Error,
                "Can't reach the recorder.",
                if enabled {
                    "Recorded history is available but won't update. Settings changes are disabled until it's running."
                } else {
                    "Waiting for the recorder to connect…"
                },
                &[
                    (
                        Button::new("Retry", ButtonKind::Secondary),
                        enabled && !self.status_pending,
                    ),
                    (Button::new("Start recorder", ButtonKind::Primary), enabled),
                ],
            );
            match clicked {
                Some(0) => self.request_status(),
                Some(1) => self.start_recorder(ui.ctx(), false),
                _ => {}
            }
        } else if let Some(status) = &self.recorder {
            if let Some(problem) = &status.problem {
                widgets::banner(
                    ui,
                    BannerKind::Error,
                    "Recorder needs attention",
                    problem,
                    &[],
                );
            } else if status.settings.enabled && status.settings.paused {
                let resume = widgets::banner(
                    ui,
                    BannerKind::Paused,
                    "Recording paused",
                    "Nothing is captured until you resume. The gap will show in timelines.",
                    &[(
                        Button::new("Resume recording", ButtonKind::Primary),
                        !self.pending,
                    )],
                );
                if resume.is_some() {
                    self.action(ControlRequest::SetPaused(false), "Recording resumed");
                }
            } else if status.settings.enabled && !status.recording {
                widgets::banner(
                    ui,
                    BannerKind::Paused,
                    "Recording paused",
                    "Recording resumes when this Windows session is active.",
                    &[],
                );
            }
        }
    }

    fn start_recorder_if_missing(&mut self, ctx: &Context) {
        if self.recorder.is_none() && self.recorder_error.is_none() {
            return;
        }
        let id = egui::Id::new(FIRST_STATUS_SEEN);
        if ctx.data(|data| data.get_temp::<bool>(id)).is_some() {
            return;
        }
        ctx.data_mut(|data| data.insert_temp(id, true));

        if matches!(
            self.recorder_error,
            Some(tempora::pipe::RequestError::NotRunning)
        ) {
            self.start_recorder(ctx, true);
        }
    }

    fn start_recorder(&mut self, ctx: &Context, quiet: bool) {
        let result = std::env::current_exe().and_then(|path| {
            let mut command = Command::new(path.with_file_name("tempora-recorder.exe"));
            if quiet {
                command.arg("--sign-in");
            }
            command.spawn()
        });
        match result {
            Ok(_) => {
                let now = Instant::now();
                self.recorder_startup = Some(RecorderStartup {
                    deadline: now + START_TIMEOUT,
                    next_check: now,
                    quiet,
                });
                ctx.request_repaint();
            }
            Err(error) if !quiet => {
                self.notify(format!("Couldn't start the recorder. {error}"), true);
            }
            Err(_) => {}
        }
    }

    fn check_recorder_start(&mut self, ctx: &Context) {
        let Some(startup) = &mut self.recorder_startup else {
            return;
        };
        let quiet = startup.quiet;
        if self.recorder.is_some() {
            self.recorder_startup = None;
            if !quiet {
                self.notify("Recorder connected", false);
            }
            return;
        }
        let now = Instant::now();
        if now >= startup.deadline {
            self.recorder_startup = None;
            if !quiet {
                self.notify("The recorder did not connect.", true);
            }
            return;
        }
        let poll = now >= startup.next_check;
        if poll {
            startup.next_check = now + START_POLL;
        }
        ctx.request_repaint_after(startup.next_check.min(startup.deadline) - now);
        if poll {
            self.request_status();
        }
    }
}

/// Apps whose name matches, best first: names starting with the query, then names with a
/// word starting with it, then any other match. A date the query names comes first.
fn search(
    query: &str,
    apps: &[tempora::model::AppIdentity],
    today: NaiveDate,
) -> Vec<SearchResult> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut results = Vec::new();
    if let Some(date) = parse_date(query, today) {
        results.push(SearchResult::Day(date));
    }
    let needle = query.to_lowercase();
    let mut matches: Vec<(u8, &tempora::model::AppIdentity)> = apps
        .iter()
        .filter_map(|app| {
            let name = app.name.to_lowercase();
            let rank = if name.starts_with(&needle) {
                0
            } else if name
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word.starts_with(&needle))
            {
                1
            } else if name.contains(&needle) {
                2
            } else {
                return None;
            };
            Some((rank, app))
        })
        .collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    results.extend(
        matches
            .into_iter()
            .take(SEARCH_RESULTS)
            .map(|(_, app)| SearchResult::App(app.key.clone(), app.name.clone(), app.category)),
    );
    results
}

/// One search result: the app's icon and name, or the Timeline for a date.
fn search_row(ui: &mut Ui, result: &SearchResult, selected: bool) -> egui::Response {
    let colors = Tokens::get(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let label = match result {
        SearchResult::App(_, name, _) => name.clone(),
        SearchResult::Day(date) => format!("Timeline, {}", date.format("%A, %b %-d, %Y")),
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    if selected || response.hovered() {
        ui.painter()
            .rect_filled(rect, Tokens::TAG_RADIUS, colors.hover);
    }
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    match result {
        SearchResult::App(key, name, category) => {
            widgets::app_cell_with(&mut row, Some(key), name, None, *category, 22.0, |_| {});
        }
        SearchResult::Day(date) => {
            row.spacing_mut().item_spacing.x = 12.0;
            row.label(
                RichText::new(icons::ROWS)
                    .font(Tokens::icon(Tokens::ICON_SIZE))
                    .color(colors.fg_muted),
            );
            row.label(
                RichText::new(format!("Timeline · {}", date.format("%A, %b %-d")))
                    .font(Tokens::medium(Tokens::BODY)),
            );
        }
    }
    response
}

fn parse_date(query: &str, today: NaiveDate) -> Option<NaiveDate> {
    let lower = query.to_lowercase();
    let date = match lower.as_str() {
        "today" => Some(today),
        "yesterday" => today.pred_opt(),
        _ => NaiveDate::parse_from_str(query, "%Y-%m-%d")
            .ok()
            .or_else(|| {
                ["%b %d %Y", "%B %d %Y", "%d %b %Y", "%d %B %Y"]
                    .into_iter()
                    .find_map(|pattern| {
                        NaiveDate::parse_from_str(&format!("{query} {}", today.year()), pattern)
                            .ok()
                    })
                    .map(|date| {
                        if date > today {
                            date.with_year(today.year() - 1).unwrap_or(date)
                        } else {
                            date
                        }
                    })
            }),
    }?;
    (date <= today).then_some(date)
}

/// The three caption buttons, side by side.
const WINDOW_CONTROLS_WIDTH: f32 =
    3.0 * widgets::WINDOW_BUTTON.x + 3.0 * widgets::WINDOW_BUTTON_GAP + 9.0;
/// How far in from the window edge a press resizes the window (the window has no frame).
pub(super) const RESIZE_EDGE: f32 = 5.0;
/// Corners take a larger target so diagonal resizing is easy to hit.
const RESIZE_CORNER: f32 = 12.0;

fn window_controls(ui: &mut Ui) {
    use widgets::WindowButton;
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
    ui.spacing_mut().item_spacing.x = widgets::WINDOW_BUTTON_GAP;
    if widgets::window_button(ui, WindowButton::Close, None).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    let kind = if maximized {
        WindowButton::Restore
    } else {
        WindowButton::Maximize
    };
    // With the hook in place Windows tracks this button (for Snap Layouts) and handles its
    // click; egui still handles it from the keyboard.
    let native = super::caption::installed().then(super::caption::maximize_state);
    let size = widgets::window_button(ui, kind, native);
    if native.is_some() {
        super::caption::set_maximize_button(size.rect, ctx.pixels_per_point());
    }
    if size.clicked() {
        toggle_maximized(&ctx);
    }
    if widgets::window_button(ui, WindowButton::Minimize, None).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
    // Separates the window's buttons from the app's own.
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 20.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1.0, Tokens::get(ui).border),
    );
}

fn toggle_maximized(ctx: &Context) {
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
}

/// Tells the window hook where the title bar's empty space is: the bar minus its controls,
/// anything drawn over it, and the resize edges.
fn publish_caption(ctx: &Context, caption: &egui::Response, claim: bool) {
    let pixels_per_point = ctx.pixels_per_point();
    if !claim {
        super::caption::set_caption(egui::Rect::NOTHING, &[], pixels_per_point);
        return;
    }
    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
    let mut area = caption.interact_rect;
    let mut holes = Vec::new();
    if !maximized {
        // The edges and top corners resize the window; see `resize_edges`.
        area.min.y += RESIZE_EDGE;
        area.min.x += RESIZE_EDGE;
        area.max.x -= RESIZE_EDGE;
        let window = ctx.content_rect();
        let corner = vec2(RESIZE_CORNER, RESIZE_CORNER);
        holes.push(egui::Rect::from_min_size(window.left_top(), corner));
        holes.push(egui::Rect::from_min_size(
            window.right_top() - vec2(RESIZE_CORNER, 0.0),
            corner,
        ));
    }
    ctx.viewport(|viewport| {
        holes.extend(
            viewport
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets.iter())
                .filter(|widget| {
                    widget.id != caption.id
                        && widget.enabled
                        && widget.sense.interactive()
                        && widget.interact_rect.intersects(area)
                })
                .map(|widget| widget.interact_rect),
        );
    });
    super::caption::set_caption(area, &holes, pixels_per_point);
}

/// The window has no frame, so its edges resize it: the cursor changes near an edge and a
/// press there hands the resize to Windows. Runs after the page so its cursor wins.
pub(super) fn resize_edges(ctx: &Context) {
    use egui::{CursorIcon, ResizeDirection as Dir};
    let (maximized, fullscreen) = ctx.input(|input| {
        let viewport = input.viewport();
        (
            viewport.maximized.unwrap_or(false),
            viewport.fullscreen.unwrap_or(false),
        )
    });
    if maximized || fullscreen {
        return;
    }
    let window = ctx.content_rect();
    let Some(pos) = ctx.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    let near = |distance: f32, zone: f32| (0.0..zone).contains(&distance);
    let (left, right) = (pos.x - window.left(), window.right() - pos.x);
    let (top, bottom) = (pos.y - window.top(), window.bottom() - pos.y);
    let corner_x = near(left, RESIZE_CORNER) || near(right, RESIZE_CORNER);
    let corner_y = near(top, RESIZE_CORNER) || near(bottom, RESIZE_CORNER);
    let west = near(left, RESIZE_EDGE) || (corner_y && near(left, RESIZE_CORNER));
    let east = near(right, RESIZE_EDGE) || (corner_y && near(right, RESIZE_CORNER));
    let north = near(top, RESIZE_EDGE) || (corner_x && near(top, RESIZE_CORNER));
    let south = near(bottom, RESIZE_EDGE) || (corner_x && near(bottom, RESIZE_CORNER));
    let (direction, cursor) = match (north, south, west, east) {
        (true, _, true, _) => (Dir::NorthWest, CursorIcon::ResizeNorthWest),
        (true, _, _, true) => (Dir::NorthEast, CursorIcon::ResizeNorthEast),
        (_, true, true, _) => (Dir::SouthWest, CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (Dir::SouthEast, CursorIcon::ResizeSouthEast),
        (true, ..) => (Dir::North, CursorIcon::ResizeNorth),
        (_, true, ..) => (Dir::South, CursorIcon::ResizeSouth),
        (_, _, true, _) => (Dir::West, CursorIcon::ResizeWest),
        (_, _, _, true) => (Dir::East, CursorIcon::ResizeEast),
        _ => return,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|input| input.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}

fn brand(ui: &mut Ui, onboarding: bool) -> egui::Response {
    let colors = Tokens::get(ui);
    let text = ui.painter().layout_no_wrap(
        APP_NAME.into(),
        Tokens::semibold(Tokens::SECTION),
        colors.fg,
    );
    let (rect, response) = ui.allocate_exact_size(
        vec2(30.0 + 10.0 + text.size().x, 30.0),
        if onboarding {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    response.widget_info(|| {
        WidgetInfo::labeled(
            if onboarding {
                WidgetType::Label
            } else {
                WidgetType::Button
            },
            ui.is_enabled(),
            "Tempora home",
        )
    });
    let mark = Rect::from_min_size(rect.min, vec2(30.0, 30.0));
    lanes_mark(ui.painter(), mark);
    ui.painter().galley(
        pos2(mark.right() + 10.0, rect.center().y - text.size().y / 2.0),
        text,
        colors.fg,
    );
    widgets::focus_ring(ui, &response, 8);
    response
}

/// The Lanes mark, as in the app icon (`assets/icons/app.ico`): three rows of colored activity on a
/// 64-unit grid, with no plate behind them.
fn lanes_mark(painter: &egui::Painter, rect: Rect) {
    const LANES: [(f32, f32, f32, usize); 6] = [
        (4.0, 5.0, 30.0, 0),
        (38.0, 5.0, 22.0, 1),
        (4.0, 25.0, 14.0, 1),
        (22.0, 25.0, 38.0, 0),
        (4.0, 45.0, 24.0, 2),
        (32.0, 45.0, 14.0, 4),
    ];
    let unit = rect.width() / 64.0;
    for (left, top, width, color) in LANES {
        let lane = Rect::from_min_size(rect.min + vec2(left, top) * unit, vec2(width, 14.0) * unit);
        painter.rect_filled(lane, 7.0 * unit, Tokens::APP_PALETTE[color]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_opens_a_day_in_the_selected_range_while_its_report_loads() {
        use crate::dashboard::nav::{Period, period_range};
        use tempora::storage::{DaySummary, Report};
        let ctx = egui::Context::default();
        let mut dashboard = Dashboard::for_test(&ctx);
        let today = Local::now().date_naive();
        let week_start = dashboard.prefs.week_start;
        // Last week is loaded, with activity on its last day.
        let last_week = period_range(Period::Week, today - chrono::Days::new(7), week_start);
        dashboard.nav.set_range(Period::Week, last_week);
        let report = Report {
            apps: Vec::new(),
            categories: Vec::new(),
            days: vec![DaySummary {
                date: last_week.end,
                categories: Vec::new(),
                covered_ms: 1,
            }],
            timeline: Vec::new(),
            coverage: Some((0, 1)),
        };
        dashboard
            .report
            .finish((last_week, false), Ok(std::sync::Arc::new(report)));
        // Stepping to this week, whose report hasn't arrived yet.
        let this_week = period_range(Period::Week, today, week_start);
        dashboard.nav.set_range(Period::Week, this_week);
        let Page::Day(date) = dashboard.timeline_page() else {
            panic!("Timeline opens a day");
        };
        assert!(
            this_week.start <= date && date <= this_week.end,
            "{date} is outside {this_week:?}"
        );
    }

    #[test]
    fn search_parses_dates_without_a_year_into_the_past() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert_eq!(
            parse_date("Nov 12", today),
            NaiveDate::from_ymd_opt(2026, 11, 12)
        );
        assert_eq!(
            parse_date("Dec 3", today),
            NaiveDate::from_ymd_opt(2025, 12, 3)
        );
        assert_eq!(parse_date("2026-11-21", today), None);
        assert_eq!(parse_date("yesterday", today), today.pred_opt());
        assert_eq!(parse_date("firefox", today), None);
    }
}
