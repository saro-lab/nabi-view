use super::*;

#[test]
fn cursor_hides_after_two_seconds_and_recovers_on_mouse_input() {
    let mut window = TestWindow::open(None);
    window.pointer(500., 300.);
    let moved_at = window.time;
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.time = moved_at + 1.9;
    window.draw(None);
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.time = moved_at + 2.;
    window.draw(None);
    assert_eq!(window.cursor, egui::CursorIcon::None);
    window.draw(Some(Key::ArrowRight));
    assert_eq!(
        window.cursor,
        egui::CursorIcon::None,
        "Keyboard navigation should not reveal the cursor"
    );
    window.pointer(501., 300.);
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.time += 2.1;
    window.draw(None);
    assert_eq!(window.cursor, egui::CursorIcon::None);
    window.draw_events(vec![Event::PointerGone], egui::Modifiers::NONE);
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.pointer(500., 300.);
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.time += 2.1;
    window.draw(None);
    window.focused = false;
    window.draw(None);
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.focused = true;
    window.draw(None);
    assert_ne!(window.cursor, egui::CursorIcon::None);
}

#[test]
fn cursor_uses_the_configured_tenth_second_delay() {
    let settings = crate::core::settings::Settings {
        cursor_hide_ms: 100,
        ..Default::default()
    };
    let mut window = TestWindow::open_with_settings(None, &settings);
    window.pointer(500., 300.);
    let moved_at = window.time;
    window.time = moved_at + 0.05;
    window.draw(None);
    assert_ne!(window.cursor, egui::CursorIcon::None);
    window.time = moved_at + 0.1;
    window.draw(None);
    assert_eq!(window.cursor, egui::CursorIcon::None);
}

#[test]
fn immersive_controls_overlay_the_image_and_hide_after_pointer_leaves() {
    let mut window = TestWindow::sample();
    window.wait_for("");
    let (text, image) = window.draw(None);
    assert!(image);
    assert!(text.is_empty(), "Startup must show only the image: {text}");
    let before = window.photo_rects();
    assert_eq!(
        before.0,
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1080., 760.))
    );

    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    window.draw(None);
    assert!(window.titlebar_visible());
    assert_eq!(
        window.photo_rects(),
        before,
        "Revealing controls resized the image"
    );

    // Moving from the activation strip onto the toolbar must not dismiss it.
    window.pointer(500., 25.);
    window.draw(None);
    assert!(window.titlebar_visible());
    window.pointer(800., 400.);
    window.time += 0.06;
    window.draw(None);
    assert!(!window.titlebar_visible());
    // The full thumbnail region reveals and holds the same chrome.
    for (x, y) in [(10., 640.), (540., 670.), (1060., 710.)] {
        window.pointer(x, y);
        window.draw(None);
        assert!(window.titlebar_visible());
    }
    window.pointer(800., 400.);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let (text, _) = window.draw(None);
        if text.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Controls remained visible away from the toolbar: {text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        window.photo_rects(),
        before,
        "Hiding controls resized the image"
    );

    window.draw(Some(Key::Tab));
    window.wait_for("314159");
    window.draw(None);
    assert!(window.titlebar_visible());
    let (canvas, photo) = window.photo_rects();
    assert_eq!(
        canvas,
        egui::Rect::from_min_max(egui::pos2(0., 47.), egui::pos2(1080., 637.)),
        "Tab must reserve the top bar, thumbnails, and status bar"
    );
    assert!(canvas.contains_rect(photo));
    assert!(photo.height() < before.1.height());
    assert!(
        (photo.aspect_ratio() - before.1.aspect_ratio()).abs() < 0.001,
        "Reserving chrome space distorted the image"
    );
    window.draw(Some(Key::Tab));
    let (text, image) = window.draw(None);
    assert!(image);
    assert!(!window.titlebar_visible());
    assert_eq!(
        text.trim(),
        "이미지 정보: 꺼짐",
        "Second Tab should leave only the status toast, with controls and metadata hidden"
    );
    assert_eq!(
        window.photo_rects(),
        before,
        "Closing Tab must restore the canvas"
    );
}

