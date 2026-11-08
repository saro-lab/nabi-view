use super::*;

impl Viewer {
    pub fn new_window(&mut self, path: Option<&Path>) {
        if crate::app::is_mobile() {
            if let Some(path) = path {
                self.open(path.to_owned());
            }
            return;
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        if let Err(error) = crate::app::launch_viewer(path) {
            self.last_error = error;
        }
    }

    pub(super) fn request_graceful_close(&mut self, ctx: &egui::Context) {
        self.closing_all = true;
        if self.file_job.is_some() {
            self.close_after_file_job = true;
            self.show_toast(
                ctx,
                tr("the-window-will-close-when-the-file-operation-finishes"),
            );
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(super) fn open_from_user(&mut self, path: PathBuf) {
        if !path.is_dir() && !media::is_supported(&path) {
            self.last_error = format!("Unsupported image: {}", path.display());
        } else if self.files.is_empty() && self.listing.is_none() {
            self.open(path);
        } else {
            self.new_window(Some(&path));
        }
    }

    pub(super) fn open(&mut self, path: PathBuf) {
        self.toast = None;
        let path = normalize_open_path(path);
        let directory = path.is_dir();
        if !directory && !media::is_supported(&path) {
            self.last_error = format!("Unsupported image: {}", path.display());
            return;
        }
        let focused_path = if directory {
            self.files
                .get(self.focus_index)
                .filter(|p| p.parent() == Some(path.as_path()))
                .cloned()
        } else {
            Some(path.clone())
        };
        let folder = if directory {
            Some(path)
        } else {
            path.parent().map(Path::to_owned)
        };
        if self.folder != folder {
            self.checked.clear();
        }
        self.random_prompt = false;
        self.media.clear_slot(0);
        self.media.clear_slot(1);
        self.tickets = [None, None];
        self.cancel_prefetch();
        self.prefetch_blocked.clear();
        self.pending_steps.clear();
        self.inspector.invalidate();
        self.span_cache.get_mut().take();
        self.files = focused_path.into_iter().collect();
        self.focus_index = 0;
        self.pan = Vec2::ZERO;
        self.canvas_drag = None;
        self.touch_gesture.cancel();
        self.zoom = 1.0;
        self.zoom_override = None;
        self.zoom_input = None;
        self.last_error.clear();
        self.failures.clear();
        self.pictures.clear();
        self.page_sizes.clear();
        self.presentation = Presentation::default();
        self.transition_waiting = false;
        self.media.clear_thumbnail();
        self.thumbs.clear();
        self.thumb_ticket = None;
        self.thumb_errors.clear();
        self.recent_open = false;
        self.move_dialog = None;
        self.folder = folder.clone();
        // Desktop phone preview uses real folders; only native mobile imports
        // are temporary copies that must never be saved as recent folders.
        self.remember = folder
            .clone()
            .filter(|_| !cfg!(any(target_os = "ios", target_os = "android")))
            .filter(|dir| {
                crate::app::sample_path()
                    .and_then(|p| p.parent().map(Path::to_owned))
                    .as_ref()
                    != Some(dir)
            });
        self.queue_images();
        self.listing = folder.map(|p| self.media.request_folder(p));
        self.folder_watch = self.listing;
    }

    pub(super) fn focus_span(&self) -> (usize, usize) {
        let key = (
            self.focus_index,
            self.files.len(),
            self.settings.spread,
            self.settings.cover,
            self.layout_monitor,
        );
        if let Some(cache) = self
            .span_cache
            .borrow()
            .as_ref()
            .filter(|cache| cache.key == key)
        {
            return cache.span;
        }
        let span = adaptive_focus_span(
            self.focus_index,
            self.files.len(),
            self.settings.spread,
            self.settings.cover,
            |index| {
                self.page_sizes
                    .get(&self.files[index])
                    .or_else(|| self.pictures.get(&self.files[index]).map(|p| &p.original))
                    .is_some_and(|size| width_limited(*size, self.layout_monitor))
            },
        );
        *self.span_cache.borrow_mut() = Some(SpanCache { key, span });
        span
    }

    pub(super) fn move_page(&mut self, ctx: &egui::Context, forward: bool) {
        if self.listing.is_some() {
            if self.pending_steps.len() < i16::MAX as usize {
                self.pending_steps.push((forward, false));
            }
            return;
        }
        if !self.files.is_empty() {
            let (_, end) = self.focus_span();
            if self.random && forward && end + 1 == self.files.len() {
                let previous: HashSet<_> = self.files[self.focus_span().0..=end]
                    .iter()
                    .cloned()
                    .collect();
                shuffle_paths(&mut self.files);
                if let Some(next) = self.files.iter().position(|p| !previous.contains(p)) {
                    self.files.swap(0, next);
                }
                self.span_cache.get_mut().take();
                self.focus(0);
                self.show_toast(ctx, tr("random-order-has-been-reset-after-a-full-cycle"));
                return;
            }
            let (start, end) = self.focus_span();
            let next = if forward {
                (end + 1) % self.files.len()
            } else if start == 0 {
                self.files.len() - 1
            } else {
                start - 1
            };
            self.focus_navigation(ctx, next);
        }
    }

    pub(super) fn focus_navigation(&mut self, ctx: &egui::Context, focus_index: usize) {
        let previous = self.focus_span();
        self.focus(focus_index);
        let (first, last) = self.focus_span();
        if self.random || self.files.is_empty() || previous == (first, last) {
            return;
        }
        if first == 0 {
            self.show_toast(ctx, tr("this-is-the-first-image"));
        } else if last + 1 == self.files.len() {
            self.show_toast(ctx, tr("this-is-the-last-image"));
        }
    }

    pub(super) fn focus(&mut self, focus_index: usize) {
        if self.focus_index != focus_index {
            self.toast = None;
            self.prefetch_blocked.clear();
        }
        self.focus_index = focus_index;
        self.pan = Vec2::ZERO;
        self.canvas_drag = None;
        self.touch_gesture.cancel();
        self.zoom = 1.0;
        self.zoom_override = None;
        self.zoom_input = None;
        self.slide_deadline = None;
    }
}
