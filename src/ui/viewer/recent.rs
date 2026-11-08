//! Recent folders and open actions share one responsive, centered overlay.
use super::*;

const ROW_HEIGHT: f32 = 68.;

impl Viewer {
    pub(super) fn reload_recent(&mut self) {
        if let Some(store) = &self.store {
            match store.recent() {
                Ok(recent) => self.recent = recent,
                Err(error) => self.last_error = error,
            }
        }
    }

    pub(super) fn recent_window(&mut self, ctx: &egui::Context) {
        if !self.recent_open {
            return;
        }
        let mut close = false;
        let mut action = None;
        let mut choice = None;
        let mut remove = None;
        let modal = overlay::show(ctx, "recent-folders", self.fullscreen, |ui| {
            close = overlay::header(ui, "recent", "recent-close");
            ui.horizontal(|ui| {
                let folder = command_supported("folder");
                let width = if folder {
                    (ui.available_width() - ui.spacing().item_spacing.x) / 2.
                } else {
                    ui.available_width()
                };
                for (id, label) in [("open", "open-image"), ("folder", "open-folder")] {
                    if command_supported(id) && open_button(ui, id, tr(label), width).clicked() {
                        action = Some(id);
                    }
                }
            });
            ui.add_space(4.);
            ui.separator();
            if !self.last_error.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(&self.last_error)
                            .small()
                            .color(Color32::from_rgb(244, 198, 145)),
                    )
                    .truncate(),
                )
                .on_hover_text(&self.last_error);
            }
            egui::ScrollArea::vertical()
                .id_salt("recent-folders-scroll")
                .max_height(ui.available_height().max(1.))
                .auto_shrink([false, false])
                .drag_to_scroll(crate::app::is_mobile())
                .show(ui, |ui| {
                    if self.recent.is_empty() {
                        ui.add_space(12.);
                        ui.label(
                            RichText::new(tr("recently-opened-items-will-appear-here"))
                                .color(MUTED),
                        );
                    }
                    for dir in &self.recent {
                        let (open, delete) = folder_row(ui, dir);
                        if open {
                            choice = Some(dir.clone());
                        }
                        if delete {
                            remove = Some(dir.clone());
                        }
                    }
                });
        });
        if close || modal.should_close() {
            self.recent_open = false;
        }
        if let Some(dir) = choice {
            let first = media::images_in_folder(&dir)
                .ok()
                .and_then(|images| images.min_by(|a, b| media::compare_file_names(a, b)));
            if let Some(first) = first {
                self.recent_open = false;
                // Open a concrete image so same-window/mobile opens also start
                // at the first page instead of retaining the previous focus.
                self.open_from_user(first);
            } else {
                // Access or folder contents may have changed since opening the list.
                remove = Some(dir);
            }
        }
        if let Some(dir) = remove {
            let result = self
                .store
                .as_ref()
                .map_or(Ok(()), |store| store.remove_recent(&dir));
            match result {
                Ok(()) => {
                    self.recent.retain(|path| path != &dir);
                    if self.remember.as_ref() == Some(&dir) {
                        self.remember = None;
                    }
                }
                Err(error) => self.last_error = error,
            }
        }
        // A cancelled picker keeps the start screen available. Opening in the same
        // window already closes it in open(); opening a new window closes it here.
        if let Some(action) = action {
            let had_images = !self.files.is_empty() || self.listing.is_some();
            self.run(action, ctx);
            if had_images {
                self.recent_open = false;
            }
        }
        self.held_key = None;
        self.held_button = None;
    }
}

fn open_button(ui: &mut egui::Ui, id: &str, label: &str, width: f32) -> egui::Response {
    let (_, rect) = ui.allocate_space(egui::vec2(width, 56.));
    let response = ui.interact(
        rect,
        egui::Id::new(("recent-action", id)),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    highlight(ui, rect, &response);
    let rtl = crate::i18n::is_rtl();
    let icon = if rtl {
        rect.right_center() - egui::vec2(24., 0.)
    } else {
        rect.left_center() + egui::vec2(24., 0.)
    };
    chrome_icons::paint(ui.painter(), icon, id, INK, false);
    single_line(
        ui,
        label,
        rect.left_top() + egui::vec2(if rtl { 8. } else { 46. }, 19.),
        width - 54.,
        14.,
        INK,
    );
    response
}

fn folder_row(ui: &mut egui::Ui, dir: &Path) -> (bool, bool) {
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), ROW_HEIGHT));
    let delete_rect = Rect::from_center_size(
        egui::pos2(rect.right() - 22., rect.center().y),
        Vec2::splat(44.),
    );
    let open_rect =
        Rect::from_min_max(rect.min, egui::pos2(delete_rect.left() - 4., rect.bottom()));
    let delete_rect = crate::ui::direction::mirror(delete_rect, rect);
    let open_rect = crate::ui::direction::mirror(open_rect, rect);
    let path = dir.display().to_string();
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let open = ui.interact(
        open_rect,
        egui::Id::new(("recent-folder", dir)),
        egui::Sense::click(),
    );
    open.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &path));
    let delete = ui.interact(
        delete_rect,
        egui::Id::new(("recent-remove", dir)),
        egui::Sense::click(),
    );
    delete.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{}: {name}", tr("remove-from-recent")),
        )
    });
    if ui.is_rect_visible(rect) {
        highlight(ui, open_rect, &open);
        highlight(ui, delete_rect, &delete);
        let width = (open_rect.width() - 20.).max(1.);
        single_line(
            ui,
            &name,
            open_rect.min + egui::vec2(10., 12.),
            width,
            16.,
            INK,
        );
        single_line(
            ui,
            &path,
            open_rect.min + egui::vec2(10., 38.),
            width,
            12.,
            MUTED,
        );
        chrome_icons::paint(ui.painter(), delete_rect.center(), "trash", MUTED, false);
        ui.painter().hline(
            rect.x_range(),
            rect.bottom(),
            egui::Stroke::new(1_f32, Color32::from_white_alpha(18)),
        );
    }
    (
        open.on_hover_text(path).clicked(),
        delete.on_hover_text(tr("remove-from-recent")).clicked(),
    )
}

fn single_line(
    ui: &egui::Ui,
    text: &str,
    position: egui::Pos2,
    width: f32,
    size: f32,
    color: Color32,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        FontId::proportional(size),
        color,
        width.max(1.),
    );
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    let x = if crate::i18n::is_rtl() {
        position.x + width - galley.size().x
    } else {
        position.x
    };
    ui.painter()
        .galley(egui::pos2(x, position.y), galley, color);
}

fn highlight(ui: &egui::Ui, rect: Rect, response: &egui::Response) {
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 6., Color32::from_white_alpha(22));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.),
            5.,
            egui::Stroke::new(1_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}
