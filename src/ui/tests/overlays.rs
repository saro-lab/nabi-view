use super::*;

#[test]
fn recent_overlay_drops_emptied_folder_and_opens_first_image() {
    let mut window = TestWindow::open(None);
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    let root = tempfile::tempdir().unwrap();
    let emptied = root.path().join("emptied");
    let album = root.path().join("album");
    for folder in [&emptied, &album] {
        std::fs::create_dir(folder).unwrap();
        store.record_folder(folder, 10).unwrap();
    }
    let pixel = image::RgbImage::from_pixel(8, 4, image::Rgb([220, 50, 50]));
    pixel.save(emptied.join("1.png")).unwrap();
    // Insertion order and lexical order both differ from the expected natural order.
    pixel.save(album.join("10.png")).unwrap();
    pixel.save(album.join("2.png")).unwrap();
    std::fs::create_dir(album.join("1.png")).unwrap();
    let album = album.canonicalize().unwrap();
    let emptied = emptied.canonicalize().unwrap();
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        None,
        Ok(store),
    );
    for _ in 0..4 {
        window.draw(None);
    }
    let row = window
        .ctx
        .read_response(egui::Id::new(("recent-folder", &emptied)))
        .unwrap();
    std::fs::remove_file(emptied.join("1.png")).unwrap();
    window.click(row.rect.center());
    for _ in 0..4 {
        window.draw(None);
    }
    assert!(emptied.is_dir());
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("recent-folder", &emptied)))
            .is_none()
    );
    let row = window
        .ctx
        .read_response(egui::Id::new(("recent-folder", &album)))
        .unwrap();
    window.click(row.rect.center());
    window.reveal_controls();
    window.wait_for("2.png");
    window.wait_for("1 / 2");
    assert!(
        window.draw(None).1,
        "The first image must actually be painted"
    );

    pixel.save(emptied.join("1.png")).unwrap();
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    assert_eq!(store.recent().unwrap(), [album]);
}

#[test]
fn recent_overlay_prunes_unavailable_items_on_display_and_click() {
    let mut window = TestWindow::open(None);
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    let root = tempfile::tempdir().unwrap();
    let folders = ["missing-at-start", "missing-on-reopen", "missing-on-click"].map(|name| {
        let path = root.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("1.png"), b"image").unwrap();
        store.record_folder(&path, 10).unwrap();
        path.canonicalize().unwrap()
    });
    std::fs::remove_dir_all(&folders[0]).unwrap();
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        None,
        Ok(store),
    );
    for _ in 0..4 {
        window.draw(None);
    }
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("recent-folder", &folders[0])))
            .is_none()
    );

    window.draw(Some(Key::Escape));
    std::fs::remove_dir_all(&folders[1]).unwrap();
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        alt: true,
        shift: true,
    };
    window.draw_events(vec![key_event(Key::O, true, false, modifiers)], modifiers);
    window.draw_events(vec![key_event(Key::O, false, false, modifiers)], modifiers);
    for _ in 0..4 {
        window.draw(None);
    }
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("recent-folder", &folders[1])))
            .is_none()
    );
    let row = window
        .ctx
        .read_response(egui::Id::new(("recent-folder", &folders[2])))
        .unwrap();
    std::fs::remove_dir_all(&folders[2]).unwrap();
    window.click(row.rect.center());
    for _ in 0..4 {
        window.draw(None);
    }
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("recent-folder", &folders[2])))
            .is_none()
    );
    assert!(window.draw(None).0.contains(crate::i18n::test_tr(
        "Recently opened items will appear here."
    )));

    // Check persistence after access is restored, without pruning anything during this read.
    for folder in &folders {
        std::fs::create_dir(folder).unwrap();
        std::fs::write(folder.join("1.png"), b"image").unwrap();
    }
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    assert!(store.recent().unwrap().is_empty());
}

