use super::*;
use crate::core::{
    commands::shortcut_label,
    settings::{KeyboardShortcut, Settings},
};
use crate::i18n::tr;

fn hover_icon(window: &mut TestWindow, icon: &str) -> String {
    window.reveal_controls();
    let position = window.titlebar_icon(icon).rect.center();
    window.pointer(position.x, position.y);
    window.time += 1.;
    for _ in 0..3 {
        window.draw(None);
    }
    window.draw(None).0
}

#[test]
fn mobile_brand_opens_recent_and_bookmarks_with_a_full_touch_target() {
    let _phone = PhonePreview::enter();
    for language in [
        crate::i18n::Language::English,
        crate::i18n::Language::Arabic,
    ] {
        let mut window = TestWindow::sample_with_settings(&Settings {
            language,
            ..Default::default()
        });
        window.screen_size = egui::vec2(390., 844.);
        window.wait_for("1 / 5");
        window.time += 0.3;
        window.draw(Some(Key::ArrowRight));
        window.wait_for("2 / 5");
        let fitted = window.photo_rects().1;
        window.draw(Some(Key::Equals));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            window.draw(None);
            if window.photo_rects().1.width() > fitted.width() + 1. {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "zoomed photo was not drawn"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let photo = window.photo_rects().1;
        let home = window.titlebar_icon("home").rect;
        assert!(home.width() >= 44. && home.height() >= 44.);
        // Press the edge of the target, outside the 24px icon itself.
        window.click(home.left_center() + egui::vec2(3., 0.));
        let text = window.draw(None).0;
        assert!(
            text.contains(tr("recent")) && text.contains(tr("bookmarks")),
            "{text}"
        );
        let close = window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "library-close")))
            .unwrap();
        window.click(close.rect.center());
        window.wait_for("2 / 5");
        assert_eq!(window.photo_rects().1, photo);
    }
}

#[test]
fn desktop_brand_keeps_its_existing_non_navigation_behavior() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("titlebar", "home")))
            .is_none()
    );
}

#[test]
fn chrome_tooltips_show_current_native_shortcuts_on_a_smaller_second_line() {
    let settings = Settings {
        language: crate::i18n::Language::English,
        keyboard_shortcuts: Some(vec![
            KeyboardShortcut {
                action: "settings".into(),
                key: "Primary+Alt+Comma".into(),
            },
            KeyboardShortcut {
                action: "settings".into(),
                key: "F2".into(),
            },
            KeyboardShortcut {
                action: "metadata".into(),
                key: "Primary+Shift+I".into(),
            },
            KeyboardShortcut {
                action: "chrome".into(),
                key: "H".into(),
            },
            KeyboardShortcut {
                action: "close-window".into(),
                key: "Primary+W".into(),
            },
            KeyboardShortcut {
                action: "spread".into(),
                key: "B".into(),
            },
            KeyboardShortcut {
                action: "random".into(),
                key: "Y".into(),
            },
        ]),
        ..Default::default()
    };
    let mut window = TestWindow::sample_with_settings(&settings);
    window.reveal_controls();
    window.wait_for("1 / 5");
    for (icon, label, keys) in [
        (
            "settings",
            "Settings",
            format!("{} / F2", shortcut_label("Primary+Alt+Comma")),
        ),
        (
            "metadata",
            tr("image-metadata"),
            shortcut_label("Primary+Shift+I"),
        ),
        ("pin", "Pin controls", "H".into()),
        ("close", "Close window", shortcut_label("Primary+W")),
        ("spread", "Two-page spread", "B".into()),
        ("random", "Random order", "Y".into()),
    ] {
        let text = hover_icon(&mut window, icon);
        assert!(text.contains(&keys), "Missing {icon} shortcut: {text}");
        let text_shape = |expected: &str| {
            window
                .paint
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    Shape::Text(text) if text.galley.job.text == expected => Some(text),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing tooltip text: {expected}"))
        };
        let title = text_shape(label);
        let shortcut = text_shape(&keys);
        assert!(shortcut.pos.y >= title.pos.y + title.galley.size().y);
        assert!(
            shortcut.galley.job.sections[0].format.font_id.size
                < title.galley.job.sections[0].format.font_id.size
        );
    }
    // Removing a binding must not restore or display the default key.
    let text = hover_icon(&mut window, "slideshow");
    assert!(text.contains("Slideshow"));
    assert!(!text.lines().any(|line| line == "Q"));
}

#[test]
fn close_button_gap_matches_the_brand_icon_gap() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    // Narrow windows use smaller buttons; the close button must still hug the edge.
    for width in [1080., 800., 699., 640., 560.] {
        window.screen_size.x = width;
        window.draw(None);
        window.reveal_controls();
        let close = window.titlebar_icon("close").rect;
        // The X glyph spans 6px either side of the button center.
        let gap = width - (close.center().x + 6.);
        assert!(
            (gap - 14.).abs() <= 1.,
            "close glyph is {gap}px from the edge at {width}px"
        );
    }
}

#[test]
fn short_chrome_tooltips_stay_on_one_line_when_the_screen_has_room() {
    let icons = [
        ("move", "move-images"),
        ("settings", "settings"),
        ("metadata", "image-metadata"),
        ("close", "close-window"),
    ];
    // egui remembers each tooltip's size and wraps at it next frame; at
    // fractional display scales that rounded size can be narrower than the text.
    for scale in [1., 1.25, 1.5, 1.75] {
        let mut window = TestWindow::sample_with_settings(&Settings {
            language: crate::i18n::Language::English,
            ..Default::default()
        });
        window.scale = scale;
        window.reveal_controls();
        window.wait_for("1 / 5");
        for (icon, key) in icons.into_iter().chain(icons) {
            hover_icon(&mut window, icon);
            let label = crate::i18n::tr(key);
            let rows = window
                .paint
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.galley.rows.len())
                    }
                    _ => None,
                });
            assert_eq!(rows, Some(1), "{icon} tooltip wrapped at {scale}x: {label}");
        }
    }
}
