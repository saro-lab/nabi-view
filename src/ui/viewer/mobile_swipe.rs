//! A touch-only page slide. Timed slideshow and command navigation stay instant.
use super::*;

pub(super) struct Swipe {
    canvas: Rect,
    offset: f32,
    direction: f32,
    settle: Option<Settle>,
}

#[cfg(test)]
#[path = "mobile_swipe_tests.rs"]
mod tests;

struct Settle {
    committed: bool,
    started: Option<f64>,
    from: f32,
}

impl Viewer {
    fn swipe_target(&self, forward: bool) -> Option<usize> {
        if self.files.is_empty() || self.listing.is_some() {
            return None;
        }
        let (first, last) = self.focus_span();
        let next = if forward {
            (last + 1) % self.files.len()
        } else if first == 0 {
            self.files.len() - 1
        } else {
            first - 1
        };
        (self.page_span(next) != (first, last)).then_some(next)
    }

    pub(super) fn drag_mobile_swipe(&mut self, ctx: &egui::Context, canvas: Rect, offset: f32) {
        if let Some(swipe) = &mut self.mobile_swipe {
            swipe.offset = offset.clamp(-canvas.width() * 0.95, canvas.width() * 0.95);
            if offset.abs() >= 8. {
                swipe.direction = offset.signum();
            }
            ctx.request_repaint();
            return;
        }
        let forward = (offset < 0.) != self.settings.rtl;
        if self.swipe_target(forward).is_none()
            || self.presentation.images.is_empty()
            || self.presentation.canvas != Some(canvas)
            || self.transition_waiting
        {
            return;
        }
        self.mobile_swipe = Some(Swipe {
            canvas,
            // Keep a sliver of the current page visible until touch-up.
            offset: offset.clamp(-canvas.width() * 0.95, canvas.width() * 0.95),
            direction: offset.signum(),
            settle: None,
        });
        ctx.request_repaint();
    }

    pub(super) fn finish_mobile_swipe(&mut self, ctx: &egui::Context, forward: Option<bool>) {
        let swipe = self.mobile_swipe.take();
        let previous = self.files.get(self.focus_index).cloned();
        if let Some(forward) = forward {
            self.move_page(ctx, forward);
        }
        if !self.settings.reduce_motion
            && let Some(mut swipe) = swipe
        {
            let committed =
                forward.is_some() && self.files.get(self.focus_index) != previous.as_ref();
            swipe.settle = Some(Settle {
                committed,
                started: None,
                from: swipe.offset,
            });
            self.mobile_swipe = Some(swipe);
        }
        ctx.request_repaint();
    }

    /// Adjacent pages reuse decoded textures. They never start full-resolution
    /// crop work or treat thumbnail pixels as a full-size image.
    fn swipe_preview(&self, ctx: &egui::Context, canvas: Rect, index: usize) -> Presentation {
        let image_transform = ImageTransform::from_settings(&self.settings);
        let (first, last) = self.page_span(index);
        let mut ids = vec![first];
        if first != last {
            ids.push(last);
            if self.settings.rtl {
                ids.reverse();
            }
        }
        let width = canvas.width() / ids.len() as f32;
        let mut images = Vec::new();
        for (col, index) in ids.iter().copied().enumerate() {
            let Some(picture) = self.pictures.get(&self.files[index]) else {
                continue;
            };
            let slot = Rect::from_min_size(
                canvas.min + egui::vec2(width * col as f32, 0.),
                egui::vec2(width, canvas.height()),
            );
            let size = image_transform.size(egui::vec2(
                picture.original[0] as f32,
                picture.original[1] as f32,
            ));
            let shown = size * self.image_scale_for(size, slot.size(), ctx, 1., None);
            let x = if ids.len() == 1 {
                slot.center().x
            } else if col == 0 {
                slot.right() - shown.x / 2.
            } else {
                slot.left() + shown.x / 2.
            };
            let rect = Rect::from_center_size(egui::pos2(x, slot.center().y), shown);
            images.push(PresentedImage {
                hdr: picture.hdr.clone(),
                handle: picture.handle.clone(),
                rect,
                clip: slot,
                backdrop: rect,
            });
        }
        Presentation {
            images,
            canvas: Some(canvas),
            pan: Vec2::ZERO,
            image_transform,
        }
    }

