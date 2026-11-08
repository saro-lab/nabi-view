use super::*;

impl Viewer {
    pub fn show_toast(&mut self, ctx: &egui::Context, message: impl Into<String>) {
        self.toast = Some(Toast {
            message: message.into(),
            deadline: std::time::Instant::now()
                + std::time::Duration::from_millis(self.settings.toast_ms as u64),
            image: self.files.get(self.focus_index).cloned(),
            keep_on_slideshow: false,
        });
        ctx.request_repaint();
    }

    pub(super) fn show_setting_toast(&mut self, ctx: &egui::Context, message: impl Into<String>) {
        self.show_toast(ctx, message);
        if let Some(toast) = &mut self.toast {
            toast.keep_on_slideshow = true;
        }
    }

    pub(super) fn show_slide_interval_toast(&mut self, ctx: &egui::Context) {
        self.show_setting_toast(
            ctx,
            crate::i18n::value_label(
                "slide-interval",
                &format!(
                    "{:.1} {}",
                    self.settings.slide_ms as f64 / 1000.,
                    tr("seconds")
                ),
            ),
        );
    }

    pub(super) fn show_action_toast(&mut self, id: &str, ctx: &egui::Context) {
        let toggle = |label, enabled| {
            crate::i18n::value_label(label, tr(if enabled { "on" } else { "off" }))
        };
        let message = match id {
            "single" | "spread-left" | "spread-right" | "spread" => {
                let layout = if !self.settings.spread {
                    "single-page-view"
                } else if self.settings.rtl {
                    "spread-left"
                } else {
                    "spread-right"
                };
                crate::i18n::value_label("layout", tr(layout))
            }
            "fit" | "fill" | "actual" | "width" | "height" => {
                let label = match id {
                    "fit" => "fit-entire-image",
                    "fill" => "fill-viewer",
                    "actual" => "actual-size-100",
                    "width" => "fit-width",
                    _ => "fit-height",
                };
                crate::i18n::value_label("image-display", tr(label))
            }
            "in" | "out" => crate::i18n::value_label(
                "zoom",
                &format!("{}%", format_zoom_percent(self.zoom_percent(ctx))),
            ),
            "flip-horizontal" => toggle("flip-horizontally", self.settings.flip_horizontal),
            "flip-vertical" => toggle("flip-vertically", self.settings.flip_vertical),
            "rtl" => crate::i18n::value_label(
                "reading-direction",
                tr(if self.settings.rtl {
                    "right-to-left"
                } else {
                    "left-to-right"
                }),
            ),
            "cover" => toggle("show-first-page-alone", self.settings.cover),
            "slideshow" => toggle("slideshow", self.slideshow),
            "random" if !self.random_prompt => toggle("random-order", self.random),
            "fullscreen" => toggle("full-screen", self.fullscreen),
            "maximize" => tr(if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
                "window-restored"
            } else {
                "window-maximized"
            })
            .to_owned(),
            "chrome" => toggle("pin-controls", !self.hide_controls),
            "metadata" => toggle("image-metadata", self.inspector.open),
            "remember" => toggle("remember-recent-items", self.settings.remember_recent),
            "check" | "check-prev" | "check-next" if !self.random && !self.files.is_empty() => {
                crate::i18n::format(
                    "value-label",
                    &[
                        ("label", tr("checked-images")),
                        ("value", &self.checked.len().to_string()),
                    ],
                )
            }
            _ => return,
        };
        if matches!(id, "in" | "out") {
            // Manual zoom resets on the next image; its value belongs to this page.
            self.show_toast(ctx, message);
        } else {
            self.show_setting_toast(ctx, message);
        }
    }

    pub(super) fn toast_overlay(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else { return };
        let now = std::time::Instant::now();
        if now >= toast.deadline || toast.image.as_ref() != self.files.get(self.focus_index) {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(toast.deadline - now);
        egui::Area::new(egui::Id::new("viewer-toast"))
            .order(egui::Order::Tooltip)
            .anchor(
                if crate::i18n::is_rtl() {
                    egui::Align2::LEFT_TOP
                } else {
                    egui::Align2::RIGHT_TOP
                },
                egui::vec2(if crate::i18n::is_rtl() { 14. } else { -14. }, 52.),
            )
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width((ctx.screen_rect().width() - 48.).clamp(80., 360.));
                    ui.label(RichText::new(&toast.message).color(INK));
                });
            });
    }
}
