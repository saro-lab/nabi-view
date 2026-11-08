use super::*;

#[test]
fn held_navigation_keys_repeat_and_stop_on_release() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    let down = |key| key_event(key, true, false, egui::Modifiers::NONE);
    let up = |key| key_event(key, false, false, egui::Modifiers::NONE);
    let text = window
        .draw_events(vec![down(Key::ArrowRight)], egui::Modifiers::NONE)
        .0;
    assert!(
        text.contains("2 / 5"),
        "First press did not advance once: {text}"
    );
    let text = window
        .draw_events(
            vec![key_event(
                Key::ArrowRight,
                true,
                true,
                egui::Modifiers::NONE,
            )],
            egui::Modifiers::NONE,
        )
        .0;
    assert!(
        text.contains("2 / 5"),
        "OS repeat bypassed the hold delay: {text}"
    );
    window.time += 0.11;
    assert!(
        window.draw(None).0.contains("2 / 5"),
        "Repeated before hold delay"
    );
    window.time += 0.20;
    let text = window.draw(None).0;
    assert!(text.contains("3 / 5"), "Held key did not repeat: {text}");
    window.draw_events(vec![up(Key::ArrowRight)], egui::Modifiers::NONE);
    window.time += 0.35;
    let text = window.draw(None).0;
    assert!(
        text.contains("3 / 5"),
        "Navigation continued after key release: {text}"
    );

    let text = window
        .draw_events(vec![down(Key::ArrowLeft)], egui::Modifiers::NONE)
        .0;
    assert!(text.contains("2 / 5"));
    window.time += 0.31;
    assert!(window.draw(None).0.contains("1 / 5"));
    window.time += 0.11;
    assert!(
        window.draw(None).0.contains("5 / 5"),
        "Reverse navigation did not wrap"
    );
    window.draw_events(vec![up(Key::ArrowLeft)], egui::Modifiers::NONE);

    window.draw_events(vec![down(Key::ArrowRight)], egui::Modifiers::NONE);
    assert!(window.draw(None).0.contains("1 / 5"));
    window.focused = false;
    window.draw_events(vec![Event::WindowFocused(false)], egui::Modifiers::NONE);
    window.time += 0.35;
    assert!(
        window.draw(None).0.contains("1 / 5"),
        "Focus loss did not stop held navigation"
    );
    window.focused = true;
    window.draw_events(vec![Event::WindowFocused(true)], egui::Modifiers::NONE);
    window.time += 0.35;
    assert!(
        window.draw(None).0.contains("1 / 5"),
        "Regaining focus restarted an old held key"
    );
}

#[test]
fn held_navigation_button_repeats_without_an_extra_release_step() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    let next = window
        .paint
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.job.text == "›" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .expect("Next-page button is missing");
    window.pointer(next.x, next.y);
    let button = |pressed| Event::PointerButton {
        pos: next,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let text = window
        .draw_events(vec![button(true)], egui::Modifiers::NONE)
        .0;
    // Some immediate-mode layouts compute the page counter before the button's action.
    let text = if text.contains("2 / 5") {
        text
    } else {
        window.draw(None).0
    };
    assert!(
        text.contains("2 / 5"),
        "Button press did not advance once: {text}"
    );
    window.time += 0.11;
    window.draw(None);
    assert!(
        window.draw(None).0.contains("3 / 5"),
        "Held button did not repeat"
    );
    window.draw_events(vec![button(false)], egui::Modifiers::NONE);
    window.time += 0.35;
    let text = window.draw(None).0;
    assert!(
        text.contains("3 / 5"),
        "Button release caused extra navigation: {text}"
    );
    // A complete click can arrive between two frames; it still advances once.
    window.draw_events(vec![button(true), button(false)], egui::Modifiers::NONE);
    let text = window.draw(None).0;
    assert!(
        text.contains("4 / 5"),
        "Quick click was dropped or advanced twice: {text}"
    );
    window.time += 0.35;
    assert!(
        window.draw(None).0.contains("4 / 5"),
        "Quick click left a repeat timer running"
    );
}

