use super::*;

#[test]
fn shortcut_toolbar_keeps_equal_heights_and_wraps_whole_controls() {
    for language in [
        crate::i18n::Language::English,
        crate::i18n::Language::Arabic,
    ] {
        let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
            language,
            ..Default::default()
        });
        window.reveal_controls();
        window.click(window.titlebar_icon("settings").rect.center());
        window.click(window.label_position(crate::i18n::tr("keyboard-shortcuts")));
        for width in [320., 390., 560., 1080.] {
            window.screen_size = egui::vec2(width, 760.);
            for _ in 0..4 {
                window.draw(None);
            }
            let filter = window
                .ctx
                .read_response(egui::Id::new("shortcut-filter"))
                .unwrap()
                .rect;
            assert!(
                (filter.width() - 180.).abs() < 1.,
                "Filter collapsed at {width}: {filter:?}"
            );
            let mut controls = vec![filter];
            for key in ["add", "search-by-shortcut", "reset-shortcuts"] {
                let label = window.label_position(crate::i18n::tr(key));
                let rect = window
                    .paint
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        Shape::Rect(r) if r.rect.contains(label) && r.rect.height() < 60. => {
                            Some(r.rect)
                        }
                        _ => None,
                    })
                    .min_by(|a, b| a.area().total_cmp(&b.area()))
                    .unwrap_or_else(|| {
                        let backgrounds: Vec<_> = window
                            .paint
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                Shape::Rect(r) if r.rect.contains(label) => Some(r.rect),
                                _ => None,
                            })
                            .collect();
                        panic!(
                            "Missing toolbar button {key} at {width}: {label:?}, {backgrounds:?}"
                        );
                    });
                controls.push(rect);
            }
            for (index, rect) in controls.iter().enumerate() {
                assert!(
                    (rect.height() - 36.).abs() < 1.,
                    "Unequal control height: {rect:?}"
                );
                assert!(
                    window.ctx.screen_rect().contains_rect(*rect),
                    "Control escaped {width}: {rect:?}"
                );
                for other in &controls[index + 1..] {
                    assert!(
                        !rect.intersects(*other),
                        "Toolbar controls overlap: {rect:?} / {other:?}"
                    );
                    if (rect.center().y - other.center().y).abs() < 36. {
                        assert!(
                            (rect.top() - other.top()).abs() < 1.,
                            "Toolbar controls are not aligned"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn shortcut_changes_render_the_current_value_in_toasts() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    for (key, expected) in [
        (Key::Period, "슬라이드 간격: 1.1 초"),
        (Key::Comma, "슬라이드 간격: 1.0 초"),
        (Key::R, "좌우 반전: 켜짐"),
        (Key::R, "좌우 반전: 꺼짐"),
        (Key::T, "상하 반전: 켜짐"),
        (Key::T, "상하 반전: 꺼짐"),
        (Key::OpenBracket, "배치: 양면 보기 (왼쪽)"),
        (Key::CloseBracket, "배치: 양면 보기 (오른쪽)"),
        (Key::P, "배치: 한면 보기 (양면 끔)"),
        (Key::Num1, "이미지 표시: 실제 크기 (100%)"),
        (Key::Equals, "배율: 125%"),
        (Key::Num0, "이미지 표시: 이미지 전체 맞춤"),
        (Key::Space, "체크한 이미지 수: 1"),
        (Key::Space, "체크한 이미지 수: 0"),
        (Key::Q, "슬라이드 쇼: 켜짐"),
        (Key::Q, "슬라이드 쇼: 꺼짐"),
        (Key::I, "이미지 정보: 켜짐"),
        (Key::I, "이미지 정보: 꺼짐"),
        (Key::F, "전체 화면: 켜짐"),
        (Key::F, "전체 화면: 꺼짐"),
    ] {
        window.draw(Some(key));
        let text = window.draw(None).0;
        assert!(
            text.contains(expected),
            "Missing toast {expected:?}: {text}"
        );
    }
    let settings = crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        keyboard_shortcuts: Some(vec![crate::core::settings::KeyboardShortcut {
            action: "flip-horizontal".into(),
            key: "F2".into(),
        }]),
        ..Default::default()
    };
    let mut remapped = TestWindow::sample_with_settings(&settings);
    remapped.draw(Some(Key::F2));
    assert!(remapped.draw(None).0.contains("Flip horizontally: On"));
    remapped.draw(Some(Key::F2));
    assert!(remapped.draw(None).0.contains("Flip horizontally: Off"));
}

#[test]
fn phone_swipe_turns_pages_and_settings_uses_full_width() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(390., 844.);
    window.wait_for("1 / 5");
    assert!(!window.drag_pointer(egui::pos2(300., 400.), egui::vec2(-150., 0.)));
    window.wait_for("02 Pelagic.png");
    let settings = window
        .ctx
        .read_response(egui::Id::new(("titlebar", "settings")))
        .unwrap()
        .rect
        .center();
    window.click(settings);
    for _ in 0..3 {
        window.draw(None);
    }
    let (text, _) = window.draw(None);
    assert!(text.contains("설정") && text.contains("언어"));
    assert!(!text.contains("연결") && !text.contains("공유"));
    for shape in &window.paint {
        if let Shape::Text(t) = &shape.shape
            && t.galley.job.text == "언어"
        {
            assert!(
                t.pos.x < 40.,
                "Phone settings must not reserve a desktop sidebar"
            );
        }
    }
    assert!(!window.started_window_drag());
}

#[test]
fn cursor_idle_applies_in_settings_and_does_not_interrupt_dragging() {
    let mut window = TestWindow::open(None);
    window.draw(Some(Key::Escape));
    window.draw(Some(Key::F1));
    window.pointer(500., 300.);
    window.time += 2.1;
    window.draw(None);
    assert_eq!(window.cursor, egui::CursorIcon::None);
    window.draw_events(
        vec![Event::PointerButton {
            pos: egui::pos2(500., 300.),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.time += 2.1;
    window.draw(None);
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.draw_events(
        vec![Event::PointerButton {
            pos: egui::pos2(500., 300.),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    window.time += 2.1;
    window.draw(None);
    assert_eq!(window.cursor, egui::CursorIcon::None);
}

#[test]
fn large_gif_plays_in_settings_and_after_navigation() {
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("01 long.gif");
    crate::core::media::animation_fixtures::gif(&path, 600, 256);
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]))
        .save(images.path().join("02 still.png"))
        .unwrap();
    let mut window = TestWindow::open(Some(path));
    window._images = Some(images);
    window.reveal_controls();
    window.wait_for("01 long.gif");
    let mut previous = Vec::new();
    for stage in 0..3 {
        if stage == 1 {
            window.draw(Some(Key::F1));
        }
        if stage == 2 {
            window.draw(Some(Key::Escape));
            window.reveal_controls();
            window.draw(Some(Key::ArrowRight));
            window.wait_for("02 still.png");
            window.draw(Some(Key::ArrowLeft));
            window.wait_for("01 long.gif");
        }
        let mut changes = 0;
        let deadline = Instant::now() + Duration::from_secs(8);
        while changes < 3 {
            let (text, drawn) = window.draw(None);
            assert!(drawn, "Animation disappeared: {text}");
            assert!(
                !text.contains("ERROR") && !text.contains("frame budget"),
                "{text}"
            );
            let texture = window.photo_texture();
            for (_, delta) in window
                .texture_updates
                .iter()
                .filter(|(id, _)| *id == texture)
            {
                let egui::ImageData::Color(image) = &delta.image else {
                    continue;
                };
                assert_eq!(image.size, [256, 256]);
                if image.pixels != previous {
                    changes += 1;
                    previous.clone_from(&image.pixels);
                }
            }
            assert!(
                Instant::now() < deadline,
                "GIF froze at stage {stage}: {text}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn animated_avif_updates_displayed_pixels_and_settings_background() {
    let source = crate::core::media::animation_fixtures::avif_sample();
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("animated.avif");
    std::fs::copy(source, &path).unwrap();

    // A thumbnail uses only the representative frame even for an AVIF sequence.
    let media = crate::core::media::Media::new(egui::Context::default());
    let thumbnail_id = media.request_thumbnail(path.clone(), 128);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(crate::core::media::MediaEvent::Thumbnail { id, result, .. }) = media.try_recv()
        {
            assert_eq!(id, thumbnail_id);
            let frame = result.expect("AVIF thumbnail failed to decode");
            assert!(frame.animation.is_none());
            assert_eq!(frame.original_size, [498, 286]);
            let [width, height] = frame.image.size;
            assert!(width > 0 && height > 0 && width <= 128 && height <= 128);
            assert!(width.max(height).abs_diff(128) <= 1);
            // Both dimensions are floored, including ordinary floating-point
            // rounding at the requested bound; allow one pixel on each axis.
            assert!((width * 286).abs_diff(height * 498) <= 498 + 286);
            break;
        }
        assert!(Instant::now() < deadline, "AVIF thumbnail did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(media);

    let mut window = TestWindow::open(Some(path));
    window._images = Some(images);
    let deadline = Instant::now() + Duration::from_secs(10);
    let (texture, mut previous_pixels) = loop {
        let (text, drawn) = window.draw(None);
        if drawn {
            let texture = window.photo_texture();
            if let Some((_, delta)) = window.texture_updates.iter().find(|(id, _)| *id == texture) {
                let egui::ImageData::Color(image) = &delta.image else {
                    panic!("Expected AVIF color pixels");
                };
                assert_eq!(image.size, [498, 286]);
                break (texture, image.pixels.clone());
            }
        }
        assert!(
            Instant::now() < deadline,
            "AVIF was never displayed: {text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    window.draw(Some(Key::R));
    window.draw(Some(Key::T));
    assert_canvas_flips(&window, true, true);
    for settings_open in [false, true] {
        if settings_open {
            window.reveal_controls();
            window.click(window.titlebar_icon("settings").rect.center());
            assert!(
                window
                    .draw(None)
                    .0
                    .contains(crate::i18n::test_tr("Language"))
            );
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(window.draw(None).1, "Animation lost its displayed image");
            assert_eq!(window.photo_texture(), texture);
            assert_canvas_flips(&window, true, true);
            assert!(
                window.repaint_delay <= Duration::from_millis(140),
                "Animation did not schedule its next frame"
            );
            if let Some((_, delta)) = window.texture_updates.iter().find(|(id, _)| *id == texture) {
                let egui::ImageData::Color(image) = &delta.image else {
                    panic!("Expected AVIF color pixels");
                };
                if image.pixels != previous_pixels {
                    previous_pixels = image.pixels.clone();
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "AVIF pixels stayed frozen (settings_open={settings_open})"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn close_all_shortcut_works_in_settings_and_can_be_removed() {
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    for registered in [true, false] {
        let mut settings = crate::core::settings::Settings::default();
        if !registered {
            settings
                .keyboard_shortcuts
                .as_mut()
                .unwrap()
                .retain(|row| row.action != "close-all");
        }
        let mut window = TestWindow::open_with_settings(None, &settings);
        window.reveal_controls();
        window.click(window.titlebar_icon("settings").rect.center());
        window.draw_events(vec![key_event(Key::Q, true, false, modifiers)], modifiers);
        assert_eq!(
            window
                .viewport_commands
                .contains(&egui::ViewportCommand::Close),
            registered
        );
    }
}

#[test]
fn toolbar_gap_moves_window_and_settings_button_remains_clickable() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    let before = window.photo_rects();
    assert!(
        window.drag_pointer(egui::pos2(500., 25.), egui::vec2(30., 0.)),
        "Empty toolbar space did not request window movement"
    );
    assert_eq!(
        window.photo_rects(),
        before,
        "Toolbar drag panned the photo"
    );
    assert!(
        window.drag_pointer(egui::pos2(500., 737.), egui::vec2(30., 0.)),
        "Empty bottom status space did not request window movement"
    );
    assert_eq!(
        window.photo_rects(),
        before,
        "Status bar drag panned the photo"
    );

    let settings = window.titlebar_icon("settings").rect.center();
    assert!(
        !window.drag_pointer(settings, egui::pos2(500., 25.) - settings),
        "Dragging from Settings onto the toolbar background started a window drag"
    );
    assert_eq!(
        window.photo_rects(),
        before,
        "Dragging the Settings button panned the photo"
    );
    window.pointer(settings.x, settings.y);
    assert!(!window.started_window_drag());
    for pressed in [true, false] {
        window.draw_events(
            vec![Event::PointerButton {
                pos: settings,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert!(
            !window.started_window_drag(),
            "Toolbar button click started a window drag"
        );
    }
    let text = window.draw(None).0;
    assert!(!window.started_window_drag());
    assert!(
        text.contains(crate::i18n::test_tr("General")),
        "Settings button no longer opens its window: {text}"
    );
}

#[test]
fn flip_shortcuts_preserve_source_pixels_navigation_and_reopened_view() {
    let mut window = TestWindow::sample();
    // These portrait fixtures must fit the half-width slots for this two-page test.
    window.screen_size.x = 1280.;
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    let first_path = window
        ._images
        .as_ref()
        .unwrap()
        .path()
        .join("01 Alpine Light.png");
    let original = std::fs::read(&first_path).unwrap();
    let texture = window.photo_texture();
    assert_canvas_flips(&window, false, false);

    window.draw(Some(Key::R));
    assert_canvas_flips(&window, true, false);
    assert_eq!(
        window.photo_texture(),
        texture,
        "A display flip must reuse decoded pixels"
    );
    window.draw(Some(Key::T));
    assert_canvas_flips(&window, true, true);
    window.draw(Some(Key::R));
    assert_canvas_flips(&window, false, true);
    window.draw(Some(Key::R));
    let settings = saved_viewer_settings(&window);
    assert!(settings.flip_horizontal && settings.flip_vertical);

    window.draw(Some(Key::D));
    window.wait_for("02 Pelagic.png");
    window.wait_for_texture_replacement(texture);
    assert_canvas_flips(&window, true, true);
    window.draw(Some(Key::CloseBracket));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        window.draw(None);
        if assert_canvas_flips(&window, true, true) == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "The flipped spread did not finish loading"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut thumbnails = 0;
    for shape in &window.paint {
        if let Shape::Mesh(mesh) = &shape.shape {
            if crate::ui::transparency::is_backdrop(&window.ctx, mesh.texture_id) {
                continue;
            }
            if mesh.texture_id != egui::TextureId::default() && shape.clip_rect.height() == 57. {
                assert_image_uv(mesh, false, false);
                thumbnails += 1;
            }
        }
    }
    assert!(
        thumbnails > 0,
        "Expected original-orientation thumbnail images"
    );
    assert_eq!(std::fs::read(&first_path).unwrap(), original);

    // Recreate the viewer from the persisted preferences, without supplying
    // the saved flags through the test's constructor settings parameter.
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        Some(first_path),
        Ok(store),
    );
    window.reveal_controls();
    window.wait_for("1–2 / 5");
    assert_canvas_flips(&window, true, true);
    window.draw(Some(Key::T));
    assert_canvas_flips(&window, true, false);
    assert!(!saved_viewer_settings(&window).flip_vertical);
}

#[test]
fn alternate_navigation_and_direct_layout_shortcuts_keep_the_current_page() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    // Focus outlines share the thumbnail Area's opening fade. Inspect their
    // final color only after that animation has completed.
    window.time += 0.25;
    window.draw(None);
    window.draw(Some(Key::D));
    assert_eq!(focused_thumbnail_pages(&window), [2]);
    window.draw(Some(Key::A));
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    window.draw(Some(Key::S));
    assert_eq!(focused_thumbnail_pages(&window), [5]);
    window.draw(Some(Key::W));
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    shift_navigation(&mut window, Key::D);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1]);
    assert_eq!(focused_thumbnail_pages(&window), [2]);
    shift_navigation(&mut window, Key::A);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    assert_eq!(focused_thumbnail_pages(&window), [1]);

    window.draw(Some(Key::OpenBracket));
    let settings = saved_viewer_settings(&window);
    assert!(settings.spread && settings.rtl);
    // Reading direction changes the page layout, never a registered key's action.
    window.draw(Some(Key::D));
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);
    window.draw(Some(Key::A));
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    window.draw(Some(Key::CloseBracket));
    let settings = saved_viewer_settings(&window);
    assert!(settings.spread && !settings.rtl);
    window.draw(Some(Key::D));
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);
    window.draw(Some(Key::P));
    assert_eq!(focused_thumbnail_pages(&window), [3]);
    assert!(!saved_viewer_settings(&window).spread);
    window.draw(Some(Key::P));
    assert_eq!(
        focused_thumbnail_pages(&window),
        [3],
        "Single view is not a toggle"
    );

    window.draw(Some(Key::I));
    window.wait_for("PNG");
    window.draw(Some(Key::I));
    assert!(!window.draw(None).0.contains("PNG"));
}

#[cfg(windows)]
#[test]
#[ignore = "Requires a desktop user session; moves only unique test fixtures through the Recycle Bin"]
fn trash_shortcuts_restore_batches_lifo_and_fall_back_to_the_focused_spread() {
    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("viewer-trash-roundtrip-")
        .tempdir_in(root)
        .unwrap();
    let paths: Vec<_> = (1..=4)
        .map(|index| fixture.path().join(format!("{index:02}.png")))
        .collect();
    for (index, path) in paths.iter().enumerate() {
        image::RgbaImage::from_pixel(16, 24, image::Rgba([40 + index as u8 * 30, 100, 180, 255]))
            .save(path)
            .unwrap();
    }
    let original_bytes: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let mut window = TestWindow::open_with_settings(
        Some(paths[0].clone()),
        &crate::core::settings::Settings {
            language: crate::i18n::Language::English,
            toast_ms: 5_000,
            ..Default::default()
        },
    );
    window.reveal_controls();
    window.wait_for("1 / 4");
    window.draw(Some(Key::H));

    // Check two images, then leave their pages. Delete must prefer the checks
    // over the currently focused third image.
    shift_navigation(&mut window, Key::ArrowRight);
    window.draw(Some(Key::Space));
    window.draw(Some(Key::ArrowRight));
    window.wait_for("3 / 4");
    window.draw(Some(Key::Delete));
    window.wait_for("Images deleted.");
    window.wait_for("1 / 2");
    assert!(!paths[0].exists() && !paths[1].exists());
    assert!(paths[2].exists() && paths[3].exists());

    // Once the checked files leave the list, Delete falls back to the single
    // focused image and records a separate undo batch.
    window.draw(Some(Key::Delete));
    window.wait_for("Images deleted.");
    window.wait_for("1 / 1");
    assert!(!paths[2].exists() && paths[3].exists());

    let undo = |window: &mut TestWindow| {
        let modifiers = egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        };
        window.draw_events(
            [true, false]
                .map(|pressed| key_event(Key::Z, pressed, false, modifiers))
                .into(),
            modifiers,
        );
        window.wait_for("Image deletion undone.");
    };
    undo(&mut window);
    window.wait_for("2 / 2");
    assert!(!paths[0].exists() && !paths[1].exists());
    assert!(paths[2].exists() && paths[3].exists());
    undo(&mut window);
    window.wait_for("4 / 4");
    assert!(paths.iter().all(|path| path.is_file()));

    // The current fourth page anchors a 3–4 spread. With no checks remaining,
    // the fallback must trash and restore both images as one operation.
    window.draw(Some(Key::CloseBracket));
    window.wait_for("3–4 / 4");
    window.draw(Some(Key::Delete));
    window.wait_for("Images deleted.");
    window.wait_for("1–2 / 2");
    assert!(paths[0].exists() && paths[1].exists());
    assert!(!paths[2].exists() && !paths[3].exists());
    undo(&mut window);
    window.wait_for("1–2 / 4");
    for (path, original) in paths.iter().zip(original_bytes) {
        assert_eq!(std::fs::read(path).unwrap(), original, "{}", path.display());
    }
}

#[test]
fn settings_sidebar_switches_the_right_hand_content() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    window.click(window.titlebar_icon("settings").rect.center());
    let text = window.draw(None).0;
    for category in [
        "General",
        "Connections",
        "Cache",
        "Keyboard shortcuts",
        "Sharing",
        "About",
    ] {
        assert!(
            text.contains(crate::i18n::test_tr(category)),
            "Missing settings category {category}: {text}"
        );
    }
    assert!(
        text.contains(crate::i18n::test_tr("Language")),
        "General settings should show the language selector"
    );
    assert!(!text.contains(crate::i18n::test_tr("Show the first page alone (cover)")));
    assert_eq!(
        text.contains(crate::i18n::test_tr(
            "Black out other monitors in full screen"
        )),
        cfg!(target_os = "macos")
    );
    let cache = window.label_position(crate::i18n::test_tr("Cache"));
    window.click(window.label_position(crate::i18n::test_tr("Connections")));
    let connections = window.draw(None).0;
    assert!(connections.contains(".jpg"));
    assert!(connections.contains(".avif"));
    assert!(connections.contains(crate::i18n::test_tr("Refresh connection status")));
    window.click(cache);
    let text = window.draw(None).0;
    assert!(text.contains(crate::i18n::test_tr("Cached images")));
    assert!(text.contains(crate::i18n::test_tr("Cache memory")));
    assert!(
        !text.contains(crate::i18n::test_tr("Language")),
        "General content was not replaced"
    );
    let control = window.label_position(crate::i18n::test_tr("Cached images"));
    assert!(
        control.x > cache.x + 70.,
        "Settings content is not to the right of the category list"
    );

    assert!(text.contains(crate::i18n::test_tr("Remember recent items")));
    assert!(text.contains(crate::i18n::test_tr("Maximum recent items")));
    assert!(text.contains(crate::i18n::test_tr("Clear recent items…")));
    assert!(!text.contains(crate::i18n::test_tr("Show the first page alone (cover)")));
    window.click(window.label_position(crate::i18n::test_tr("Keyboard shortcuts")));
    let text = window.draw(None).0;
    assert!(text.contains(crate::i18n::test_tr(
        "Click a row to change its key or action. Add more than one shortcut for the same action."
    )));
    assert!(!text.contains(crate::i18n::test_tr("Maximum recent items")));
}

#[test]
fn shortcut_text_filter_matches_names_and_keys_and_edits_the_original_row() {
    use crate::core::{
        commands::shortcut_label,
        settings::{KeyboardShortcut, Settings},
    };
    use crate::i18n::{Language, tr};
    for language in [Language::English, Language::Korean, Language::Arabic] {
        let rows: Vec<_> = [
            ("prev", "A"),
            ("next", "B"),
            ("next", "C"),
            ("fullscreen", "Primary+Enter"),
            ("close-all", "Primary+Q"),
        ]
        .into_iter()
        .map(|(action, key)| KeyboardShortcut {
            action: action.into(),
            key: key.into(),
        })
        .collect();
        let mut window = TestWindow::sample_with_settings(&Settings {
            language,
            keyboard_shortcuts: Some(rows.clone()),
            ..Default::default()
        });
        window.reveal_controls();
        window.wait_for("1 / 5");
        window.click(window.titlebar_icon("settings").rect.center());
        window.click(window.label_position(tr("keyboard-shortcuts")));
        let filter_id = egui::Id::new("shortcut-filter");
        let primary = egui::Modifiers {
            command: true,
            mac_cmd: cfg!(target_os = "macos"),
            ctrl: !cfg!(target_os = "macos"),
            ..Default::default()
        };
        let set_filter = |window: &mut TestWindow, text: String| {
            let field = window.ctx.read_response(filter_id).unwrap();
            window.click(field.rect.center());
            window.draw_events(
                vec![
                    key_event(Key::A, true, false, primary),
                    key_event(Key::A, false, false, primary),
                ],
                primary,
            );
            window.draw_events(vec![Event::Text(text)], egui::Modifiers::NONE);
            window.draw(None);
            window.draw(None);
        };
        let visible = |window: &TestWindow, index| {
            window
                .ctx
                .read_response(egui::Id::new(("shortcut-row", index)))
                .is_some()
        };
        // Switching from key capture to text typing must not capture typed letters.
        window.click(window.label_position(tr("search-by-shortcut")));
        set_filter(
            &mut window,
            format!("  {}  ", tr("next-page-or-spread").to_uppercase()),
        );
        assert!(!visible(&window, 0usize));
        assert!(visible(&window, 1usize) && visible(&window, 2usize));
        assert!(!visible(&window, 3usize));
        assert_eq!(
            saved_viewer_settings(&window).keyboard_shortcuts.as_ref(),
            Some(&rows)
        );
        window.click(window.label_position(tr("search-by-shortcut")));
        window.draw(Some(Key::B));
        window.draw(None);
        window.draw(None);
        assert!(visible(&window, 1usize) && !visible(&window, 2usize));
        window.click(window.label_position(tr("clear-shortcut-search")));
        set_filter(&mut window, shortcut_label("Primary+Enter"));
        assert!(visible(&window, 3usize) && !visible(&window, 1usize));
        // Typing in the filter must not dispatch the viewer's quit shortcut.
        window.draw_events(
            vec![
                key_event(Key::Q, true, false, primary),
                key_event(Key::Q, false, false, primary),
            ],
            primary,
        );
        assert!(
            !window
                .viewport_commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Close))
        );
        set_filter(&mut window, "no matching shortcut".into());
        assert!(
            window
                .draw(None)
                .0
                .contains(tr("no-shortcuts-match-filter"))
        );
        set_filter(&mut window, tr("next-page-or-spread").into());
        window.click(shortcut_row(&window, 2).rect.center());
        window.click(window.label_position(tr("delete-shortcut")));
        let expected: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 2)
            .map(|(_, row)| row.clone())
            .collect();
        assert_eq!(
            saved_viewer_settings(&window).keyboard_shortcuts,
            Some(expected)
        );
        // Narrow settings still keep the wrapped filter inside the viewport.
        window.screen_size = egui::vec2(560., 420.);
        window.draw(None);
        window.draw(None);
        let field = window.ctx.read_response(filter_id).unwrap();
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, window.screen_size)
                .contains_rect(field.rect)
        );
    }
}

#[test]
fn shortcut_rows_can_be_added_edited_deleted_and_reset_without_resetting_other_settings() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        flip_horizontal: true,
        toast_ms: 3_700,
        keyboard_shortcuts: Some(Vec::new()),
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    // Even an empty shortcut list can be edited entirely through the controls.
    window.click(window.titlebar_icon("pin").rect.center());
    window.click(window.titlebar_icon("settings").rect.center());
    window.click(window.label_position("Keyboard shortcuts"));
    let text = window.draw(None).0;
    assert!(text.contains("No shortcuts. Use + Add to create one."));
    assert_eq!(text.matches("Reset shortcuts").count(), 1);

    add_shortcut_row(&mut window, Key::J, "Next page or spread");
    add_shortcut_row(&mut window, Key::K, "Next page or spread");
    let rows = saved_viewer_settings(&window).keyboard_shortcuts.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|row| row.action == "next"),
        "Unexpected rows: {rows:?}"
    );
    assert_eq!(
        rows.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(),
        ["J", "K"]
    );
    // The function and key occupy opposite ends of a single understated row.
    let row = shortcut_row(&window, 0).rect;
    let label_x = |label: &str| {
        window
            .paint
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text)
                    if text.galley.job.text == label
                        && row.contains(text.visual_bounding_rect().center()) =>
                {
                    Some(text.visual_bounding_rect().center().x)
                }
                _ => None,
            })
            .unwrap()
    };
    assert!(label_x("Next page or spread") < label_x("J"));
    window.click(window.label_position("Search by shortcut"));
    window.draw(Some(Key::J));
    window.draw(None);
    window.draw(None);
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("shortcut-row", 0)))
            .is_some()
    );
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("shortcut-row", 1)))
            .is_none()
    );
    window.draw(Some(Key::Escape));
    window.draw(None);
    assert!(
        window.draw(None).0.contains("Search by shortcut"),
        "Esc search cancellation closed settings"
    );
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("shortcut-row", 1)))
            .is_some()
    );
    window.click(window.label_position("Search by shortcut"));
    window.draw(Some(Key::Z));
    assert!(window.draw(None).0.contains("No shortcuts match this key."));
    window.click(window.label_position("Clear shortcut search"));
    close_settings_screen(&mut window);
    window.draw(Some(Key::J));
    assert!(window.draw(None).0.contains("2 / 5"));
    window.draw(Some(Key::K));
    assert!(window.draw(None).0.contains("3 / 5"));

    window.click(window.titlebar_icon("settings").rect.center());
    window.click(shortcut_row(&window, 0).rect.center());
    window.wait_for("Edit shortcut");
    window.click(window.label_position("J"));
    window.draw(Some(Key::L));
    choose_shortcut_action(
        &mut window,
        "Next page or spread",
        "Previous page or spread",
    );
    window.click(window.label_position("Save shortcut"));
    let rows = saved_viewer_settings(&window).keyboard_shortcuts.unwrap();
    assert_eq!(rows[0].key, "L");
    assert_eq!(rows[0].action, "prev");
    assert_eq!(rows[1].key, "K");

    // A failed duplicate save keeps both the existing rows and the editor.
    record_new_shortcut_row(&mut window, Key::L, "Next page or spread");
    let text = window.draw(None).0;
    assert!(text.contains("Add shortcut"));
    assert!(
        text.contains("conflicts"),
        "Missing duplicate-key feedback: {text}"
    );
    assert_eq!(
        saved_viewer_settings(&window)
            .keyboard_shortcuts
            .unwrap()
            .len(),
        2
    );
    window.click(window.label_position("Save shortcut"));
    assert!(
        window.draw(None).0.contains("Add shortcut"),
        "Duplicate save should be disabled"
    );
    assert_eq!(
        saved_viewer_settings(&window)
            .keyboard_shortcuts
            .unwrap()
            .len(),
        2
    );
    window.click(window.label_position("Cancel"));
    for index in [1, 0] {
        window.click(shortcut_row(&window, index).rect.center());
        window.wait_for("Edit shortcut");
        window.click(window.label_position("Delete shortcut"));
    }
    assert_eq!(
        saved_viewer_settings(&window).keyboard_shortcuts,
        Some(Vec::new())
    );
    assert!(
        window
            .draw(None)
            .0
            .contains("No shortcuts. Use + Add to create one.")
    );
    close_settings_screen(&mut window);
    for key in [Key::ArrowRight, Key::L, Key::K] {
        window.draw(Some(key));
        assert!(
            window.draw(None).0.contains("3 / 5"),
            "Empty list restored a default binding"
        );
    }

    window.click(window.titlebar_icon("settings").rect.center());
    window.click(window.label_position("Reset shortcuts"));
    assert!(window.draw(None).0.contains("Reset all shortcuts?"));
    assert_eq!(
        saved_viewer_settings(&window).keyboard_shortcuts,
        Some(Vec::new())
    );
    window.click(window.label_position("Cancel"));
    assert_eq!(
        saved_viewer_settings(&window).keyboard_shortcuts,
        Some(Vec::new())
    );
    window.click(window.label_position("Reset shortcuts"));
    window.draw(Some(Key::Escape));
    assert!(window.draw(None).0.contains("Search by shortcut"));
    assert!(!window.draw(None).0.contains("Reset all shortcuts?"));
    assert_eq!(
        saved_viewer_settings(&window).keyboard_shortcuts,
        Some(Vec::new())
    );
    window.click(window.label_position("Reset shortcuts"));
    window.click(window.label_position("Restore default"));
    let settings = saved_viewer_settings(&window);
    assert!(settings.keyboard_shortcuts.is_some());
    assert_eq!(
        settings.keyboard_shortcuts,
        crate::core::settings::Settings::default().keyboard_shortcuts
    );
    assert!(settings.shortcuts.is_empty());
    assert!(settings.flip_horizontal);
    assert_eq!(settings.toast_ms, 3_700);
    let text = window.draw(None).0;
    assert_eq!(text.matches("Reset shortcuts").count(), 1);
    assert!(!text.contains("No shortcuts. Use + Add to create one."));
    let escape_row = settings
        .keyboard_shortcuts
        .as_ref()
        .unwrap()
        .iter()
        .position(|row| row.action == "close-window" && row.key == "Escape")
        .unwrap();
    window.click(window.label_position("Search by shortcut"));
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close))
    );
    let text = window.draw(None).0;
    assert!(text.contains("Search by shortcut") && text.contains("Clear shortcut search"));
    assert!(
        text.contains("Close window"),
        "The first Esc must search for its registered row"
    );
    assert!(!text.contains("No shortcuts match this key."));
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("shortcut-row", escape_row)))
            .is_some()
    );
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close))
    );
    let text = window.draw(None).0;
    assert!(
        text.contains("Search by shortcut"),
        "The second Esc must leave settings open"
    );
    assert!(
        !text.contains("Clear shortcut search"),
        "The second Esc must clear the filter"
    );
    close_settings_screen(&mut window);
    window.draw(Some(Key::ArrowRight));
    assert!(window.draw(None).0.contains("4 / 5"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn imported_ctrl_shortcut_matches_primary_input_and_shortcut_search() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        keyboard_shortcuts: Some(vec![crate::core::settings::KeyboardShortcut {
            // Bind to navigation so this regression never opens a native file dialog.
            action: "next".into(),
            key: "Ctrl+O".into(),
        }]),
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.click(window.titlebar_icon("pin").rect.center());
    let primary_o = |window: &mut TestWindow| {
        let modifiers = egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        };
        window.draw_events(
            [true, false]
                .map(|pressed| key_event(Key::O, pressed, false, modifiers))
                .into(),
            modifiers,
        );
    };
    primary_o(&mut window);
    assert!(window.draw(None).0.contains("2 / 5"));
    window.click(window.titlebar_icon("settings").rect.center());
    window.click(window.label_position("Keyboard shortcuts"));
    window.click(window.label_position("Search by shortcut"));
    primary_o(&mut window);
    let text = window.draw(None).0;
    assert!(text.contains("Next page or spread") && text.contains("Ctrl+O"));
    assert!(!text.contains("No shortcuts match this key."));
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("shortcut-row", 0)))
            .is_some()
    );
    window.click(window.label_position("Clear shortcut search"));
    close_settings_screen(&mut window);
    assert!(
        window.draw(None).0.contains("2 / 5"),
        "Searching must not execute the matched action"
    );
}

