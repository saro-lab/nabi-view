//! Centered command menu using the same actions as keyboard shortcuts.
use super::*;

const ITEMS: &[&str] = &[
    "open",
    "folder",
    "recent",
    "new-window",
    "metadata",
    "fit",
    "width",
    "height",
    "fill",
    "actual",
    "single",
    "spread-left",
    "spread-right",
    "flip-horizontal",
    "flip-vertical",
    "check",
    "move",
    "trash",
    "undo-trash",
    "fullscreen",
];
const TILE_HEIGHT: f32 = 108.;
const GAP: f32 = 8.;

impl Viewer {
    pub(super) fn menu_window(&mut self, ctx: &egui::Context) {
        if !self.menu_open {
            return;
        }
        let mut items: Vec<_> = ITEMS
            .iter()
            .copied()
            .filter(|id| command_supported(id))
            .collect();
        if crate::app::is_mobile() {
            items.extend(["random", "slideshow"]);
        }
        let mut selected = None;
        let mut close = false;
        let modal = overlay::show(ctx, "command-menu", self.fullscreen, |ui| {
            close = overlay::header(ui, "menu", "menu-close");
            egui::ScrollArea::vertical()
                .id_salt("command-menu-scroll")
                .max_height(ui.available_height().max(1.))
                .auto_shrink([false, false])
                .drag_to_scroll(crate::app::is_mobile())
                .show(ui, |ui| {
                    // Reserve room even with the platform's floating scrollbar style.
                    let width = (ui.available_width() - 14.).max(100.);
                    let columns = ((width + GAP) / (124. + GAP)).floor().clamp(1., 5.) as usize;
                    let tile_width = (width - GAP * (columns - 1) as f32) / columns as f32;
                    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                    for row in items.chunks(columns) {
                        ui.horizontal(|ui| {
                            for &id in row {
                                let label = COMMANDS.iter().find(|c| c.0 == id).unwrap().1;
                                let keys = if crate::app::is_mobile() {
                                    String::new()
                                } else {
                                    self.shortcut_text(id)
                                };
                                if tile(ui, id, tr(label), &keys, tile_width, self.menu_active(id))
                                    .clicked()
                                {
                                    selected = Some(id);
                                }
                            }
                        });
                    }
                });
        });
        if close || modal.should_close() || selected.is_some() {
            self.menu_open = false;
            self.held_key = None;
            self.held_button = None;
        }
        // Close the layer before opening a dialog or dispatching the command.
        if let Some(id) = selected {
            self.run(id, ctx);
        }
    }

    fn menu_active(&self, id: &str) -> bool {
        match id {
            "metadata" => self.inspector.open,
            "fit" | "width" | "height" | "fill" | "actual" => {
                self.zoom_override.is_none() && self.settings.view_mode == id
            }
            "single" => !self.settings.spread,
            "spread-left" => self.settings.spread && self.settings.rtl,
            "spread-right" => self.settings.spread && !self.settings.rtl,
            "flip-horizontal" => self.settings.flip_horizontal,
            "flip-vertical" => self.settings.flip_vertical,
            "fullscreen" => self.fullscreen,
            _ => false,
        }
    }
}

fn tile(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    keys: &str,
    width: f32,
    active: bool,
) -> egui::Response {
    let (_, rect) = ui.allocate_space(egui::vec2(width, TILE_HEIGHT));
    let response = ui.interact(
        rect,
        egui::Id::new(("menu-command", id)),
        egui::Sense::click(),
    );
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    if ui.is_rect_visible(rect) {
        let color = if active { ACCENT } else { INK };
        let painter = ui.painter();
        if response.hovered() || response.has_focus() {
            painter.rect_filled(rect, 6., Color32::from_white_alpha(22));
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect.shrink(1.),
                7.,
                egui::Stroke::new(1_f32, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        chrome_icons::paint(
            painter,
            egui::pos2(rect.center().x, rect.top() + 24.),
            id,
            color,
            active,
        );
        let text = painter.layout(
            label.to_owned(),
            FontId::proportional(13.),
            color,
            width - 12.,
        );
        let shortcut_top = rect.top() + 44. + text.size().y + 2.;
        painter.galley(
            egui::pos2(rect.center().x - text.size().x / 2., rect.top() + 44.),
            text,
            color,
        );
        let mut job = egui::text::LayoutJob::simple(
            keys.to_owned(),
            FontId::proportional(10.),
            MUTED,
            width - 12.,
        );
        job.wrap.max_rows = 1;
        let shortcut = painter.layout_job(job);
        painter.galley(
            egui::pos2(rect.center().x - shortcut.size().x / 2., shortcut_top),
            shortcut,
            MUTED,
        );
    }
    response.on_hover_text(if keys.is_empty() {
        label.to_owned()
    } else {
        format!("{label}\n{keys}")
    })
}
