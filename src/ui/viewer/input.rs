use super::*;

impl Viewer {
    pub(super) fn prepare_pointer_input(
        &mut self,
        ctx: &egui::Context,
        input: &mut egui::RawInput,
    ) {
        if crate::app::is_mobile() {
            return;
        }
        let interrupted = input.events.iter().rposition(|event| {
            matches!(
                event,
                egui::Event::PointerGone | egui::Event::WindowFocused(false)
            )
        });
        if !input.focused || ctx.input(|i| i.focused) != input.focused || interrupted.is_some() {
            self.canvas_drag = None;
            self.held_button = None;
            // PointerGone and focus loss do not release egui's held buttons.
            // Reset before processing re-entry movement, including a leave and
            // a new press arriving together in the same native event batch.
            let position = ctx.pointer_interact_pos().unwrap_or_default();
            let cutoff = if input.focused {
                interrupted.map_or(0, |index| index + 1)
            } else {
                input.events.len()
            };
            let pointer_gone = ctx.input(|i| i.pointer.latest_pos().is_none())
                || input.events[..cutoff]
                    .iter()
                    .any(|event| matches!(event, egui::Event::PointerGone));
            ctx.input_mut(|i| i.pointer = egui::PointerState::default());
            ctx.stop_dragging();
            let mut index = 0;
            input.events.retain(|event| {
                let stale = index < cutoff
                    && matches!(
                        event,
                        egui::Event::PointerMoved(_)
                            | egui::Event::PointerButton { .. }
                            | egui::Event::PointerGone
                            | egui::Event::MouseMoved(_)
                    );
                index += 1;
                !stale
            });
            // With the button state already cleared, this release cannot click.
            // It also clears egui's old potential click target before a new press.
            input.events.insert(
                0,
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            );
            if pointer_gone {
                input.events.insert(1, egui::Event::PointerGone);
            }
            window_frame::reset_drag(ctx);
        }
        if input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                }
            )
        }) {
            window_frame::reset_drag(ctx);
        }
    }

    /// The recent/bookmark screen covers the canvas. Clicks assigned to the
    /// commands its keyboard shortcuts also allow there (so the button that
    /// opened it closes it again) keep working over it.
    pub(super) fn library_mouse(&mut self, ctx: &egui::Context) {
        if crate::app::is_mobile()
            || !self.library.open
            || self.library.clear_prompt
            || self.library.dialog.is_some()
            || self.settings_open
            || self.menu_open
            || self.random_prompt
            || self.dialog_open()
            || self.popup_open_at_input
            || ctx.any_popup_open()
            || !ctx.input(|i| i.focused)
        {
            return;
        }
        let mouse = &self.settings.mouse;
        let action = [
            (egui::PointerButton::Secondary, &mouse.right_click),
            (egui::PointerButton::Middle, &mouse.middle_click),
            (egui::PointerButton::Extra1, &mouse.back_button),
            (egui::PointerButton::Extra2, &mouse.forward_button),
        ]
        .into_iter()
        .find(|(button, action)| {
            matches!(action.as_str(), "open" | "folder" | "recent")
                && command_supported(action)
                && ctx.input(|i| i.pointer.button_clicked(*button))
        })
        .map(|(_, action)| action.clone());
        if let Some(action) = action {
            self.run(&action, ctx);
        }
    }

    pub(super) fn shortcut_input_active(&self) -> bool {
        self.shortcut_editor.is_some()
            || self.shortcut_search.is_some()
            || self.shortcut_filter_focused
            || self.shortcut_reset_prompt
            || self.reset_prompt.is_some()
    }

    pub(super) fn sync_native_shortcut_capture(&self) {
        #[cfg(all(target_os = "macos", not(test)))]
        crate::platform::mac_open::set_shortcut_capture(self.shortcut_input_active());
    }

    pub(super) fn bindings(&self, id: &str) -> Vec<String> {
        effective_keys(&self.settings, id)
    }

    pub(super) fn shortcut_text(&self, id: &str) -> String {
        self.bindings(id)
            .iter()
            .map(|key| shortcut_label(key))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub(super) fn shortcut_rows(&self) -> Vec<KeyboardShortcut> {
        self.settings.keyboard_shortcuts.clone().unwrap_or_default()
    }

    pub(super) fn save_shortcut_rows(&mut self, rows: Vec<KeyboardShortcut>) -> Result<(), String> {
        let mut candidate = self.settings.clone();
        candidate.shortcuts.clear();
        candidate.keyboard_shortcuts = Some(rows);
        self.commit_shortcuts(candidate)
    }

    pub(super) fn reset_shortcuts(&mut self) -> Result<(), String> {
        let mut candidate = self.settings.clone();
        candidate.shortcuts.clear();
        candidate.keyboard_shortcuts = Some(default_shortcut_rows());
        self.commit_shortcuts(candidate)
    }

    pub(super) fn commit_shortcuts(&mut self, candidate: Settings) -> Result<(), String> {
        validate_keys(&candidate)?;
        if let Some(store) = &self.store {
            store.save_shortcuts(&candidate)?;
        }
        self.settings = candidate;
        self.held_key = None;
        self.last_error.clear();
        Ok(())
    }

    pub(super) fn keyboard(&mut self, ctx: &egui::Context) {
        let mut events = ctx.input(|i| i.events.clone());
        // Restore shortcut order for the app only, after egui's focus pass.
        for (at, tab) in self.tab_presses.drain(..) {
            events.insert(at.min(events.len()), tab);
        }
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if crate::core::commands::modifier_key_name(key.name()) {
                continue;
            }
            let chord = key_chord(key, modifiers);
            if self.shortcut_reset_prompt || self.reset_prompt.is_some() {
                if key == Key::Escape {
                    self.shortcut_reset_prompt = false;
                    self.reset_prompt = None;
                    ctx.input_mut(|i| i.consume_key(modifiers, key));
                }
                continue;
            }
            if let Some(editor) = self.shortcut_editor.as_mut().filter(|e| e.listening) {
                if key == Key::Escape && editor.action != "close-window" {
                    editor.listening = false;
                } else {
                    editor.key = chord;
                    editor.listening = false;
                    editor.error.clear();
                }
                ctx.input_mut(|i| i.consume_key(modifiers, key));
                continue;
            }
            if self
                .shortcut_search
                .as_ref()
                .is_some_and(|search| search.is_empty())
            {
                // Escape is a searchable registered key too; another Escape clears the filter.
                self.shortcut_search = Some(chord);
                ctx.input_mut(|i| i.consume_key(modifiers, key));
                continue;
            }
            if self.bindings("close-all").contains(&chord) {
                ctx.input_mut(|i| i.consume_key(modifiers, key));
                if !self.shortcut_input_active() {
                    self.run("close-all", ctx);
                }
                continue;
            }
            if self.dialog_open() || self.library.dialog.is_some() || self.zoom_input.is_some() {
                self.release_held_input();
                continue;
            }
            if self.inspector.filter_focused(ctx) {
                // Keys go to the filter text, except Escape, which clears it
                // and then releases focus so later keys are shortcuts again.
                if key == Key::Escape {
                    self.inspector.escape_focused_filter(ctx);
                    ctx.input_mut(|i| i.consume_key(modifiers, key));
                }
                self.release_held_input();
                continue;
            }
            if key == Key::Escape {
                // A popup may already have consumed Escape during egui's focus pass.
                // Preserve its input-time state so this same press cannot close the viewer.
                let handled = if self.popup_open_at_input || ctx.any_popup_open() {
                    egui::Popup::close_all(ctx);
                    self.menu_open = false;
                    true
                } else if self.menu_open {
                    self.menu_open = false;
                    true
                } else if self.random_prompt {
                    self.random_prompt = false;
                    true
                } else if self.library.clear_prompt {
                    self.library.clear_prompt = false;
                    true
                } else if self.library.open {
                    self.library.open = false;
                    true
                } else if self.settings_open {
                    if self.shortcut_editor.take().is_none()
                        && self.shortcut_search.take().is_none()
                    {
                        if self.shortcut_filter.is_empty() {
                            self.settings_open = false;
                            self.shortcut_filter_focused = false;
                        } else {
                            self.shortcut_filter.clear();
                        }
                    }
                    true
                } else {
                    false
                };
                if handled {
                    ctx.input_mut(|i| i.consume_key(modifiers, key));
                    self.held_key = None;
                    continue;
                }
            }
            if self.library.open
                && !self.settings_open
                && !self.menu_open
                && !self.library.clear_prompt
                && !self.random_prompt
                && let Some(id) = ["open", "folder", "recent"]
                    .into_iter()
                    .find(|id| command_supported(id) && self.bindings(id).contains(&chord))
            {
                ctx.input_mut(|i| i.consume_key(modifiers, key));
                self.run(id, ctx);
                continue;
            }
            if self.settings_open
                && !self.shortcut_input_active()
                && !self.library.modal()
                && !self.random_prompt
                && !self.popup_open_at_input
                && !ctx.any_popup_open()
                && self.bindings("about").contains(&chord)
            {
                ctx.input_mut(|i| i.consume_key(modifiers, key));
                self.run("about", ctx);
                continue;
            }
            if self.settings_open
                || self.overlay_open()
                || self.popup_open_at_input
                || ctx.any_popup_open()
            {
                continue;
            }
            if let Some((id, _, _)) = COMMANDS
                .iter()
                .find(|c| self.bindings(c.0).contains(&chord))
            {
                ctx.input_mut(|i| i.consume_key(modifiers, key));
                self.run(id, ctx);
                if matches!(
                    *id,
                    "prev"
                        | "next"
                        | "check-prev"
                        | "check-next"
                        | "slideshow-decrease"
                        | "slideshow-increase"
                ) {
                    self.held_key = Some((
                        key,
                        *id,
                        ctx.input(|i| i.time) + self.settings.key_delay_ms as f64 / 1000.,
                    ));
                }
            }
        }
        let blocked = self.settings_open
            || self.overlay_open()
            || self.zoom_input.is_some()
            || self.inspector.filter_focused(ctx)
            || self.popup_open_at_input
            || ctx.any_popup_open()
            || self.shortcut_editor.is_some()
            || !ctx.input(|i| i.focused);
        if let Some((key, action, next)) = self.held_key {
            let (down, chord, now) =
                ctx.input(|i| (i.key_down(key), key_chord(key, i.modifiers), i.time));
            if blocked || !down || !self.bindings(action).contains(&chord) {
                self.held_key = None;
            } else {
                if now >= next {
                    self.run(action, ctx);
                    self.held_key = Some((
                        key,
                        action,
                        now + self.settings.key_repeat_ms as f64 / 1000.,
                    ));
                }
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    (self.held_key.unwrap().2 - now).max(0.001),
                ));
            }
        }
        if blocked || ctx.input(|i| !i.pointer.primary_down() && !i.pointer.any_released()) {
            self.held_button = None;
        }
    }
}
