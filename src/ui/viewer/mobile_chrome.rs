//! Touch gestures for phones and tablets; controls share the desktop bars.
use super::*;

impl Viewer {
    /// Compare the rendered image with its normal view, rather than treating
    /// every manual percentage as zoomed. A small image at 100% can still fit.
    pub(super) fn touch_pan_mode(&self, canvas: Vec2, ctx: &egui::Context) -> bool {
        let (first, last) = self.focus_span();
        let slot = egui::vec2(canvas.x / if first == last { 1. } else { 2. }, canvas.y);
        let transform = ImageTransform::from_settings(&self.settings);
        (first..=last).any(|index| {
            self.files
                .get(index)
                .and_then(|path| self.pictures.get(path))
                .is_some_and(|picture| {
                    let size = transform.size(egui::vec2(
                        picture.original[0] as f32,
                        picture.original[1] as f32,
                    ));
                    let shown = size * self.image_scale(size, slot, ctx);
                    let normal = size * self.image_scale_for(size, slot, ctx, 1., None);
                    shown.x > slot.x + 0.001
                        || shown.y > slot.y + 0.001
                        || shown.x > normal.x + 0.001
                        || shown.y > normal.y + 0.001
                })
        })
    }

    pub(super) fn touch_canvas(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        reading: bool,
    ) {
        if ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Touch {
                        phase: egui::TouchPhase::Cancel,
                        ..
                    }
                )
            })
        }) {
            self.touch_gesture.cancel();
            self.mobile_swipe = None;
            return;
        }
        // A fullscreen tap always returns to the normal viewer, even over
        // metadata or a letterbox. A swipe/pinch must never count as this tap.
        if self.fullscreen
            && !self.touch_gesture.was_multiple()
            && ctx.input(|i| i.focused && i.pointer.any_click())
        {
            self.run("fullscreen", ctx);
            // A menu opened by a long press is drawn after the canvas. Close
            // it now so this same tap cannot run its fullscreen tile again.
            self.menu_open = false;
            egui::Popup::close_all(ctx);
            self.touch_gesture.cancel();
            self.mobile_swipe = None;
            return;
        }
        if reading
            || self.settings_open
            || self.overlay_open()
            || self.zoom_input.is_some()
            || ctx.any_popup_open()
            || !ctx.input(|i| i.focused)
        {
            self.touch_gesture.cancel();
            self.mobile_swipe = None;
            return;
        }
        if self.mobile_swipe_settling() {
            if !ctx.input(|i| i.pointer.primary_pressed()) {
                return;
            }
            self.interrupt_mobile_swipe(ctx, response.rect);
        }
        if ctx.input(|i| i.pointer.primary_pressed())
            && !self.touch_gesture.active()
            && response.contains_pointer()
            && let Some(position) = ctx.pointer_interact_pos()
        {
            self.touch_gesture.start(position);
        }
        if !self.touch_gesture.active() {
            return;
        }
        // Some backends release/reassign the primary pointer when its finger
        // lifts during a pinch. Keep the same gesture locked until all fingers
        // are up, including when the remaining finger becomes primary.
        if self.touch_gesture.was_multiple()
            && ctx.input(|i| !i.any_touches() && !i.pointer.primary_down())
        {
            self.touch_gesture.cancel();
            self.mobile_swipe = None;
            return;
        }
        if self.touch_pan_mode(response.rect.size(), ctx) {
            self.touch_gesture.lock_pan();
            self.mobile_swipe = None;
        }
        if let Some(touch) = ctx
            .input(|i| i.multi_touch())
            .filter(|touch| touch.num_touches >= 2)
        {
            self.touch_gesture.multiple();
            self.mobile_swipe = None;
            if self.settings.touch.pinch_zoom
                && touch.zoom_delta.is_finite()
                && touch.zoom_delta > 0.
            {
                self.change_zoom(ctx, touch.zoom_delta);
            }
            if self.settings.touch.two_finger_pan
                && touch.translation_delta.is_finite()
                && self.images_can_pan(response.rect.size(), ctx)
            {
                self.pan += touch.translation_delta;
            }
            return;
        }
        if response.long_touched() && !self.touch_gesture.was_multiple() {
            let action = self.settings.touch.long_press.clone();
            self.touch_gesture.cancel();
            self.mobile_swipe = None;
            self.mouse_click(ctx, &action);
            return;
        }
        let swipe_offset = ctx.pointer_interact_pos().and_then(|position| {
            if self.mobile_swipe.is_some() {
                self.touch_gesture.horizontal_offset(position)
            } else {
                self.touch_gesture.swipe_offset(position)
            }
        });
        if self.settings.touch.swipe_navigation
            && let Some(offset) = swipe_offset
        {
            self.drag_mobile_swipe(ctx, response.rect, offset);
        }
        if self.mobile_swipe.is_none()
            && self.settings.touch.one_finger_pan
            && !self.touch_gesture.was_multiple()
            && response.dragged()
            && self.images_can_pan(response.rect.size(), ctx)
        {
            self.pan += ctx.input(|i| i.pointer.delta());
        }
        if ctx.input(|i| i.pointer.primary_released()) {
            if self.touch_gesture.was_multiple() && ctx.input(|i| i.any_touches()) {
                return;
            }
            let multiple = self.touch_gesture.was_multiple();
            // Native touch-up also sends PointerGone in this frame. The
            // interaction position survives it; latest_pos does not.
            if let Some(end) = ctx.pointer_interact_pos() {
                if let Some(forward) = self.touch_gesture.finish(end, self.settings.rtl) {
                    if self.settings.touch.swipe_navigation {
                        self.finish_mobile_swipe(ctx, Some(forward));
                    }
                } else {
                    self.finish_mobile_swipe(ctx, None);
                    if !multiple && response.double_clicked() {
                        let action = self.settings.touch.double_tap.clone();
                        self.mouse_click(ctx, &action);
                    }
                }
            }
            self.touch_gesture.cancel();
        }
    }
}
