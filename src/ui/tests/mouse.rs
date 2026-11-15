use super::*;
use crate::core::settings::{MouseSettings, Settings};
use crate::i18n::{Language, tr};

fn english(mouse: MouseSettings) -> Settings {
    Settings {
        language: Language::English,
        mouse,
        ..Default::default()
    }
}

/// A sample window with the page counter pinned and the pointer over the photo.
fn viewer(mouse: MouseSettings) -> TestWindow {
    let mut window = TestWindow::sample_with_settings(&english(mouse));
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::H));
    window.pointer(540., 380.);
    window.time += 0.1;
    window.draw(None);
    window
}

/// Sends press/release pairs and returns every viewport command they caused.
fn press(
    window: &mut TestWindow,
    position: egui::Pos2,
    button: egui::PointerButton,
    times: usize,
) -> Vec<egui::ViewportCommand> {
    let mut commands = Vec::new();
    window.pointer(position.x, position.y);
    for _ in 0..times {
        for pressed in [true, false] {
            window.draw_events(
                vec![Event::PointerButton {
                    pos: position,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
            commands.append(&mut window.viewport_commands);
        }
    }
    for _ in 0..4 {
        window.draw(None);
        commands.append(&mut window.viewport_commands);
    }
    commands
}

fn wheel(window: &mut TestWindow, y: f32, modifiers: egui::Modifiers) {
    window.draw_events(
        vec![Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., y),
            modifiers,
        }],
        egui::Modifiers::NONE,
    );
    window.draw(None);
}

fn menu_open(window: &TestWindow) -> bool {
    window
        .ctx
        .read_response(egui::Id::unique(("menu-command", "open")))
        .is_some()
}

fn library_open(window: &mut TestWindow) -> bool {
    window.draw(None).0.contains(tr("bookmarks"))
}

fn fullscreen(commands: &[egui::ViewportCommand]) -> bool {
    commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::Fullscreen(true)))
}

const CTRL: egui::Modifiers = egui::Modifiers {
    ctrl: true,
    command: true,
    alt: false,
    shift: false,
    mac_cmd: false,
};

#[test]
fn mouse_section_sits_above_shortcuts_and_saves_each_gesture() {
    let mut window = TestWindow::sample_with_settings(&english(MouseSettings::default()));
    window.reveal_controls();
    window.click(window.titlebar_icon("settings").rect.center());
    for _ in 0..3 {
        window.draw(None);
    }
    let mouse = window.label_position(tr("mouse"));
    let shortcuts = window.label_position(tr("keyboard-shortcuts"));
    let cache = window.label_position(tr("cache"));
    assert!(cache.y < mouse.y && mouse.y < shortcuts.y);
    assert!(!window.draw(None).0.lines().any(|line| line == tr("touch")));
    window.click(mouse);
    for _ in 0..3 {
        window.draw(None);
    }
    let text = window.draw(None).0;
    let modifier = crate::core::commands::shortcut_label("Primary");
    assert_eq!(
        modifier,
        if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl"
        }
    );
    for label in [
        tr("mouse-wheel").to_owned(),
        format!("{modifier} + {}", tr("mouse-wheel")),
        format!("{modifier} + {}", tr("mouse-drag")),
        tr("reverse-wheel-direction").into(),
        tr("zoom").into(),
        tr("no-action").into(),
        tr("open-menu").into(),
        tr("recent-and-bookmarks").into(),
        tr("toggle-full-screen").into(),
        tr("previous-page-or-spread").into(),
        tr("next-page-or-spread").into(),
        tr("move-image").into(),
    ] {
        assert!(text.contains(&label), "Missing {label}: {text}");
    }

    choose_shortcut_action(&mut window, tr("move-image"), tr("move-window"));
    assert_eq!(saved_viewer_settings(&window).mouse.drag, "window");
    choose_shortcut_action(&mut window, tr("recent-and-bookmarks"), tr("no-action"));
    assert_eq!(saved_viewer_settings(&window).mouse.middle_click, "none");

    window.click(window.label_position(tr("restore-default")));
    assert_eq!(
        saved_viewer_settings(&window).mouse,
        MouseSettings::default()
    );
}

