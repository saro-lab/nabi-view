//! A display-only transform shared by whole images, retained frames and crops.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ImageTransform {
    pub quarter_turns: u8,
    pub horizontal: bool,
    pub vertical: bool,
}

impl ImageTransform {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            quarter_turns: settings.rotation_quarter_turns % 4,
            horizontal: settings.flip_horizontal,
            vertical: settings.flip_vertical,
        }
    }

    /// Axis-aligned display dimensions; applying this twice restores the size.
    pub fn size(self, source: Vec2) -> Vec2 {
        if self.quarter_turns % 2 == 0 {
            source
        } else {
            egui::vec2(source.y, source.x)
        }
    }

    pub fn source_to_display(self, source: egui::Pos2) -> egui::Pos2 {
        let mut display = match self.quarter_turns % 4 {
            1 => egui::pos2(1. - source.y, source.x),
            2 => egui::pos2(1. - source.x, 1. - source.y),
            3 => egui::pos2(source.y, 1. - source.x),
            _ => source,
        };
        if self.horizontal {
            display.x = 1. - display.x;
        }
        if self.vertical {
            display.y = 1. - display.y;
        }
        display
    }

    pub fn display_to_source(self, mut display: egui::Pos2) -> egui::Pos2 {
        if self.horizontal {
            display.x = 1. - display.x;
        }
        if self.vertical {
            display.y = 1. - display.y;
        }
        match self.quarter_turns % 4 {
            1 => egui::pos2(display.y, 1. - display.x),
            2 => egui::pos2(1. - display.x, 1. - display.y),
            3 => egui::pos2(1. - display.y, display.x),
            _ => display,
        }
    }

    pub fn source_rect_to_display(self, source: Rect) -> Rect {
        Rect::from_two_pos(
            self.source_to_display(source.min),
            self.source_to_display(source.max),
        )
    }

    pub fn display_rect_to_source(self, display: Rect) -> Rect {
        Rect::from_two_pos(
            self.display_to_source(display.min),
            self.display_to_source(display.max),
        )
    }

    pub fn paint(self, painter: &egui::Painter, texture: egui::TextureId, rect: Rect) {
        // Rotating UVs avoids pixel copies and uses the same HDR texture path.
        let mut mesh = egui::epaint::Mesh::with_texture(texture);
        mesh.add_rect_with_uv(rect, uv(), Color32::WHITE);
        for vertex in &mut mesh.vertices {
            vertex.uv = self.display_to_source(vertex.uv);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clockwise_rotation_and_screen_flips_map_asymmetric_source_corners() {
        let corners = [
            egui::pos2(0., 0.),
            egui::pos2(1., 0.),
            egui::pos2(1., 1.),
            egui::pos2(0., 1.),
        ];
        for quarter_turns in 0..4 {
            for horizontal in [false, true] {
                for vertical in [false, true] {
                    let transform = ImageTransform {
                        quarter_turns,
                        horizontal,
                        vertical,
                    };
                    for (index, source) in corners.into_iter().enumerate() {
                        let mut expected = corners[(index + quarter_turns as usize) % 4];
                        if horizontal {
                            expected.x = 1. - expected.x;
                        }
                        if vertical {
                            expected.y = 1. - expected.y;
                        }
                        assert_eq!(transform.source_to_display(source), expected);
                        assert_eq!(transform.display_to_source(expected), source);
                    }
                    let dimensions = egui::vec2(640., 320.);
                    assert_eq!(transform.size(transform.size(dimensions)), dimensions);
                }
            }
        }
    }

    #[test]
    fn visible_detail_crop_round_trips_through_all_rotations_and_flips() {
        // Use exact binary fractions to make incorrect axis/sign combinations
        // visible independently from floating-point rounding.
        let source = Rect::from_min_max(egui::pos2(0.125, 0.25), egui::pos2(0.5, 0.875));
        for quarter_turns in 0..4 {
            for horizontal in [false, true] {
                for vertical in [false, true] {
                    let transform = ImageTransform {
                        quarter_turns,
                        horizontal,
                        vertical,
                    };
                    let display = transform.source_rect_to_display(source);
                    assert_eq!(transform.display_rect_to_source(display), source);
                    assert_eq!(display.size(), transform.size(source.size()));
                }
            }
        }
        let clockwise = ImageTransform {
            quarter_turns: 1,
            ..Default::default()
        };
        assert_eq!(
            clockwise.display_rect_to_source(Rect::from_min_max(
                egui::pos2(0., 0.),
                egui::pos2(0.25, 0.5),
            )),
            Rect::from_min_max(egui::pos2(0., 0.75), egui::pos2(0.5, 1.)),
        );
    }

    #[test]
    fn retained_detail_tiles_rotate_with_their_base_image_and_return_without_drift() {
        let ctx = egui::Context::default();
        let handle = ctx.load_texture(
            "rotation-retained-tile",
            egui::ColorImage::filled([8, 4], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let canvas = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000., 800.));
        let backdrop = Rect::from_center_size(canvas.center(), egui::vec2(600., 300.));
        let crop = Rect::from_min_size(backdrop.min, egui::vec2(300., 75.));
        let mut presentation = Presentation {
            images: [backdrop, crop]
                .into_iter()
                .map(|rect| PresentedImage {
                    hdr: None,
                    handle: handle.clone(),
                    rect,
                    clip: canvas,
                    backdrop,
                })
                .collect(),
            canvas: Some(canvas),
            ..Default::default()
        };
        presentation.reorient(ImageTransform {
            quarter_turns: 1,
            ..Default::default()
        });
        assert_eq!(
            presentation.images[0].rect,
            Rect::from_min_size(egui::pos2(350., 100.), egui::vec2(300., 600.)),
        );
        assert_eq!(
            presentation.images[1].rect,
            Rect::from_min_size(egui::pos2(575., 100.), egui::vec2(75., 300.)),
        );
        for quarter_turns in [2, 3, 0] {
            presentation.reorient(ImageTransform {
                quarter_turns,
                ..Default::default()
            });
        }
        assert_eq!(presentation.images[0].rect, backdrop);
        assert_eq!(presentation.images[1].rect, crop);
        assert_eq!(presentation.images[1].backdrop, backdrop);
    }
}
