use super::*;

impl Viewer {
    #[cfg(all(target_os = "macos", not(test)))]
    fn pick_mac_source(&mut self, folder: bool) {
        let initial = self
            .archive_source()
            .or(self.files.get(self.focus_index).map(PathBuf::as_path))
            .or(self.folder.as_deref());
        let supported = [media::EXTENSIONS, crate::core::archive::EXTENSIONS].concat();
        match crate::platform::mac_access::select(folder, initial, &supported) {
            Ok(Some(selection)) => {
                for access in selection.access {
                    if selection.warning.is_some() {
                        // A full/unwritable permission registry must not reject
                        // this selection. Keep its original Powerbox URL in this
                        // process instead of sending a plain path to a new one.
                        let previous = self.mac_access.take();
                        self.open(access.path.clone());
                        if self.mac_access.is_some() {
                            self.mac_access = Some(access);
                        } else {
                            self.mac_access = previous;
                        }
                    } else {
                        self.open_from_user(access.path.clone());
                    }
                }
                if let Some(error) = selection.warning {
                    self.last_error = error;
                }
            }
            Ok(None) => {}
            Err(error) => self.last_error = error,
        }
    }
    pub(super) fn run(&mut self, id: &str, ctx: &egui::Context) {
        // Portrait displays always show one page; their layout cannot change.
        if !command_supported(id) || (!self.landscape && is_layout_command(id)) {
            return;
        }
        if self.blocked_in_archive(ctx, id) {
            return;
        }
        match id {
            "check" => self.toggle_focused_checks(ctx),
            "check-prev" | "check-next" => {
                if self.listing.is_some() {
                    // Toggle only when this queued step is replayed. Applying it
                    // now as well would undo it after the folder finishes loading.
                    if self.pending_steps.len() < MAX_PENDING_STEPS {
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
            "reveal" => self.reveal_current(ctx),
            "share" => {
                #[cfg(any(target_os = "ios", target_os = "android"))]
                if let Some(path) = self.files.get(self.focus_index) {
                    crate::platform::mobile::share(path);
                }
                // The desktop phone preview has no share sheet; show the file instead.
                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                if crate::platform::reveal_file::supported() {
                    self.reveal_current(ctx);
                }
            }
            "orientation"
            | "orientation-auto"
            | "orientation-portrait"
            | "orientation-landscape" => {
                let mode = id.strip_prefix("orientation-").unwrap_or(
                    match self.settings.orientation.as_str() {
                        "auto" => "portrait",
                        "portrait" => "landscape",
                        _ => "auto",
                    },
                );
                self.settings.orientation = mode.into();
                self.save();
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
                #[cfg(all(target_os = "macos", not(test)))]
                self.pick_mac_source(false);
                #[cfg(not(any(
                    target_os = "ios",
                    target_os = "android",
                    all(target_os = "macos", not(test))
                )))]
                {
                    let archives = crate::core::archive::EXTENSIONS;
                    let supported = [media::EXTENSIONS, archives].concat();
                    let mut dialog = rfd::FileDialog::new()
                        .add_filter(tr("images-and-archives"), &supported)
                        .add_filter(tr("images"), media::EXTENSIONS)
                        .add_filter(tr("archives"), archives);
                    // Inside an archive the dialog starts at the archive file.
                    let shown = self
                        .archive_source()
                        .or(self.files.get(self.focus_index).map(PathBuf::as_path));
                    if let Some(path) = shown {
                        if let Some(folder) = path.parent() {
                            dialog = dialog.set_directory(folder);
                        }
                        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                            dialog = dialog.set_file_name(name);
                        }
                    } else if let Some(folder) = self.folder.as_deref().filter(|f| f.is_dir()) {
                        dialog = dialog.set_directory(folder);
                    }
                    if let Some(paths) = dialog.pick_files() {
                        for path in paths {
                            self.open_from_user(path);
                        }
                    }
                }
            }
            "folder" => {
                // Phones import a copy of the folder's images through the system picker.
                #[cfg(any(target_os = "ios", target_os = "android"))]
                crate::platform::mobile::pick_folder();
                #[cfg(all(target_os = "macos", not(test)))]
                self.pick_mac_source(true);
                #[cfg(not(any(
                    target_os = "ios",
                    target_os = "android",
                    all(target_os = "macos", not(test))
                )))]
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.open_from_user(p);
                }
            }
            "recent" => {
                self.library.open = !self.library.open;
                if self.library.open {
                    // Show the page in view as this folder's thumbnail.
                    self.remember_page();
                    self.reload_library();
                }
            }
            "bookmark" => self.open_bookmark_dialog(),
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
                    self.release_held_input();
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
                    self.settings.slide_ms.saturating_sub(limits::TIME_STEP_MS)
                } else {
                    self.settings.slide_ms.saturating_add(limits::TIME_STEP_MS)
                }
                .clamp(*limits::TIME_MS.start(), *limits::TIME_MS.end());
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
            "in" => self.change_zoom(ctx, ZOOM_STEP),
            "out" => self.change_zoom(ctx, 1. / ZOOM_STEP),
            "fullscreen" => {
                // Phones hide the system bars; the platform host applies it each frame.
                #[cfg(any(target_os = "ios", target_os = "android"))]
                {
                    self.fullscreen = !self.fullscreen;
                }
                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                {
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
                self.close_shortcut_editing();
            }
            "remember" => {
                self.settings.remember_recent = !self.settings.remember_recent;
                self.save();
            }
            "clear" => self.library.clear_prompt = true,
            "export" => {
                // Phones write a private copy, then let the user save it with the system picker.
                #[cfg(any(target_os = "ios", target_os = "android"))]
                if let Some(store) = &self.store {
                    match crate::platform::mobile::settings_export_path()
                        .and_then(|path| store.export(&path, &self.settings).map(|()| path))
                    {
                        Ok(path) => crate::platform::mobile::export_settings(&path),
                        Err(error) => self.last_error = error,
                    }
                }
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
                // Phones deliver the picked file asynchronously; see Viewer::update.
                #[cfg(any(target_os = "ios", target_os = "android"))]
                crate::platform::mobile::import_settings();
                #[cfg(not(any(target_os = "ios", target_os = "android")))]
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
                {
                    self.import_settings_from(&path);
                }
            }
            "about" | "licenses" => {
                self.menu_open = false;
                self.library.open = false;
                self.library.dialog = None;
                self.move_dialog = None;
                self.settings_open = true;
                self.settings_section = settings::ABOUT_SECTION;
                self.close_shortcut_editing();
                self.zoom_input = None;
            }
            _ => {}
        }
        self.show_action_toast(id, ctx);
    }

    fn reveal_current(&mut self, ctx: &egui::Context) {
        if self.reveal_job.is_none()
            && crate::platform::reveal_file::supported()
            // Extracted pages are private copies; show the archive itself.
            && let Some(path) = self
                .archive_source()
                .map(Path::to_owned)
                .or_else(|| self.files.get(self.focus_index).cloned())
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

    /// Collects the answer of the file manager opened by `reveal_current`.
    pub(super) fn poll_reveal(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(receiver) = &self.reveal_job else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(tr("file-manager-request-failed").into()),
        };
        self.reveal_job = None;
        if let Err(error) = result {
            self.last_error = error;
        }
    }

    pub(super) fn import_settings_from(&mut self, path: &Path) {
        let Some(store) = &self.store else { return };
        match store.import(path).and_then(|settings| {
            validate_keys(&settings)?;
            store.replace_from_import(&settings)?;
            Ok(settings)
        }) {
            Ok(settings) => self.replace_settings(settings),
            Err(error) => self.last_error = error,
        }
    }

    /// Switches to settings that replace the current ones: an import or a reset.
    pub(super) fn replace_settings(&mut self, settings: Settings) {
        let language_changed = self.settings.language != settings.language;
        self.settings = settings;
        self.release_held_input();
        self.close_shortcut_editing();
        if language_changed {
            crate::i18n::set_language(self.settings.language.clone());
            crate::platform::locale::set_application_language(&self.settings.language);
        }
        self.focus(self.focus_index);
    }
}
