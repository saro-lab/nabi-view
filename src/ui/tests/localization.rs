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
        window.draw(None);
        let mut remaining = vec![
            "language",
            "sort-images",
            "sort-natural",
            "sort-descending",
            "background-color",
            "transparency-checker",
            "metadata-font-size",
            "reduce-motion",
            "image-display",
            "original",
            "contain",
            "slide-interval",
        ];
        let content = window
            .paint
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text) if text.galley.job.text == tr("language") => {
                    Some(shape.clip_rect.intersect(window.ctx.viewport_rect()))
                }
                _ => None,
            })
            .expect("settings content viewport");
        // Longer translations and additional settings extend below the window.
        // Check labels only when they are visible, reaching later rows with the
        // same wheel input a user sends instead of inspecting clipped text.
        for _ in 0..12 {
            remaining.retain(|id| {
                !window.paint.iter().any(|shape| {
                    matches!(&shape.shape, Shape::Text(text)
                        if text.galley.job.text == tr(id)
                            && shape.clip_rect.intersect(window.ctx.viewport_rect())
                                .contains_rect(text.visual_bounding_rect()))
                })
            });
            if remaining.is_empty() {
                break;
            }
            window.pointer(content.center().x, content.center().y);
            window.draw_events(
                vec![Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -240.),
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
            for _ in 0..24 {
                window.draw(None);
            }
        }
        assert!(
            remaining.is_empty(),
            "{} settings could not scroll to {remaining:?}",
            language.code()
        );
        assert_eq!(saved_viewer_settings(&window).language, *language);
        assert!(
            !saved_viewer_settings(&window).rtl,
            "UI locale changed image direction"
        );
        let close = window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "settings-close")))
            .expect("settings close button");
        assert!(window.ctx.viewport_rect().contains_rect(close.rect));
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
        .map(crate::core::paths::simplified)
        .unwrap();
    window.reveal_controls();
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
    for _ in 0..3 {
        window.draw(None);
    }
    let open = window
        .ctx
        .read_response(egui::Id::unique(("recent-folder", &dir)))
        .unwrap();
    let remove = window
        .ctx
        .read_response(egui::Id::unique(("recent-remove", &dir)))
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
