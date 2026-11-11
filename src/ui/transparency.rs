//! Display-only transparency backdrop; source pixels and alpha stay untouched.
use eframe::egui::{self, Color32, Rect};

fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    let key = egui::Id::unique("transparency-checkerboard");
    if let Some(handle) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(key)) {
        return handle;
    }
    let light = Color32::from_gray(192);
    let dark = Color32::from_gray(152);
    let handle = ctx.load_texture(
        "transparency-checkerboard",
        egui::ColorImage::new([2, 2], vec![light, dark, dark, light]),
        egui::TextureOptions::NEAREST_REPEAT,
    );
    ctx.data_mut(|data| data.insert_temp(key, handle.clone()));
    handle
}

pub fn paint(painter: &egui::Painter, image: Rect, cell: f32) {
    let visible = image.intersect(painter.clip_rect());
    if !visible.is_positive() || !image.is_finite() {
        return;
    }
    let uv = Rect::from_min_max(
        ((visible.min - image.min) / (cell * 2.)).to_pos2(),
        ((visible.max - image.min) / (cell * 2.)).to_pos2(),
    );
    // A single repeated quad keeps work constant even at very large zoom levels.
    painter.image(texture(painter.ctx()).id(), visible, uv, Color32::WHITE);
}

#[cfg(test)]
pub fn is_backdrop(ctx: &egui::Context, id: egui::TextureId) -> bool {
    id == texture(ctx).id()
}