#[test]
fn pinned_canvas_drives_zoom_and_preserves_manual_scale() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    let full_canvas = window.photo_rects().0;
    window.click(window.titlebar_icon("pin").rect.center());
    let (inner_canvas, fitted) = window.photo_rects();
    assert_eq!(inner_canvas.top(), 47.);
    assert_eq!(inner_canvas.bottom(), window.screen_size.y - 123.);
    assert!(inner_canvas.contains_rect(fitted));

    let original = image::image_dimensions(sample_path().unwrap()).unwrap();
    let expected_percent =
        fitted.width() * window.ctx.pixels_per_point() / original.0 as f32 * 100.;
    let label = window.draw(None).0;
    let shown_percent = label
        .split_whitespace()
        .find_map(|word| {
            word.strip_suffix('%')
                .and_then(|value| value.parse::<f32>().ok())
        })
        .expect("Missing displayed zoom percentage");
    assert!((shown_percent - expected_percent).abs() < 0.01);

    window.draw(Some(Key::Equals));
    window.draw(None);
    let enlarged = window.photo_rects().1;
    assert!((enlarged.width() / fitted.width() - 1.25).abs() < 0.001);
    window.draw(Some(Key::H));
    assert_eq!(window.photo_rects().0, full_canvas);
    assert_eq!(window.photo_rects().1.size(), enlarged.size());
    window.draw(Some(Key::H));
    assert_eq!(window.photo_rects().0, inner_canvas);
    assert_eq!(window.photo_rects().1.size(), enlarged.size());
}

#[test]
fn dragging_letterbox_or_photo_moves_the_window_without_panning() {
    let mut window = TestWindow::sample_with_modifier_gestures();
    window.wait_for("");
    let before = window.photo_rects();
    let margin = egui::pos2(
        (before.0.left() + before.1.left()) / 2.,
        before.1.center().y,
    );
    assert!(before.0.contains(margin) && !before.1.contains(margin));
    // Crossing between the letterbox and photo must keep moving the window.
    assert!(
        window.drag_pointer(margin, before.1.center() - margin),
        "Letterbox drag crossing onto the photo did not request window movement"
    );
    assert_eq!(
        window.photo_rects(),
        before,
        "Letterbox drag panned the photo"
    );

    assert!(
        window.drag_pointer(before.1.center(), margin - before.1.center()),
        "Photo drag crossing into the letterbox did not move the window"
    );
    assert_eq!(
        window.photo_rects(),
        before,
        "Dragging the photo changed its geometry"
    );
}

#[test]
fn modified_drag_moves_the_photo_without_moving_the_window() {
    for modifiers in [
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        },
    ] {
        let mut window = TestWindow::sample_with_modifier_gestures();
        window.wait_for("");
        let before = window.photo_rects();
        let delta = egui::vec2(72., -38.);
        assert!(!window.drag_pointer_with_modifiers(before.1.center(), delta, modifiers));
        let after = window.photo_rects();
        assert_eq!(after.0, before.0, "Panning changed the canvas");
        assert_eq!(after.1, before.1.translate(delta));
        window.pointer(400., 400.);
        assert_eq!(
            window.photo_rects(),
            after,
            "Panning continued after release"
        );

        // Fullscreen still allows image panning, though native window dragging is disabled.
        window.draw(Some(Key::F));
        let before = window.photo_rects();
        assert!(!window.drag_pointer_with_modifiers(before.0.center(), -delta, modifiers));
        assert_eq!(window.photo_rects().1, before.1.translate(-delta));
    }
}

