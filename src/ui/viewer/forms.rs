//! Shared sizing for settings and dialog controls, independent of viewer chrome.
use super::*;

pub(super) const GAP: f32 = 8.;

pub(super) fn height() -> f32 {
    if crate::app::is_mobile() {
        TOUCH_TARGET
    } else {
        36.
    }
}

pub(super) fn style(ui: &mut egui::Ui) {
    // ComboBox positions its contents using padding before applying interact_size.
    let content_height = ui
        .text_style_height(&egui::TextStyle::Button)
        .max(ui.spacing().icon_width);
    // Widget frames use integer margins; round down and let min_size fill the remainder.
    let spacing = ui.spacing_mut();
    spacing.interact_size = Vec2::splat(height());
    spacing.item_spacing = Vec2::splat(GAP);
    spacing.button_padding = egui::vec2(10., ((height() - content_height) / 2.).floor().max(0.));
}

pub(super) fn text_edit(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .font(egui::TextStyle::Body)
        .margin(egui::Margin::symmetric(10, 6))
        .min_size(egui::vec2(0., height()))
        .align(egui::Align2::LEFT_CENTER)
}

pub(super) fn number(ui: &mut egui::Ui, value: egui::DragValue<'_>) -> egui::Response {
    ui.add_sized([80., height()], value)
}

pub(super) fn button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let text_width = ui
        .painter()
        .layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            INK,
        )
        .size()
        .x;
    let natural_width = (text_width + ui.spacing().button_padding.x * 2.).max(height());
    let width = natural_width.min(ui.max_rect().width());
    // Reserve a whole control before a wrapping row chooses its next line.
    let response = ui.add_sized([width, height()], egui::Button::new(label).truncate());
    if width < natural_width {
        response.hover_text(label)
    } else {
        response
    }
}

pub(super) fn default_button(ui: &mut egui::Ui) -> egui::Response {
    ui.add_sized([78., height()], egui::Button::new(tr("default")).truncate())
        .hover_text(tr("default"))
}

pub(super) fn icon(ui: &mut egui::Ui, name: &str, label: &str) -> egui::Response {
    chrome_icons::sized_button(ui, name, label, false, "", height())
}
