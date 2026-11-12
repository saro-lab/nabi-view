use super::*;

fn drag_touch(window: &mut TestWindow, start: egui::Pos2, delta: egui::Vec2) {
    let touch = |phase, pos| Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase,
        pos,
        force: None,
    };
    window.pointer(start.x, start.y);
    window.draw_events(
        vec![
            touch(egui::TouchPhase::Start, start),
            Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        egui::Modifiers::NONE,
    );
    let end = start + delta;
    window.draw_events(
        vec![touch(egui::TouchPhase::Move, end), Event::PointerMoved(end)],
        egui::Modifiers::NONE,
    );
    window.draw_events(
        vec![
            touch(egui::TouchPhase::End, end),
            Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerGone,
        ],
        egui::Modifiers::NONE,
    );
    window.draw(None);
    assert!(!window.started_window_drag());
}

#[test]
fn phone_bars_stay_visible_and_fullscreen_taps_restore_them() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(390., 844.);
    window.wait_for("1 / 5");
    window.time += 5.;
    window.click(egui::pos2(195., 400.));
    window.draw(Some(Key::H)); // The desktop hide-controls command cannot hide phone bars.
    window.draw(Some(Key::I));
    window.draw(Some(Key::I)); // Closing metadata used to hide the mobile bars too.
    window.draw(None);
    assert!(window.titlebar_visible());
    assert!(window.draw(None).0.contains("1 / 5"));
    let normal_canvas = window.photo_rects().0;
    assert!(normal_canvas.top() > 0. && normal_canvas.bottom() < window.screen_size.y);
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "pin")))
            .is_none()
    );

    // The fullscreen button replaces the mobile pin. Top/bottom letterboxes
    // and the photograph itself must all offer the same way back.
    for position in [
        egui::pos2(5., 5.),
        egui::pos2(195., 400.),
        egui::pos2(385., 839.),
    ] {
        let fullscreen = window.titlebar_icon("fullscreen").rect.center();
        window.click(fullscreen);
        window.draw(None);
        assert!(!window.titlebar_visible());
        assert_eq!(window.photo_rects().0, window.ctx.viewport_rect());
        window.click(position);
        window.draw(None);
        assert!(window.titlebar_visible());
        assert_eq!(window.photo_rects().0, normal_canvas);
    }

    // Metadata covers the canvas with interactive text, but a fullscreen tap
    // still exits instead of requiring a tap on an uncovered part of the photo.
    window.draw(Some(Key::I));
    window.draw(Some(Key::F));
    window.draw(None);
    assert!(!window.titlebar_visible());
    window.click(egui::pos2(50., 100.));
    window.draw(None);
    assert!(window.titlebar_visible());

    // A long-press menu must not re-enter fullscreen when its fullscreen tile
    // receives the same tap that exits fullscreen from the canvas handler.
    window.draw(Some(Key::I));
    window.draw(Some(Key::F));
    let hold = egui::pos2(195., 400.);
    window.pointer(hold.x, hold.y);
    window.draw_events(
        vec![
            Event::Touch {
                device_id: egui::TouchDeviceId(2),
                id: egui::TouchId(2),
                phase: egui::TouchPhase::Start,
                pos: hold,
                force: None,
            },
            Event::PointerButton {
                pos: hold,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        egui::Modifiers::NONE,
    );
    window.time += 1.;
    window.draw(None);
    window.draw_events(
        vec![
            Event::Touch {
                device_id: egui::TouchDeviceId(2),
                id: egui::TouchId(2),
                phase: egui::TouchPhase::End,
                pos: hold,
                force: None,
            },
            Event::PointerButton {
                pos: hold,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerGone,
        ],
        egui::Modifiers::NONE,
    );
    for _ in 0..3 {
        window.draw(None);
    }
    let fullscreen_tile = window
        .ctx
        .read_response(egui::Id::unique(("menu-command", "fullscreen")))
        .expect("Long press should open the fullscreen menu")
        .rect
        .center();
    window.click(fullscreen_tile);
    window.draw(None);
    assert!(window.titlebar_visible());
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("menu-command", "fullscreen")))
            .is_none()
    );
}