#[test]
fn empty_window_opens_centered_recent_overlay_and_escape_only_dismisses_it() {
    let mut window = TestWindow::open(None);
    for _ in 0..4 {
        window.draw(None);
    }
    window.time += 0.3;
    let (text, image) = window.draw(None);
    for label in [
        "Open image",
        "Open folder",
        "Recently opened items will appear here.",
    ] {
        assert!(
            text.contains(crate::i18n::test_tr(label)),
            "Missing {label}: {text}"
        );
    }
    assert!(!text.contains(crate::i18n::test_tr("Recent")));
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("titlebar", "recent-close")))
            .is_none()
    );
    assert!(!image);
    let overlay = window
        .ctx
        .memory(|m| m.area_rect(egui::Id::new("recent-folders")))
        .unwrap();
    assert!((overlay.center() - window.screen_size.to_pos2() / 2.).length() < 2.);
    assert!(window.paint.iter().any(|s| {
        matches!(&s.shape, Shape::Rect(r)
            if r.fill == Color32::from_rgba_unmultiplied(21, 23, 25, 230) && r.rect == overlay)
    }));
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .contains(&egui::ViewportCommand::Close)
    );
    assert!(window.draw(None).0.is_empty());
    window.reveal_controls();
    window.draw(None);
    assert!(window.titlebar_visible());
}

#[test]
fn recent_overlay_resizes_scrolls_and_removes_only_selected_history() {
    let mut window = TestWindow::open(None);
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    let root = tempfile::tempdir().unwrap();
    let mut folders = Vec::new();
    for index in 0..35 {
        let dir = root.path().join(format!(
            "{index:02} A long folder name with enough text to overflow a phone screen"
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("1.png"), b"image").unwrap();
        store.record_folder(&dir, 50).unwrap();
        folders.push(dir.canonicalize().unwrap());
    }
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        None,
        Ok(store),
    );
    for size in [
        egui::vec2(1080., 760.),
        egui::vec2(600., 420.),
        egui::vec2(320., 568.),
        egui::vec2(390., 844.),
        egui::vec2(800., 360.),
    ] {
        window.screen_size = size;
        for _ in 0..5 {
            window.draw(None);
        }
        let screen = window.ctx.screen_rect();
        let overlay = window
            .ctx
            .memory(|m| m.area_rect(egui::Id::new("recent-folders")))
            .unwrap();
        assert!(
            screen.expand(1.).contains_rect(overlay),
            "Overlay escaped {size:?}: {overlay:?}"
        );
        assert!((overlay.center() - screen.center()).length() < 2.);
        if size.x <= 600. {
            assert!((overlay.width() - size.x).abs() < 1.);
        }
        assert_eq!(
            window
                .ctx
                .read_response(egui::Id::new(("titlebar", "recent-close")))
                .is_some(),
            size.x <= 600.
        );
        assert_eq!(
            window
                .paint
                .iter()
                .any(|shape| matches!(&shape.shape, Shape::Text(text)
            if text.galley.job.text == crate::i18n::test_tr("Recent"))),
            size.x <= 600.
        );
        for action in ["open", "folder"] {
            let button = window
                .ctx
                .read_response(egui::Id::new(("recent-action", action)))
                .unwrap();
            assert!(overlay.contains_rect(button.rect));
        }
        let delete = window
            .ctx
            .read_response(egui::Id::new(("recent-remove", &folders[34])))
            .unwrap();
        assert!(overlay.contains_rect(delete.rect));
        assert_eq!(delete.rect.size(), egui::Vec2::splat(44.));
        let open = window
            .ctx
            .read_response(egui::Id::new(("recent-folder", &folders[34])))
            .unwrap();
        assert!(open.rect.right() < delete.rect.left());
        for shape in &window.paint {
            if let Shape::Text(text) = &shape.shape
                && text.galley.job.text.contains("34 A long folder")
            {
                assert!(text.pos.x + text.galley.size().x <= open.rect.right());
            }
        }
    }
    let delete = window
        .ctx
        .read_response(egui::Id::new(("recent-remove", &folders[34])))
        .unwrap();
    window.click(delete.rect.center());
    assert!(
        folders[34].is_dir(),
        "Removing history must leave the user's folder intact"
    );
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    assert_eq!(store.recent().unwrap().len(), 34);
    assert!(!store.recent().unwrap().contains(&folders[34]));
    // Deleting a missing folder must not try to canonicalize or open it.
    std::fs::remove_dir_all(&folders[33]).unwrap();
    for _ in 0..4 {
        window.draw(None);
    }
    let delete = window
        .ctx
        .read_response(egui::Id::new(("recent-remove", &folders[33])))
        .unwrap();
    window.click(delete.rect.center());
    assert_eq!(store.recent().unwrap().len(), 33);
    assert!(!store.recent().unwrap().contains(&folders[33]));
    window.pointer(400., 230.);
    window.draw_events(
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -5000.),
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    for _ in 0..30 {
        window.draw(None);
    }
    let last = window
        .ctx
        .read_response(egui::Id::new(("recent-remove", &folders[0])))
        .unwrap();
    let overlay = window
        .ctx
        .memory(|m| m.area_rect(egui::Id::new("recent-folders")))
        .unwrap();
    assert!(
        overlay.contains_rect(last.rect),
        "Cannot scroll to the final history entry"
    );
    window.click(egui::pos2(10., 180.));
    assert!(
        !window
            .draw(None)
            .0
            .contains(crate::i18n::test_tr("Open folder"))
    );
}

