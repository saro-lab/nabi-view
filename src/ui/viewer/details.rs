//! Keep the small presentation image while a visible original-resolution crop
//! is decoded. Only the two displayed pages have detail textures or requests.
use super::*;
use crate::core::media::regions::{Area, MAX_REGION_PIXELS, MAX_REGION_SIDE, Request, Worker};

#[derive(Default)]
struct Slot {
    requested: Option<Request>,
    ready: Option<(
        Request,
        egui::TextureHandle,
        Option<std::sync::Arc<crate::ui::hdr::Texture>>,
    )>,
    active: bool,
    failed: bool,
}
impl Slot {
    fn bytes(&self) -> usize {
        self.ready.as_ref().map_or(0, |(_, handle, hdr)| {
            handle.byte_size() + hdr.as_ref().map_or(0, |hdr| hdr.bytes())
        })
    }
}
pub(super) struct Details {
    worker: Worker,
    slots: [Slot; 2],
    budget: usize,
    max_texture_side: u32,
}
impl Details {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            worker: Worker::new(ctx),
            slots: Default::default(),
            budget: 0,
            max_texture_side: MAX_REGION_SIDE,
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.slots.iter().map(Slot::bytes).sum()
    }
    pub fn begin(
        &mut self,
        ctx: &egui::Context,
        state: Option<&eframe::egui_wgpu::RenderState>,
        budget: usize,
    ) {
        self.budget = budget;
        self.max_texture_side = state.map_or(MAX_REGION_SIDE, |state| {
            state
                .device
                .limits()
                .max_texture_dimension_2d
                .min(MAX_REGION_SIDE)
        });
        if budget == 0 || self.retained_bytes() > budget {
            self.clear();
            return;
        }
        let mut used = self.retained_bytes();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            slot.active = false;
            if let Some(result) = self.worker.take(index) {
                if slot.requested.as_ref() == Some(&result.request) {
                    match result.result {
                        Ok((image, hdr)) => {
                            let bytes = image.pixels.len() * 4
                                + hdr.as_ref().map_or(0, |hdr| hdr.bytes() * 3 / 2);
                            let without = used.saturating_sub(slot.bytes());
                            if without.saturating_add(bytes) > budget {
                                slot.failed = true;
                                continue;
                            }
                            used = without.saturating_add(bytes);
                            let hdr = hdr
                                .and_then(|hdr| {
                                    state.and_then(|state| crate::ui::hdr::Texture::new(state, hdr))
                                })
                                .map(std::sync::Arc::new);
                            slot.ready = Some((
                                result.request,
                                ctx.load_texture(
                                    format!("visible-detail-{index}"),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                ),
                                hdr,
                            ));
                            slot.failed = false;
                        }
                        Err(_) => slot.failed = true,
                    }
                }
            }
        }
    }
    pub fn finish(&mut self) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if !slot.active && slot.requested.is_some() {
                self.worker.clear(index);
                *slot = Slot::default();
            }
        }
    }
    pub fn pending(&self) -> bool {
        self.slots.iter().any(|slot| {
            !slot.failed
                && slot.requested.as_ref().is_some_and(|request| {
                    slot.ready
                        .as_ref()
                        .is_none_or(|(ready, _, _)| ready != request)
                })
        })
    }
    pub fn invalidate(&mut self, path: &Path) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.requested.as_ref().is_some_and(|r| r.path == path) {
                self.worker.clear(index);
                *slot = Slot::default();
            }
        }
    }
    pub fn textures(&self) -> impl Iterator<Item = &egui::TextureHandle> {
        self.slots
            .iter()
            .filter_map(|s| s.ready.as_ref())
            .map(|(_, h, _)| h)
    }
    pub fn hdr_textures(&self) -> impl Iterator<Item = &std::sync::Arc<crate::ui::hdr::Texture>> {
        self.slots
            .iter()
            .filter_map(|s| s.ready.as_ref())
            .filter_map(|(_, _, hdr)| hdr.as_ref())
    }
    pub fn clear(&mut self) {
        for index in 0..2 {
            self.worker.clear(index);
            self.slots[index] = Slot::default();
        }
    }
    pub fn paint(
        &mut self,
        index: usize,
        path: &Path,
        original: [u32; 2],
        rect: Rect,
        clip: Rect,
        flip: [bool; 2],
        ctx: &egui::Context,
        ui: &egui::Ui,
        background: Color32,
        checker: bool,
        base_density: f32,
    ) -> Option<PresentedImage> {
        let visible = rect.intersect(clip);
        if !visible.is_positive() || !rect.is_finite() || original.contains(&0) {
            return None;
        }
        let size = egui::vec2(original[0] as f32, original[1] as f32);
        let mut lo = (visible.min - rect.min) / rect.size();
        let mut hi = (visible.max - rect.min) / rect.size();
        if flip[0] {
            (lo.x, hi.x) = (1. - hi.x, 1. - lo.x);
        }
        if flip[1] {
            (lo.y, hi.y) = (1. - hi.y, 1. - lo.y);
        }
        // A surrounding 128-source-pixel band avoids restarting at every pointer pixel.
        let start = [
            ((lo.x * size.x).floor().max(0.) as u32 / 128) * 128,
            ((lo.y * size.y).floor().max(0.) as u32 / 128) * 128,
        ];
        let end = [
            ((hi.x * size.x).ceil().max(1.) as u32)
                .div_ceil(128)
                .saturating_mul(128)
                .min(original[0]),
            ((hi.y * size.y).ceil().max(1.) as u32)
                .div_ceil(128)
                .saturating_mul(128)
                .min(original[1]),
        ];
        let area = Area {
            x: start[0],
            y: start[1],
            width: end[0].saturating_sub(start[0]),
            height: end[1].saturating_sub(start[1]),
        };
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let physical = (rect.width() / size.x * ctx.pixels_per_point()).min(1.);
        let other_bytes = self
            .slots
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, s)| {
                s.bytes().max(
                    s.requested
                        .as_ref()
                        .filter(|_| !s.failed)
                        .map_or(0, Request::reserved_bytes),
                )
            })
            .sum::<usize>();
        // Include the retained float source, FP16 texture and SDR fallback.
        // Preserve wide/tall crops instead of budgeting an unused square.
        let pixel_budget =
            (self.budget.saturating_sub(other_bytes) / 28).min(MAX_REGION_PIXELS as usize);
        let area_pixels = f64::from(area.width) * f64::from(area.height);
        let max_budget_side = ((pixel_budget as f64 / area_pixels).sqrt()
            * f64::from(area.width.max(area.height)))
        .floor() as u32;
        if max_budget_side < 64 {
            return None;
        }
        let max_side =
            (((area.width.max(area.height) as f32 * physical).ceil() as u32).div_ceil(256) * 256)
                .clamp(256, self.max_texture_side)
                .min(max_budget_side);
        if max_side as f32 / area.width.max(area.height) as f32 <= base_density * 1.02 {
            return None;
        }
        let request = Request {
            path: path.to_owned(),
            area,
            max_side,
        };
        let slot = &mut self.slots[index];
        slot.active = true;
        if slot.requested.as_ref() != Some(&request) {
            self.worker.request(index, request.clone());
            if slot
                .ready
                .as_ref()
                .is_some_and(|(old, _, _)| old.path != path)
            {
                slot.ready = None;
            }
            slot.requested = Some(request);
            slot.failed = false;
        }
        let (ready, handle, hdr) = slot.ready.as_ref()?;
        let area = ready.area;
        let x = if flip[0] {
            original[0] - area.x - area.width
        } else {
            area.x
        };
        let y = if flip[1] {
            original[1] - area.y - area.height
        } else {
            area.y
        };
        let tile = Rect::from_min_size(
            rect.min + egui::vec2(x as f32, y as f32) / size * rect.size(),
            egui::vec2(area.width as f32, area.height as f32) / size * rect.size(),
        );
        let painter = ui.painter().with_clip_rect(clip.intersect(tile));
        painter.rect_filled(tile, 0., background);
        if checker {
            crate::ui::transparency::paint(&painter, rect, 12.);
        }
        painter.image(
            hdr.as_ref()
                .filter(|hdr| hdr.active())
                .map_or_else(|| handle.id(), |hdr| hdr.id()),
            tile,
            image_uv(flip[0], flip[1]),
            Color32::WHITE,
        );
        Some(PresentedImage {
            hdr: hdr.clone(),
            handle: handle.clone(),
            rect: tile,
            clip: clip.intersect(tile),
            backdrop: rect,
        })
    }
}
