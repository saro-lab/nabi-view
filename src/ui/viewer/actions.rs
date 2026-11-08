use super::*;

impl Viewer {
    pub(super) fn run(&mut self, id: &str, ctx: &egui::Context) {
        if !command_supported(id) {
            return;
        }
        match id {
            "check" => self.toggle_focused_checks(ctx),
            "check-prev" | "check-next" => {
                if self.listing.is_some() {
                    // Toggle only when this queued step is replayed. Applying it
                    // now as well would undo it after the folder finishes loading.
                    if self.pending_steps.len() < i16::MAX as usize {
                        self.pending_steps.push((id == "check-next", !self.random));
                    }
                } else {
                    if !self.random {
                        self.toggle_focused_checks(ctx);
                    }
                    self.move_page(ctx, id == "check-next");
                }
                if self.random {
                    self.show_toast(ctx, tr("checks-are-not-available-in-random-mode"));
                }
            }
            "move" => self.open_move_dialog(ctx),
            "trash" => self.trash_files(ctx),
            "undo-trash" => self.undo_trash(ctx),
            "metadata" => {
                if self.inspector.open {
                    self.inspector.close();
                } else {
                    self.inspector.open = true;
                }
                self.hide_controls = !self.inspector.open;
                self.hover_until = 0.;
            }
            "new-window" => self.new_window(None),
            "reveal" => {
                if self.reveal_job.is_none()
                    && command_supported("reveal")
                    && let Some(path) = self.files.get(self.focus_index).cloned()
                {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    let context = ctx.clone();
                    match std::thread::Builder::new()
                        .name("file-manager".into())
                        .spawn(move || {
                            let _ = sender.send(crate::platform::reveal_file::reveal(&path));
                            context.request_repaint();
                        }) {
                        Ok(_) => self.reveal_job = Some(receiver),
                        Err(error) => self.last_error = error.to_string(),
                    }
                }
            }
            "close-window" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "close-all" => {
                let result = self
                    .close_signal
                    .as_mut()
                    .ok_or_else(|| {
                        "App state is unavailable; cannot close other windows.".to_owned()
                    })
                    .and_then(|signal| signal.broadcast());
                match result {
                    Ok(()) => self.request_graceful_close(ctx),
                    Err(error) => self.last_error = error,
                }
            }
            "minimize" => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            "maximize" => {
                if self.fullscreen {
                    return;
                }
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            "open" => {
                #[cfg(any(target_os = "ios", target_os = "android"))]
                crate::platform::mobile::pick();
                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                {
                    let mut dialog =
                        rfd::FileDialog::new().add_filter(tr("images"), media::EXTENSIONS);
                    if let Some(path) = self.files.get(self.focus_index) {
                        if let Some(folder) = path.parent() {
                            dialog = dialog.set_directory(folder);
                        }
                        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                            dialog = dialog.set_file_name(name);
                        }
                    } else if let Some(folder) = &self.folder {
                        dialog = dialog.set_directory(folder);
                    }
                    if let Some(paths) = dialog.pick_files() {
                        for path in paths {
                            self.open_from_user(path);
                        }
                    }
                }
            }
            "folder" =>
            {
                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.open_from_user(p);
                }
            }
            "recent" => {
                self.recent_open = !self.recent_open;
                if self.recent_open {
                    self.reload_recent();
                }
            }
            "prev" => self.move_page(ctx, false),
            "next" => self.move_page(ctx, true),
            "first" => self.focus_navigation(ctx, 0),
            "last" => self.focus_navigation(ctx, self.files.len().saturating_sub(1)),
            "single" | "spread-left" | "spread-right" => {
                self.settings.spread = id != "single";
                self.settings.rtl = id == "spread-left";
                self.pan = Vec2::ZERO;
                self.save();
            }
            "flip-horizontal" => {
                self.settings.flip_horizontal = !self.settings.flip_horizontal;
                self.save();
            }
            "flip-vertical" => {
                self.settings.flip_vertical = !self.settings.flip_vertical;
                self.save();
            }
            "spread" => {
                if !self.settings.spread {
                    self.settings.spread = true;
                    self.settings.rtl = false;
                } else if !self.settings.rtl {
                    self.settings.rtl = true;
                } else {
                    self.settings.spread = false;
                    self.settings.rtl = false;
                }
                self.pan = Vec2::ZERO;
                self.save();
            }
            "rtl" => {
                self.settings.rtl = !self.settings.rtl;
                self.save();
            }
            "cover" => {
                self.settings.cover = !self.settings.cover;
                self.save();
            }
            "random" => {
                if !self.random && !self.checked.is_empty() {
                    self.random_prompt = true;
                    self.held_key = None;
                    self.held_button = None;
                } else {
                    self.toggle_random();
                }
            }
            "slideshow" => {
                self.slideshow = !self.slideshow;
                self.slide_deadline = None;
            }
            "slideshow-decrease" | "slideshow-increase" => {
                let interval = if id == "slideshow-decrease" {
                    self.settings.slide_ms.saturating_sub(100).max(100)
                } else {
                    self.settings.slide_ms.saturating_add(100).min(100_000)
                };
                if interval != self.settings.slide_ms {
                    self.settings.slide_ms = interval;
                    self.slide_deadline = None;
                    self.save();
                }
                self.show_slide_interval_toast(ctx);
            }
            "fit" | "fill" | "actual" | "width" | "height" => {
                self.settings.view_mode = id.to_owned();
                self.zoom = 1.0;
                self.zoom_override = None;
                self.zoom_input = None;
                self.pan = Vec2::ZERO;
                self.save();
            }
            "in" => {
                self.change_zoom(ctx, 1.25);
            }
            "out" => {
                self.change_zoom(ctx, 1. / 1.25);
            }
            "fullscreen" => {
                self.fullscreen =
                    !ctx.input(|i| i.viewport().fullscreen.unwrap_or(self.fullscreen));
                #[cfg(all(target_os = "macos", not(test)))]
                {
                    self.fullscreen = crate::platform::mac_open::fullscreen(
                        self.fullscreen,
                        self.settings.blackout_other_monitors,
                    );
                }
                #[cfg(any(not(target_os = "macos"), test))]
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            "chrome" => {
                self.hide_controls = !self.hide_controls;
                self.hover_until = 0.;
                if self.hide_controls {
                    self.inspector.close();
                }
            }
            "settings" => {
                self.settings_open = !self.settings_open;
                self.shortcut_editor = None;
                self.shortcut_search = None;
                self.shortcut_filter.clear();
                self.shortcut_filter_focused = false;
                self.shortcut_reset_prompt = false;
            }
            "remember" => {
                self.settings.remember_recent = !self.settings.remember_recent;
                self.save();
            }
            "clear" => self.clear_prompt = true,
            "export" => {
                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("nabi-view-settings.json")
                    .add_filter("JSON", &["json"])
                    .save_file()
                    && let Some(store) = &self.store
                    && let Err(e) = store.export(&path, &self.settings)
                {
                    self.last_error = e;
                }
            }
            "import" => {
                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
                    && let Some(store) = &self.store
                {
                    match store.import(&path).and_then(|s| {
                        validate_keys(&s)?;
                        store.replace_from_import(&s)?;
                        Ok(s)
                    }) {
                        Ok(s) => {
                            self.settings = s;
                            self.focus(self.focus_index);
                        }
                        Err(e) => self.last_error = e,
                    }
                }
            }
            "about" | "licenses" => {
                self.menu_open = false;
                self.recent_open = false;
                self.move_dialog = None;
                self.settings_open = true;
                self.settings_section = settings::ABOUT_SECTION;
                self.shortcut_editor = None;
                self.shortcut_search = None;
                self.shortcut_filter.clear();
                self.shortcut_filter_focused = false;
                self.shortcut_reset_prompt = false;
                self.zoom_input = None;
            }
            _ => {}
        }
        self.show_action_toast(id, ctx);
    }
}