#[test]
fn direct_open_preserves_selected_file_after_folder_discovery() {
    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    let folder = tempfile::tempdir_in(root).unwrap();
    for name in ["01.png", "02 한글.PNG", "03 끝.PNG"] {
        image::RgbaImage::from_pixel(16, 24, image::Rgba([80, 120, 160, 255]))
            .save(folder.path().join(name))
            .unwrap();
    }
    for (stored, aliases, position, next) in [
        (
            "02 한글.PNG",
            ["02 한글.PNG", "02 한글.png"],
            "2 / 3",
            "3 / 3",
        ),
        ("03 끝.PNG", ["03 끝.PNG", "03 끝.png"], "3 / 3", "1 / 3"),
    ] {
        let mut inputs = vec![folder.path().join(stored)];
        // Only exercise aliases that this filesystem considers identical.
        for name in aliases {
            let alias = folder.path().join(name);
            if alias.exists() {
                inputs.push(alias);
            }
        }
        for input in inputs {
            let mut window = TestWindow::open(Some(input));
            window.reveal_controls();
            window.wait_for(position);
            window.wait_for(stored);
            window.draw(Some(Key::ArrowRight));
            window.wait_for(next);
            window.draw(Some(Key::ArrowLeft));
            window.wait_for(position);
        }
    }
}