#[test]
fn a_partial_shortcut_list_runs_only_its_registered_rows() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        keyboard_shortcuts: Some(vec![crate::core::settings::KeyboardShortcut {
            action: "next".into(),
            key: "J".into(),
        }]),
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.click(window.titlebar_icon("pin").rect.center());
    for key in [Key::D, Key::ArrowRight, Key::N] {
        window.draw(Some(key));
        assert!(
            window.draw(None).0.contains("1 / 5"),
            "Unregistered key {key:?} ran a default action"
        );
    }
    window.draw(Some(Key::J));
    assert!(window.draw(None).0.contains("2 / 5"));
    for key in [Key::A, Key::ArrowLeft, Key::W, Key::Home, Key::S, Key::End] {
        window.draw(Some(key));
        assert!(
            window.draw(None).0.contains("2 / 5"),
            "Missing action acquired a default key {key:?}"
        );
    }
    window.click(window.titlebar_icon("settings").rect.center());
    window.click(window.label_position("Keyboard shortcuts"));
    let text = window.draw(None).0;
    assert!(text.contains("Next page or spread"));
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("shortcut-row", 0)))
            .is_some()
    );
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("shortcut-row", 1)))
            .is_none()
    );
    let rows = saved_viewer_settings(&window).keyboard_shortcuts.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "J");
}

