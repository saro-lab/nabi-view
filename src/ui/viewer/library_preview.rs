//! Retain an exact, already decoded PhotoKit page across a native reopen.
//! A library row's representative thumbnail is never a display image.
use super::*;

#[cfg(test)]
#[path = "library_preview_tests.rs"]
mod tests;

pub(super) struct OpeningPreview {
    #[cfg(any(target_os = "ios", test))]
    target: PathBuf,
    #[cfg(any(target_os = "ios", test))]
    source: PathBuf,
    #[cfg(any(target_os = "ios", test))]
    identity: crate::core::documents::ProviderIdentity,
    pub(super) texture: egui::TextureHandle,
    waiting_for_import: bool,
}

impl Library {
    pub(in crate::ui::viewer) fn opening_preview_active(&self) -> bool {
        self.opening_preview.is_some()
    }
}

impl Viewer {
    #[cfg(any(target_os = "ios", test))]
    pub(super) fn library_preview_for_open(
        &self,
        index: usize,
        target: &Path,
        status: crate::core::ios_access::PhotoStatus,
    ) -> Option<OpeningPreview> {
        use crate::core::ios_access::PhotoStatus;
        if status != PhotoStatus::Authorized {
            return None;
        }
        let (source, _) = crate::core::documents::source_identity(target)?;
        let item = self.library.items.get(index)?;
        if item.entry.folder != source {
            return None;
        }
        self.exact_photo_preview(target)
    }

    #[cfg(any(target_os = "ios", test))]
    fn exact_photo_preview(&self, target: &Path) -> Option<OpeningPreview> {
        let (source, id) = crate::core::documents::source_identity(target)?;
        // A collection request has no selected page yet. Its current page or
        // representative row image must not decide what the user will open.
        if target == source || !crate::core::photo_library::is_ios(&id) {
            return None;
        }
        let selection = self.mobile_selection.as_ref()?;
        if selection.source()?.root != source {
            return None;
        }
        let local = selection.local_path(target)?;
        let picture = self.pictures.get(&local)?;
        let identity = crate::core::documents::provider_cache_identity(&local).ok()??;
        Some(OpeningPreview {
            target: target.to_owned(),
            source,
            identity,
            texture: picture.handle.clone(),
            waiting_for_import: true,
        })
    }

    /// Keep the exact current display page only while its provider identity is
    /// still valid. The caller has verified full authorization.
    #[cfg(any(target_os = "ios", test))]
    pub(in crate::ui::viewer) fn preserve_current_photo_preview(&mut self, target: &Path) {
        let Some(selection) = self.mobile_selection.as_ref() else {
            return;
        };
        let Some(source) = selection.source() else {
            return;
        };
        if !crate::core::photo_library::is_ios(&source.id) {
            return;
        }
        let Some(current) = self.files.get(self.focus_index) else {
            return;
        };
        if selection.logical_path(current).as_deref() != Some(target) {
            return;
        }
        self.library.opening_preview = self.exact_photo_preview(target);
        self.release_held_input();
        self.touch_gesture.cancel();
        self.mobile_swipe = None;
    }

    #[cfg(any(target_os = "ios", target_os = "android", test))]
    pub(in crate::ui::viewer) fn library_preview_import_accepted(&mut self) {
        if self.preview_matches_selection(true) {
            self.library
                .opening_preview
                .as_mut()
                .unwrap()
                .waiting_for_import = false;
        } else {
            self.library.opening_preview = None;
        }
    }

    #[cfg(any(target_os = "ios", test))]
    pub(super) fn start_library_preview(
        &mut self,
        preview: Option<OpeningPreview>,
        ctx: &egui::Context,
    ) {
        self.library.opening_preview = preview;
        if self.library.opening_preview_active() {
            self.menu_open = false;
            self.inspector.close();
            self.release_held_input();
            self.touch_gesture.cancel();
            self.mobile_swipe = None;
            ctx.request_repaint();
        }
    }

    pub(in crate::ui::viewer) fn cancel_library_preview(&mut self) {
        if self.library.opening_preview.take().is_some() {
            self.library.open = true;
            self.release_held_input();
        }
    }

    #[cfg(any(target_os = "ios", test))]
    pub(in crate::ui::viewer) fn clear_library_preview_for_source(&mut self, target: &Path) {
        if self
            .library
            .opening_preview
            .as_ref()
            .is_some_and(|preview| preview.target != target)
        {
            self.cancel_library_preview();
        }
    }

