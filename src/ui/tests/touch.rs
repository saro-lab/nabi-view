use super::*;
use crate::core::settings::{MouseSettings, Settings, TouchSettings};

fn viewer(touch: TouchSettings, size: egui::Vec2) -> TestWindow {
    let mut window = TestWindow::sample_with_settings(&Settings {
        language: crate::i18n::Language::English,
        touch,
        // Mobile gestures must not inherit any desktop gesture assignment.
        mouse: MouseSettings {
            double_click: "fullscreen".into(),
            right_click: "recent".into(),
            drag: "window".into(),
            ..Default::default()
        },
        ..Default::default()
    });
    window.screen_size = size;
    window.wait_for("1 / 5");
    window
}

fn touch(id: u64, phase: egui::TouchPhase, pos: egui::Pos2) -> Event {
    Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos,
        force: None,
    }
}

fn pointer_button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn long_press(window: &mut TestWindow, position: egui::Pos2) {
    window.pointer(position.x, position.y);
    window.draw_events(
        vec![
            touch(1, egui::TouchPhase::Start, position),
            pointer_button(position, true),
        ],
        egui::Modifiers::NONE,
    );
    window.time += 1.;
    window.draw(None);
    window.draw_events(
        vec![
            touch(1, egui::TouchPhase::End, position),
            pointer_button(position, false),
            Event::PointerGone,
        ],
        egui::Modifiers::NONE,
    );
    window.draw(None);
}

#[test]
fn touch_swipe_and_one_finger_pan_can_be_disabled_independently() {
    let _phone = PhonePreview::enter();
    for size in [egui::vec2(390., 844.), egui::vec2(834., 1194.)] {
        let start = egui::pos2(size.x * 0.6, size.y * 0.5);
        let mut window = viewer(
            TouchSettings {
                swipe_navigation: false,
                one_finger_pan: false,
                ..Default::default()
            },
            size,
        );
        for _ in 0..4 {
            window.draw(Some(Key::Equals));
        }
        window.draw(None);
        let before = window.photo_rects().1;
        assert!(!window.drag_pointer(start, egui::vec2(-100., 0.)));
        assert!(!window.drag_pointer(start, egui::vec2(0., 80.)));
        assert_eq!(window.photo_rects().1, before);
        assert!(window.draw(None).0.contains("1 / 5"));

        let mut window = viewer(
            TouchSettings {
                one_finger_pan: false,
                ..Default::default()
            },
            size,
        );
        assert!(!window.drag_pointer(start, egui::vec2(-100., 0.)));
        window.wait_for("2 / 5");

        let mut window = viewer(
            TouchSettings {
                swipe_navigation: false,
                ..Default::default()
            },
            size,
        );
        for _ in 0..4 {
            window.draw(Some(Key::Equals));
        }
        window.draw(None);
        let before = window.photo_rects().1;
        assert!(!window.drag_pointer(start, egui::vec2(-100., 0.)));
        assert!(window.photo_rects().1.center().x < before.center().x - 50.);
        assert!(window.draw(None).0.contains("1 / 5"));
    }
}

#[test]
fn touch_double_tap_and_long_press_use_touch_assignments() {
    let _phone = PhonePreview::enter();
    for size in [egui::vec2(390., 844.), egui::vec2(834., 1194.)] {
        let position = egui::pos2(size.x * 0.5, size.y * 0.5);
        let mut window = viewer(
            TouchSettings {
                double_tap: "next".into(),
                long_press: "prev".into(),
                ..Default::default()
            },
            size,
        );
        window.click(position);
        window.click(position);
        window.wait_for("2 / 5");
        assert!(window.titlebar_visible());
        window.time += 1.;
        long_press(&mut window, position);
        window.wait_for("1 / 5");
        assert!(!window.draw(None).0.contains("Bookmarks"));
    }
}

#[test]
fn disabled_touch_taps_preserve_zoom_and_fullscreen_exit() {
    let _phone = PhonePreview::enter();
    let mut window = viewer(
        TouchSettings {
            double_tap: "none".into(),
            long_press: "none".into(),
            ..Default::default()
        },
        egui::vec2(390., 844.),
    );
    for _ in 0..4 {
        window.draw(Some(Key::Equals));
    }
    window.draw(None);
    let before = window.photo_rects().1;
    let position = egui::pos2(195., 400.);
    window.click(position);
    window.click(position);
    assert_eq!(window.photo_rects().1, before);
    window.time += 1.;
    long_press(&mut window, position);
    assert_eq!(window.photo_rects().1, before);
    assert!(
        window
            .ctx
            .read_response(egui::Id::unique(("menu-command", "open")))
            .is_none()
    );
    window.draw(Some(Key::F));
    window.draw(None);
    assert!(!window.titlebar_visible());
    window.click(position);
    assert!(window.titlebar_visible());
}

#[test]
fn desktop_double_click_keeps_its_mouse_assignment() {
    let mut window = TestWindow::sample_with_settings(&Settings {
        language: crate::i18n::Language::English,
        mouse: MouseSettings {
            double_click: "next".into(),
            ..Default::default()
        },
        touch: TouchSettings {
            double_tap: "prev".into(),
            swipe_navigation: false,
            pinch_zoom: false,
            one_finger_pan: false,
            two_finger_pan: false,
            ..Default::default()
        },
        ..Default::default()
    });
    window.reveal_controls();
    window.draw(Some(Key::H));
    window.wait_for("1 / 5");
    window.click(egui::pos2(540., 380.));
    window.click(egui::pos2(540., 380.));
    window.wait_for("2 / 5");
}

#[test]
fn touch_pinch_and_two_finger_pan_have_independent_switches() {
    let _phone = PhonePreview::enter();
    for pinch_zoom in [false, true] {
        for two_finger_pan in [false, true] {
            let mut window = viewer(
                TouchSettings {
                    pinch_zoom,
                    two_finger_pan,
                    ..Default::default()
                },
                egui::vec2(390., 844.),
            );
            window.draw(Some(Key::F));
            window.draw(None);
            let before = window.photo_rects().1;
            let first = egui::pos2(160., 400.);
            let second = egui::pos2(230., 400.);
            window.draw_events(
                vec![
                    Event::PointerMoved(first),
                    pointer_button(first, true),
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
            if pinch_zoom {
                assert!(after.width() > before.width() * 2.);
            } else {
                assert!((after.width() - before.width()).abs() < 1.);
            }
            if two_finger_pan && pinch_zoom {
                assert!(after.center().x < before.center().x - 20.);
                assert!(after.center().y > before.center().y + 20.);
            } else {
                // A fitted image stays centered even with two-finger pan enabled.
                assert!(after.center().distance(before.center()) < 1.);
            }
            window.draw_events(
                vec![
                    touch(1, egui::TouchPhase::End, moved_first),
                    touch(2, egui::TouchPhase::End, moved_second),
                    pointer_button(moved_first, false),
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
    }
}