#[test]
fn resetting_a_legacy_shortcut_map_removes_its_window_binding_before_navigation() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        shortcuts: std::collections::BTreeMap::from([("new-window".into(), "D".into())]),
        keyboard_shortcuts: None,
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.click(window.titlebar_icon("pin").rect.center());
    window.click(window.titlebar_icon("settings").rect.center());
    window.click(window.label_position("Keyboard shortcuts"));
    let text = window.draw(None).0;
    assert!(text.contains("New window"));
    assert!(!text.contains("Next page or spread"));
    // Never invoke the old D binding: it would create a real process. Reset
    // through the UI first, then check the persisted and live replacement.
    window.click(window.label_position("Reset shortcuts"));
    window.click(window.label_position("Restore default"));
    let settings = saved_viewer_settings(&window);
    assert!(settings.shortcuts.is_empty());
    let rows = settings
        .keyboard_shortcuts
        .expect("Reset must save explicit default rows");
    assert!(
        rows.iter()
            .any(|row| row.action == "next" && row.key == "D")
    );
    assert!(
        !rows
            .iter()
            .any(|row| row.action == "new-window" && row.key == "D")
    );
    close_settings_screen(&mut window);
    window.draw(Some(Key::D));
    assert!(window.draw(None).0.contains("2 / 5"));
}

