use super::*;

impl Viewer {
    pub(super) fn image_scale(&self, size: Vec2, slot: Vec2, ppp: f32) -> f32 {
        if let Some(zoom) = self.zoom_override {
            return zoom / ppp;
        }
        let x = slot.x / size.x;
        let y = slot.y / size.y;
        let factor = match self.settings.view_mode.as_str() {
            "actual" => return self.zoom / ppp,
            "fill" => x.max(y),
            "width" => x,
            "height" => y,
            _ => x.min(y),
        };
        factor.min(self.settings.small_image_percent as f32 / 100. / ppp) * self.zoom
    }

    pub(super) fn image_canvas(&self, mut screen: Rect) -> Rect {
        // Hover reveals an overlay; explicit pin/Tab reserves space for chrome.
        if !self.hide_controls && !self.settings_open {
            screen.min.y = (screen.top()
                + if crate::app::is_mobile() {
                    crate::ui::mobile::BAR_HEIGHT
                } else {
                    TOP_BAR_HEIGHT
                })
            .min(screen.bottom());
            screen.max.y = (screen.bottom()
                - if crate::app::is_mobile() {
                    crate::ui::mobile::BAR_HEIGHT
                } else {
                    BOTTOM_CHROME_HEIGHT
                })
            .max(screen.top());
        }
        screen
    }

    pub(super) fn zoom_percent(&self, ctx: &egui::Context) -> f32 {
        if let Some(zoom) = self.zoom_override {
            return zoom * 100.;
        }
        let Some(picture) = self
            .files
            .get(self.focus_index)
            .and_then(|p| self.pictures.get(p))
        else {
            return self.zoom * 100.;
        };
        let (a, b) = self.focus_span();
        let mut slot = self.image_canvas(ctx.screen_rect()).size();
        if a != b {
            slot.x /= 2.;
        }
        self.image_scale(
            egui::vec2(picture.original[0] as f32, picture.original[1] as f32),
            slot,
            ctx.pixels_per_point(),
        ) * ctx.pixels_per_point()
            * 100.
    }

    pub(super) fn change_zoom(&mut self, ctx: &egui::Context, factor: f32) {
        let percent = (self.zoom_percent(ctx) * factor).clamp(10., 3000.);
        self.zoom_override = Some(percent / 100.);
    }

    pub(super) fn wheel_zoom(&mut self, ctx: &egui::Context, up: bool) -> bool {
        let before = self.zoom_percent(ctx);
        let next = (before * if up { 1.25 } else { 1. / 1.25 }).clamp(10., 3000.);
        let stopped = (before < 100. && next >= 100.) || (before > 100. && next <= 100.);
        self.zoom_override = Some(if stopped { 1. } else { next / 100. });
        ctx.request_repaint();
        stopped
    }

