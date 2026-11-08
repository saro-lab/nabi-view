use super::{ACCENT, MUTED, egui, tr};

pub(super) fn button(ui: &mut egui::Ui, name: &str, label: &str, active: bool) -> egui::Response {
    button_with_shortcut(ui, name, label, active, "")
}

pub(super) fn button_with_shortcut(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    active: bool,
    keys: &str,
) -> egui::Response {
    let size = if crate::app::is_mobile() {
        44.
    } else if ui.ctx().screen_rect().width() < 700. {
        28.
    } else {
        32.
    };
    sized_button(ui, name, label, active, keys, size)
}

pub(super) fn sized_button(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    active: bool,
    keys: &str,
    size: f32,
) -> egui::Response {
    let (_, rect) = ui.allocate_space(egui::Vec2::splat(size));
    let response = ui.interact(
        rect,
        egui::Id::new((
            "titlebar",
            if name.starts_with("spread") {
                "spread"
            } else {
                name
            },
        )),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, tr(label))
    });
    let painter = ui.painter();
    let color = if active { ACCENT } else { MUTED };
    if active || response.hovered() || response.has_focus() {
        painter.rect_filled(
            rect,
            6.,
            if active {
                egui::Color32::from_rgb(48, 62, 39)
            } else {
                egui::Color32::from_gray(48)
            },
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.),
            5.,
            egui::Stroke::new(1_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    paint(painter, rect.center(), name, color, active);
    tooltip(response, label, keys)
}

pub(super) fn tooltip(response: egui::Response, label: &str, keys: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.label(tr(label));
        if !keys.is_empty() {
            ui.label(egui::RichText::new(keys).size(11.).weak());
        }
    })
}

