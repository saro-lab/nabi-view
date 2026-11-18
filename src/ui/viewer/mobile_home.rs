//! Mobile start destination. Returning home retains the completed photo cache.
use super::*;

impl Viewer {
    pub(super) fn mobile_home_button(&mut self, ui: &mut egui::Ui) {
        let (_, target) = ui.allocate_space(Vec2::splat(TOUCH_TARGET));
        let response = ui.interact(
            target,
            egui::Id::unique(("titlebar", "home")),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, tr("recent-and-bookmarks"))
        });
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(target, 6., Color32::from_gray(48));
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                target.shrink(1.),
                5.,
                egui::Stroke::new(1., ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().image(
            self.app_icon.id(),
            Rect::from_center_size(target.center(), Vec2::splat(24.)),
            uv(),
            Color32::WHITE,
        );
        if chrome_icons::tooltip(
            response,
            "recent-and-bookmarks",
            &self.shortcut_text("recent"),
        )
        .clicked()
        {
            self.run("recent", ui.ctx());
        }
    }

    /// Explicit navigation, never a toggle. Pending opens may not take the
    /// screen back from home; already complete images remain available to reuse.
    pub(super) fn open_mobile_home(&mut self, ctx: &egui::Context) {
        if !self.source_list_pending() {
            self.remember_page();
        }
        #[cfg(target_os = "ios")]
        self.cancel_ios_recovery();
        #[cfg(not(target_os = "ios"))]
        self.cancel_library_preview();
        #[cfg(all(test, not(target_os = "ios")))]
        self.discard_mobile_seed();
        #[cfg(target_os = "android")]
        crate::platform::mobile::cancel_android_open();
        self.menu_open = false;
        self.settings_open = false;
        self.close_shortcut_editing();
        self.inspector.close();
        self.fullscreen = false;
        self.zoom_input = None;
        self.slideshow = false;
        self.mobile_swipe = None;
        self.touch_gesture.cancel();
        self.release_held_input();
        self.cancel_prefetch();
        egui::Popup::close_all(ctx);
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
        self.library.open = true;
        self.reload_library();
        ctx.request_repaint();
    }
}