    pub(super) fn canvas(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::BLACK))
            .show(ctx, |ui| {
                let canvas = self.image_canvas(ui.available_rect_before_wrap());
                let response = ui.allocate_rect(canvas, egui::Sense::click_and_drag());
                if self.files.is_empty() {
                    self.canvas_pointer(ctx, &response, false);
                    return;
                }
                let ppp = ctx.pixels_per_point();
                // Bucket requests so resizing the window a few pixels does not restart the decoder.
                self.fit_limit =
                    ((((canvas.width().max(canvas.height()) * ppp) as u32).div_ceil(512)) * 512)
                        .clamp(1024, 4096);
                let (first, last) = self.focus_span();
                let mut ids = vec![first];
                if first != last {
                    ids.push(last);
                    if self.settings.rtl {
                        ids.reverse();
                    }
                }
                let pending = ids.iter().any(|&index| {
                    let path = &self.files[index];
                    !self.pictures.contains_key(path) && !self.failures.contains_key(path)
                });
                self.transition_waiting = pending && !self.presentation.images.is_empty();
                let image_uv = image_uv(self.settings.flip_horizontal, self.settings.flip_vertical);
                if self.transition_waiting {
                    self.presentation.paint(ui, canvas, image_uv);
                } else {
                    let mut presented = Vec::new();
                    let spread = ids.len() > 1;
                    let width = canvas.width() / ids.len() as f32;
                    for (col, &index) in ids.iter().enumerate() {
                        let slot = Rect::from_min_size(
                            canvas.min + egui::vec2(width * col as f32, 0.),
                            egui::vec2(width, canvas.height()),
                        );
                        let path = &self.files[index];
                        if let Some(p) = self.pictures.get(path) {
                            let size = egui::vec2(p.original[0] as f32, p.original[1] as f32);
                            let factor = self.image_scale(size, slot.size(), ppp);
                            let shown = size * factor;
                            let center = if spread {
                                let x = if col == 0 {
                                    slot.right() - shown.x / 2.
                                } else {
                                    slot.left() + shown.x / 2.
                                };
                                egui::pos2(x, slot.center().y)
                            } else {
                                slot.center()
                            };
                            let rect = Rect::from_center_size(center + self.pan, shown);
                            crate::ui::transparency::paint(
                                &ui.painter().with_clip_rect(slot),
                                rect,
                                12.,
                            );
                            ui.painter().with_clip_rect(slot).image(
                                p.handle.id(),
                                rect,
                                image_uv,
                                Color32::WHITE,
                            );
                            presented.push(PresentedImage {
                                handle: p.handle.clone(),
                                rect,
                                clip: slot,
                            });
                            let source_px_per_texel = size.x / p.handle.size_vec2().x;
                            if factor * ppp * source_px_per_texel > 1.02
                                && source_px_per_texel > 1.02
                            {
                                let tag = Rect::from_min_size(
                                    slot.left_bottom() + egui::vec2(10., -32.),
                                    egui::vec2(127., 22.),
                                );
                                ui.painter()
                                    .rect_filled(tag, 3., Color32::from_black_alpha(200));
                                ui.painter().text(
                                    tag.center(),
                                    egui::Align2::CENTER_CENTER,
                                    tr("reduced-preview"),
                                    FontId::proportional(12.),
                                    INK,
                                );
                            }
                        } else {
                            let error = self.failures.get(path);
                            let caption = match error {
                                Some(error) => format!(
                                    "{}\nERROR\n{}",
                                    path.file_name().unwrap_or_default().to_string_lossy(),
                                    error
                                ),
                                None => tr("opening-image-dialog").to_owned(),
                            };
                            ui.painter().with_clip_rect(slot).text(
                                slot.center(),
                                egui::Align2::CENTER_CENTER,
                                caption,
                                FontId::proportional(14.),
                                MUTED,
                            );
                        }
                    }
                    self.presentation = Presentation {
                        images: presented,
                        canvas: Some(canvas),
                    };
                }
                let mut base = canvas.shrink(6.);
                if self.controls_visible && self.hide_controls && !self.settings_open {
                    base.min.y += TOP_BAR_HEIGHT;
                }
                let mut overlays = vec![Rect::from_min_max(
                    base.min,
                    egui::pos2(canvas.center().x, base.bottom()),
                )];
                if ids.len() == 2 {
                    overlays.push(Rect::from_min_max(
                        egui::pos2(canvas.center().x, base.top()),
                        base.max,
                    ));
                }
                let reading = self.inspector.open
                    && !self.transition_waiting
                    && ctx
                        .pointer_hover_pos()
                        .is_some_and(|p| overlays.iter().any(|overlay| overlay.contains(p)));
                if self.inspector.open && !self.transition_waiting && !self.settings_open {
                    let visible: Vec<_> = ids.iter().map(|&i| self.files[i].as_path()).collect();
                    let copied = self.inspector.show(ui, &overlays, &visible);
                    if copied {
                        self.show_toast(ctx, tr("copied"));
                    }
                }
                self.canvas_pointer(ctx, &response, reading);
                if crate::app::is_mobile() {
                    return;
                }
                if !reading
                    && !self.settings_open
                    && !self.random_prompt
                    && !self.menu_open
                    && self.move_dialog.is_none()
                    && !self.popup_open_at_input
                {
                    if response.double_clicked() {
                        self.run("fullscreen", ctx);
                    }
                    if response.hovered()
                        && ctx.input(|i| i.focused)
                        && !self.settings_open
                        && !self.recent_open
                        && !self.clear_prompt
                        && self.zoom_input.is_none()
                        && !ctx.memory(|m| m.any_popup_open())
                    {
                        // Use each event's modifiers, not the final keyboard state or
                        // egui's smoothed scroll/zoom delta. Show the 100% stop even
                        // when several wheel events arrive together in one frame.
                        let mut stopped_at_actual = false;
                        for event in ctx.input(|i| i.events.clone()) {
                            match event {
                                egui::Event::MouseWheel {
                                    delta, modifiers, ..
                                } if delta.y.is_finite() && delta.y != 0. => {
                                    if modifiers.ctrl || modifiers.command {
                                        if !stopped_at_actual {
                                            stopped_at_actual = self.wheel_zoom(ctx, delta.y > 0.);
                                        }
                                    } else {
                                        self.move_page(ctx, delta.y < 0.);
                                    }
                                }
                                egui::Event::Zoom(factor) if factor.is_finite() && factor > 0. => {
                                    self.change_zoom(ctx, factor);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            });
    }

    pub(super) fn canvas_pointer(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        reading: bool,
    ) {
        if crate::app::is_mobile() {
            self.touch_canvas(ctx, response, reading);
            return;
        }
        if self.settings_open
            || self.random_prompt
            || self.recent_open
            || self.move_dialog.is_some()
            || self.clear_prompt
            || self.menu_open
            || self.popup_open_at_input
            || self.zoom_input.is_some()
            || !ctx.input(|i| i.focused)
        {
            self.canvas_drag = None;
            return;
        }
        if !reading && response.secondary_clicked() {
            self.menu_open = true;
            self.canvas_drag = None;
            self.held_key = None;
            self.held_button = None;
            return;
        }
        if ctx.input(|i| i.pointer.primary_pressed()) {
            self.canvas_drag = None;
            if !reading && response.contains_pointer() {
                let press = ctx.input(|i| {
                    i.events.iter().find_map(|event| match event {
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers,
                        } => Some((*pos, *modifiers)),
                        _ => None,
                    })
                });
                if let Some((pointer_origin, modifiers)) = press {
                    self.canvas_drag = if modifiers.ctrl || modifiers.command {
                        (!self.presentation.images.is_empty() && !self.transition_waiting)
                            .then_some(CanvasDrag::Image {
                                pointer_origin,
                                pan_origin: self.pan,
                            })
                    } else {
                        (!self.fullscreen).then_some(CanvasDrag::Window)
                    };
                }
            }
        }
        match self.canvas_drag {
            Some(CanvasDrag::Window) if !self.fullscreen => {
                window_frame::drag(ctx, response);
            }
            Some(CanvasDrag::Image {
                pointer_origin,
                pan_origin,
            }) if response.dragged_by(egui::PointerButton::Primary)
                || response.drag_stopped_by(egui::PointerButton::Primary) =>
            {
                if let Some(pointer) = ctx.pointer_interact_pos() {
                    let pan = pan_origin + (pointer - pointer_origin);
                    if self.pan != pan {
                        self.pan = pan;
                        ctx.request_repaint();
                    }
                }
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            _ => {
                if !reading
                    && response.hovered()
                    && !self.presentation.images.is_empty()
                    && ctx.input(|i| i.modifiers.ctrl || i.modifiers.command)
                {
                    ctx.set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.canvas_drag = None;
        }
    }
}
