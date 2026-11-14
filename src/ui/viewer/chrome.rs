use super::*;

/// Right-hand file commands, in desktop right-to-left order: the bookmark
/// button sits left of move.
const FILE_COMMANDS: &[&str] = &["trash", "move", "bookmark"];

impl Viewer {
    pub(super) fn update_controls(&mut self, ctx: &egui::Context) {
        // The library fills narrow windows and phone screens, covering the bars.
        // On desktop, chrome revealed by hovering an edge would also land above
        // it and cover its header icons, as with settings.
        if self.library.open && overlay::compact(ctx) {
            self.controls_visible = false;
            return;
        }
        if crate::app::is_mobile() {
            self.controls_visible = !self.fullscreen;
            return;
        }
        let now = ctx.input(|i| i.time);
        let screen = ctx.viewport_rect();
        if ctx.pointer_hover_pos().is_some_and(|p| {
            screen.contains(p)
                && (p.y <= screen.top() + TOP_BAR_HEIGHT
                    || p.y >= screen.bottom() - BOTTOM_CHROME_HEIGHT)
        }) || ctx.any_popup_open()
            || self.held_button.is_some()
            || self.menu_open
            || self.zoom_input.is_some()
        {
            self.hover_until = now + 0.05;
        }
        self.controls_visible = !self.hide_controls || now < self.hover_until;
        if now < self.hover_until {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(self.hover_until - now));
        }
    }

    pub(super) fn navigation_button(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        forward: bool,
    ) {
        let response = chrome_icons::tooltip(
            ui.add_enabled(
                !self.files.is_empty(),
                egui::Button::new(if forward { "›" } else { "‹" }),
            ),
            if forward { "next" } else { "previous" },
            &self.shortcut_text(if forward { "next" } else { "prev" }),
        );
        if response.is_pointer_button_down_on() && ctx.input(|i| i.focused) {
            let now = ctx.input(|i| i.time);
            if self
                .held_button
                .is_none_or(|(direction, next)| direction != forward || now >= next)
            {
                self.move_page(ctx, forward);
                self.held_button = Some((forward, now + 0.1));
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if response.clicked() && self.held_button.is_none() {
            // Includes a quick press+release delivered in one frame, and keyboard activation.
            self.move_page(ctx, forward);
        }
    }

    pub(super) fn top(&mut self, ctx: &egui::Context) {
        let screen = ctx.viewport_rect();
        let mobile = crate::app::is_mobile();
        egui::Area::new(egui::Id::unique("tools"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.left_top())
            .movable(false)
            .show(ctx, |ui| {
                if !mobile {
                    chrome_drag(
                        ui,
                        Rect::from_min_size(
                            screen.left_top(),
                            egui::vec2(screen.width(), TOP_BAR_HEIGHT),
                        ),
                    );
                }
                ui.style_mut().interaction.selectable_labels = false;
                // The close glyph sits inside a 28-32px button, so a smaller trailing
                // margin keeps its visible gap equal to the brand icon's 14px.
                // The X spans 12px of its button: leave 14px beside the glyph itself.
                let trail = (14. - (chrome_icons::size(ui) - 12.) / 2.).round() as i8;
                let (lead, trail): (i8, i8) = if mobile { (14, 14) } else { (14, trail) };
                let (left, right) = if crate::i18n::is_rtl() {
                    (trail, lead)
                } else {
                    (lead, trail)
                };
                let margin = egui::Margin {
                    left,
                    right,
                    top: 0,
                    bottom: 0,
                };
                let inner = (screen.width() - f32::from(lead + trail)).max(0.);
                egui::Frame::NONE
                    .fill(BAR)
                    .inner_margin(margin)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(inner, TOP_BAR_HEIGHT));
                        ui.set_max_size(egui::vec2(inner, TOP_BAR_HEIGHT));
                        ui.allocate_ui_with_layout(
                            ui.available_size(),
                            crate::ui::direction::flow(),
                            |ui| {
                                let (brand, _) = ui.allocate_exact_size(
                                    egui::vec2(24., 24.),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .image(self.app_icon.id(), brand, uv(), Color32::WHITE);
                                // Reserve exactly the trailing buttons (pin, menu, settings,
                                // plus orientation and/or close), which shrink on narrow
                                // windows, so the last one stays on the right edge.
                                let count = 3
                                    + usize::from(command_supported("orientation"))
                                    + usize::from(command_supported("close-window"));
                                let buttons = count as f32
                                    * (chrome_icons::size(ui) + ui.spacing().item_spacing.x);
                                let title_width = (ui.available_width() - buttons).max(20.);
                                let filename = self
                                    .files
                                    .get(self.focus_index)
                                    .and_then(|p| p.file_name())
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "NABI VIEW".into());
                                let title = ui
                                    .allocate_ui_with_layout(
                                        egui::vec2(title_width, 32.),
                                        crate::ui::direction::flow(),
                                        |ui| {
                                            ui.set_min_width(title_width);
                                            ui.set_max_width(title_width);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&filename).color(INK),
                                                )
                                                .truncate()
                                                .sense(if mobile {
                                                    egui::Sense::hover()
                                                } else {
                                                    egui::Sense::click_and_drag()
                                                }),
                                            )
                                        },
                                    )
                                    .inner
                                    .hover_text(&filename);
                                if !mobile {
                                    if !self.fullscreen
                                        && title.drag_started_by(egui::PointerButton::Primary)
                                    {
                                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                                    }
                                    if title.double_clicked() {
                                        self.run("maximize", ctx);
                                    }
                                }

                                if command_supported("orientation")
                                    && chrome_icons::button_with_shortcut(
                                        ui,
                                        orientation_command(&self.settings.orientation),
                                        orientation_command(&self.settings.orientation),
                                        self.settings.orientation != "auto",
                                        &self.shortcut_text("orientation"),
                                    )
                                    .clicked()
                                {
                                    self.run("orientation", ctx);
                                }
                                let (control, label, active, command) = if mobile {
                                    (
                                        "fullscreen",
                                        "toggle-full-screen",
                                        self.fullscreen,
                                        "fullscreen",
                                    )
                                } else {
                                    ("pin", "pin-controls", !self.hide_controls, "chrome")
                                };
                                if chrome_icons::button_with_shortcut(
                                    ui,
                                    control,
                                    label,
                                    active,
                                    &self.shortcut_text(command),
                                )
                                .clicked()
                                {
                                    self.run(command, ctx);
                                }
                                let menu = chrome_icons::button(ui, "menu", "menu", self.menu_open);
                                if menu.clicked() {
                                    self.menu_open = !self.menu_open;
                                    self.release_held_input();
                                    self.canvas_drag = None;
                                }
                                if chrome_icons::button_with_shortcut(
                                    ui,
                                    "settings",
                                    "settings",
                                    self.settings_open,
                                    &self.shortcut_text("settings"),
                                )
                                .clicked()
                                {
                                    self.run("settings", ctx);
                                }
                                if command_supported("close-window")
                                    && chrome_icons::button_with_shortcut(
                                        ui,
                                        "close",
                                        "close-window",
                                        false,
                                        &self.shortcut_text("close-window"),
                                    )
                                    .clicked()
                                {
                                    self.run("close-window", ctx);
                                }
                            },
                        );
                    });
            });
    }

    pub(super) fn zoom_control(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let hint = tr("zoom-10-3000-up-to-2-decimal-places-enter-applies-esc-cancels");
        if let Some(text) = &mut self.zoom_input {
            let cancel = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
            let apply = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
            let mut output = egui::TextEdit::singleline(text)
                .id(egui::Id::unique("zoom-input"))
                .desired_width(if ctx.viewport_rect().width() < NARROW_WIDTH {
                    68.
                } else {
                    82.
                })
                .align(egui::Align2::CENTER_TOP)
                .font(egui::TextStyle::Monospace)
                .char_limit(32)
                .show(ui);
            if self.zoom_input_focus {
                self.zoom_input_focus = false;
                output.response.request_focus();
                output
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(text.chars().count()),
                    )));
                output.state.store(ctx, output.response.id);
            }
            let percent = parse_zoom_percent(text);
            if percent.is_none() {
                ui.painter().rect_stroke(
                    output.response.rect,
                    2.,
                    egui::Stroke::new(1_f32, Color32::LIGHT_RED),
                    egui::StrokeKind::Inside,
                );
            }
            let lost_focus = output.response.lost_focus();
            if cancel || (apply && percent.is_some()) || (lost_focus && !apply) {
                if !cancel && let Some(percent) = percent {
                    self.zoom_override = Some(percent / 100.);
                    self.pan = Vec2::ZERO;
                }
                self.zoom_input = None;
                output.response.surrender_focus();
            } else if apply {
                output.response.request_focus();
            }
            output.response.response.clone().hover_text(hint);
        } else {
            let label = format!("{}%", format_zoom_percent(self.zoom_percent(ctx)));
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(
                    if ctx.viewport_rect().width() < NARROW_WIDTH {
                        76.
                    } else {
                        90.
                    },
                    28.,
                ),
                egui::Sense::hover(),
            );
            let response = ui.interact(rect, egui::Id::unique("zoom-value"), egui::Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            let visuals = ui.style().interact(&response);
            ui.painter().rect_filled(rect, 4., visuals.weak_bg_fill);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &label,
                FontId::monospace(14.),
                visuals.text_color(),
            );
            if response.clicked() {
                self.zoom_input = Some(format_zoom_percent(self.zoom_percent(ctx)));
                self.zoom_input_focus = true;
            }
            response.hover_text(hint);
        }
    }

    pub(super) fn bottom(&mut self, ctx: &egui::Context) {
        let screen = ctx.viewport_rect();
        let mobile = crate::app::is_mobile();
        egui::Area::new(egui::Id::unique("status"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(
                screen.left(),
                screen.bottom() - STATUS_BAR_HEIGHT,
            ))
            .movable(false)
            .show(ctx, |ui| {
                if !mobile {
                    chrome_drag(
                        ui,
                        Rect::from_min_size(
                            egui::pos2(screen.left(), screen.bottom() - STATUS_BAR_HEIGHT),
                            egui::vec2(screen.width(), STATUS_BAR_HEIGHT),
                        ),
                    );
                }
                ui.style_mut().interaction.selectable_labels = false;
                egui::Frame::NONE
                    .fill(BAR)
                    .inner_margin(egui::Margin::symmetric(14, 0))
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(
                            (screen.width() - 28.).max(0.),
                            STATUS_BAR_HEIGHT,
                        ));
                        ui.set_max_size(egui::vec2(
                            (screen.width() - 28.).max(0.),
                            STATUS_BAR_HEIGHT,
                        ));
                        if screen.width() < NARROW_WIDTH {
                            ui.spacing_mut().item_spacing.x = 4.;
                            ui.spacing_mut().button_padding.x = 6.;
                        }
                        if mobile {
                            // Same controls as desktop without the zoom field; pinch
                            // zooms instead. Narrow phones scroll the row sideways.
                            egui::ScrollArea::horizontal()
                                .id_salt("status-scroll")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                )
                                .scroll_source(egui::scroll_area::ScrollSource::ALL)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.horizontal_centered(|ui| {
                                        self.status_commands(ui, ctx);
                                        let files = FILE_COMMANDS
                                            .iter()
                                            .filter(|id| command_supported(id))
                                            .count();
                                        if files > 0 {
                                            let spacing = ui.spacing().item_spacing.x;
                                            let width = files as f32
                                                * (chrome_icons::size(ui) + spacing)
                                                + 6.
                                                + spacing;
                                            ui.add_space((ui.available_width() - width).max(0.));
                                            ui.separator();
                                            for &id in FILE_COMMANDS.iter().rev() {
                                                self.file_command_button(ui, ctx, id);
                                            }
                                        }
                                    });
                                });
                            return;
                        }
                        ui.horizontal_centered(|ui| {
                            self.status_commands(ui, ctx);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let zoom_in = chrome_icons::tooltip(
                                        ui.button("+"),
                                        "zoom-in",
                                        &self.shortcut_text("in"),
                                    )
                                    .clicked();
                                    self.zoom_control(ui, ctx);
                                    let zoom_out = chrome_icons::tooltip(
                                        ui.button("−"),
                                        "zoom-out",
                                        &self.shortcut_text("out"),
                                    )
                                    .clicked();
                                    ui.separator();
                                    for &id in FILE_COMMANDS {
                                        self.file_command_button(ui, ctx, id);
                                    }
                                    if zoom_in {
                                        self.run("in", ctx);
                                    } else if zoom_out {
                                        self.run("out", ctx);
                                    }
                                },
                            );
                        });
                    });
            });
    }

    /// Paging, layout and view commands shared by the desktop and phone status bars.
    fn status_commands(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (a, b) = self.focus_span();
        self.navigation_button(ui, ctx, false);
        let count = self.files.len();
        let pages = if count == 0 {
            "0 / 0".to_string()
        } else if a == b {
            format!("{} / {}", a + 1, count)
        } else {
            format!("{}–{} / {}", a + 1, b + 1, count)
        };
        ui.label(RichText::new(pages).monospace().color(INK));
        self.navigation_button(ui, ctx, true);
        ui.separator();
        // Portrait displays have no spread control, nor the separator after it.
        if self.landscape {
            self.spread_button(ui, ctx);
            ui.separator();
        }
        if chrome_icons::button_with_shortcut(
            ui,
            "random",
            "random-order",
            self.random,
            &self.shortcut_text("random"),
        )
        .clicked()
        {
            self.run("random", ctx);
        }
        if chrome_icons::button_with_shortcut(
            ui,
            "slideshow",
            "slideshow",
            self.slideshow,
            &self.shortcut_text("slideshow"),
        )
        .clicked()
        {
            self.run("slideshow", ctx);
        }
        ui.separator();
        if command_supported("reveal")
            && chrome_icons::button_with_shortcut(
                ui,
                "reveal",
                "reveal-in-file-manager",
                false,
                &self.shortcut_text("reveal"),
            )
            .clicked()
        {
            self.run("reveal", ctx);
        }
        if command_supported("share") {
            let clicked = ui
                .add_enabled_ui(!self.files.is_empty(), |ui| {
                    chrome_icons::button_with_shortcut(
                        ui,
                        "share",
                        "share",
                        false,
                        &self.shortcut_text("share"),
                    )
                    .clicked()
                })
                .inner;
            if clicked {
                self.run("share", ctx);
            }
        }
        if chrome_icons::button_with_shortcut(
            ui,
            "metadata",
            "image-metadata",
            self.inspector.open,
            &self.shortcut_text("metadata"),
        )
        .clicked()
        {
            self.run("metadata", ctx);
        }
    }

    fn spread_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if chrome_icons::button_with_shortcut(
            ui,
            if !self.settings.spread {
                "spread"
            } else if self.settings.rtl {
                "spread-rtl"
            } else {
                "spread-ltr"
            },
            if !self.settings.spread {
                "two-page-spread"
            } else if self.settings.rtl {
                "spread-left"
            } else {
                "spread-right"
            },
            self.settings.spread,
            &self.shortcut_text("spread"),
        )
        .clicked()
        {
            self.run("spread", ctx);
        }
    }

    fn file_command_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: &str) {
        if !command_supported(id) {
            return;
        }
        let clicked = ui
            .add_enabled_ui(!self.files.is_empty() && self.file_job.is_none(), |ui| {
                chrome_icons::button_with_shortcut(
                    ui,
                    id,
                    command_label(id),
                    false,
                    &self.shortcut_text(id),
                )
                .clicked()
            })
            .inner;
        if clicked {
            self.run(id, ctx);
        }
    }

    pub(super) fn thumbnail_bar(&mut self, ctx: &egui::Context) {
        let mut wanted = None;
        let mut next_focus = None;
        let (focus_start, focus_end) = self.focus_span();
        let focused_pages = focus_start..=focus_end;
        let screen = ctx.viewport_rect();
        egui::Area::new(egui::Id::unique("thumbnails"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(
                screen.left(),
                screen.bottom() - BOTTOM_CHROME_HEIGHT,
            ))
            .movable(false)
            .show(ctx, |ui| {
                if !crate::app::is_mobile() {
                    chrome_drag(
                        ui,
                        Rect::from_min_size(
                            egui::pos2(screen.left(), screen.bottom() - BOTTOM_CHROME_HEIGHT),
                            egui::vec2(screen.width(), THUMBNAIL_BAR_HEIGHT),
                        ),
                    );
                }
                ui.style_mut().interaction.selectable_labels = false;
                egui::Frame::NONE
                    .fill(Color32::from_gray(16))
                    .inner_margin(egui::Margin::symmetric(12, 0))
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(
                            (screen.width() - 24.).max(0.),
                            THUMBNAIL_BAR_HEIGHT,
                        ));
                        ui.set_max_size(egui::vec2(
                            (screen.width() - 24.).max(0.),
                            THUMBNAIL_BAR_HEIGHT,
                        ));
                        ui.horizontal_centered(|ui| {
                            let visible = thumbnail_range(
                                self.files.len(),
                                self.focus_index,
                                ui.available_width(),
                                ui.spacing().item_spacing.x,
                            );
                            for i in visible {
                                let path = self.files[i].clone();
                                let (r, resp) = ui.allocate_exact_size(
                                    egui::vec2(THUMBNAIL_WIDTH, 57.),
                                    egui::Sense::click(),
                                );
                                if focused_pages.contains(&i) {
                                    ui.painter().rect_filled(r.expand(2.), 2., ACCENT);
                                }
                                ui.painter().rect_filled(r, 1., Color32::from_gray(30));
                                let tex = self
                                    .thumbs
                                    .get(&path)
                                    .or_else(|| self.pictures.get(&path).map(|p| &p.handle));
                                if let Some(tex) = tex {
                                    let size = tex.size_vec2();
                                    let ratio = (r.width() / size.x).max(r.height() / size.y);
                                    ui.painter().with_clip_rect(r).image(
                                        tex.id(),
                                        Rect::from_center_size(r.center(), size * ratio),
                                        uv(),
                                        Color32::WHITE,
                                    );
                                } else if !self.thumb_errors.contains(&path) && wanted.is_none() {
                                    wanted = Some(path.clone());
                                }
                                let tag = Rect::from_min_size(
                                    r.left_bottom() - egui::vec2(0., 15.),
                                    egui::vec2(29., 15.),
                                );
                                ui.painter()
                                    .rect_filled(tag, 1., Color32::from_black_alpha(176));
                                ui.painter().text(
                                    tag.center(),
                                    egui::Align2::CENTER_CENTER,
                                    (i + 1).to_string(),
                                    FontId::monospace(10.),
                                    INK,
                                );
                                if self.checked.contains(&path) {
                                    self.paint_check(ui, r, &path);
                                }
                                if resp
                                    .hover_text(
                                        path.file_name().unwrap_or_default().to_string_lossy(),
                                    )
                                    .clicked()
                                {
                                    next_focus = Some(i);
                                }
                            }
                        });
                    });
            });
        if let Some(focus_index) = next_focus {
            self.focus_navigation(ctx, focus_index);
        }
        if let Some(path) = wanted {
            if !self.memory_policy.background_allowed() || self.details.pending() || self.renaming()
            {
                return;
            }
            if self.thumb_ticket.as_ref().is_none_or(|(_, p)| p != &path) {
                self.cancel_prefetch();
                let reserved = (THUMBNAIL_DECODE_SIDE * THUMBNAIL_DECODE_SIDE * 4) as usize;
                self.trim_cache(reserved);
                if self.cache_bytes().saturating_add(reserved) > self.cache_budget() {
                    return;
                }
                let id = self
                    .media
                    .request_thumbnail(path.clone(), THUMBNAIL_DECODE_SIDE);
                self.thumb_ticket = Some((id, path));
            }
        }
    }
}