#[test]
fn phone_horizontal_swipes_change_pages_even_when_zoomed_or_fullscreen() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(390., 844.);
    window.wait_for("1 / 5");
    let start = egui::pos2(280., 400.);
    drag_touch(&mut window, start, egui::vec2(-160., 5.));
    window.wait_for("2 / 5");
    assert!(window.titlebar_visible());
    drag_touch(&mut window, egui::pos2(110., 400.), egui::vec2(160., -5.));
    window.wait_for("1 / 5");

    // The fit view can be zoomed, but a clear horizontal swipe still advances.
    for _ in 0..4 {
        window.draw(Some(Key::Equals));
    }
    let zoomed = window.photo_rects().1;
    assert!(zoomed.width() > window.photo_rects().0.width());
    drag_touch(&mut window, start, egui::vec2(5., 80.));
    let panned = window.photo_rects().1;
    assert!(panned.center().y > zoomed.center().y + 50.);
    assert!(window.draw(None).0.contains("1 / 5"));
    let previous = window.photo_texture();
    drag_touch(&mut window, start, egui::vec2(-160., 0.));
    window.wait_for("2 / 5");
    window.wait_for_texture_replacement(previous);
    assert!(
        window
            .photo_rects()
            .0
            .expand(1.)
            .contains_rect(window.photo_rects().1)
    );

    window.draw(Some(Key::F));
    window.draw(None);
    drag_touch(&mut window, start, egui::vec2(-160., 0.));
    assert!(
        !window.titlebar_visible(),
        "A swipe must not exit fullscreen"
    );
    window.click(egui::pos2(195., 400.));
    window.wait_for("3 / 5");
    assert!(window.titlebar_visible());
}

#[test]
fn phone_pinch_and_two_finger_pan_do_not_swipe_or_exit_fullscreen() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(390., 844.);
    window.wait_for("1 / 5");
    window.draw(Some(Key::F));
    window.draw(None);
    let before = window.photo_rects().1;
    let touch = |id, phase, pos| Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos,
        force: None,
    };
    let first = egui::pos2(160., 400.);
    let second = egui::pos2(230., 400.);
    window.draw_events(
        vec![
            Event::PointerMoved(first),
            Event::PointerButton {
                pos: first,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            touch(1, egui::TouchPhase::Start, first),
        ],
        egui::Modifiers::NONE,
    );
    window.draw_events(
        vec![touch(2, egui::TouchPhase::Start, second)],
        egui::Modifiers::NONE,
    );
    let moved_first = egui::pos2(60., 430.);
    let moved_second = egui::pos2(260., 430.);
    window.draw_events(
        vec![
            Event::PointerMoved(moved_first),
            touch(1, egui::TouchPhase::Move, moved_first),
            touch(2, egui::TouchPhase::Move, moved_second),
        ],
        egui::Modifiers::NONE,
    );
    window.draw(None);
    let after = window.photo_rects().1;
    assert!(
        after.width() > before.width() * 2.,
        "before={before:?}, after={after:?}, touch={:?}",
        window.ctx.input(|i| i.multi_touch())
    );
    assert!(after.center().x < before.center().x - 20.);
    assert!(after.center().y > before.center().y + 20.);
    window.draw_events(
        vec![
            touch(1, egui::TouchPhase::End, moved_first),
            touch(2, egui::TouchPhase::End, moved_second),
            Event::PointerButton {
                pos: moved_first,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerGone,
        ],
        egui::Modifiers::NONE,
    );
    window.draw(None);
    assert!(!window.titlebar_visible());
    window.click(egui::pos2(195., 400.));
    window.wait_for("1 / 5");
    assert!(window.titlebar_visible());
}

#[test]
fn tablet_settings_adapt_to_rotation_and_split_screen() {
    let _mobile = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(834., 1194.);
    window.wait_for("1 / 5");
    let settings = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "settings")))
        .unwrap();
    window.click(settings.rect.center());
    for _ in 0..3 {
        window.draw(None);
    }
    // A full-width tablet exposes categories alongside the selected page.
    let text = window.draw(None).0;
    assert!(
        text.lines()
            .any(|line| line == crate::i18n::tr("keyboard-shortcuts"))
    );
    for hidden in ["connections", "mouse"] {
        assert!(
            !text.lines().any(|line| line == crate::i18n::tr(hidden)),
            "{hidden}: {text}"
        );
    }
    window.click(window.label_position(crate::i18n::tr("data")));
    window.draw(None);

    // Resizing to a narrow multitasking window swaps in the category picker,
    // retaining the selected page. Re-expanding restores the sidebar.
    for (size, sidebar) in [
        (egui::vec2(507., 1194.), false),
        (egui::vec2(1194., 834.), true),
        (egui::vec2(600., 834.), false),
        (egui::vec2(834., 1194.), true),
    ] {
        window.screen_size = size;
        for _ in 0..4 {
            window.draw(None);
        }
        let text = window.draw(None).0;
        assert!(
            text.contains(crate::i18n::tr("export-settings")),
            "{size:?}: {text}"
        );
        assert_eq!(
            text.lines()
                .any(|line| line == crate::i18n::tr("keyboard-shortcuts")),
            sidebar,
            "Category navigation at {size:?}: {text}"
        );
        for hidden in ["connections", "mouse"] {
            assert!(
                !text.lines().any(|line| line == crate::i18n::tr(hidden)),
                "{hidden}: {text}"
            );
        }
        let close = window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "settings-close")))
            .unwrap();
        assert!(window.ctx.viewport_rect().contains_rect(close.rect));
        assert!(close.rect.width() >= 44. && close.rect.height() >= 44.);
    }
    assert!(!window.started_window_drag());
}

