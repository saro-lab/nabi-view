use super::*;
use crate::i18n::tr;

#[test]
fn checks_survive_a_full_cycle_and_shift_toggles_before_moving() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.time += 0.25;
    window.draw(Some(Key::Space));
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1]);
    assert_eq!(focused_thumbnail_pages(&window), [1]);

    for _ in 0..5 {
        window.draw(Some(Key::ArrowRight));
    }
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    assert_eq!(checked_sample_pages(&window), [1]);
    window.draw(Some(Key::Space));
    settle_checks(&mut window);
    assert!(checked_sample_pages(&window).is_empty());

    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [2]);
    assert_eq!(checked_sample_pages(&window), [1]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [2]);
    assert_eq!(focused_thumbnail_pages(&window), [2]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert!(checked_sample_pages(&window).is_empty());
    assert_eq!(focused_thumbnail_pages(&window), [1]);
}

#[test]
fn spread_checks_complete_a_partial_selection_and_keep_key_actions_in_rtl() {
    let mut window = TestWindow::sample();
    // These portrait fixtures must fit the half-width slots for this two-page test.
    window.screen_size.x = 1280.;
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.time += 0.25;
    window.draw(Some(Key::Space));
    window.draw(Some(Key::CloseBracket));
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    assert_eq!(checked_sample_pages(&window), [1]);

    window.draw(Some(Key::Space));
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    window.draw(Some(Key::OpenBracket));
    window.draw(Some(Key::Space));
    settle_checks(&mut window);
    assert!(checked_sample_pages(&window).is_empty());
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);
    window.draw(Some(Key::Space));
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3, 4]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert!(checked_sample_pages(&window).is_empty());
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);
}

#[test]
fn random_mode_requires_confirmation_to_clear_checks_and_blocks_new_checks() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::English,
        toast_ms: 5_000,
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.time += 0.25;
    // Keep the thumbnail badges observable while clicking the centered dialog.
    window.draw(Some(Key::H));
    window.draw(Some(Key::Space));
    window.click(window.titlebar_icon("random").rect.center());
    window.wait_for("Clear checks and enable random order?");
    assert_eq!(checked_sample_pages(&window), [1]);
    window.draw(Some(Key::Space));
    shift_navigation(&mut window, Key::ArrowRight);
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    window.click(window.label_position("Cancel"));
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1]);
    assert!(window.icon_has_stroke("random", Color32::from_rgb(159, 165, 161)));

    window.click(window.titlebar_icon("random").rect.center());
    window.wait_for("Clear checks and enable random order?");
    window.click(window.label_position("Clear checks and continue"));
    settle_checks(&mut window);
    assert!(window.draw(None).0.contains("Random order: On"));
    assert!(checked_sample_pages(&window).is_empty());
    assert!(window.icon_has_stroke("random", Color32::from_rgb(204, 232, 165)));

    window.reveal_controls();
    window.draw(Some(Key::Space));
    window.wait_for(tr("checks-are-not-available-in-random-mode"));
    settle_checks(&mut window);
    assert!(checked_sample_pages(&window).is_empty());
    let before = focused_thumbnail_pages(&window);
    shift_navigation(&mut window, Key::ArrowRight);
    window.wait_for(tr("checks-are-not-available-in-random-mode"));
    settle_checks(&mut window);
    assert_ne!(focused_thumbnail_pages(&window), before);
    assert!(checked_sample_pages(&window).is_empty());

    let mut korean = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::Korean,
        toast_ms: 5_000,
        ..Default::default()
    });
    korean.reveal_controls();
    korean.wait_for("1 / 5");
    korean.click(korean.titlebar_icon("random").rect.center());
    korean.draw(Some(Key::Space));
    korean.wait_for(crate::i18n::tr("checks-are-not-available-in-random-mode"));
}

#[test]
fn removed_files_lose_checks_and_recreated_paths_do_not_inherit_them() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.time += 0.25;
    shift_navigation(&mut window, Key::ArrowRight);
    window.draw(Some(Key::Space));
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);

    let removed = window
        ._images
        .as_ref()
        .unwrap()
        .path()
        .join("01 Alpine Light.png");
    let pixels = std::fs::read(&removed).unwrap();
    std::fs::remove_file(&removed).unwrap();
    window.wait_for("1 / 4");
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [2]);

    std::fs::write(&removed, pixels).unwrap();
    window.wait_for("2 / 5");
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [2]);
}
