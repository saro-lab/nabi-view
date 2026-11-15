//! Shared appearance and responsive sizing for the menu and library overlays.
use super::*;

pub(super) fn background() -> Color32 {
    Color32::from_rgba_unmultiplied(21, 23, 25, 250)
}

/// Up to this window width, overlays and settings fill the whole window.
const COMPACT_WIDTH: f32 = 600.;

pub(super) fn compact(ctx: &egui::Context) -> bool {
    ctx.viewport_rect().width() <= COMPACT_WIDTH
}

pub(super) fn header(ui: &mut egui::Ui, title: &str, close_id: &str) -> bool {
    if !compact(ui.ctx()) {
        return false;
    }
    let mut close = false;
    crate::ui::direction::horizontal(ui, |ui| {
        ui.heading(tr(title));
        ui.with_layout(crate::ui::direction::trailing(), |ui| {
            close = forms::icon(ui, close_id, "back-to-viewer").clicked();
        });
    });
    ui.add_space(4.);
    close
}

pub(super) fn show<T>(
    ctx: &egui::Context,
    id: &'static str,
    fullscreen: bool,
    content: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {
    show_impl(ctx, id, fullscreen, false, content).expect("modal response")
}

/// The empty viewer's menu is a base screen, so its titlebar remains usable.
pub(super) fn show_menu(
    ctx: &egui::Context,
    fullscreen: bool,
    default_menu: bool,
    content: impl FnOnce(&mut egui::Ui),
) -> bool {
    show_impl(ctx, "command-menu", fullscreen, default_menu, content)
        .is_some_and(|modal| modal.should_close())
}

fn show_impl<T>(
    ctx: &egui::Context,
    id: &'static str,
    fullscreen: bool,
    base_screen: bool,
    content: impl FnOnce(&mut egui::Ui) -> T,
) -> Option<egui::ModalResponse<T>> {
    let screen = ctx.viewport_rect();
    let compact = compact(ctx);
    let size = if compact {
        screen.size()
    } else {
        (screen.size() - Vec2::splat(48.)).min(egui::vec2(640., 560.))
    };
    let margin = if compact { 12 } else { 20 };
    let id = egui::Id::unique(id);
    let draggable = !fullscreen && !crate::app::is_mobile();
    let frame = egui::Frame::NONE
        .fill(background())
        .corner_radius(if compact { 0 } else { 12 })
        .inner_margin(margin);
    let content = |ui: &mut egui::Ui| {
        forms::style(ui);
        let inner = (size - Vec2::splat(2. * margin as f32)).max(Vec2::splat(1.));
        ui.set_min_size(inner);
        ui.set_max_size(inner);
        if draggable {
            chrome_drag(ui, ui.min_rect().expand(margin as f32));
        }
        ui.with_layout(crate::ui::direction::vertical(), content)
            .inner
    };
    if base_screen {
        let area = egui::Area::new(id)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .sense(egui::Sense::hover())
            .movable(false)
            .default_size(size)
            .show(ctx, |ui| {
                ui.scope_builder(
                    egui::UiBuilder::new().sense(egui::Sense::CLICK | egui::Sense::DRAG),
                    |ui| frame.show(ui, content),
                )
            });
        ctx.move_to_top(area.response.layer_id);
        return None;
    }
    let modal = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).default_size(size))
        .frame(frame)
        .backdrop_color(Color32::TRANSPARENT)
        .show(ctx, content);
    // Hover-revealed viewer bars share Foreground order. Keep the modal above
    // newly visible bars, also in wide windows too short to leave room for both.
    if modal.is_top_modal && !modal.any_popup_open {
        ctx.move_to_top(modal.response.layer_id);
    }
    // The backdrop keeps its click-to-dismiss behavior; a drag only moves the window.
    if draggable
        && modal
            .backdrop_response
            .dragged_by(egui::PointerButton::Primary)
    {
        window_frame::drag(ctx, &modal.backdrop_response);
    }
    Some(modal)
}
