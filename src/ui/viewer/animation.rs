use super::*;

impl Viewer {
    pub(super) fn advance_slideshow(&mut self, ctx: &egui::Context) {
        if !self.slideshow
            || self.files.is_empty()
            || self.listing.is_some()
            || self.recent_open
            || self.move_dialog.is_some()
            || self.menu_open
            || self.clear_prompt
            || self.random_prompt
            || self.file_job.is_some()
            || self.zoom_input.is_some()
            || ctx.memory(|m| m.any_popup_open())
        {
            self.slide_deadline = None;
            return;
        }
        let now = ctx.input(|i| i.time);
        let deadline = *self
            .slide_deadline
            .get_or_insert(now + self.settings.slide_ms as f64 / 1000.);
        if now >= deadline {
            // Let the user read a setting change even when slides advance faster
            // than the toast duration. Keep its original expiry, not a new timer.
            let notice = self.toast.take_if(|toast| {
                toast.keep_on_slideshow && std::time::Instant::now() < toast.deadline
            });
            self.move_page(ctx, true);
            if let Some(mut notice) = notice {
                notice.image = self.files.get(self.focus_index).cloned();
                self.toast = Some(notice);
            }
            self.slide_deadline = Some(now + self.settings.slide_ms as f64 / 1000.);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(
            (self.slide_deadline.unwrap_or(deadline) - now).max(0.001),
        ));
    }

    pub(super) fn animate(&mut self, ctx: &egui::Context) {
        let (a, b) = self.focus_span();
        let visible = [self.files.get(a), self.files.get(b)];
        let now = std::time::Instant::now();
        for (path, picture) in &mut self.pictures {
            let Some(animation) = &mut picture.animation else {
                continue;
            };
            if !visible.contains(&Some(path)) {
                animation.stop();
                continue;
            }
            match animation.advance(ctx, now) {
                Ok(Some(image)) => picture.handle.set(image, egui::TextureOptions::LINEAR),
                Ok(None) => {}
                Err(error) => {
                    // Keep the last good texture, but release a failed worker
                    // and avoid retrying the same corrupt frame on every paint.
                    picture.animation = None;
                    self.last_error = format!("{}: {error}", path.display());
                }
            }
        }
    }
}
