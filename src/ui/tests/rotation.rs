use super::*;

/// Read source corners from the actual painted mesh, in screen TL/TR/BR/BL order.
fn canvas_corners(window: &TestWindow) -> Vec<[egui::Pos2; 4]> {
    window
        .paint
        .iter()
        .filter_map(|shape| match &shape.shape {
            Shape::Mesh(mesh)
                if window.is_photo_texture(mesh.texture_id) && shape.clip_rect.height() > 200. =>
            {
                let rect = mesh.calc_bounds();
                Some(
                    [
                        rect.left_top(),
                        rect.right_top(),
                        rect.right_bottom(),
                        rect.left_bottom(),
                    ]
                    .map(|corner| {
                        mesh.vertices
                            .iter()
                            .find(|vertex| vertex.pos.distance(corner) < 0.01)
                            .expect("image corner vertex")
                            .uv
                    }),
                )
            }
            _ => None,
        })
        .collect()
}

#[test]
fn rotation_and_screen_flips_follow_each_command_without_changing_source_pixels() {
    let mut settings = crate::core::settings::Settings {
        language: crate::i18n::Language::Korean,
        ..Default::default()
    };
    settings
        .keyboard_shortcuts
        .as_mut()
        .unwrap()
        .push(crate::core::settings::KeyboardShortcut {
            action: "rotate-left".into(),
            key: "Z".into(),
        });
    let mut window = TestWindow::sample_with_settings(&settings);
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    let path = window
        ._images
        .as_ref()
        .unwrap()
        .path()
        .join("01 Alpine Light.png");
    let original = std::fs::read(&path).unwrap();
    let size = image::image_dimensions(&path).unwrap();
    let texture = window.photo_texture();
    let mut expected = [
        egui::pos2(0., 0.),
        egui::pos2(1., 0.),
        egui::pos2(1., 1.),
        egui::pos2(0., 1.),
    ];
    assert_eq!(canvas_corners(&window), vec![expected]);
    // Clockwise/counterclockwise and both mirrors must compose in screen space.
    for key in [
        Key::R,
        Key::T,
        Key::R,
        Key::Y,
        Key::Z,
        Key::Z,
        Key::R,
        Key::R,
        Key::R,
        Key::R,
    ] {
        expected = match key {
            Key::R => [expected[3], expected[0], expected[1], expected[2]],
            Key::Z => [expected[1], expected[2], expected[3], expected[0]],
            Key::T => [expected[1], expected[0], expected[3], expected[2]],
            Key::Y => [expected[3], expected[2], expected[1], expected[0]],
            _ => unreachable!(),
        };
        window.draw(Some(key));
        assert_eq!(canvas_corners(&window), vec![expected], "after {key:?}");
        assert_eq!(
            window.photo_texture(),
            texture,
            "display transforms must reuse pixels"
        );
        let stored = saved_viewer_settings(&window);
        let (_, photo) = window.photo_rects();
        let aspect = if stored.rotation_quarter_turns % 2 == 0 {
            size.0 as f32 / size.1 as f32
        } else {
            size.1 as f32 / size.0 as f32
        };
        assert!((photo.aspect_ratio() - aspect).abs() < 0.001);
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);

    // The retired U binding no longer flips the image.
    let before = saved_viewer_settings(&window);
    window.draw(Some(Key::U));
    assert_eq!(saved_viewer_settings(&window), before);
    window.draw(Some(Key::D));
    window.wait_for("02 Pelagic.png");
    window.wait_for_texture_replacement(texture);
    assert_eq!(canvas_corners(&window), vec![expected]);

    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        Some(path),
        Ok(store),
    );
    window.reveal_controls();
    window.wait_for("1 / 5");
    assert_eq!(canvas_corners(&window), vec![expected]);
}

#[test]
fn mobile_transform_defaults_and_both_rotation_menu_commands_work() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(1200., 1000.);
    window.wait_for("1 / 5");
    for key in [Key::R, Key::T, Key::Y] {
        window.draw(Some(key));
    }
    let settings = saved_viewer_settings(&window);
    assert_eq!(settings.rotation_quarter_turns, 1);
    assert!(settings.flip_horizontal && settings.flip_vertical);
    for key in [Key::T, Key::Y] {
        window.draw(Some(key));
    }

    for (action, expected) in [("rotate-left", 0), ("rotate-right", 1)] {
        let menu = window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "menu")))
            .unwrap()
            .rect
            .center();
        window.click(menu);
        for _ in 0..3 {
            window.draw(None);
        }
        let command = window
            .ctx
            .read_response(egui::Id::unique(("menu-command", action)))
            .unwrap();
        window.click(command.rect.center());
        let settings = saved_viewer_settings(&window);
        assert_eq!(settings.rotation_quarter_turns, expected, "{action}");
        assert!(!settings.flip_horizontal && !settings.flip_vertical);
    }
}

#[test]
fn rotated_spread_keeps_each_page_fitted_and_thumbnails_upright() {
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(1280., 760.);
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::CloseBracket));
    window.draw(Some(Key::R));
    let expected = [
        egui::pos2(0., 1.),
        egui::pos2(0., 0.),
        egui::pos2(1., 0.),
        egui::pos2(1., 1.),
    ];
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        window.draw(None);
        if canvas_corners(&window).len() == 2 {
            break;
        }
        assert!(Instant::now() < deadline, "rotated spread did not load");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(canvas_corners(&window), vec![expected; 2]);
    for shape in &window.paint {
        if let Shape::Mesh(mesh) = &shape.shape {
            if crate::ui::transparency::is_backdrop(&window.ctx, mesh.texture_id) {
                continue;
            }
            if window.is_photo_texture(mesh.texture_id) && shape.clip_rect.height() > 200. {
                let bounds = mesh.calc_bounds();
                assert!(shape.clip_rect.expand(0.01).contains_rect(bounds));
                assert!(
                    bounds.width() > bounds.height(),
                    "portrait sources must become landscape"
                );
            } else if mesh.texture_id != egui::TextureId::default()
                && shape.clip_rect.height() == 57.
            {
                assert_image_uv(mesh, false, false);
            }
        }
    }
}