#[test]
fn phone_uses_desktop_bars_without_zoom_and_hides_desktop_commands() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    for width in [320., 390., 430., 844., 1200.] {
        window.screen_size = egui::vec2(width, 844.);
        for _ in 0..4 {
            window.draw(None);
        }
        let screen = window.ctx.viewport_rect();
        // Two-page spreads are offered only when the screen is wider than tall.
        let landscape = screen.width() > screen.height();
        assert_eq!(
            window
                .ctx
                .read_response(egui::Id::unique(("titlebar", "spread")))
                .is_some(),
            landscape,
            "Spread control visibility at {width} points"
        );
        // The desktop top bar, status bar and thumbnail strip are all present.
        for name in ["orientation", "fullscreen", "menu", "settings", "random"] {
            let response = window
                .ctx
                .read_response(egui::Id::unique(("titlebar", name)))
                .unwrap_or_else(|| panic!("Missing phone control: {name}"));
            assert!(
                screen.contains_rect(response.rect),
                "{name} outside {width}-point screen: {:?}",
                response.rect
            );
            assert!(response.rect.width() >= 44. && response.rect.height() >= 44.);
        }
        for name in ["slideshow", "share", "metadata", "trash"] {
            assert!(
                window
                    .ctx
                    .read_response(egui::Id::unique(("titlebar", name)))
                    .is_some(),
                "Missing phone status command: {name}"
            );
        }
        assert!(
            window
                .ctx
                .memory(|m| m.area_rect(egui::Id::unique("thumbnails")))
                .is_some()
        );
        // Pinch zooms on phones; the zoom field and window commands are desktop-only.
        assert!(
            window
                .ctx
                .read_response(egui::Id::unique("zoom-value"))
                .is_none()
        );
        for name in ["close", "reveal", "move"] {
            assert!(
                window
                    .ctx
                    .read_response(egui::Id::unique(("titlebar", name)))
                    .is_none(),
                "{name} shown on a phone"
            );
        }
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
        for action in [
            "open",
            "folder",
            "recent",
            "share",
            "fullscreen",
            "trash",
            "undo-trash",
            "orientation-auto",
            "orientation-portrait",
            "orientation-landscape",
        ] {
            assert!(
                window
                    .ctx
                    .read_response(egui::Id::unique(("menu-command", action)))
                    .is_some(),
                "Missing phone menu action: {action}"
            );
        }
        let open = window
            .ctx
            .read_response(egui::Id::unique(("menu-command", "open")))
            .unwrap();
        assert!(screen.contains_rect(open.rect));
        window.draw(Some(Key::Escape));
    }
    let menu = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "menu")))
        .unwrap()
        .rect
        .center();
    window.click(menu);
    let (text, _) = window.draw(None);
    assert!(!text.contains("새 창") && !text.contains("최소화"));
    for action in ["new-window", "move", "reveal", "minimize", "close-window"] {
        assert!(
            window
                .ctx
                .read_response(egui::Id::unique(("menu-command", action)))
                .is_none(),
            "{action} shown on a phone"
        );
    }
    assert!(!window.started_window_drag());
}