#[test]
fn about_f1_opens_settings_integrates_licenses_and_preserves_image() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    window.draw(Some(Key::F1));
    let text = window.draw(None).0;
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let brand_y = window.label_position("NABI VIEW").y;
    let version_y = window.label_position(&version).y;
    let website_y = window.label_position(env!("CARGO_PKG_HOMEPAGE")).y;
    let licenses_y = window
        .label_position(crate::i18n::test_tr("Open source licenses"))
        .y;
    assert!(brand_y < version_y && version_y < website_y && website_y < licenses_y);
    assert!(text.contains(crate::i18n::test_tr("Open source licenses")));
    assert!(text.contains(crate::i18n::test_tr("General")));
    assert!(text.contains(crate::i18n::test_tr("About")));
    assert!(text.contains("NABI VIEW · MIT"));
    assert!(text.contains("ab_glyph"));
    assert!(!text.contains("Permission is hereby granted"));
    window.click(window.label_position(crate::i18n::tr("show-full-license-texts")));
    assert!(window.draw(None).0.contains("Permission is hereby granted"));
    window.click(window.label_position(crate::i18n::tr("show-full-license-texts")));
    assert!(window.draw(None).0.contains("NABI VIEW · MIT"));
    assert!(!window.titlebar_visible());
    window.draw(Some(Key::ArrowRight));
    window.draw(None);
    assert!(!window.titlebar_visible());
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .contains(&egui::ViewportCommand::Close)
    );
    window.reveal_controls();
    assert!(window.titlebar_visible());
    assert!(window.draw(None).0.contains("01 Alpine Light.png"));
    // A cached license document must not continue to block image navigation.
    window.draw(Some(Key::ArrowRight));
    window.wait_for("02 Pelagic.png");
    window.draw(Some(Key::F1));
    window.draw(None);
    let close = window
        .ctx
        .read_response(egui::Id::new(("titlebar", "settings-close")))
        .unwrap();
    window.click(close.rect.center());
    window.reveal_controls();
    assert!(window.titlebar_visible());
}

