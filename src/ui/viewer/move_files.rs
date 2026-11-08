//! Destination picker and machine-local move history.
use super::*;

pub(super) struct MoveDialog {
    pub paths: Vec<PathBuf>,
    text: String,
    exact_path: Option<PathBuf>,
    selected: Option<usize>,
    focus_input: bool,
    pub error: String,
}

impl MoveDialog {
    fn select(&mut self, path: &Path, index: Option<usize>) {
        self.text = path.display().to_string();
        self.exact_path = Some(path.to_owned());
        self.selected = index;
    }

    fn destination(&self) -> PathBuf {
        if let Some(path) = &self.exact_path {
            return path.clone();
        }
        let text = self.text.trim();
        if let Some(rest) = text.strip_prefix("~/")
            && let Some(home) = dirs::home_dir()
        {
            return home.join(rest);
        }
        PathBuf::from(text)
    }
}

impl Viewer {
    pub(super) fn reload_recent_moves(&mut self) {
        if let Some(store) = &self.store {
            match store.recent_moves(self.settings.recent_move_limit) {
                Ok(paths) => self.recent_moves = paths,
                Err(error) => self.last_error = error,
            }
        }
    }

    pub(super) fn record_move_destination(&mut self, path: &Path) {
        if let Some(store) = &self.store {
            if let Err(error) = store.record_move(path, self.settings.recent_move_limit) {
                self.last_error = error;
            }
            self.reload_recent_moves();
        } else {
            self.recent_moves.retain(|item| item != path);
            self.recent_moves.insert(0, path.to_owned());
            self.recent_moves.truncate(self.settings.recent_move_limit);
        }
    }

    fn remove_move_history(&mut self, path: Option<&Path>) {
        if let Some(store) = &self.store
            && let Err(error) = store.remove_recent_move(path)
        {
            self.last_error = error;
            return;
        }
        self.recent_moves
            .retain(|item| path.is_some_and(|path| item != path));
    }

    pub(super) fn open_move_dialog(&mut self, ctx: &egui::Context) {
        if self.file_job.is_some() {
            self.show_toast(ctx, tr("a-file-operation-is-still-in-progress"));
            return;
        }
        let paths = self.file_targets();
        if paths.is_empty() {
            return;
        }
        self.reload_recent_moves();
        self.move_dialog = Some(MoveDialog {
            paths,
            text: String::new(),
            exact_path: None,
            selected: None,
            focus_input: true,
            error: String::new(),
        });
        self.menu_open = false;
        self.recent_open = false;
        self.zoom_input = None;
        self.held_key = None;
        self.held_button = None;
        self.canvas_drag = None;
    }

