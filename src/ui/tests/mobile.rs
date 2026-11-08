use super::*;

#[test]
fn phone_controls_fit_narrow_screens_and_hide_desktop_commands() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample();
    for width in [320., 390., 430.] {
        window.screen_size = egui::vec2(width, 844.);
        for _ in 0..4 {
            window.draw(None);
        }
        let screen = window.ctx.screen_rect();
        for name in ["open", "menu", "settings", "metadata", "slideshow"] {
            let response = window
                .ctx
                .read_response(egui::Id::new(("titlebar", name)))
                .unwrap();
            assert!(
                screen.contains_rect(response.rect),
                "{name} outside {width}-point screen: {:?}",
                response.rect
            );
            assert!(response.rect.width() >= 44. && response.rect.height() >= 44.);
        }
        let menu = window
            .ctx
            .read_response(egui::Id::new(("titlebar", "menu")))
            .unwrap()
            .rect
            .center();
        window.click(menu);
        for _ in 0..3 {
            window.draw(None);
        }
        for action in ["open", "folder", "recent"] {
            let button = window
                .ctx
                .read_response(egui::Id::new(("menu-command", action)))
                .unwrap_or_else(|| panic!("Missing phone menu action: {action}"));
            assert!(
                screen.contains_rect(button.rect),
                "{action} outside {width}-point screen"
            );
        }
        window.draw(Some(Key::Escape));
    }
    let menu = window
        .ctx
        .read_response(egui::Id::new(("titlebar", "menu")))
        .unwrap()
        .rect
        .center();
    window.click(menu);
    let (text, _) = window.draw(None);
    assert!(!text.contains("새 창") && !text.contains("휴지통") && !text.contains("최소화"));
    assert!(
        window
            .ctx
            .read_response(egui::Id::new(("menu-command", "open")))
            .is_some()
    );
    assert!(!window.started_window_drag());
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
        .unwrap();
    assert_eq!(store.recent().unwrap(), std::slice::from_ref(&dir));

    // Reopening this same folder must start at its first image, not this page.
    window.draw(Some(Key::ArrowRight));
    window.wait_for("2 / 5");

    let menu = window
        .ctx
        .read_response(egui::Id::new(("titlebar", "menu")))
        .unwrap();
    window.click(menu.rect.center());
    for _ in 0..3 {
        window.draw(None);
    }
    let recent = window
        .ctx
        .read_response(egui::Id::new(("menu-command", "recent")))
        .unwrap();
    window.click(recent.rect.center());
    for _ in 0..4 {
        window.draw(None);
    }
    let folder = window
        .ctx
        .read_response(egui::Id::new(("recent-folder", &dir)))
        .unwrap();
    window.click(folder.rect.center());
    window.wait_for("1 / 5");
    let text = window.draw(None).0;
    assert!(
        !text.contains(crate::i18n::test_tr("Open folder")),
        "Recent overlay did not close: {text}"
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
    let screen = window.ctx.screen_rect();
    for action in ["open", "folder"] {
        let button = window
            .ctx
            .read_response(egui::Id::new(("recent-action", action)))
            .unwrap();
        assert!(screen.contains_rect(button.rect));
    }
    let folder = window
        .ctx
        .read_response(egui::Id::new(("recent-folder", &dir)))
        .unwrap();
    assert!(screen.contains_rect(folder.rect));
    window.click(folder.rect.center());
    window.wait_for("1 / 5");
}