#[test]
fn image_drag_keeps_its_mode_when_the_modifier_is_released() {
    let mut window = TestWindow::sample_with_modifier_gestures();
    window.wait_for("");
    let before = window.photo_rects();
    let start = before.1.center();
    let modifiers = egui::Modifiers {
        command: true,
        ..Default::default()
    };
    window.draw_events(vec![Event::PointerMoved(start)], modifiers);
    window.draw_events(
        vec![Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        }],
        modifiers,
    );
    // Include sub-threshold movement: the entire drag displacement must be preserved.
    window.pointer(start.x + 1., start.y);
    window.pointer(start.x + 60., start.y + 20.);
    assert!(!window.started_window_drag());
    window.draw(None);
    assert_eq!(
        window.photo_rects().1,
        before.1.translate(egui::vec2(60., 20.))
    );
    window.draw_events(
        vec![Event::PointerButton {
            pos: start + egui::vec2(65., 25.),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    window.draw(None);
    assert_eq!(
        window.photo_rects().1,
        before.1.translate(egui::vec2(65., 25.))
    );
}

#[test]
fn dragging_an_empty_black_canvas_moves_the_window() {
    let mut window = TestWindow::open(None);
    window.draw(Some(Key::Escape));
    window.draw(None);
    assert!(
        window.drag_pointer(egui::pos2(540., 380.), egui::vec2(40., 20.)),
        "Empty canvas did not request window movement"
    );
    let (text, image) = window.draw(None);
    assert!(text.is_empty());
    assert!(!image);
}

#[test]
fn returning_to_window_cancels_an_interrupted_image_drag() {
    for lose_focus in [false, true] {
        let mut window = TestWindow::sample_with_modifier_gestures();
        window.wait_for("");
        let start = window.photo_rects().1.center();
        let modifiers = egui::Modifiers::CTRL;
        window.draw_events(
            vec![
                Event::PointerMoved(start),
                Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
            modifiers,
        );
        window.pointer(start.x + 40., start.y + 20.);
        window.draw(None);
        let before = window.photo_rects();

        // Native window interactions can consume the release outside the viewport.
        window.focused = !lose_focus;
        window.draw_events(
            if lose_focus {
                vec![Event::WindowFocused(false)]
            } else {
                vec![Event::PointerGone]
            },
            egui::Modifiers::NONE,
        );
        window.focused = true;
        window.pointer(start.x - 100., start.y - 60.);
        assert!(!window.started_window_drag());
        window.draw(None);
        assert_eq!(
            window.photo_rects(),
            before,
            "Stale drag: focus={lose_focus}"
        );
        window.click(start - egui::vec2(100., 60.));
        assert_eq!(window.photo_rects(), before);

        // A fresh modified drag still works from its new press position.
        let delta = egui::vec2(25., 12.);
        assert!(!window.drag_pointer_with_modifiers(start, delta, modifiers));
        assert_eq!(window.photo_rects().1, before.1.translate(delta));
    }
}

#[test]
fn leaving_and_reentering_in_one_frame_does_not_keep_the_old_click_target() {
    let mut window = TestWindow::sample();
    window.wait_for("");
    window.draw(Some(Key::H));
    window.draw(None);
    let old = window.titlebar_icon("menu").rect.center();
    let new = window.titlebar_icon("settings").rect.center();
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    // The old control never receives its mouse-up. All subsequent native events
    // may be delivered in one redraw when the viewer becomes active again.
    window.draw_events(
        vec![Event::PointerMoved(old), button(old, true)],
        egui::Modifiers::NONE,
    );
    window.draw_events(
        vec![
            Event::PointerMoved(old + egui::vec2(60., 30.)),
            Event::PointerGone,
            Event::WindowFocused(false),
            Event::WindowFocused(true),
            Event::PointerMoved(new),
            button(new, true),
        ],
        egui::Modifiers::NONE,
    );
    assert!(!window.started_window_drag());
    window.draw_events(vec![button(new, false)], egui::Modifiers::NONE);
    window.draw(None);
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "settings-close")))
            .is_some()
    );
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("menu-command", "open")))
            .is_none()
    );
}

