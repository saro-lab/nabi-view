use super::*;

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
        for name in ["orientation", "pin", "menu", "settings", "random"] {
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