#[test]
fn phones_have_no_mouse_section() {
    let _phone = PhonePreview::enter();
    let mut window = TestWindow::sample_with_settings(&english(MouseSettings::default()));
    window.screen_size = egui::vec2(390., 844.);
    window.wait_for("1 / 5");
    let settings = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "settings")))
        .unwrap();
    window.click(settings.rect.center());
    window.draw(None);
    // Open the category list.
    window.click(window.label_position(tr("general")));
    window.draw(None);
    let label = |window: &TestWindow, label: &str| {
        window
            .paint
            .iter()
            .any(|shape| matches!(&shape.shape, Shape::Text(text) if text.galley.job.text == label))
    };
    assert!(label(&window, tr("keyboard-shortcuts")));
    assert!(label(&window, tr("touch")));
    assert!(!label(&window, tr("mouse")));
}

#[test]
fn wheel_follows_its_assigned_action_and_direction() {
    let mut window = viewer(MouseSettings {
        wheel: "zoom".into(),
        primary_wheel: "page".into(),
        reverse_wheel: true,
        ..Default::default()
    });
    let before = window.photo_rects().1;
    // Reversed: wheel down zooms in instead of turning to the next page.
    wheel(&mut window, -1., egui::Modifiers::NONE);
    assert!(window.draw(None).0.contains("1 / 5"));
    assert!(window.photo_rects().1.width() > before.width());
    // Reversed: Ctrl + wheel up turns to the next page.
    wheel(&mut window, 1., CTRL);
    assert!(window.draw(None).0.contains("2 / 5"));

    let mut window = viewer(MouseSettings {
        wheel: "none".into(),
        ..Default::default()
    });
    let before = window.photo_rects();
    wheel(&mut window, -1., egui::Modifiers::NONE);
    assert!(window.draw(None).0.contains("1 / 5"));
    assert_eq!(window.photo_rects(), before);
}

#[test]
fn drag_modes_can_be_swapped_or_disabled() {
    let mut window = TestWindow::sample_with_settings(&english(MouseSettings {
        drag: "pan".into(),
        primary_drag: "window".into(),
        ..Default::default()
    }));
    window.wait_for("");
    window.draw(Some(Key::Num1));
    window.draw(None);
    let before = window.photo_rects();
    let delta = egui::vec2(64., -30.);
    assert!(!window.drag_pointer(before.1.center(), delta));
    let panned = window.photo_rects();
    assert_eq!(panned.1, before.1.translate(delta));
    assert!(window.drag_pointer_with_modifiers(panned.1.center(), delta, CTRL));
    assert_eq!(window.photo_rects(), panned, "Window drag panned the photo");

    let mut window = TestWindow::sample_with_settings(&english(MouseSettings {
        drag: "none".into(),
        ..Default::default()
    }));
    window.wait_for("");
    let before = window.photo_rects();
    assert!(!window.drag_pointer(before.1.center(), delta));
    assert_eq!(window.photo_rects(), before);
}

#[test]
fn default_gestures_zoom_and_pan_and_ignore_the_modifier() {
    let mut window = viewer(MouseSettings::default());
    let before = window.photo_rects();
    wheel(&mut window, 1., CTRL);
    assert!(window.draw(None).0.contains("1 / 5"));
    assert_eq!(window.photo_rects(), before, "Modified wheel did something");
    wheel(&mut window, 1., egui::Modifiers::NONE);
    assert!(window.draw(None).0.contains("1 / 5"));
    assert!(window.photo_rects().1.width() > before.1.width());

    let mut window = TestWindow::sample_with_settings(&english(MouseSettings::default()));
    window.wait_for("");
    window.draw(Some(Key::Num1));
    window.draw(None);
    let before = window.photo_rects();
    let delta = egui::vec2(64., -30.);
    assert!(!window.drag_pointer_with_modifiers(before.1.center(), delta, CTRL));
    assert_eq!(window.photo_rects(), before, "Modified drag did something");
    assert!(!window.drag_pointer(before.1.center(), delta));
    assert_eq!(window.photo_rects().1, before.1.translate(delta));
}