    pub(super) fn move_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.move_dialog.take() else {
            return;
        };
        let busy = self.file_job.is_some();
        let mut submit = false;
        let mut close = false;
        let mut remove = None;
        let mut clear = false;
        // Consume navigation before TextEdit and button focus traversal see it.
        let navigation = ctx.input_mut(|input| {
            let up = input.consume_key(egui::Modifiers::NONE, Key::ArrowUp);
            let down = input.consume_key(egui::Modifiers::NONE, Key::ArrowDown);
            let enter = input.consume_key(egui::Modifiers::NONE, Key::Enter);
            (up, down, enter)
        });
        if !busy {
            if (navigation.0 || navigation.1) && !self.recent_moves.is_empty() {
                let count = self.recent_moves.len();
                let index = match dialog.selected {
                    Some(index) if navigation.0 => (index + count - 1) % count,
                    Some(index) => (index + 1) % count,
                    None if navigation.0 => count - 1,
                    None => 0,
                };
                dialog.select(&self.recent_moves[index], Some(index));
            }
            submit = navigation.2 && !dialog.text.trim().is_empty();
        }
        let modal = overlay::show(ctx, "move-images", self.fullscreen, |ui| {
            close = overlay::header(ui, "move-images", "move-close");
            ui.label(tr("move-image-count").replace("{count}", &dialog.paths.len().to_string()));
            ui.add_space(forms::GAP);
            ui.add_enabled_ui(!busy, |ui| {
                // Paths remain LTR, including in RTL locales.
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), forms::height()),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if forms::icon(ui, "move-browse", "choose-move-folder").clicked() {
                            #[cfg(not(any(target_os = "ios", target_os = "android")))]
                            {
                                let mut picker = rfd::FileDialog::new();
                                let destination = dialog.destination();
                                if destination.is_dir() {
                                    picker = picker.set_directory(destination);
                                } else if let Some(folder) = &self.folder {
                                    picker = picker.set_directory(folder);
                                }
                                if let Some(path) = picker.pick_folder() {
                                    dialog.select(&path, None);
                                }
                            }
                        }
                        let button_width = ui
                            .painter()
                            .layout_no_wrap(
                                tr("move-images").into(),
                                egui::TextStyle::Button.resolve(ui.style()),
                                INK,
                            )
                            .size()
                            .x
                            + 20.;
                        let button_width = button_width.max(72.).min(ui.available_width() * 0.45);
                        let width =
                            (ui.available_width() - button_width - ui.spacing().item_spacing.x)
                                .max(20.);
                        let response = ui.add_sized(
                            [width, forms::height()],
                            forms::text_edit(&mut dialog.text)
                                .id(egui::Id::new("move-path"))
                                .hint_text(tr("destination-path")),
                        );
                        if dialog.focus_input {
                            response.request_focus();
                            dialog.focus_input = false;
                        }
                        if response.changed() {
                            dialog.exact_path = None;
                            dialog.selected = None;
                            dialog.error.clear();
                        }
                        submit |= ui
                            .add_enabled_ui(!dialog.text.trim().is_empty(), |ui| {
                                ui.add_sized(
                                    [button_width, forms::height()],
                                    egui::Button::new(tr("move-images")).truncate(),
                                )
                            })
                            .inner
                            .clicked();
                    },
                );
                ui.add_space(forms::GAP);
                crate::ui::direction::horizontal(ui, |ui| {
                    let label_width = (ui.available_width() - forms::height() - forms::GAP).max(1.);
                    ui.add_sized(
                        [label_width, forms::height()],
                        egui::Label::new(tr("recent-move-paths"))
                            .halign(crate::ui::direction::leading())
                            .truncate(),
                    );
                    ui.with_layout(crate::ui::direction::trailing(), |ui| {
                        clear =
                            forms::icon(ui, "move-history-clear", "clear-move-history").clicked();
                    });
                });
                egui::ScrollArea::vertical()
                    .id_salt("move-history-scroll")
                    .max_height(
                        (ui.available_height()
                            - if busy || !dialog.error.is_empty() {
                                80.
                            } else {
                                0.
                            })
                        .max(1.),
                    )
                    .auto_shrink([false, true])
                    .drag_to_scroll(false)
                    .show(ui, |ui| {
                        for (index, path) in self.recent_moves.iter().enumerate() {
                            ui.push_id(("move-history", path), |ui| {
                                crate::ui::direction::horizontal(ui, |ui| {
                                    let width =
                                        (ui.available_width() - forms::height() - forms::GAP)
                                            .max(1.);
                                    let selected = dialog.selected == Some(index);
                                    let response = ui.add_sized(
                                        [width, forms::height()],
                                        egui::Button::new(path.display().to_string())
                                            .selected(selected)
                                            .truncate(),
                                    );
                                    if selected && (navigation.0 || navigation.1) {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                    }
                                    if response.clicked() {
                                        dialog.select(path, Some(index));
                                    }
                                    response.on_hover_text(path.display().to_string());
                                    let (_, rect) = ui.allocate_space(Vec2::splat(forms::height()));
                                    let response = ui.interact(
                                        rect,
                                        egui::Id::new(("move-history-remove", path)),
                                        egui::Sense::click(),
                                    );
                                    if response.hovered() || response.has_focus() {
                                        ui.painter().rect_filled(rect, 5., Color32::from_gray(48));
                                    }
                                    chrome_icons::paint(
                                        ui.painter(),
                                        rect.center(),
                                        "trash",
                                        MUTED,
                                        false,
                                    );
                                    if response.on_hover_text(tr("remove-move-path")).clicked() {
                                        remove = Some(path.clone());
                                    }
                                });
                            });
                        }
                    });
            });
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("moving-images"));
                });
            }
            if !dialog.error.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt("move-errors")
                    .max_height(70.)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&dialog.error)
                                .color(Color32::LIGHT_RED)
                                .small(),
                        );
                    });
            }
        });
        if clear || remove.is_some() {
            self.remove_move_history(remove.as_deref().filter(|_| !clear));
            dialog.selected = None;
        }
        if (close || modal.should_close()) && !busy {
            ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::new("move-path")));
            self.held_key = None;
            return;
        }
        let destination = dialog.destination();
        let paths = dialog.paths.clone();
        self.move_dialog = Some(dialog);
        if submit && !busy {
            if !destination.is_absolute() {
                self.move_dialog.as_mut().unwrap().error = tr("choose-absolute-move-path").into();
            } else {
                self.start_move(paths, destination, ctx);
            }
        }
    }

    pub(super) fn move_history_settings(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.);
        ui.heading(tr("recent-move-paths"));
        let before = self.settings.recent_move_limit;
        settings::settings_row(ui, tr("maximum-recent-moves"), 230., |ui| {
            if forms::icon(ui, "move-history-clear", "clear-move-history").clicked() {
                self.remove_move_history(None);
            }
            if forms::default_button(ui).clicked() {
                self.settings.recent_move_limit = Settings::default().recent_move_limit;
            }
            forms::number(
                ui,
                egui::DragValue::new(&mut self.settings.recent_move_limit).range(1..=500),
            );
        });
        if before != self.settings.recent_move_limit {
            self.save();
            if let Some(store) = &self.store
                && let Err(error) = store.trim_recent_moves(self.settings.recent_move_limit)
            {
                self.last_error = error;
            }
            self.recent_moves.truncate(self.settings.recent_move_limit);
        }
    }
}