#[test]
fn refocusing_waits_for_actual_pointer_movement_on_all_window_drag_surfaces() {
    for surface in ["canvas", "chrome", "settings", "menu", "recent", "backdrop"] {
        let mut window = TestWindow::sample_with_modifier_gestures();
        window.wait_for("");
        window.draw(Some(Key::H));
        window.draw(None);
        let start = match surface {
            "canvas" => egui::pos2(540., 380.),
            "chrome" => egui::pos2(500., 24.),
            "settings" => {
                window.click(window.titlebar_icon("settings").rect.center());
                egui::pos2(500., 28.)
            }
            _ => {
                window.click(window.titlebar_icon("menu").rect.center());
                if surface == "recent" {
                    let button = window
                        .ctx
                        .read_response(egui::Id::unique(("menu-command", "recent")))
                        .unwrap();
                    window.click(button.rect.center());
                }
                for _ in 0..4 {
                    window.draw(None);
                }
                if surface == "backdrop" {
                    egui::pos2(24., 180.)
                } else {
                    let id = egui::Id::unique(if surface == "recent" {
                        "library"
                    } else {
                        "command-menu"
                    });
                    window
                        .ctx
                        .memory(|m| m.area_rect(id))
                        .unwrap()
                        .left_center()
                        + egui::vec2(8., 0.)
                }
            }
        };
        window.pointer(start.x, start.y);
        window.focused = false;
        window.draw_events(
            vec![Event::PointerGone, Event::WindowFocused(false)],
            egui::Modifiers::NONE,
        );
        window.focused = true;
        window.draw_events(
            vec![
                Event::WindowFocused(true),
                Event::PointerMoved(start),
                Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
        );
        assert!(
            !window.started_window_drag(),
            "Activation click started native dragging"
        );
        window.time += 1.;
        window.draw(None);
        assert!(
            !window.started_window_drag(),
            "Stationary hold started native dragging"
        );
        window.pointer(start.x + 1., start.y + 1.);
        assert!(
            window.started_window_drag(),
            "Small real drag should still work"
        );
        window.pointer(start.x + 5., start.y + 5.);
        assert!(
            !window.started_window_drag(),
            "Native drag was requested twice on {surface}"
        );
    }
}

#[test]
fn chrome_icons_follow_requested_order_and_pin_keeps_them_visible() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    // Inspect final icon colors after the Area's appearance animation settles.
    window.time += 0.25;
    window.draw(None);
    let names = ["pin", "menu", "settings", "close"];
    let rects: Vec<_> = names
        .iter()
        .map(|name| window.titlebar_icon(name).rect)
        .collect();
    assert!(
        (rects.last().unwrap().right() - (1080. - 4.)).abs() < 1.,
        "Titlebar icons must stay attached to the right edge: {:?}",
        rects
    );
    for adjacent in rects.windows(2) {
        assert!(
            adjacent[0].right() <= adjacent[1].left(),
            "Titlebar icons are not in the requested order"
        );
    }
    let filename = window
        .paint
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.job.text == "01 Alpine Light.png" && t.pos.y < 47. => {
                Some(t.visual_bounding_rect())
            }
            _ => None,
        })
        .expect("Filename is missing from the titlebar");
    assert!(
        filename.right() < rects[0].left(),
        "Filename overlaps the titlebar controls"
    );
    let spread = window.titlebar_icon("spread").rect;
    let random = window.titlebar_icon("random").rect;
    assert!(spread.top() >= window.screen_size.y - 45.);
    assert!(spread.right() < random.left(), "Spread must precede random");
    for (left, right) in [
        (spread.left() - 24., spread.left()),
        (spread.right(), random.left()),
    ] {
        assert!(
            window.paint.iter().any(|shape| matches!(&shape.shape,
                Shape::LineSegment { points, .. }
                    if (points[0].x - points[1].x).abs() < 0.1
                        && points[0].x > left && points[0].x < right
                        && (points[0].y - points[1].y).abs() > 10.
                        && points[0].y >= window.screen_size.y - 45.
            )),
            "Missing separator beside the spread control"
        );
    }
    assert!(
        window.icon_has_stroke("spread", Color32::from_rgb(159, 165, 161)),
        "Inactive spread icon is not muted"
    );
    window.click(window.titlebar_icon("spread").rect.center());
    // The wider spread page counter moves the icon; let the cached hit-test rect catch up.
    window.draw(None);
    assert!(
        window.icon_has_stroke("spread", Color32::from_rgb(204, 232, 165)),
        "Active spread icon is not highlighted"
    );
    window.click(window.titlebar_icon("pin").rect.center());
    assert!(
        window.icon_has_stroke("pin", Color32::from_rgb(204, 232, 165)),
        "Pinned controls icon is not highlighted"
    );
    window.pointer(800., 400.);
    window.time += 1.;
    window.draw(None);
    assert!(
        window.titlebar_visible(),
        "Pinned controls hid when the pointer left"
    );
    assert_eq!(window.photo_rects().0.top(), 47.);
    assert_eq!(window.photo_rects().0.bottom(), window.screen_size.y - 123.);
    window.click(window.titlebar_icon("pin").rect.center());
    assert!(
        window.icon_has_stroke("pin", Color32::from_rgb(159, 165, 161)),
        "Unpinned controls icon stayed highlighted"
    );
    window.pointer(800., 400.);
    window.time += 1.;
    window.draw(None);
    assert!(!window.titlebar_visible(), "Unpinned controls did not hide");
    assert_eq!(window.photo_rects().0.top(), 0.);
    assert_eq!(window.photo_rects().0.bottom(), window.screen_size.y);

    window.reveal_controls();
    let metadata = window.titlebar_icon("metadata").rect;
    let reveal = window.titlebar_icon("reveal").rect;
    assert!(reveal.right() < metadata.left());
    assert_eq!(reveal.center().y, metadata.center().y);
    window.click(metadata.center());
    window.draw(None);
    assert!(window.icon_has_stroke("metadata", Color32::from_rgb(204, 232, 165)));
    window.draw(Some(Key::Tab));
    window.reveal_controls();
    assert!(window.icon_has_stroke("metadata", Color32::from_rgb(159, 165, 161)));
}