    pub(in crate::ui::viewer) fn stop_library_preview(&mut self) {
        #[cfg(target_os = "ios")]
        self.cancel_ios_recovery();
        #[cfg(not(target_os = "ios"))]
        self.cancel_library_preview();
    }

    fn preview_matches_selection(&self, require_focus: bool) -> bool {
        #[cfg(any(target_os = "ios", test))]
        {
            let Some(preview) = &self.library.opening_preview else {
                return false;
            };
            let Some(selection) = &self.mobile_selection else {
                return false;
            };
            if selection
                .source()
                .is_none_or(|source| source.root != preview.source)
            {
                return false;
            }
            let Some(local) = selection.local_path(&preview.target) else {
                return false;
            };
            if require_focus && self.files.get(self.focus_index) != Some(&local) {
                return false;
            }
            crate::core::documents::provider_cache_identity(&local)
                .ok()
                .flatten()
                .as_ref()
                == Some(&preview.identity)
        }
        #[cfg(not(any(target_os = "ios", test)))]
        {
            let _ = require_focus;
            false
        }
    }

    /// Returns true while the read-only preview covers the library/viewer.
    pub(super) fn library_opening_preview_window(&mut self, ctx: &egui::Context) -> bool {
        if !self.library.opening_preview_active() {
            return false;
        }
        #[cfg(target_os = "ios")]
        if crate::platform::mobile::photo_status()
            != crate::core::ios_access::PhotoStatus::Authorized
        {
            self.stop_library_preview();
            return false;
        }
        let preview = self.library.opening_preview.as_ref().unwrap();
        if !self.preview_matches_selection(!preview.waiting_for_import) {
            self.library.opening_preview = None;
            return false;
        }
        if !preview.waiting_for_import {
            // Import acceptance closes the list. Keep this through local scan
            // and decode, then reveal only this request's actual image.
            if !self.files.is_empty() && self.listing.is_none() {
                let (first, last) = self.focus_span();
                if self.files[first..=last]
                    .iter()
                    .all(|path| self.pictures.contains_key(path))
                {
                    self.library.opening_preview = None;
                    return false;
                }
                if let Some(error) = self.files[first..=last]
                    .iter()
                    .find_map(|path| self.failures.get(path).cloned())
                {
                    self.last_error = error;
                    self.stop_library_preview();
                    return false;
                }
            } else if self.listing.is_none() && !self.last_error.is_empty() {
                self.stop_library_preview();
                return false;
            }
        }
        let texture = self
            .library
            .opening_preview
            .as_ref()
            .unwrap()
            .texture
            .clone();
        let screen = ctx.viewport_rect();
        let transform = ImageTransform::from_settings(&self.settings);
        let id = egui::Id::unique("library");
        let mut close = false;
        // Reuse the already measured library area, without its entry fade-in.
        let modal = egui::Modal::new(id)
            .area(
                egui::Modal::default_area(id)
                    .default_size(screen.size())
                    .fade_in(false),
            )
            .frame(egui::Frame::NONE.fill(Color32::BLACK))
            .backdrop_color(Color32::BLACK)
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                ui.set_max_size(screen.size());
                let rect = ui.max_rect();
                let controls = ctx.content_rect().intersect(rect);
                let size = transform.size(texture.size_vec2());
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                let image = Rect::from_center_size(rect.center(), size * scale);
                transform.paint(ui.painter(), texture.id(), image);
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_size(
                        controls.min + Vec2::splat(12.),
                        egui::vec2(44., 44.),
                    )),
                    |ui| {
                        close = forms::icon(ui, "library-close", "back-to-viewer").clicked();
                    },
                );
                let loading = Rect::from_center_size(
                    controls.center_bottom() - egui::vec2(0., 32.),
                    egui::vec2(28., 28.),
                );
                ui.painter()
                    .circle_filled(loading.center(), 14., Color32::from_black_alpha(160));
                ui.interact(
                    loading,
                    egui::Id::unique("library-preview-loading"),
                    egui::Sense::hover(),
                );
                ui.put(loading, egui::Spinner::new().size(14.));
            });
        ctx.move_to_top(modal.response.layer_id);
        if close || modal.should_close() {
            self.stop_library_preview();
        }
        self.release_held_input();
        true
    }
}