pub(super) fn paint(
    painter: &egui::Painter,
    c: egui::Pos2,
    name: &str,
    color: egui::Color32,
    active: bool,
) {
    let stroke = egui::Stroke::new(1.6_f32, color);
    let point = |x: f32, y: f32| c + egui::vec2(x, y);
    let line = |a, b| {
        painter.line_segment([a, b], stroke);
    };
    match name {
        "open" => {
            painter.rect_stroke(
                egui::Rect::from_min_max(point(-10., -9.), point(10., 9.)),
                2.,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(point(4., -3.), 2., stroke);
            painter.add(egui::Shape::line(
                vec![
                    point(-9., 6.),
                    point(-3., -1.),
                    point(2., 4.),
                    point(6., 1.),
                    point(9., 5.),
                ],
                stroke,
            ));
        }
        "folder" | "move-browse" => {
            painter.add(egui::Shape::closed_line(
                vec![
                    point(-10., -7.),
                    point(-3., -7.),
                    point(0., -4.),
                    point(10., -4.),
                    point(10., 8.),
                    point(-10., 8.),
                ],
                stroke,
            ));
            line(point(-10., -1.), point(10., -1.));
        }
        "recent" => {
            painter.circle_stroke(c, 9., stroke);
            line(c, point(0., -6.));
            line(c, point(5., 3.));
        }
        "new-window" | "maximize" => {
            painter.rect_stroke(
                egui::Rect::from_min_max(point(-10., -8.), point(10., 8.)),
                2.,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(point(-10., -3.), point(10., -3.));
            if name == "new-window" {
                line(point(-3., 2.), point(3., 2.));
                line(point(0., -1.), point(0., 5.));
            }
        }
        "metadata" => {
            painter.circle_stroke(c, 9., stroke);
            painter.circle_filled(point(0., -4.), 1., color);
            line(point(0., 0.), point(0., 5.));
        }
        "fit" | "fill" | "width" | "height" | "actual" | "fullscreen" => {
            for x in [-1., 1.] {
                for y in [-1., 1.] {
                    line(point(5. * x, 10. * y), point(10. * x, 10. * y));
                    line(point(10. * x, 10. * y), point(10. * x, 5. * y));
                }
            }
            match name {
                "fit" | "fill" => {
                    let (w, h) = if name == "fit" { (5., 7.) } else { (8., 8.) };
                    painter.rect_stroke(
                        egui::Rect::from_min_max(point(-w, -h), point(w, h)),
                        1.,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                "width" | "height" => {
                    let p = |x, y| {
                        if name == "width" {
                            point(x, y)
                        } else {
                            point(y, x)
                        }
                    };
                    line(p(-7., 0.), p(7., 0.));
                    for direction in [-1., 1.] {
                        line(p(4. * direction, -3.), p(7. * direction, 0.));
                        line(p(4. * direction, 3.), p(7. * direction, 0.));
                    }
                }
                "actual" => {
                    painter.text(
                        c,
                        egui::Align2::CENTER_CENTER,
                        "1:1",
                        egui::FontId::proportional(10.),
                        color,
                    );
                }
                _ => {}
            }
        }
        "flip-horizontal" | "flip-vertical" => {
            let p = |x, y| {
                if name == "flip-horizontal" {
                    point(x, y)
                } else {
                    point(y, x)
                }
            };
            for y in [-9., -3., 3.] {
                line(p(0., y), p(0., y + 3.));
            }
            for direction in [-1., 1.] {
                painter.add(egui::Shape::closed_line(
                    vec![
                        p(3. * direction, -6.),
                        p(9. * direction, 6.),
                        p(3. * direction, 6.),
                    ],
                    stroke,
                ));
            }
        }
        "check" => {
            painter.rect_stroke(
                egui::Rect::from_min_max(point(-9., -9.), point(9., 9.)),
                3.,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::line(
                vec![point(-5., 0.), point(-1., 4.), point(6., -4.)],
                stroke,
            ));
        }
        "move" => {
            painter.add(egui::Shape::line(
                vec![
                    point(0., -7.),
                    point(-3., -10.),
                    point(-10., -10.),
                    point(-10., 9.),
                    point(0., 9.),
                ],
                stroke,
            ));
            line(point(-5., 0.), point(10., 0.));
            line(point(5., -5.), point(10., 0.));
            line(point(5., 5.), point(10., 0.));
        }
        "trash" | "move-history-clear" | "move-history-remove" => {
            line(point(-10., -6.), point(10., -6.));
            painter.add(egui::Shape::line(
                vec![
                    point(-7., -6.),
                    point(-6., 9.),
                    point(6., 9.),
                    point(7., -6.),
                ],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    point(-4., -6.),
                    point(-4., -9.),
                    point(4., -9.),
                    point(4., -6.),
                ],
                stroke,
            ));
            for x in [-2., 2.] {
                line(point(x, -2.), point(x, 5.));
            }
        }
        "undo-trash" => {
            painter.add(egui::Shape::line(
                vec![point(-4., -9.), point(-10., -3.), point(-4., 3.)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    point(-10., -3.),
                    point(3., -3.),
                    point(8., 1.),
                    point(8., 6.),
                    point(4., 9.),
                    point(-2., 9.),
                ],
                stroke,
            ));
        }
        "minimize" => line(point(-8., 5.), point(8., 5.)),
        "close-all" => {
            painter.rect_stroke(
                egui::Rect::from_min_max(point(-6., -6.), point(10., 10.)),
                2.,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::line(
                vec![point(-10., 5.), point(-10., -10.), point(5., -10.)],
                stroke,
            ));
            line(point(-2., -2.), point(6., 6.));
            line(point(-2., 6.), point(6., -2.));
        }
        "reveal" => {
            painter.circle_stroke(c, 8., stroke);
            for direction in [-1., 1.] {
                line(point(3. * direction, 0.), point(11. * direction, 0.));
                line(point(0., 3. * direction), point(0., 11. * direction));
            }
            painter.circle_filled(c, 1.2, color);
        }
        "slideshow" => {
            if active {
                for x in [-4., 4.] {
                    line(point(x, -7.), point(x, 7.));
                }
            } else {
                painter.add(egui::Shape::closed_line(
                    vec![point(-5., -8.), point(8., 0.), point(-5., 8.)],
                    stroke,
                ));
            }
        }
        "random" => {
            line(point(-10., -6.), point(-5., -6.));
            line(point(-5., -6.), point(5., 6.));
            line(point(5., 6.), point(10., 6.));
            line(point(-10., 6.), point(-5., 6.));
            line(point(-5., 6.), point(5., -6.));
            line(point(5., -6.), point(10., -6.));
            for y in [-6., 6.] {
                line(point(7., y - 3.), point(10., y));
                line(point(7., y + 3.), point(10., y));
            }
        }
        "spread" | "single" => {
            painter.add(egui::Shape::closed_line(
                vec![
                    point(-8., -8.),
                    point(8., -8.),
                    point(8., 8.),
                    point(-8., 8.),
                ],
                stroke,
            ));
            line(point(-4., -2.), point(4., -2.));
            line(point(-4., 3.), point(2., 3.));
        }
        "spread-ltr" | "spread-rtl" | "spread-left" | "spread-right" => {
            // Match the 24×24 split-arrow references, centered in the button.
            let stroke = egui::Stroke::new(1.5_f32, color);
            let line = |a, b| {
                painter.line_segment([a, b], stroke);
            };
            painter.rect_stroke(
                egui::Rect::from_min_max(point(-10.5, -7.), point(10.5, 7.)),
                2.,
                stroke,
                egui::StrokeKind::Middle,
            );
            line(point(0., -7.), point(0., 7.));
            let direction = if matches!(name, "spread-rtl" | "spread-left") {
                -1.
            } else {
                1.
            };
            for y in [-1.75, 1.75] {
                line(point(-8. * direction, y), point(-2.5 * direction, y));
            }
            let tip = point(8. * direction, 0.);
            line(point(2.5 * direction, 0.), tip);
            painter.add(egui::Shape::line(
                vec![
                    point(5.75 * direction, -2.25),
                    tip,
                    point(5.75 * direction, 2.25),
                ],
                stroke,
            ));
        }

        "pin" => {
            painter.add(egui::Shape::closed_line(
                vec![
                    point(-4., -8.),
                    point(4., -8.),
                    point(3., -2.),
                    point(6., 2.),
                    point(-6., 2.),
                    point(-3., -2.),
                ],
                stroke,
            ));
            line(point(0., 2.), point(0., 10.));
        }
        "menu" => {
            for y in [-6., 0., 6.] {
                line(point(-8., y), point(8., y));
            }
        }
        "settings" => {
            let teeth = (0..32)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / 32.;
                    let radius = if matches!(i % 4, 0 | 3) { 9. } else { 7. };
                    point(angle.cos() * radius, angle.sin() * radius)
                })
                .collect();
            painter.add(egui::Shape::closed_line(teeth, stroke));
            painter.circle_stroke(c, 3., stroke);
        }
        "close" | "settings-close" | "menu-close" | "recent-close" | "move-close"
        | "close-window" => {
            line(point(-6., -6.), point(6., 6.));
            line(point(-6., 6.), point(6., -6.));
        }
        _ => {}
    }
}