#[test]
fn recent_overlay_blocks_background_and_opens_a_folder_in_an_empty_window() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    window.click(window.titlebar_icon("menu").rect.center());
    let recent = window
        .ctx
        .read_response(egui::Id::new(("menu-command", "recent")))
        .unwrap();
    window.click(recent.rect.center());
    for key in [Key::ArrowRight, Key::Delete, Key::F] {
        window.draw(Some(key));
    }
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    assert!(!window.started_window_drag());
    window.click(egui::pos2(24., 180.));
    assert!(
        !window
            .draw(None)
            .0
            .contains(crate::i18n::test_tr("Open folder"))
    );

    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    let dir = window
        ._images
        .as_ref()
        .unwrap()
        .path()
        .canonicalize()
        .unwrap();
    store.record_folder(&dir, 50).unwrap();
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        None,
        Ok(store),
    );
    for _ in 0..4 {
        window.draw(None);
    }
    let folder = window
        .ctx
        .read_response(egui::Id::new(("recent-folder", &dir)))
        .unwrap();
    window.click(folder.rect.center());
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    assert!(
        !window
            .draw(None)
            .0
            .contains(crate::i18n::test_tr("Open folder"))
    );
}

#[test]
fn centered_icon_menu_scrolls_and_executes_without_background_input() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        ..Default::default()
    });
    window.screen_size = egui::vec2(520., 360.);
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    window.click(window.titlebar_icon("menu").rect.center());
    for size in [
        egui::vec2(1080., 760.),
        egui::vec2(600., 420.),
        egui::vec2(320., 568.),
        egui::vec2(390., 844.),
        egui::vec2(520., 360.),
    ] {
        window.screen_size = size;
        for _ in 0..5 {
            window.draw(None);
        }
        let screen = window.ctx.screen_rect();
        let overlay = window
            .ctx
            .memory(|m| m.area_rect(egui::Id::new("command-menu")))
            .unwrap();
        assert!(
            screen.expand(1.).contains_rect(overlay),
            "Menu escaped {size:?}: {overlay:?}"
        );
        assert!((overlay.center() - screen.center()).length() < 2.);
        if size.x <= 600. {
            assert_eq!(overlay, screen);
        }
        assert_eq!(
            window
                .ctx
                .read_response(egui::Id::new(("titlebar", "menu-close")))
                .is_some(),
            size.x <= 600.
        );
        assert_eq!(
            window
                .paint
                .iter()
                .any(|shape| matches!(&shape.shape, Shape::Text(text)
            if text.galley.job.text == "Menu")),
            size.x <= 600.
        );
        for id in ["maximize", "minimize", "close-window", "close-all"] {
            assert!(
                window
                    .ctx
                    .read_response(egui::Id::new(("menu-command", id)))
                    .is_none()
            );
        }
    }
    window.time += 0.25;
    window.draw(None);
    let menu = window
        .ctx
        .memory(|m| m.area_rect(egui::Id::new("command-menu")))
        .unwrap();
    assert!(
        window
            .paint
            .iter()
            .any(|s| matches!(&s.shape, Shape::Rect(r)
        if r.fill == Color32::from_rgba_unmultiplied(21, 23, 25, 230) && r.rect == menu))
    );
    assert!(window.draw(None).0.contains("Recent"));
    assert!(
        (menu.center() - window.screen_size.to_pos2() / 2.).length() < 2.,
        "Menu is not centered: {menu:?}"
    );
    assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, window.screen_size).contains_rect(menu));
    let open = window
        .ctx
        .read_response(egui::Id::new(("menu-command", "open")))
        .unwrap()
        .rect;
    assert!(
        window
            .paint
            .iter()
            .any(|s| matches!(&s.shape, Shape::Circle(c)
        if open.contains(c.center) && c.stroke.width > 0.)),
        "Missing image icon"
    );
    assert!(window.drag_pointer(menu.left_top() + egui::vec2(80., 24.), egui::vec2(30., 5.)));
    for key in [Key::ArrowRight, Key::R, Key::Delete, Key::F] {
        window.draw(Some(key));
        assert!(!window.viewport_commands.iter().any(|c| matches!(
            c,
            egui::ViewportCommand::Close | egui::ViewportCommand::Fullscreen(_)
        )));
    }
    assert!(!saved_viewer_settings(&window).flip_horizontal);
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    let first_y = open.top();
    window.pointer(menu.center().x, menu.center().y);
    window.draw_events(
        vec![
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., -1800.),
                modifiers: egui::Modifiers::NONE,
            },
            Event::Zoom(1.5),
        ],
        egui::Modifiers::NONE,
    );
    for _ in 0..30 {
        window.draw(None);
    }
    let shifted = window
        .ctx
        .read_response(egui::Id::new(("menu-command", "open")))
        .unwrap()
        .rect;
    assert!(
        shifted.top() < first_y - 100.,
        "Wheel did not scroll the icon grid"
    );
    assert_eq!(
        focused_thumbnail_pages(&window),
        [1],
        "Menu wheel navigated the image"
    );
    let fullscreen = window
        .ctx
        .read_response(egui::Id::new(("menu-command", "fullscreen")))
        .unwrap()
        .rect;
    assert!(
        menu.contains_rect(fullscreen),
        "Last commands cannot be reached by scrolling"
    );
    // Keep the output from the release frame, where the command is emitted.
    let position = fullscreen.center();
    window.pointer(position.x, position.y);
    for pressed in [true, false] {
        window.draw_events(
            vec![Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
    }
    assert!(
        window
            .viewport_commands
            .contains(&egui::ViewportCommand::Fullscreen(true))
    );
    assert!(!window.draw(None).0.contains("Open image"));
}

#[test]
fn menu_and_recent_blank_space_moves_window_without_dismissing_or_activating_controls() {
    for mobile in [false, true] {
        let _phone = mobile.then(PhonePreview::enter);
        for fullscreen in [false, true] {
            if mobile && fullscreen {
                continue;
            }
            for recent in [false, true] {
                let mut window = TestWindow::open(None);
                for _ in 0..4 {
                    window.draw(None);
                }
                window.draw(Some(Key::Escape));
                if fullscreen {
                    window.draw(Some(Key::F));
                }
                window.reveal_controls();
                window.click(window.titlebar_icon("menu").rect.center());
                if recent {
                    let button = window
                        .ctx
                        .read_response(egui::Id::new(("menu-command", "recent")))
                        .unwrap();
                    window.click(button.rect.center());
                }
                let area_id = egui::Id::new(if recent {
                    "recent-folders"
                } else {
                    "command-menu"
                });
                let control_id = egui::Id::new((
                    if recent {
                        "recent-action"
                    } else {
                        "menu-command"
                    },
                    "open",
                ));
                let movable = !mobile && !fullscreen;
                for width in [1080., 520.] {
                    window.screen_size.x = width;
                    for _ in 0..5 {
                        window.draw(None);
                    }
                    let panel = window.ctx.memory(|m| m.area_rect(area_id)).unwrap();
                    let mut blanks = vec![panel.left_center() + egui::vec2(8., 0.)];
                    if recent {
                        blanks.push(panel.center());
                    } else {
                        let tile = window.ctx.read_response(control_id).unwrap().rect;
                        blanks.push(tile.right_center() + egui::vec2(4., 0.));
                    }
                    if width > 600. {
                        blanks.push(egui::pos2(24., 180.));
                    }
                    for start in blanks {
                        if panel.contains(start) {
                            assert_eq!(
                                window.drag_pointer(start, egui::vec2(1., 1.)),
                                movable,
                                "Small blank-space drag: recent={recent}, mobile={mobile}, fullscreen={fullscreen}, width={width}"
                            );
                        }
                        assert_eq!(
                            window.drag_pointer(start, egui::vec2(30., 8.)),
                            movable,
                            "recent={recent}, mobile={mobile}, fullscreen={fullscreen}, width={width}, start={start:?}"
                        );
                        assert!(
                            window.ctx.read_response(control_id).is_some(),
                            "Dragging dismissed the overlay"
                        );
                    }
                    let control = window.ctx.read_response(control_id).unwrap();
                    assert!(
                        !window.drag_pointer(control.rect.center(), egui::vec2(30., 8.)),
                        "Dragging a control must not move the window"
                    );
                    assert!(
                        window.ctx.read_response(control_id).is_some(),
                        "Dragging activated a control"
                    );
                }
            }
        }
    }
}

#[test]
fn centered_icon_menu_applies_view_mode_and_dismisses_with_close_or_backdrop() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    window.click(window.titlebar_icon("menu").rect.center());
    let width = window
        .ctx
        .read_response(egui::Id::new(("menu-command", "width")))
        .unwrap()
        .rect;
    window.click(width.center());
    assert_eq!(saved_viewer_settings(&window).view_mode, "width");
    assert!(!window.draw(None).0.contains("Open image"));
    for close_button in [true, false] {
        window.screen_size.x = if close_button { 520. } else { 1080. };
        for _ in 0..4 {
            window.draw(None);
        }
        window.click(window.titlebar_icon("menu").rect.center());
        for _ in 0..4 {
            window.draw(None);
        }
        let position = if close_button {
            window.titlebar_icon("menu-close").rect.center()
        } else {
            egui::pos2(24., 180.)
        };
        window.click(position);
        assert!(
            !window.draw(None).0.contains("Open image"),
            "Dismiss failed (close_button={close_button})"
        );
        assert_eq!(focused_thumbnail_pages(&window), [1]);
        assert!(
            !window
                .viewport_commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Close))
        );
    }
}

#[test]
fn right_clicking_empty_canvas_opens_menu_and_backdrop_only_dismisses_it() {
    let mut window = TestWindow::open(None);
    for _ in 0..4 {
        window.draw(None);
    }
    let position = egui::pos2(540., 380.);
    let secondary_click = |window: &mut TestWindow| {
        window.pointer(position.x, position.y);
        for pressed in [true, false] {
            window.draw_events(
                vec![Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
        for _ in 0..4 {
            window.draw(None);
        }
    };
    // The start overlay must not let a right click open another layer underneath.
    secondary_click(&mut window);
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("menu-command", "open")))
            .is_none()
    );
    window.click(egui::pos2(24., 180.));
    secondary_click(&mut window);
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("menu-command", "open")))
            .is_some()
    );
    assert!(!window.started_window_drag());
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("titlebar", "menu-close")))
            .is_none()
    );
    window.click(egui::pos2(24., 180.));
    for _ in 0..4 {
        window.draw(None);
    }
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("menu-command", "open")))
            .is_none()
    );
    assert!(
        !window
            .viewport_commands
            .contains(&egui::ViewportCommand::Close)
    );
}