#[test]
fn panning_from_the_background_moves_the_window_instead() {
    let mut window = TestWindow::sample_with_settings(&english(MouseSettings::default()));
    window.wait_for("");
    window.draw(Some(Key::Num1));
    window.draw(None);
    let before = window.photo_rects();
    let margin = egui::pos2(
        (before.0.left() + before.1.left()) / 2.,
        before.1.center().y,
    );
    assert!(before.0.contains(margin) && !before.1.contains(margin));
    // Crossing onto the photo keeps moving the window.
    assert!(
        window.drag_pointer(margin, before.1.center() - margin),
        "Background drag did not move the window"
    );
    assert_eq!(window.photo_rects(), before, "Background drag panned");
    // Starting on the photo still pans, even when it ends on the background.
    let delta = margin - before.1.center();
    assert!(!window.drag_pointer(before.1.center(), delta));
    let panned = window.photo_rects();
    let expected = before.1.translate(delta);
    assert!((panned.1.min - expected.min).length() < 0.01);
    assert!((panned.1.max - expected.max).length() < 0.01);

    // The background follows the photo: its old center is now empty.
    let vacated = egui::pos2(before.1.right() - 4., before.1.center().y);
    assert!(!panned.1.contains(vacated));
    assert!(window.drag_pointer(vacated, egui::vec2(20., 10.)));
    assert_eq!(window.photo_rects(), panned);

    // A full-screen window cannot move, so its background drag does nothing.
    window.draw(Some(Key::F));
    let before = window.photo_rects();
    let margin = egui::pos2(
        (before.0.right() + before.1.right()) / 2.,
        before.1.center().y,
    );
    assert!(before.0.contains(margin) && !before.1.contains(margin));
    assert!(!window.drag_pointer(margin, egui::vec2(30., 0.)));
    assert_eq!(window.photo_rects(), before);
}

#[test]
fn fitted_images_stay_centered_and_drag_the_window() {
    for scale in [1., 1.25, 2.] {
        for pinned in [false, true] {
            let mut window = TestWindow::sample_with_settings(&english(MouseSettings {
                primary_drag: "pan".into(),
                ..Default::default()
            }));
            window.scale = scale;
            window.wait_for("");
            if pinned {
                window.draw(Some(Key::H));
            }
            // Cover exact fit and a manually reduced image with margins on both axes.
            for reduced in [false, true] {
                if reduced {
                    window.draw(Some(Key::Minus));
                    window.draw(None);
                }
                let before = window.photo_rects();
                assert!(before.0.expand(0.5 / scale).contains_rect(before.1));
                assert!(before.0.center().distance(before.1.center()) <= 0.71 / scale);
                window.pointer(before.1.center().x, before.1.center().y);
                assert_ne!(window.cursor, egui::CursorIcon::Grab);
                for modifiers in [egui::Modifiers::NONE, CTRL] {
                    assert!(window.drag_pointer_with_modifiers(
                        before.1.center(),
                        egui::vec2(50., 30.),
                        modifiers,
                    ));
                    assert_eq!(window.photo_rects(), before);
                }
            }
            window.draw(Some(Key::F));
            window.draw(None);
            let before = window.photo_rects();
            assert!(!window.drag_pointer(before.1.center(), egui::vec2(50., 30.)));
            assert_eq!(window.photo_rects(), before);
        }
    }
}

