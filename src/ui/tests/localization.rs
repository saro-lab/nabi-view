use super::*;
use crate::i18n::{Language, tr};

#[test]
fn all_locales_render_settings_and_keep_their_saved_preference() {
    for language in Language::ALL.iter().skip(1) {
        let settings = crate::core::settings::Settings {
            language: language.clone(),
            ..Default::default()
        };
        let mut window = TestWindow::sample_with_settings(&settings);
        window.draw(Some(Key::F1));
        for _ in 0..3 {
            window.draw(None);
        }
        let text = window.draw(None).0;
        assert!(text.contains(tr("settings")), "{}: {text}", language.code());
        window.click(window.label_position(tr("general")));
        for _ in 0..3 {
            window.draw(None);
        }
        let general = window.draw(None).0;
        for id in [
            "language",
            "image-display",
            "original",
            "contain",
            "slide-interval",
        ] {
            assert!(
                general.contains(tr(id)),
                "{} is missing {id}: {general}",
                language.code()
            );
        }
        assert_eq!(saved_viewer_settings(&window).language, *language);
        assert!(
            !saved_viewer_settings(&window).rtl,
            "UI locale changed image direction"
        );
        let close = window
            .ctx
            .read_response(egui::Id::new(("titlebar", "settings-close")))
            .expect("settings close button");
        assert!(window.ctx.screen_rect().contains_rect(close.rect));
        let rtl = language.is_rtl();
        assert_eq!(close.rect.center().x < window.screen_size.x / 2., rtl);
        window.click(close.rect.center());
        for _ in 0..3 {
            window.draw(None);
        }
        assert!(!window.draw(None).0.contains(tr("open-source-licenses")));
    }
}

#[test]
fn rtl_recent_rows_mirror_their_controls_and_remove_only_history() {
    let settings = crate::core::settings::Settings {
        language: Language::Arabic,
        ..Default::default()
    };
    let mut window = TestWindow::sample_with_settings(&settings);
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(None);
    let dir = window
        ._images
        .as_ref()
        .unwrap()
        .path()
        .canonicalize()
        .unwrap();
    window.reveal_controls();
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
    for _ in 0..3 {
        window.draw(None);
    }
    let open = window
        .ctx
        .read_response(egui::Id::new(("recent-folder", &dir)))
        .unwrap();
    let remove = window
        .ctx
        .read_response(egui::Id::new(("recent-remove", &dir)))
        .unwrap();
    assert!(remove.rect.right() <= open.rect.left());
    window.click(remove.rect.center());
    for _ in 0..3 {
        window.draw(None);
    }
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    assert!(store.recent().unwrap().is_empty());
    assert!(dir.exists());
}