#[test]
fn escape_closes_only_the_active_dialog_or_the_unblocked_viewer() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    window.click(window.titlebar_icon("settings").rect.center());
    for key in [Key::R, Key::T, Key::OpenBracket, Key::D, Key::Q, Key::F] {
        window.draw(Some(key));
        assert!(!window.viewport_commands.iter().any(|command| {
            matches!(
                command,
                egui::ViewportCommand::Close | egui::ViewportCommand::Fullscreen(_)
            )
        }));
    }
    let settings = saved_viewer_settings(&window);
    assert!(!settings.flip_horizontal && !settings.flip_vertical && !settings.spread);
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
    assert!(!window.draw(None).0.contains("Language"));
    // Settings removes the chrome Areas; returning starts their fade-in again.
    window.time += 0.25;
    window.draw(None);
    assert_eq!(focused_thumbnail_pages(&window), [1]);

    window.draw(Some(Key::Space));
    window.click(window.titlebar_icon("random").rect.center());
    window.wait_for("Clear checks and enable random order?");
    window.draw(Some(Key::R));
    window.draw(Some(Key::D));
    assert!(!saved_viewer_settings(&window).flip_horizontal);
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
    assert!(
        !window
            .draw(None)
            .0
            .contains("Clear checks and enable random order?")
    );
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1]);

    window.click(window.titlebar_icon("menu").rect.center());
    assert!(window.draw(None).0.contains("Open image"));
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
    assert!(!window.draw(None).0.contains("Open image"));

    // Metadata and fullscreen are viewer states, not blocking dialogs. Esc
    // must close the viewer even when both are active.
    window.draw(Some(Key::I));
    window.wait_for("314159");
    window.draw(Some(Key::F));
    assert!(
        window
            .viewport_commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Fullscreen(true)))
    );
    window.draw(Some(Key::Escape));
    assert!(
        window
            .viewport_commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
}