#[test]
fn shrinking_a_panned_image_or_enlarging_the_window_discards_the_offset() {
    for resize in [false, true] {
        let mut window = TestWindow::sample_with_settings(&english(MouseSettings::default()));
        window.wait_for("");
        window.draw(Some(Key::Num1));
        window.draw(None);
        let original = window.photo_rects();
        let delta = egui::vec2(64., -30.);
        assert!(!window.drag_pointer(original.1.center(), delta));
        assert_eq!(window.photo_rects().1, original.1.translate(delta));
        if resize {
            window.screen_size = egui::vec2(1600., 1400.);
        } else {
            for _ in 0..3 {
                window.draw(Some(Key::Minus));
            }
        }
        window.draw(None);
        let centered = window.photo_rects();
        assert!(centered.0.expand(0.5).contains_rect(centered.1));
        assert!(centered.0.center().distance(centered.1.center()) <= 0.71);
        assert!(window.drag_pointer(centered.1.center(), delta));
        assert_eq!(window.photo_rects(), centered);
        // Returning to the original overflowing size must not restore the old pan.
        window.screen_size = egui::vec2(1080., 760.);
        if !resize {
            for _ in 0..3 {
                window.draw(Some(Key::Equals));
            }
        }
        window.draw(None);
        let restored = window.photo_rects();
        assert_eq!(restored.0, original.0);
        assert!((restored.1.min - original.1.min).length() < 0.01);
        assert!((restored.1.max - original.1.max).length() < 0.01);
    }
}

#[test]
fn small_originals_stay_centered_at_auto_and_os_scaled_actual_size() {
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("small.png");
    image::RgbImage::new(101, 79).save(&path).unwrap();
    for scale in [1., 1.25, 2.] {
        let mut window =
            TestWindow::open_with_settings(Some(path.clone()), &english(MouseSettings::default()));
        window.scale = scale;
        window.wait_for("");
        for actual in [false, true] {
            if actual {
                window.draw(Some(Key::Num1));
                window.draw(None);
            }
            let before = window.photo_rects();
            assert!(before.0.contains_rect(before.1));
            assert!(before.0.center().distance(before.1.center()) <= 0.71 / scale);
            if actual {
                assert!((before.1.width() - 101.).abs() < 0.001);
                assert!((before.1.height() - 79.).abs() < 0.001);
            }
            assert!(window.drag_pointer(before.1.center(), egui::vec2(50., 30.)));
            assert_eq!(window.photo_rects(), before);
        }
    }
}

#[test]
fn default_buttons_open_menu_and_library_and_turn_pages() {
    let mut window = viewer(MouseSettings::default());
    let center = egui::pos2(540., 380.);
    press(&mut window, center, egui::PointerButton::Extra2, 1);
    assert!(window.draw(None).0.contains("2 / 5"));
    press(&mut window, center, egui::PointerButton::Extra1, 1);
    assert!(window.draw(None).0.contains("1 / 5"));

    press(&mut window, center, egui::PointerButton::Middle, 1);
    assert!(
        library_open(&mut window),
        "Middle click did not open the library"
    );
    // The same button closes it again from over the library screen.
    press(&mut window, center, egui::PointerButton::Middle, 1);
    assert!(
        !library_open(&mut window),
        "Middle click did not close the library"
    );

    assert!(!menu_open(&window));
    press(&mut window, center, egui::PointerButton::Secondary, 1);
    assert!(menu_open(&window));
    assert!(!window.started_window_drag());
}

#[test]
fn clicks_run_any_assigned_command_or_nothing() {
    let mut window = viewer(MouseSettings {
        right_click: "fullscreen".into(),
        middle_click: "none".into(),
        double_click: "none".into(),
        back_button: "last".into(),
        ..Default::default()
    });
    let center = egui::pos2(540., 380.);
    assert!(!fullscreen(&press(
        &mut window,
        center,
        egui::PointerButton::Primary,
        2
    )));
    press(&mut window, center, egui::PointerButton::Middle, 1);
    assert!(!library_open(&mut window));
    press(&mut window, center, egui::PointerButton::Extra1, 1);
    assert!(window.draw(None).0.contains("5 / 5"));
    assert!(fullscreen(&press(
        &mut window,
        center,
        egui::PointerButton::Secondary,
        1
    )));
    assert!(!menu_open(&window));

    let mut window = viewer(MouseSettings::default());
    assert!(fullscreen(&press(
        &mut window,
        center,
        egui::PointerButton::Primary,
        2
    )));
}