    pub(super) fn mobile_swipe_settling(&self) -> bool {
        self.mobile_swipe
            .as_ref()
            .is_some_and(|swipe| swipe.settle.is_some())
    }

    pub(super) fn interrupt_mobile_swipe(&mut self, ctx: &egui::Context, canvas: Rect) {
        let committed = self
            .mobile_swipe
            .as_ref()
            .and_then(|swipe| swipe.settle.as_ref())
            .is_some_and(|settle| settle.committed);
        if committed {
            let (first, last) = self.focus_span();
            let ready = self.files[first..=last]
                .iter()
                .all(|p| self.pictures.contains_key(p));
            if ready {
                self.presentation = self.swipe_preview(ctx, canvas, self.focus_index);
            }
            // A second swipe can reverse/skip a pending decode. Keep the last
            // visible image if necessary, but never swallow the new gesture.
            self.transition_waiting = !ready;
        }
        self.mobile_swipe = None;
    }

    pub(super) fn paint_mobile_swipe(
        &mut self,
        ui: &egui::Ui,
        canvas: Rect,
        background: Color32,
    ) -> bool {
        let Some(mut swipe) = self.mobile_swipe.take() else {
            return false;
        };
        let ctx = ui.ctx();
        if !crate::app::is_mobile()
            || canvas != swipe.canvas
            || !self.settings.touch.swipe_navigation
            || self.settings_open
            || self.overlay_open()
            || self.zoom_input.is_some()
            || ctx.any_popup_open()
            || !ctx.input(|i| i.focused)
        {
            self.touch_gesture.cancel();
            return false;
        }
        let committed = swipe.settle.as_ref().is_some_and(|s| s.committed);
        let target = if committed {
            Some(self.focus_index)
        } else {
            let forward = (swipe.direction < 0.) != self.settings.rtl;
            // The random cycle is reshuffled on release; do not preview a page
            // that is about to be replaced by that shuffle.
            if self.random && forward && self.focus_span().1 + 1 == self.files.len() {
                None
            } else {
                self.swipe_target(forward)
            }
        };
        let ready = target.is_some_and(|index| {
            let (first, last) = self.page_span(index);
            self.files[first..=last]
                .iter()
                .all(|path| self.pictures.contains_key(path) || self.failures.contains_key(path))
        });
        if let Some(settle) = &mut swipe.settle {
            if !settle.committed || ready {
                let now = ctx.input(|i| i.time);
                let started = *settle.started.get_or_insert(now);
                let progress = ((now - started) / 0.22).clamp(0., 1.) as f32;
                if progress >= 1. || self.settings.reduce_motion {
                    return false;
                }
                let end = if settle.committed {
                    swipe.direction * canvas.width()
                } else {
                    0.
                };
                let eased = 1. - (1. - progress).powi(3);
                swipe.offset = settle.from + (end - settle.from) * eased;
                ctx.request_repaint();
            }
            // While decoding, keep the released position. Media completion
            // wakes the UI; an idle decoder does not need a busy repaint loop.
        }
        let painter = ui.painter().with_clip_rect(canvas);
        let image_transform = ImageTransform::from_settings(&self.settings);
        let checker = self.settings.transparency_checker;
        self.presentation.paint_clipped(
            &painter,
            canvas.translate(egui::vec2(swipe.offset, 0.)),
            image_transform,
            background,
            checker,
        );
        if let Some(index) = target {
            let shifted = canvas.translate(egui::vec2(
                swipe.offset - swipe.direction * canvas.width(),
                0.,
            ));
            let mut preview = self.swipe_preview(ctx, canvas, index);
            preview.paint_clipped(&painter, shifted, image_transform, background, checker);
            let (first, last) = self.page_span(index);
            if !ready
                || self.files[first..=last]
                    .iter()
                    .any(|p| self.failures.contains_key(p))
            {
                painter.with_clip_rect(shifted).text(
                    shifted.center(),
                    egui::Align2::CENTER_CENTER,
                    self.files[first..=last]
                        .iter()
                        .find_map(|p| self.failures.get(p))
                        .map_or_else(|| tr("opening-image-dialog"), String::as_str),
                    FontId::proportional(14.),
                    MUTED,
                );
            }
        }
        self.mobile_swipe = Some(swipe);
        true
    }
}