#[test]
fn zoom_percent_edits_absolute_scale_and_preserves_input_focus() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    window.click(zoom_widget(&window, "zoom-value").rect.center());
    assert!(zoom_widget(&window, "zoom-input").has_focus());
    replace_zoom_text(&mut window, "123.45");
    // Arrow keys edit the number without advancing to another file.
    window.draw(Some(Key::ArrowLeft));
    window.draw(Some(Key::ArrowRight));
    window.draw(Some(Key::Enter));
    window.draw(None);
    assert_photo_scale(&window, 1.2345);
    assert!(window.draw(None).0.contains("123.45%"));
    assert!(window.draw(None).0.contains("01 Alpine Light.png"));

    window.click(zoom_widget(&window, "zoom-value").rect.center());
    replace_zoom_text(&mut window, "200");
    // The editor keeps chrome visible even after the pointer leaves its reveal zone.
    window.pointer(800., 400.);
    window.time += 1.;
    window.draw(None);
    assert!(window.titlebar_visible());
    assert!(zoom_widget(&window, "zoom-input").has_focus());
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .iter()
            .any(|command| { matches!(command, egui::ViewportCommand::Close) }),
        "Cancelling zoom input must not close the viewer"
    );
    assert_photo_scale(&window, 1.2345);
    window.reveal_controls();
    assert!(window.draw(None).0.contains("123.45%"));

    window.click(zoom_widget(&window, "zoom-value").rect.center());
    replace_zoom_text(&mut window, "250.25%");
    window.click(egui::pos2(800., 400.));
    assert_photo_scale(&window, 2.5025);
}

#[test]
fn zoom_percent_rejects_invalid_values_and_clamps_buttons_at_limits() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    window.click(zoom_widget(&window, "zoom-value").rect.center());
    replace_zoom_text(&mut window, "100");
    window.draw(Some(Key::Enter));
    window.draw(None);
    assert_photo_scale(&window, 1.);
    window.click(zoom_widget(&window, "zoom-value").rect.center());
    for invalid in [
        "19.99", "3000.01", "123.456", "NaN", "inf", "1e2", "", "-100",
    ] {
        replace_zoom_text(&mut window, invalid);
        window.draw(Some(Key::Enter));
        window.draw(None);
        assert!(
            zoom_widget(&window, "zoom-input").has_focus(),
            "Invalid entry closed the editor: {invalid}"
        );
        assert_photo_scale(&window, 1.);
    }
    replace_zoom_text(&mut window, "20.00");
    window.draw(Some(Key::Enter));
    window.draw(None);
    assert_photo_scale(&window, 0.2);
    window.click(window.label_position("−"));
    assert_photo_scale(&window, 0.2);
    window.click(zoom_widget(&window, "zoom-value").rect.center());
    replace_zoom_text(&mut window, "3000.00");
    window.draw(Some(Key::Enter));
    window.draw(None);
    assert_photo_scale(&window, 30.);
    window.click(window.label_position("+"));
    assert_photo_scale(&window, 30.);
}

#[test]
fn bottom_controls_fit_the_minimum_window_with_zoom_in_visual_order() {
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(560., 420.);
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    let minus = window.label_position("−");
    let percent = zoom_widget(&window, "zoom-value").rect;
    let plus = window.label_position("+");
    let slideshow = window.titlebar_icon("slideshow").rect;
    let reveal = window.titlebar_icon("reveal").rect;
    let metadata = window.titlebar_icon("metadata").rect;
    assert!(slideshow.right() < reveal.left() && reveal.right() < metadata.left());
    assert!(metadata.right() < minus.x && minus.x < percent.left());
    assert!(percent.right() < plus.x && plus.x < 560.);
    assert!(minus.y >= 375. && plus.y >= 375.);
    window.click(percent.center());
    let input = zoom_widget(&window, "zoom-input").rect;
    assert!(input.left() > minus.x && input.right() < plus.x);
    replace_zoom_text(&mut window, "3000.00");
    window.draw(Some(Key::Enter));
    window.draw(None);
    let value = zoom_widget(&window, "zoom-value").rect;
    assert!(value.left() > minus.x && value.right() < plus.x);
}