#[test]
fn f1_switches_settings_to_about_and_licenses_stay_in_settings() {
    let mut window = TestWindow::open(None);
    window.draw(Some(Key::Escape));
    window.reveal_controls();
    window.click(window.titlebar_icon("settings").rect.center());
    window.draw(None);
    window.draw(Some(Key::F1));
    window.draw(None);
    let text = window.draw(None).0;
    assert!(text.contains(crate::i18n::test_tr("Settings")));
    assert!(text.contains(crate::i18n::test_tr("Open source licenses")));
    assert!(text.contains(crate::i18n::test_tr("General")));
    window.click(window.label_position(crate::i18n::test_tr("General")));
    assert!(
        window
            .draw(None)
            .0
            .contains(crate::i18n::test_tr("Language"))
    );
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .contains(&egui::ViewportCommand::Close)
    );
    window.reveal_controls();
    assert!(window.titlebar_visible());
}

#[test]
fn settings_replaces_viewer_and_close_returns_without_exiting() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    window.click(window.titlebar_icon("settings").rect.center());
    let text = window.draw(None).0;
    assert!(text.contains(crate::i18n::test_tr("Language")));
    assert!(!window.titlebar_visible());
    let close = window
        .ctx
        .read_response(egui::Id::new(("titlebar", "settings-close")))
        .expect("Missing settings close icon");
    assert_eq!(close.rect.size(), egui::vec2(36., 36.));
    window.click(close.rect.center());
    let text = window.draw(None).0;
    assert!(!text.contains(crate::i18n::test_tr("Language")));
    window.reveal_controls();
    assert!(window.titlebar_visible());
}

