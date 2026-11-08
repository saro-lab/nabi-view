//! Phone controls use the same viewer commands and image canvas as desktop.
use super::*;

impl Viewer {
    pub(super) fn mobile_chrome(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        let height = crate::ui::mobile::BAR_HEIGHT;
        egui::Area::new(egui::Id::new("mobile-top"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .movable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE.fill(BAR).inner_margin(4).show(ui, |ui| {
                    ui.set_width((screen.width() - 8.).max(0.));
                    ui.set_height(height - 8.);
                    crate::ui::direction::horizontal(ui, |ui| {
                        if chrome_icons::button_with_shortcut(
                            ui,
                            "open",
                            "open-images",
                            false,
                            &self.shortcut_text("open"),
                        )
                        .clicked()
                        {
                            self.run("open", ctx);
                        }
                        let title = self
                            .files
                            .get(self.focus_index)
                            .and_then(|p| p.file_name())
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "NABI VIEW".into());
                        let width = (ui.available_width() - 104.).max(0.);
                        ui.add_sized([width, 44.], egui::Label::new(title).truncate());
                        if chrome_icons::button(ui, "menu", "menu", self.menu_open).clicked() {
                            self.menu_open = !self.menu_open;
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
                    });
                });
            });
        egui::Area::new(egui::Id::new("mobile-bottom"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.left(), screen.bottom() - height))
            .movable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE.fill(BAR).inner_margin(4).show(ui, |ui| {
                    ui.set_width((screen.width() - 8.).max(0.));
                    ui.set_height(height - 8.);
                    ui.horizontal(|ui| {
                        if chrome_icons::tooltip(
                            ui.add_sized([44., 44.], egui::Button::new("‹")),
                            "previous",
                            &self.shortcut_text("prev"),
                        )
                        .clicked()
                        {
                            self.run("prev", ctx);
                        }
                        let label = if self.files.is_empty() {
                            "0 / 0".into()
                        } else {
                            format!("{} / {}", self.focus_index + 1, self.files.len())
                        };
                        ui.add_sized(
                            [(ui.available_width() - 156.).max(0.), 44.],
                            egui::Label::new(label).truncate(),
                        );
                        if chrome_icons::tooltip(
                            ui.add_sized([44., 44.], egui::Button::new("›")),
                            "next",
                            &self.shortcut_text("next"),
                        )
                        .clicked()
                        {
                            self.run("next", ctx);
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
                    });
                });
            });
    }

    pub(super) fn touch_canvas(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        reading: bool,
    ) {
        if reading
            || self.settings_open
            || self.menu_open
            || self.recent_open
            || self.move_dialog.is_some()
            || self.clear_prompt
            || self.random_prompt
            || self.zoom_input.is_some()
            || ctx.memory(|m| m.any_popup_open())
            || !ctx.input(|i| i.focused)
        {
            self.touch_gesture.cancel();
            return;
        }
        if ctx.input(|i| i.pointer.primary_pressed())
            && response.contains_pointer()
            && let Some(position) = ctx.pointer_interact_pos()
        {
            self.touch_gesture.start(position);
        }
        if !self.touch_gesture.active() {
            return;
        }
        if let Some(touch) = ctx
            .input(|i| i.multi_touch())
            .filter(|touch| touch.num_touches >= 2)
        {
            self.touch_gesture.multiple();
            if touch.zoom_delta.is_finite() && touch.zoom_delta > 0. {
                self.change_zoom(ctx, touch.zoom_delta);
                self.pan += touch.translation_delta;
            }
            return;
        }
        if response.dragged() && self.zoom_override.is_some() {
            self.pan += ctx.input(|i| i.pointer.delta());
        }
        if ctx.input(|i| i.pointer.primary_released()) {
            let multiple = self.touch_gesture.was_multiple();
            if let Some(end) = ctx.input(|i| i.pointer.latest_pos()) {
                if let Some(forward) =
                    self.touch_gesture
                        .finish(end, self.zoom_override.is_some(), self.settings.rtl)
                {
                    self.move_page(ctx, forward);
                } else if !multiple && response.double_clicked() {
                    self.run("fit", ctx);
                } else if !multiple && response.clicked() {
                    self.run("chrome", ctx);
                }
            }
            self.touch_gesture.cancel();
        }
    }
}