#[test]
fn native_resize_edges_take_priority_over_titlebar_and_canvas_dragging() {
    for (point, direction, cursor) in [
        (
            egui::pos2(500., 2.),
            egui::ResizeDirection::North,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            egui::pos2(2., 380.),
            egui::ResizeDirection::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            egui::pos2(1078., 758.),
            egui::ResizeDirection::SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
    ] {
        let mut window = TestWindow::open(None);
        window.draw(None);
        window.reveal_controls();
        window.pointer(point.x, point.y);
        window.draw(None);
        assert_eq!(
            window.cursor, cursor,
            "Wrong cursor over {direction:?} edge"
        );
        window.draw_events(
            vec![Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert!(
            window
                .viewport_commands
                .iter()
                .any(|command| *command == egui::ViewportCommand::BeginResize(direction)),
            "Pressing {direction:?} did not begin native resizing"
        );
        assert!(
            !window.started_window_drag(),
            "Resize edge {direction:?} also started window movement"
        );
        window.draw_events(
            vec![Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert!(
            !window.started_window_drag(),
            "Releasing resize edge {direction:?} started window movement"
        );
    }
}

#[test]
fn modified_wheel_zooms_and_visits_actual_size_in_both_directions() {
    for modifiers in [
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        },
    ] {
        let mut window = TestWindow::sample_with_modifier_gestures();
        window
            .ctx
            .set_pixels_per_point(if modifiers.mac_cmd { 2. } else { 1. });
        window.reveal_controls();
        window.wait_for("1 / 5");
        window.draw(Some(Key::H));
        let wheel = |y| Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0., y),
            modifiers,
        };
        for (percent, direction, next) in [("90", 1., 1.25), ("110", -1., 0.8)] {
            window.click(zoom_widget(&window, "zoom-value").rect.center());
            replace_zoom_text(&mut window, percent);
            window.draw(Some(Key::Enter));
            window.pointer(540., 380.);
            // Event modifiers remain authoritative even if the key was released
            // later in the same frame. A batch must visibly stop at 100%.
            window.draw_events(
                vec![wheel(direction), wheel(direction)],
                egui::Modifiers::NONE,
            );
            window.draw(None);
            assert_photo_scale(&window, 1.);
            for _ in 0..12 {
                assert!(window.draw(None).0.contains("1 / 5"));
                assert_photo_scale(&window, 1.);
            }
            window.draw_events(vec![wheel(direction)], modifiers);
            window.draw(None);
            assert_photo_scale(&window, next);
        }
        for (percent, direction, expected) in [("20", -1., 0.2), ("3000", 1., 30.)] {
            window.click(zoom_widget(&window, "zoom-value").rect.center());
            replace_zoom_text(&mut window, percent);
            window.draw(Some(Key::Enter));
            window.pointer(540., 380.);
            window.draw_events(vec![wheel(direction)], modifiers);
            window.draw(None);
            assert_photo_scale(&window, expected);
        }
    }
}

#[test]
fn portrait_monitor_hides_the_spread_control_with_its_separator() {
    let separators_before_random = |window: &TestWindow| {
        let random = window.titlebar_icon("random").rect;
        window
            .paint
            .iter()
            .filter(|shape| {
                matches!(&shape.shape,
                    Shape::LineSegment { points, .. }
                        if (points[0].x - points[1].x).abs() < 0.1
                            && points[0].x < random.left()
                            && (points[0].y - points[1].y).abs() > 10.
                            && points[0].y >= window.screen_size.y - 45.
                )
            })
            .count()
    };
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    assert!(window.titlebar_icon("spread").rect.is_positive());
    assert_eq!(separators_before_random(&window), 2);

    window.monitor_size = Some(egui::vec2(1080., 1920.));
    for _ in 0..3 {
        window.draw(None);
    }
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "spread")))
            .is_none(),
        "Spread control shown on a portrait monitor"
    );
    assert_eq!(separators_before_random(&window), 1);
    // Layout shortcuts do nothing while only one page can be shown.
    window.draw(Some(Key::OpenBracket));
    assert!(!saved_viewer_settings(&window).spread);

    window.monitor_size = Some(egui::vec2(1920., 1080.));
    for _ in 0..3 {
        window.draw(None);
    }
    assert!(window.titlebar_icon("spread").rect.is_positive());
}
