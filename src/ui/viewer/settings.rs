//! Categorized settings share the viewer's existing commands and persistence.
use super::*;

pub(super) struct ShortcutEditor {
    pub(super) index: Option<usize>,
    pub(super) action: String,
    pub(super) key: String,
    pub(super) listening: bool,
    pub(super) error: String,
}

const SECTIONS: &[(&str, &str)] = &[
    ("general", "choose-the-language-used-in-menus-and-settings"),
    (
        "connections",
        "choose-which-image-formats-open-with-nabi-view",
    ),
    ("cache", "manage-cached-images-and-recent-items"),
    (
        "keyboard-shortcuts",
        "customize-the-keys-used-to-control-the-viewer",
    ),
    ("sharing", "move-your-preferences-between-devices"),
    ("about", "open-source-licenses"),
];

pub(super) const ABOUT_SECTION: usize = 5;

impl Viewer {
    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut section = self.settings_section.min(SECTIONS.len() - 1);
        let mobile = crate::app::is_mobile();
        let compact = mobile || ctx.screen_rect().width() <= 600.;
        if mobile && !matches!(section, 0 | 2 | 5) {
            section = 0;
        }
        let mut open = true;
        egui::Area::new(egui::Id::new("settings-screen"))
            .order(egui::Order::Foreground)
            .fixed_pos(ctx.screen_rect().min)
            .movable(false)
            .show(ctx, |ui| {
                if !self.fullscreen && !mobile {
                    chrome_drag(ui, ctx.screen_rect());
                }
                egui::Frame::NONE
                    .fill(overlay::background())
                    .inner_margin(12)
                    .show(ui, |ui| {
                        forms::style(ui);
                        ui.set_min_size(ctx.screen_rect().size() - egui::vec2(24., 24.));
                        ui.set_max_size(ctx.screen_rect().size() - egui::vec2(24., 24.));
                        crate::ui::direction::horizontal(ui, |ui| {
                            ui.heading(tr("settings"));
                            ui.with_layout(crate::ui::direction::trailing(), |ui| {
                                if forms::icon(ui, "settings-close", "back-to-viewer").clicked() {
                                    open = false;
                                }
                            });
                        });
                        ui.separator();
                        if compact {
                            egui::ComboBox::from_id_salt("mobile-settings-category")
                                .width(ui.available_width())
                                .selected_text(tr(SECTIONS[section].0))
                                .show_ui(ui, |ui| {
                                    for index in 0..SECTIONS.len() {
                                        if mobile && !matches!(index, 0 | 2 | 5) {
                                            continue;
                                        }
                                        ui.selectable_value(
                                            &mut section,
                                            index,
                                            tr(SECTIONS[index].0),
                                        );
                                    }
                                });
                            ui.separator();
                        }
                        let (body, _) =
                            ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                        let sidebar_width = if compact {
                            0.
                        } else if body.width() < 620. {
                            144.
                        } else {
                            172.
                        };
                        let sidebar = Rect::from_min_max(
                            body.min,
                            egui::pos2(body.left() + sidebar_width, body.bottom()),
                        );
                        let content =
                            Rect::from_min_max(egui::pos2(sidebar.right(), body.top()), body.max);
                        let sidebar = crate::ui::direction::mirror(sidebar, body);
                        let content = crate::ui::direction::mirror(content, body);
                        if !compact {
                            ui.painter().rect_filled(sidebar, 0., Color32::TRANSPARENT);
                            ui.painter().line_segment(
                                if crate::i18n::is_rtl() {
                                    [sidebar.left_top(), sidebar.left_bottom()]
                                } else {
                                    [sidebar.right_top(), sidebar.right_bottom()]
                                },
                                egui::Stroke::new(1_f32, Color32::from_gray(48)),
                            );

                            let mut sidebar_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt("settings-categories")
                                    .max_rect(sidebar.shrink(10.))
                                    .layout(crate::ui::direction::vertical()),
                            );
                            sidebar_ui.set_clip_rect(sidebar.intersect(ui.clip_rect()));
                            egui::ScrollArea::vertical()
                                .id_salt("category-scroll")
                                .drag_to_scroll(crate::app::is_mobile())
                                .auto_shrink([false, false])
                                .show(&mut sidebar_ui, |ui| {
                                    for (index, &(title, _)) in SECTIONS.iter().enumerate() {
                                        let selected = section == index;
                                        let label = RichText::new(tr(title)).color(if selected {
                                            ACCENT
                                        } else {
                                            MUTED
                                        });
                                        if ui
                                            .add_sized(
                                                [ui.available_width(), 38.],
                                                egui::SelectableLabel::new(selected, label),
                                            )
                                            .clicked()
                                            && !selected
                                        {
                                            section = index;
                                            self.shortcut_editor = None;
                                            self.shortcut_search = None;
                                            self.shortcut_filter.clear();
                                            self.shortcut_filter_focused = false;
                                            self.shortcut_reset_prompt = false;
                                        }
                                        ui.add_space(3.);
                                    }
                                });
                        }
                        let mut content_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt("settings-details")
                                .max_rect(content.shrink2(if compact {
                                    egui::vec2(0., 8.)
                                } else {
                                    egui::vec2(22., 16.)
                                }))
                                .layout(crate::ui::direction::vertical()),
                        );
                        content_ui.set_clip_rect(content.intersect(ui.clip_rect()));
                        let ui = &mut content_ui;
                        if section == ABOUT_SECTION {
                            self.about_settings(ui);
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt(("settings-content", section))
                                .drag_to_scroll(crate::app::is_mobile())
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    match section {
                                        0 => self.general_settings(ui, ctx),
                                        1 => self.connection_settings(ui),
                                        2 => self.cache_settings(ui, ctx),
                                        3 => self.shortcut_settings(ui),
                                        4 => self.transfer_settings(ui, ctx),
                                        _ => {}
                                    }
                                });
                        }
                    });
            });
        self.settings_section = section;
        self.settings_open = open;
        if !open {
            self.shortcut_editor = None;
            self.shortcut_search = None;
            self.shortcut_filter.clear();
            self.shortcut_filter_focused = false;
            self.shortcut_reset_prompt = false;
        }
        self.shortcut_reset_confirmation(ctx);
    }

    fn about_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.image((self.app_icon.id(), egui::vec2(40., 40.)));
            ui.label(RichText::new("NABI VIEW").size(24.).strong());
        });
        ui.add_space(4.);
        ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
        ui.hyperlink_to(env!("CARGO_PKG_HOMEPAGE"), env!("CARGO_PKG_HOMEPAGE"));
        ui.add_space(12.);
        ui.separator();
        ui.add_space(8.);
        ui.heading(tr("open-source-licenses"));
        ui.checkbox(&mut self.show_full_notices, tr("show-full-license-texts"));
        ui.add_space(12.);
        if !self.show_full_notices {
            egui::ScrollArea::vertical()
                .id_salt("about-license-summary")
                .drag_to_scroll(crate::app::is_mobile())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.label("NABI VIEW · MIT");
                    ui.add_space(8.);
                    for entry in crate::app::licenses::direct_dependencies() {
                        ui.horizontal_wrapped(|ui| {
                            ui.hyperlink_to(&entry.name, &entry.url);
                            if !entry.version.is_empty() {
                                ui.label(RichText::new(&entry.version).small().color(MUTED));
                            }
                            ui.label(&entry.license);
                        });
                        ui.add_space(4.);
                    }
                });
            return;
        }
        let lines = self.notices.get_or_insert_with(|| {
            let third_party = crate::app::notices_path()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .unwrap_or_else(|| tr("the-bundled-notices-file-could-not-be-read").into());
            let text = format!(
                "{}\n\n{}\n\n{third_party}",
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/LICENSE")),
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/THIRD_PARTY.md"))
            );
            text.lines().map(str::to_owned).collect()
        });
        let height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both()
            .id_salt("about-licenses")
            .drag_to_scroll(crate::app::is_mobile())
            .auto_shrink([false, false])
            .show_rows(ui, height, lines.len(), |ui, rows| {
                for row in rows {
                    ui.add(egui::Label::new(RichText::new(&lines[row]).monospace()).extend());
                }
            });
    }

    fn connection_settings(&mut self, ui: &mut egui::Ui) {
        self.associations.poll();
        if self.associations.states.is_empty() {
            self.associations.refresh(None);
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
        if forms::button(ui, tr("refresh-connection-status")).clicked() {
            self.associations.refresh(None);
        }
        if cfg!(target_os = "windows") {
            ui.label(tr("choose-nabi-view-in-windows-default-apps-settings"));
        }
        if !crate::platform::associations::per_type()
            && forms::button(ui, tr("image-default-app-settings")).clicked()
        {
            self.associations.refresh(Some(String::new()));
        }
        ui.add_space(12.);
        let columns = ((ui.available_width() + forms::GAP) / (116. + forms::GAP))
            .floor()
            .clamp(1., 3.) as usize;
        egui::Grid::new("extension-connections")
            .num_columns(columns)
            .spacing([forms::GAP, forms::GAP])
            .show(ui, |ui| {
                for (index, extension) in media::EXTENSIONS.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if crate::platform::associations::per_type() {
                            let active =
                                self.associations.states.get(*extension).copied().flatten();
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::splat(forms::height()),
                                egui::Sense::click(),
                            );
                            let color = if active == Some(true) { ACCENT } else { MUTED };
                            let stroke = egui::Stroke::new(1.6_f32, color);
                            for offset in [-3., 3.] {
                                ui.painter().rect_stroke(
                                    Rect::from_center_size(
                                        rect.center() + egui::vec2(offset, -offset),
                                        egui::vec2(11., 7.),
                                    ),
                                    3.,
                                    stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let response = response.on_hover_text(tr(if active == Some(true) {
                                "default-app"
                            } else if active.is_none() {
                                "connection-status-unavailable"
                            } else {
                                "connect-this-format"
                            }));
                            if response.clicked() {
                                self.associations.refresh(Some(extension.to_string()));
                            }
                        }
                        ui.label(format!(".{extension}"));
                    });
                    if index % columns == columns - 1 {
                        ui.end_row();
                    }
                }
            });
        if !self.associations.message.is_empty() {
            ui.colored_label(MUTED, &self.associations.message);
        }
    }

    fn general_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let before = self.settings.clone();
        settings_row(ui, tr("language"), 250., |ui| {
            let before = self.settings.language.clone();
            egui::ComboBox::from_id_salt("settings-language")
                .width(ui.available_width().min(250.))
                .truncate()
                .selected_text(if before == Language::System {
                    crate::i18n::format(
                        "system-language",
                        &[("language", crate::i18n::current().native_name())],
                    )
                } else {
                    before.native_name().to_owned()
                })
                .show_ui(ui, |ui| {
                    for language in Language::ALL {
                        ui.selectable_value(
                            &mut self.settings.language,
                            language.clone(),
                            language.native_name(),
                        );
                    }
                });
            if before != self.settings.language {
                crate::i18n::set_language(self.settings.language.clone());
                self.save();
                crate::platform::locale::set_application_language(&self.settings.language);
            }
        });
        settings_row(ui, tr("image-display"), 370., |ui| {
            ui.horizontal_wrapped(|ui| {
                for (id, label) in [
                    ("actual", "original"),
                    ("fit", "contain"),
                    ("width", "fit-width"),
                    ("height", "fit-height"),
                    ("fill", "cover-image"),
                ] {
                    ui.selectable_value(&mut self.settings.view_mode, id.to_owned(), tr(label));
                }
            });
        });
        settings_row(ui, tr("small-image-maximum-size"), 230., |ui| {
            if forms::default_button(ui).clicked() {
                self.settings.small_image_percent = Settings::default().small_image_percent;
            }
            forms::number(
                ui,
                egui::DragValue::new(&mut self.settings.small_image_percent)
                    .range(50..=500)
                    .suffix("%"),
            );
        });
        settings_row(ui, tr("layout"), 370., |ui| {
            ui.horizontal_wrapped(|ui| {
                for (spread, rtl, label) in [
                    (false, false, "single"),
                    (true, false, "spread-right"),
                    (true, true, "spread-left"),
                ] {
                    if ui
                        .selectable_label(
                            self.settings.spread == spread && (!spread || self.settings.rtl == rtl),
                            tr(label),
                        )
                        .clicked()
                    {
                        self.settings.spread = spread;
                        self.settings.rtl = rtl;
                    }
                }
            });
        });
        #[cfg(target_os = "macos")]
        settings_row(
            ui,
            tr("black-out-other-monitors-in-full-screen"),
            32.,
            |ui| {
                ui.checkbox(&mut self.settings.blackout_other_monitors, "")
                    .on_hover_text(tr(
                        "off-keeps-other-monitors-available-this-does-not-turn-off-their-power",
                    ));
            },
        );
        settings_row(ui, tr("prevent-display-sleep"), 32., |ui| {
            ui.checkbox(&mut self.settings.prevent_display_sleep, "")
                .on_hover_text(tr("keep-the-display-awake-while-nabi-view-is-running"));
        });
        if let Some(error) = self.display_sleep.error() {
            ui.label(
                RichText::new(tr("could-not-prevent-display-sleep"))
                    .color(Color32::from_rgb(244, 198, 145)),
            )
            .on_hover_text(error);
        }
        let defaults = Settings::default();
        for (label, value, default) in [
            (
                "hide-cursor-after",
                &mut self.settings.cursor_hide_ms,
                defaults.cursor_hide_ms,
            ),
            (
                "toast-duration",
                &mut self.settings.toast_ms,
                defaults.toast_ms,
            ),
            (
                "slide-interval",
                &mut self.settings.slide_ms,
                defaults.slide_ms,
            ),
            (
                "key-hold-delay",
                &mut self.settings.key_delay_ms,
                defaults.key_delay_ms,
            ),
            (
                "key-repeat-interval",
                &mut self.settings.key_repeat_ms,
                defaults.key_repeat_ms,
            ),
        ] {
            if crate::app::is_mobile()
                && matches!(
                    label,
                    "hide-cursor-after" | "key-hold-delay" | "key-repeat-interval"
                )
            {
                continue;
            }
            settings_row(ui, tr(label), 280., |ui| {
                if forms::default_button(ui).clicked() {
                    *value = default;
                }
                if ui.button("+").clicked() {
                    *value = (*value + 100).min(100_000);
                }
                let mut seconds = *value as f64 / 1000.;
                if forms::number(
                    ui,
                    egui::DragValue::new(&mut seconds)
                        .range(0.1..=100.)
                        .speed(0.1)
                        .fixed_decimals(1),
                )
                .changed()
                {
                    *value = ((seconds * 10.).round() as u32 * 100).clamp(100, 100_000);
                }
                if ui.button("−").clicked() {
                    *value = value.saturating_sub(100).max(100);
                }
            });
        }
        if before != self.settings {
            self.pan = Vec2::ZERO;
            self.zoom = 1.;
            self.slide_deadline = None;
            self.save();
            if before.slide_ms != self.settings.slide_ms {
                self.show_slide_interval_toast(ctx);
            }
        }
    }

    fn cache_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let before = self.settings.clone();
        let defaults = Settings::default();
        settings_row(ui, tr("cached-images"), 230., |ui| {
            if forms::default_button(ui).clicked() {
                self.settings.cache_image_limit = defaults.cache_image_limit;
            }
            forms::number(
                ui,
                egui::DragValue::new(&mut self.settings.cache_image_limit).range(2..=10_000),
            );
        });
        settings_hint(ui, "keeps-current-and-nearby-images-ready-for-display");
        settings_row(ui, tr("cache-memory"), 230., |ui| {
            if forms::default_button(ui).clicked() {
                self.settings.cache_memory_mb = defaults.cache_memory_mb;
            }
            forms::number(
                ui,
                egui::DragValue::new(&mut self.settings.cache_memory_mb)
                    .range(32..=4096)
                    .suffix(" MiB"),
            );
        });
        settings_hint(
            ui,
            "includes-images-animation-frames-and-thumbnails-the-current-view-may-exceed-this-limit",
        );
        settings_hint(
            ui,
            "these-are-upper-limits-the-cache-shrinks-automatically-when-system-memory-is-low",
        );
        if before != self.settings {
            self.save();
        }

        if !crate::app::is_mobile() {
            ui.add_space(12.);
            ui.heading(tr("recent"));
            self.history_settings(ui, ctx);
            self.move_history_settings(ui);
        }
    }

    fn history_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        settings_row(ui, tr("remember-recent-items"), 32., |ui| {
            let mut value = self.settings.remember_recent;
            if ui
                .checkbox(&mut value, "")
                .on_hover_text(tr("remember-recent-items"))
                .changed()
            {
                self.run("remember", ctx);
            }
        });
        settings_row(ui, tr("maximum-recent-items"), 90., |ui| {
            if forms::number(
                ui,
                egui::DragValue::new(&mut self.settings.recent_limit).range(1..=500),
            )
            .changed()
            {
                self.save();
            }
        });
        ui.add_space(10.);
        if forms::button(ui, tr("clear-recent-items-dialog")).clicked() {
            self.run("clear", ctx);
        }
    }

    fn shortcut_settings(&mut self, ui: &mut egui::Ui) {
        settings_hint(
            ui,
            "click-a-row-to-change-its-key-or-action-add-more-than-one-shortcut-for-the-same-action",
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .push_id("shortcut-add", |ui| forms::button(ui, tr("add")))
                .inner
                .clicked()
            {
                self.shortcut_editor = Some(ShortcutEditor {
                    index: None,
                    action: "open".into(),
                    key: String::new(),
                    listening: true,
                    error: String::new(),
                });
            }
            if forms::button(ui, tr("search-by-shortcut")).clicked() {
                self.shortcut_editor = None;
                self.shortcut_search = Some(String::new());
                ui.memory_mut(|m| m.surrender_focus(egui::Id::new("shortcut-filter")));
            }
            let filter = ui.add_sized(
                [ui.max_rect().width().min(180.), forms::height()],
                forms::text_edit(&mut self.shortcut_filter)
                    .id(egui::Id::new("shortcut-filter"))
                    .hint_text(tr("shortcut-filter")),
            );
            self.shortcut_filter_focused = filter.has_focus();
            if filter.gained_focus()
                && self
                    .shortcut_search
                    .as_ref()
                    .is_some_and(|key| key.is_empty())
            {
                self.shortcut_search = None;
            }
            if ui
                .push_id("shortcut-reset", |ui| {
                    forms::button(ui, tr("reset-shortcuts"))
                })
                .inner
                .clicked()
            {
                self.shortcut_reset_prompt = true;
            }
        });
        if let Some(search) = self.shortcut_search.clone() {
            ui.horizontal(|ui| {
                let label = if search.is_empty() {
                    tr("press-a-key-dialog").to_owned()
                } else {
                    shortcut_label(&search)
                };
                ui.label(RichText::new(label).color(ACCENT));
                if forms::button(ui, tr("clear-shortcut-search")).clicked() {
                    self.shortcut_search = None;
                }
            });
        }
        ui.add_space(10.);
        egui::Frame::NONE.fill(Color32::BLACK).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(34.);
            ui.spacing_mut().item_spacing.y = 0.;
            self.shortcut_list(ui);
        });
        if !self.last_error.is_empty() {
            ui.add_space(8.);
            ui.colored_label(
                Color32::LIGHT_RED,
                crate::i18n::value_label("error-details", &self.last_error),
            );
        }
        self.shortcut_editor_window(ui.ctx());
    }

    fn shortcut_list(&mut self, ui: &mut egui::Ui) {
        let rows = self.shortcut_rows();
        if rows.is_empty() {
            ui.label(RichText::new(tr("no-shortcuts-use-add-to-create-one")).color(MUTED));
        }
        let mut visible_rows = 0;
        let search = self.shortcut_search.clone();
        let filter = self.shortcut_filter.trim().to_lowercase();
        for (index, row) in rows.iter().enumerate().filter(|(_, row)| {
            search.as_ref().is_none_or(|key| {
                key.is_empty() || canonical_chord(key) == canonical_chord(&row.key)
            })
        }) {
            let title = COMMANDS
                .iter()
                .find(|command| command.0 == row.action)
                .map(|command| tr(command.1))
                .unwrap_or(&row.action);
            let key_label = shortcut_label(&row.key);
            if !filter.is_empty()
                && !title.to_lowercase().contains(&filter)
                && !key_label.to_lowercase().contains(&filter)
                && !row.key.to_lowercase().contains(&filter)
            {
                continue;
            }
            visible_rows += 1;
            let supported = command_supported(&row.action);
            let selected = self
                .shortcut_editor
                .as_ref()
                .is_some_and(|editor| editor.index == Some(index));
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.), egui::Sense::hover());
            let response = ui.interact(
                rect,
                egui::Id::new(("shortcut-row", index)),
                egui::Sense::click(),
            );
            let response = if supported {
                response
            } else {
                response.on_hover_text(tr("unavailable-on-this-platform"))
            };
            if ui.is_rect_visible(rect) {
                if selected || response.hovered() || response.has_focus() {
                    ui.painter().rect_filled(
                        rect,
                        0.,
                        if selected {
                            Color32::from_rgb(29, 39, 23)
                        } else {
                            Color32::from_gray(28)
                        },
                    );
                }
                ui.painter().line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    egui::Stroke::new(1_f32, Color32::from_gray(36)),
                );
                let key_width = (rect.width() * 0.45).min(240.);
                let split = rect.right() - key_width - 10.;
                let title_rect = crate::ui::direction::mirror(
                    Rect::from_min_max(rect.min, egui::pos2(split - 8., rect.bottom())),
                    rect,
                );
                let key_rect = crate::ui::direction::mirror(
                    Rect::from_min_max(egui::pos2(split, rect.top()), rect.max),
                    rect,
                );
                let rtl = crate::i18n::is_rtl();
                ui.painter()
                    .with_clip_rect(ui.clip_rect().intersect(title_rect))
                    .text(
                        egui::pos2(
                            if rtl {
                                title_rect.right() - 10.
                            } else {
                                title_rect.left() + 10.
                            },
                            rect.center().y,
                        ),
                        if rtl {
                            egui::Align2::RIGHT_CENTER
                        } else {
                            egui::Align2::LEFT_CENTER
                        },
                        title,
                        FontId::proportional(14.),
                        if supported { INK } else { MUTED },
                    );
                ui.painter()
                    .with_clip_rect(ui.clip_rect().intersect(key_rect))
                    .text(
                        egui::pos2(
                            if rtl {
                                key_rect.left() + 10.
                            } else {
                                key_rect.right() - 10.
                            },
                            rect.center().y,
                        ),
                        if rtl {
                            egui::Align2::LEFT_CENTER
                        } else {
                            egui::Align2::RIGHT_CENTER
                        },
                        key_label,
                        FontId::proportional(14.),
                        MUTED,
                    );
            }
            if response.clicked() {
                self.shortcut_editor = Some(ShortcutEditor {
                    index: Some(index),
                    action: row.action.clone(),
                    key: row.key.clone(),
                    listening: false,
                    error: String::new(),
                });
            }
        }
        if visible_rows == 0 && !rows.is_empty() {
            ui.label(
                RichText::new(tr(if filter.is_empty() {
                    "no-shortcuts-match-this-key"
                } else {
                    "no-shortcuts-match-filter"
                }))
                .color(MUTED),
            );
        }
    }

    fn shortcut_reset_confirmation(&mut self, ctx: &egui::Context) {
        if !self.shortcut_reset_prompt {
            return;
        }
        let mut reset = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("shortcut-reset-confirmation"))
            .frame(
                egui::Frame::NONE
                    .fill(Color32::BLACK)
                    .inner_margin(18)
                    .corner_radius(4.),
            )
            .show(ctx, |ui| {
                forms::style(ui);
                ui.set_max_width((ctx.screen_rect().width() - 64.).clamp(240., 420.));
                ui.heading(tr("reset-shortcuts-confirm"));
                ui.add_space(8.);
                ui.label(tr("reset-shortcuts-warning"));
                ui.add_space(16.);
                ui.horizontal_wrapped(|ui| {
                    cancel = forms::button(ui, tr("cancel")).clicked();
                    reset = forms::button(ui, tr("restore-default")).clicked();
                });
            });
        if reset {
            match self.reset_shortcuts() {
                Ok(()) => {
                    self.shortcut_editor = None;
                    self.shortcut_search = None;
                    self.shortcut_filter.clear();
                    self.shortcut_filter_focused = false;
                    self.shortcut_reset_prompt = false;
                }
                Err(error) => {
                    self.last_error = error;
                    self.shortcut_reset_prompt = false;
                }
            }
        } else if cancel || response.should_close() {
            self.shortcut_reset_prompt = false;
        }
    }

    fn shortcut_editor_window(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.shortcut_editor.take() else {
            return;
        };
        let mut save = false;
        let mut delete = false;
        let mut cancel = false;
        // Escape is handled by keyboard(): record it for Close window, cancel
        // other captures, or dismiss an idle editor without closing settings.
        egui::Modal::new(egui::Id::new("shortcut-editor"))
            .frame(egui::Frame::NONE.fill(Color32::from_black_alpha(235)).inner_margin(18).corner_radius(4.))
            .show(ctx, |ui| {
            forms::style(ui);
            ui.set_width((ctx.screen_rect().width() - 64.).clamp(240., 420.));
            ui.heading(tr(if editor.index.is_some() { "edit-shortcut" } else { "add-shortcut" }));
            ui.add_space(12.);
            ui.label(tr("shortcut-key"));
            let key_label = if editor.listening {
                tr("press-a-key-dialog").to_owned()
            } else if editor.key.is_empty() {
                tr("record-shortcut").to_owned()
            } else {
                shortcut_label(&editor.key)
            };
            if ui.push_id("shortcut-capture", |ui| {
                ui.add_sized(
                    [ui.available_width(), forms::height()],
                    egui::Button::new(key_label).selected(editor.listening).truncate(),
                )
            }).inner.clicked() {
                editor.listening = !editor.listening;
                editor.error.clear();
            }
            ui.label(RichText::new(tr("esc-can-be-assigned-to-close-window-for-other-actions-it-cancels-key-recording")).small().color(MUTED));
            ui.add_space(12.);
            ui.label(tr("shortcut-action"));
            let action_label = COMMANDS.iter().find(|command| command.0 == editor.action)
                .map(|command| tr(command.1)).unwrap_or(&editor.action).to_owned();
            let action_response = egui::ComboBox::from_id_salt("shortcut-action")
                .width(ui.available_width())
                .truncate()
                .selected_text(action_label)
                .show_ui(ui, |ui| {
                    for &(id, title, _) in COMMANDS.iter().filter(|command| command_supported(command.0)) {
                        if ui.selectable_value(&mut editor.action, id.to_owned(), tr(title)).changed() {
                            editor.error.clear();
                        }
                    }
                });
            if action_response.response.clicked() {
                editor.listening = false;
            }
            let validation_error = if editor.key.is_empty() {
                None
            } else {
                let mut rows = self.shortcut_rows();
                let row = crate::core::settings::KeyboardShortcut {
                    action: editor.action.clone(), key: editor.key.clone(),
                };
                if let Some(index) = editor.index.filter(|&index| index < rows.len()) {
                    rows[index] = row;
                } else {
                    rows.push(row);
                }
                let mut candidate = self.settings.clone();
                candidate.shortcuts.clear();
                candidate.keyboard_shortcuts = Some(rows);
                validate_keys(&candidate).err()
            };
            let error = validation_error.as_deref().unwrap_or(&editor.error);
            if !error.is_empty() {
                ui.add_space(8.);
                ui.colored_label(Color32::LIGHT_RED, crate::i18n::value_label("error-details", error));
            }
            ui.add_space(16.);
            ui.horizontal_wrapped(|ui| {
                save = ui.push_id("shortcut-save", |ui| {
                    ui.add_enabled_ui(
                        !editor.key.is_empty() && !editor.listening && validation_error.is_none(),
                        |ui| forms::button(ui, tr("save-shortcut")),
                    ).inner
                }).inner.clicked();
                if editor.index.is_some() {
                    delete = ui.push_id("shortcut-delete", |ui| forms::button(ui, tr("delete-shortcut"))).inner.clicked();
                }
                cancel = ui.push_id("shortcut-cancel", |ui| forms::button(ui, tr("cancel"))).inner.clicked();
            });
        });
        if cancel {
            return;
        }
        if save || delete {
            let mut rows = self.shortcut_rows();
            if let Some(index) = editor.index {
                if index >= rows.len() {
                    editor.error =
                        tr("the-shortcut-list-changed-cancel-and-choose-the-row-again").into();
                    self.shortcut_editor = Some(editor);
                    return;
                }
                if delete {
                    rows.remove(index);
                } else {
                    rows[index] = crate::core::settings::KeyboardShortcut {
                        action: editor.action.clone(),
                        key: editor.key.clone(),
                    };
                }
            } else if save {
                rows.push(crate::core::settings::KeyboardShortcut {
                    action: editor.action.clone(),
                    key: editor.key.clone(),
                });
            }
            match self.save_shortcut_rows(rows) {
                Ok(()) => {
                    self.last_error.clear();
                    return;
                }
                Err(error) => editor.error = error,
            }
        }
        self.shortcut_editor = Some(editor);
    }

    fn transfer_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        settings_hint(ui, "includes-preferences-and-shortcuts");
        settings_row(ui, tr("export-settings"), 160., |ui| {
            if forms::button(ui, tr("export-json-dialog")).clicked() {
                self.run("export", ctx);
            }
        });
        settings_row(ui, tr("import-settings"), 160., |ui| {
            if forms::button(ui, tr("import-json-dialog")).clicked() {
                self.run("import", ctx);
            }
        });
    }
}

pub(super) fn settings_row(
    ui: &mut egui::Ui,
    label: &str,
    controls_width: f32,
    controls: impl FnOnce(&mut egui::Ui),
) {
    let width = ui.available_width();
    if width < controls_width + 150. {
        ui.add(egui::Label::new(RichText::new(label).color(INK)).wrap());
        ui.horizontal(|ui| {
            ui.with_layout(crate::ui::direction::trailing(), controls);
        });
    } else {
        crate::ui::direction::horizontal(ui, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(
                    (width - controls_width - forms::GAP).max(40.),
                    forms::height(),
                ),
                crate::ui::direction::flow(),
                |ui| {
                    ui.add(egui::Label::new(RichText::new(label).color(INK)).wrap());
                },
            );
            ui.with_layout(crate::ui::direction::trailing(), controls);
        });
    }
    ui.separator();
}

fn settings_hint(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(tr(text)).small().color(MUTED)).wrap());
}