#[test]
fn settings_blank_space_moves_window_but_controls_do_not() {
    let mut window = TestWindow::open(None);
    window.draw(Some(Key::Escape));
    window.reveal_controls();
    window.click(window.titlebar_icon("settings").rect.center());
    for start in [
        egui::pos2(500., 28.),
        egui::pos2(80., 690.),
        egui::pos2(500., 690.),
    ] {
        assert!(
            window.drag_pointer(start, egui::vec2(1., 1.)),
            "Small settings drag at {start:?}"
        );
        assert!(
            window.drag_pointer(start, egui::vec2(35., 8.)),
            "Blank settings space at {start:?}"
        );
    }
    let category = window.label_position(crate::i18n::test_tr("General"));
    assert!(!window.drag_pointer(category, egui::vec2(250., 0.)));
    let reset = window.label_position(crate::i18n::test_tr("Default"));
    assert!(!window.drag_pointer(reset, egui::vec2(-100., 0.)));
    let close = window
        .ctx
        .read_response(egui::Id::new(("titlebar", "settings-close")))
        .unwrap();
    assert!(!window.drag_pointer(close.rect.center(), egui::vec2(-100., 0.)));
    // Scrollable content must also leave gaps available for window movement.
    window.click(window.label_position(crate::i18n::test_tr("Connections")));
    window.draw(None);
    assert!(window.drag_pointer(egui::pos2(950., 690.), egui::vec2(-30., 0.)));
}

