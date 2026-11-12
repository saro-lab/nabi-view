//! Touch gestures for phones and tablets; controls share the desktop bars.
use super::*;

impl Viewer {
    pub(super) fn touch_canvas(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        reading: bool,
    ) {
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
        // Long press is the touch equivalent of the desktop right-click menu.
        if response.long_touched() && !self.touch_gesture.was_multiple() {
            self.touch_gesture.cancel();
            self.menu_open = true;
            self.release_held_input();
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
        if response.dragged()
            && self.presentation.images.iter().any(|image| {
                image.rect.width() > image.clip.width() + 1.
                    || image.rect.height() > image.clip.height() + 1.
            })
        {
            self.pan += ctx.input(|i| i.pointer.delta());
        }
        if ctx.input(|i| i.pointer.primary_released()) {
            let multiple = self.touch_gesture.was_multiple();
            // Native touch-up also sends PointerGone in this frame. The
            // interaction position survives it; latest_pos does not.
            if let Some(end) = ctx.pointer_interact_pos() {
                if let Some(forward) = self.touch_gesture.finish(end, self.settings.rtl) {
                    self.move_page(ctx, forward);
                } else if !multiple && response.double_clicked() {
                    self.run("fit", ctx);
                }
            }
            self.touch_gesture.cancel();
        }
    }
}