#[test]
fn phone_orientation_toggle_cycles_and_menu_sets_it() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(390., 844.);
    window.wait_for("1 / 5");
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    for mode in ["portrait", "landscape", "auto"] {
        let toggle = window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "orientation")))
            .unwrap();
        window.click(toggle.rect.center());
        let text = window.draw(None).0;
        let label = crate::i18n::tr(match mode {
            "portrait" => "orientation-portrait",
            "landscape" => "orientation-landscape",
            _ => "orientation-auto",
        });
        assert!(text.contains(label), "Missing {label}: {text}");
        assert_eq!(store.load().unwrap().orientation, mode);
    }

    // A tall screen keeps the orientation tiles inside the menu without scrolling.
    window.screen_size = egui::vec2(560., 1400.);
    for _ in 0..3 {
        window.draw(None);
    }
    let menu = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "menu")))
        .unwrap();
    window.click(menu.rect.center());
    for _ in 0..3 {
        window.draw(None);
    }
    let landscape = window
        .ctx
        .read_response(egui::Id::unique(("menu-command", "orientation-landscape")))
        .unwrap();
    window.click(landscape.rect.center());
    window.draw(None);
    assert_eq!(store.load().unwrap().orientation, "landscape");
}

#[test]
fn desktop_keeps_zoom_and_close_without_orientation() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique("zoom-value"))
            .is_some()
    );
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "close")))
            .is_some()
    );
    for name in ["orientation", "share"] {
        assert!(
            window
                .ctx
                .read_response(egui::Id::unique(("titlebar", name)))
                .is_none(),
            "{name} shown on desktop"
        );
    }
}

#[test]
fn phone_settings_show_data_resets_and_file_transfer() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(390., 844.);
    window.wait_for("1 / 5");
    window.draw(None); // History is saved after the first displayed image.
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    assert_eq!(store.recent().unwrap().len(), 1);

    let settings = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "settings")))
        .unwrap();
    window.click(settings.rect.center());
    window.draw(None);
    window.click(window.label_position(crate::i18n::tr("general")));
    window.draw(None);
    window.click(window.label_position(crate::i18n::tr("data")));
    let text = window.draw(None).0;
    for key in ["reset-settings", "reset-data", "reset-everything"] {
        assert!(text.contains(crate::i18n::tr(key)), "Missing {key}: {text}");
    }
    assert!(text.contains(crate::i18n::tr("export-settings")));
    assert!(text.contains(crate::i18n::tr("import-settings")));

    // Press the reset button on the same row as "Reset data", then confirm.
    let row = window.label_position(crate::i18n::tr("reset-data"));
    let button = window
        .paint
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.job.text == crate::i18n::tr("reset-dialog") => {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
        .min_by(|a, b| (a.y - row.y).abs().total_cmp(&(b.y - row.y).abs()))
        .unwrap();
    window.click(button);
    window.draw(None);
    let text = window.draw(None).0;
    assert!(
        text.contains(crate::i18n::tr("reset-data-confirm")),
        "{text}"
    );
    window.click(window.label_position(crate::i18n::tr("reset")));
    window.draw(None);
    assert!(store.recent().unwrap().is_empty());
}

#[test]
fn phone_preview_remembers_and_reopens_recent_folders() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    window.screen_size = egui::vec2(320., 568.);
    window.wait_for("1 / 5");
    window.draw(None); // History is saved after the first displayed image.
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
        .map(crate::core::paths::simplified)
        .unwrap();
    assert_eq!(store.recent().unwrap(), std::slice::from_ref(&dir));

    // Reopening a recent folder starts at the page it showed last.
    window.draw(Some(Key::ArrowRight));
    window.wait_for("2 / 5");

    let menu = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "menu")))
        .unwrap();
    window.click(menu.rect.center());
    for _ in 0..3 {
        window.draw(None);
    }
    let recent = window
        .ctx
        .read_response(egui::Id::unique(("menu-command", "recent")))
        .unwrap();
    window.click(recent.rect.center());
    for _ in 0..4 {
        window.draw(None);
    }
    let folder = window
        .ctx
        .read_response(egui::Id::unique(("recent-folder", &dir)))
        .unwrap();
    window.click(folder.rect.center());
    window.wait_for("2 / 5");
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "library-folder")))
            .is_none(),
        "Recent overlay did not close"
    );

    // A new viewer restores the saved list and can reopen it in the same window.
    window.app = Viewer::new(
        &CreationContext::_new_kittest(window.ctx.clone()),
        None,
        Ok(store),
    );
    for _ in 0..4 {
        window.draw(None);
    }
    let screen = window.ctx.viewport_rect();
    for action in ["library-open", "library-folder"] {
        let button = window
            .ctx
            .read_response(egui::Id::unique(("titlebar", action)))
            .unwrap();
        assert!(screen.contains_rect(button.rect));
    }
    let folder = window
        .ctx
        .read_response(egui::Id::unique(("recent-folder", &dir)))
        .unwrap();
    assert!(screen.contains_rect(folder.rect));
    window.click(folder.rect.center());
    window.wait_for("2 / 5");
}