#[test]
fn settings_columns_remain_visible_inside_a_small_viewport() {
    for resize_after_open in [false, true] {
        let mut window = TestWindow::open(None);
        window.draw(Some(Key::Escape));
        if !resize_after_open {
            window.screen_size = egui::vec2(560., 420.);
        }
        window.reveal_controls();
        window.click(window.titlebar_icon("settings").rect.center());
        window.screen_size = egui::vec2(560., 420.);
        window.time += 0.25;
        for _ in 0..3 {
            window.draw(None);
        }
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, window.screen_size);
        let area = screen;
        assert!(
            screen.expand(1.).contains_rect(area),
            "Settings escaped the small viewport (resized={resize_after_open}): {area:?}"
        );
        assert!(!window.titlebar_visible());
        for label in [
            crate::i18n::test_tr("General"),
            crate::i18n::test_tr("Language"),
            "한국어",
        ] {
            let mut count = 0;
            for s in &window.paint {
                if let Shape::Text(t) = &s.shape
                    && t.galley.job.text == label
                {
                    let bounds = t.visual_bounding_rect();
                    assert!(
                        area.expand(1.).contains_rect(bounds),
                        "{label} escaped the settings window: {bounds:?} / {area:?}"
                    );
                    assert!(
                        s.clip_rect.expand(1.).contains_rect(bounds),
                        "Selected settings content {label} is clipped: {bounds:?} / {:?}",
                        s.clip_rect
                    );
                    count += 1;
                }
            }
            assert!(count > 0, "Required settings content is missing: {label}");
        }
        // Lower sidebar categories may scroll out of view, but any visible
        // portion must stay within the same settings window and its clip.
        for s in &window.paint {
            if let Shape::Text(t) = &s.shape
                && ["Cache", "Keyboard shortcuts", "Sharing", "About"]
                    .iter()
                    .any(|label| t.galley.job.text == crate::i18n::test_tr(label))
            {
                let visible = t.visual_bounding_rect().intersect(s.clip_rect);
                if visible.is_positive() {
                    assert!(
                        area.expand(1.).contains_rect(visible),
                        "Sidebar category escaped its window: {visible:?} / {area:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn modified_mouse_input_respects_settings_and_zoom_editing() {
    let modifiers = egui::Modifiers {
        command: true,
        ..Default::default()
    };
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    let before = window.photo_rects();
    window.click(zoom_widget(&window, "zoom-value").rect.center());
    window.pointer(540., 380.);
    window.draw_events(
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 100.),
            modifiers,
        }],
        modifiers,
    );
    window.draw(Some(Key::Escape));
    window.draw(None);
    assert_eq!(window.photo_rects(), before);

    window.click(window.titlebar_icon("settings").rect.center());
    window.draw(None);
    let settings_photo = window.photo_rects();
    window.drag_pointer_with_modifiers(egui::pos2(950., 690.), egui::vec2(-30., 0.), modifiers);
    window.draw_events(
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 100.),
            modifiers,
        }],
        modifiers,
    );
    window.draw(None);
    assert_eq!(
        window.photo_rects(),
        settings_photo,
        "Settings changed the background image"
    );
    window.draw(Some(Key::Escape));
    window.draw(None);
    assert_eq!(window.photo_rects(), before);
}
