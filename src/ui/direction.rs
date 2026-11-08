//! UI reading direction is independent of the image spread's reading direction.
use eframe::egui::{self, Align, Layout, Rect};
pub fn leading() -> Align {
    if crate::i18n::is_rtl() {
        Align::Max
    } else {
        Align::Min
    }
}
pub fn vertical() -> Layout {
    Layout::top_down(leading())
}
pub fn flow() -> Layout {
    if crate::i18n::is_rtl() {
        Layout::right_to_left(Align::Center)
    } else {
        Layout::left_to_right(Align::Center)
    }
}
pub fn trailing() -> Layout {
    if crate::i18n::is_rtl() {
        Layout::left_to_right(Align::Center)
    } else {
        Layout::right_to_left(Align::Center)
    }
}
pub fn mirror(rect: Rect, within: Rect) -> Rect {
    if crate::i18n::is_rtl() {
        Rect::from_min_max(
            egui::pos2(within.left() + within.right() - rect.right(), rect.top()),
            egui::pos2(within.left() + within.right() - rect.left(), rect.bottom()),
        )
    } else {
        rect
    }
}

pub fn horizontal<R>(
    ui: &mut egui::Ui,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let size = egui::vec2(
        ui.available_size_before_wrap().x,
        ui.spacing().interact_size.y,
    );
    ui.allocate_ui_with_layout(size, flow(), content)
}
