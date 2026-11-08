use super::*;
use crate::core::{
    commands::shortcut_label,
    settings::{KeyboardShortcut, Settings},
};

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
            "Image metadata",
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