#[test]
fn folder_changes_advance_deleted_focus_clear_canvas_and_restore_new_images() {
    let folder = tempfile::tempdir().unwrap();
    let image_path = |name: &str| folder.path().join(name);
    let write_image = |name: &str| {
        image::RgbaImage::from_pixel(16, 24, image::Rgba([80, 120, 160, 255]))
            .save(image_path(name))
            .unwrap();
    };
    for name in ["01.png", "02.png", "03.png"] {
        write_image(name);
    }
    let mut window = TestWindow::open(Some(image_path("02.png")));
    window.reveal_controls();
    window.wait_for("2 / 3");

    let second = window.photo_texture();
    std::fs::remove_file(image_path("02.png")).unwrap();
    window.wait_for("03.png");
    window.wait_for("2 / 2");
    window.wait_for_texture_replacement(second);
    // Removing the last focused file wraps to the first surviving image.
    let third = window.photo_texture();
    std::fs::remove_file(image_path("03.png")).unwrap();
    window.wait_for("01.png");
    window.wait_for("1 / 1");
    window.wait_for_texture_replacement(third);

    window.pointer(800., 400.);
    window.time += 1.;
    window.draw(None);
    std::fs::remove_file(image_path("01.png")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let (text, image) = window.draw(None);
        if !image && text.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Deleting all images did not clear the canvas; image={image}, text={text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // The watcher must stay alive while the folder has no supported images.
    write_image("04.png");
    window.wait_for("");
    window.reveal_controls();
    window.wait_for("04.png");
    window.wait_for("1 / 1");
}

#[test]
fn wide_page_uses_full_canvas_and_navigation_resumes_with_a_pair() {
    let folder = tempfile::tempdir().unwrap();
    for page in 1..=10 {
        let (width, height) = if [2, 5, 8].contains(&page) {
            (1200, 900)
        } else {
            (400, 800)
        };
        image::RgbaImage::from_pixel(width, height, image::Rgba([100, 150, 200, 255]))
            .save(folder.path().join(format!("{page:02}.png")))
            .unwrap();
    }
    for rtl in [false, true] {
        let settings = crate::core::settings::Settings {
            spread: true,
            rtl,
            ..Default::default()
        };
        let mut window =
            TestWindow::open_with_settings(Some(folder.path().join("01.png")), &settings);
        window.screen_size = egui::vec2(1920., 1080.);
        window.monitor_size = Some(egui::vec2(1920., 1080.));
        window.reveal_controls();
        window.wait_for("1 / 10");
        window.draw(Some(Key::ArrowRight));
        window.wait_for("2 / 10");
        let (clip, photo) = window.photo_rects();
        assert!((clip.width() - window.screen_size.x).abs() < 1.);
        assert!((photo.height() - clip.height()).abs() < 1.);
        assert!((photo.width() / photo.height() - 4. / 3.).abs() < 0.001);
        for label in ["3–4 / 10", "5 / 10", "6–7 / 10", "8 / 10", "9–10 / 10"] {
            window.draw(Some(Key::ArrowRight));
            window.wait_for(label);
        }
        for label in [
            "8 / 10",
            "6–7 / 10",
            "5 / 10",
            "3–4 / 10",
            "2 / 10",
            "1 / 10",
        ] {
            window.draw(Some(Key::ArrowLeft));
            window.wait_for(label);
        }
        // Window geometry never changes grouping on the same monitor.
        window.screen_size = egui::vec2(3000., 760.);
        window.wait_for("1 / 10");
        window.screen_size = egui::vec2(560., 900.);
        window.wait_for("1 / 10");
        window.draw(Some(Key::ArrowRight));
        window.wait_for("2 / 10");
        let previous = window.photo_texture();
        window.draw(Some(Key::ArrowRight));
        window.wait_for("3–4 / 10");
        window.wait_for_texture_replacement(previous);
        let (_, photo) = window.photo_rects();
        assert!(photo.width() <= window.screen_size.x / 2. + 1.);
        for _ in 0..2 {
            window.draw(Some(Key::H));
            window.wait_for("3–4 / 10");
        }
        window.draw(Some(Key::ArrowLeft));
        window.wait_for("2 / 10");

        // Moving the unchanged window to a wider monitor updates grouping,
        // preserves the current file, and ignores an equivalent DPI scale.
        for size in [egui::vec2(3000., 760.), egui::vec2(1500., 380.)] {
            window.monitor_size = Some(size);
            window.wait_for("1–2 / 10");
            window.wait_for("02.png");
        }
        // Temporary missing or invalid monitor data keeps the last grouping.
        for size in [
            None,
            Some(egui::Vec2::ZERO),
            Some(egui::vec2(f32::NAN, 1080.)),
        ] {
            window.monitor_size = size;
            window.screen_size = egui::vec2(1080., 760.);
            window.wait_for("1–2 / 10");
        }
        window.monitor_size = Some(egui::vec2(1080., 1920.));
        window.wait_for("2 / 10");
        window.wait_for("02.png");
        window.monitor_size = Some(egui::vec2(3000., 760.));
        window.wait_for("1–2 / 10");
    }
}

#[test]
fn thumbnail_count_tracks_window_width_and_keeps_the_last_page_visible() {
    let folder = tempfile::tempdir().unwrap();
    for page in 1..=100 {
        image::RgbaImage::from_pixel(16, 24, image::Rgba([100, 150, 200, 255]))
            .save(folder.path().join(format!("{page:03}.png")))
            .unwrap();
    }
    let mut window = TestWindow::open(Some(folder.path().join("001.png")));
    window.reveal_controls();
    window.wait_for("1 / 100");
    let mut counts = Vec::new();
    for width in [720., 1920., 3840., 720.] {
        window.screen_size.x = width;
        window.reveal_controls();
        window.draw(None);
        let thumbnails: Vec<_> = window
            .paint
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Rect(r)
                    if r.fill == Color32::from_gray(30)
                        && r.rect.size() == egui::vec2(65., 57.) =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert!(
            thumbnails
                .iter()
                .all(|r| r.left() >= 0. && r.right() <= width)
        );
        counts.push(thumbnails.len());
        window.draw(Some(Key::End));
        window.wait_for("100 / 100");
        assert_eq!(focused_thumbnail_pages(&window), [100]);
    }
    assert!(counts[0] < counts[1] && counts[1] < counts[2]);
    assert!(counts[1] > 15);
    assert_eq!(counts[0], counts[3]);
}

#[test]
fn cold_and_rapid_navigation_keep_a_photo_until_the_target_is_ready() {
    let folder = tempfile::tempdir().unwrap();
    for (name, width, height, color) in [
        ("01.png", 24, 40, [200, 40, 40, 255]),
        ("02.png", 40, 24, [40, 200, 40, 255]),
        ("03.png", 30, 30, [40, 40, 200, 255]),
    ] {
        image::RgbaImage::from_pixel(width, height, image::Rgba(color))
            .save(folder.path().join(name))
            .unwrap();
    }
    let mut window = TestWindow::open(Some(folder.path().join("01.png")));
    window.reveal_controls();
    window.wait_for("1 / 3");
    let first_texture = window.photo_texture();
    let first_geometry = window.photo_rects();

    // Exercise the real toolbar path, where the navigation command arrives
    // after this frame's canvas was painted.
    let next = window
        .paint
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.job.text == "›" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .expect("Next-page button is missing");
    window.pointer(next.x, next.y);
    let events = [true, false].map(|pressed| Event::PointerButton {
        pos: next,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
    assert!(window.draw_events(events.into(), egui::Modifiers::NONE).1);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let (text, image) = window.draw(None);
        assert!(image, "Cold navigation submitted a blank frame: {text}");
        if window.photo_texture() != first_texture {
            let rect = window.photo_rects().1;
            assert!((rect.width() / rect.height() - 40. / 24.).abs() < 0.001);
            break;
        }
        assert_eq!(
            window.photo_rects(),
            first_geometry,
            "Pending navigation moved the old photo"
        );
        assert!(
            Instant::now() < deadline,
            "Navigation never replaced the retained photo"
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    // Request multiple targets without waiting for their decoder results.
    // The final requested file is the square third image (2 -> 3 -> 1 -> 3).
    for key in [Key::ArrowRight, Key::ArrowRight, Key::ArrowLeft] {
        let (text, image) = window.draw(Some(key));
        assert!(image, "Rapid navigation submitted a blank frame: {text}");
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let third_texture = loop {
        let (text, image) = window.draw(None);
        assert!(
            image,
            "Waiting for the latest target submitted a blank frame: {text}"
        );
        let rect = window.photo_rects().1;
        if (rect.width() / rect.height() - 1.).abs() < 0.001 {
            assert!(text.contains("3 / 3"));
            break window.photo_texture();
        }
        assert!(Instant::now() < deadline, "Latest target was not displayed");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_ne!(third_texture, first_texture);
    // A late result for an earlier request must not replace the final target.
    for _ in 0..20 {
        assert!(window.draw(None).1);
        assert_eq!(window.photo_texture(), third_texture);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn thumbnail_focus_tracks_the_whole_spread_and_preserves_the_clicked_page() {
    let mut window = TestWindow::sample();
    // These portrait fixtures must fit the half-width slots for this two-page test.
    window.screen_size.x = 1280.;
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.time += 0.25;
    window.draw(None);
    assert_eq!(focused_thumbnail_pages(&window), [1]);

    window.draw(Some(Key::CloseBracket));
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    window.click(window.label_position("2"));
    window.wait_for("02 Pelagic.png");
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);

    // Direct layout commands preserve the current page, and reading direction
    // never changes focus membership.
    window.draw(Some(Key::OpenBracket));
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    window.draw(Some(Key::P));
    assert_eq!(focused_thumbnail_pages(&window), [2]);
    assert!(window.draw(None).0.contains("2 / 5"));

    window.draw(Some(Key::CloseBracket));
    window.draw(Some(Key::ArrowRight));
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);
    window.draw(Some(Key::End));
    assert_eq!(focused_thumbnail_pages(&window), [5]);
    window.draw(Some(Key::ArrowRight));
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    window.draw(Some(Key::ArrowLeft));
    assert_eq!(focused_thumbnail_pages(&window), [5]);
}

#[test]
fn thumbnail_focus_respects_a_single_cover_and_reversed_spreads() {
    let settings = crate::core::settings::Settings {
        spread: true,
        cover: true,
        ..Default::default()
    };
    let mut window = TestWindow::sample_with_settings(&settings);
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.time += 0.25;
    window.draw(None);
    assert_eq!(focused_thumbnail_pages(&window), [1]);

    window.draw(Some(Key::ArrowRight));
    assert_eq!(focused_thumbnail_pages(&window), [2, 3]);
    window.click(window.label_position("3"));
    window.wait_for("03 Dune Forms.png");
    window.draw(Some(Key::OpenBracket));
    assert_eq!(focused_thumbnail_pages(&window), [2, 3]);
    window.draw(Some(Key::End));
    assert_eq!(focused_thumbnail_pages(&window), [4, 5]);
    window.draw(Some(Key::Home));
    assert_eq!(focused_thumbnail_pages(&window), [1]);

    // A single-page view contains the same current page when leaving a spread.
    window.click(window.label_position("3"));
    window.draw(Some(Key::P));
    assert_eq!(focused_thumbnail_pages(&window), [3]);

    let mut empty = TestWindow::open(None);
    empty.reveal_controls();
    assert!(focused_thumbnail_pages(&empty).is_empty());
}

#[test]
fn wheel_navigation_counts_events_instead_of_magnitude_or_smooth_scroll_frames() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H)); // Keep the page counter visible without opening metadata.
    window.pointer(540., 380.);
    window.time += 0.1;
    window.draw(None);
    let wheel = |x, y| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(x, y),
        modifiers: egui::Modifiers::NONE,
    };
    for (delta, page) in [(-1., 2), (-1000., 3), (1., 2), (1000., 1)] {
        window.draw_events(vec![wheel(0., delta)], egui::Modifiers::NONE);
        let expected = format!("{page} / 5");
        let text = window.draw(None).0;
        assert!(
            text.contains(&expected),
            "Wheel delta {delta} did not move exactly once: {text}"
        );
        // egui continues smoothing large wheel deltas over several frames.
        // Those frames must never turn into extra page-navigation commands.
        for _ in 0..12 {
            let text = window.draw(None).0;
            assert!(
                text.contains(&expected),
                "Wheel smoothing caused extra navigation: {text}"
            );
        }
    }
    window.draw_events(vec![wheel(1000., 0.)], egui::Modifiers::NONE);
    assert!(
        window.draw(None).0.contains("1 / 5"),
        "Horizontal wheel input changed pages"
    );
    window.draw_events(
        vec![wheel(0., -1.), wheel(0., -1000.)],
        egui::Modifiers::NONE,
    );
    assert!(
        window.draw(None).0.contains("3 / 5"),
        "Two wheel events in one frame did not advance two pages"
    );
}
