use super::*;
use crate::i18n::tr;

#[test]
fn checks_survive_a_full_cycle_and_shift_selects_and_shrinks_an_inclusive_range() {
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
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [3]);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [2]);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1]);
    assert_eq!(focused_thumbnail_pages(&window), [1]);
    // Range selection stops at the boundary instead of wrapping to the last page.
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1]);
    assert_eq!(focused_thumbnail_pages(&window), [1]);
}

#[test]
fn shift_unchecks_a_range_and_restores_previous_checks_when_reversing() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    for _ in 0..4 {
        shift_navigation(&mut window, Key::ArrowRight);
    }
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3, 4, 5]);
    shift_navigation(&mut window, Key::ArrowRight);
    assert_eq!(focused_thumbnail_pages(&window), [5]);

    // An ordinary move ends the previous range. The checked starting page
    // makes this new range remove checks, including its destination.
    window.draw(Some(Key::ArrowLeft));
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [3]);
    assert_eq!(checked_sample_pages(&window), [1, 2, 5]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 5]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 5]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3, 5]);
    // Crossing the anchor keeps the initial uncheck intent.
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [5]);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3]);
}

#[test]
fn shift_range_keeps_existing_checks_outside_the_range_and_space_starts_a_new_range() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::End));
    window.draw(Some(Key::Space));
    window.draw(Some(Key::Home));
    shift_navigation(&mut window, Key::ArrowRight);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3, 5]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 5]);

    window.draw(Some(Key::Space));
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 5]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3, 5]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 5]);
}

#[test]
fn remapped_selection_commands_keep_the_range_without_a_shift_modifier() {
    use crate::core::settings::{KeyboardShortcut, Settings};

    let mut window = TestWindow::sample_with_settings(&Settings {
        keyboard_shortcuts: Some(
            [("check-prev", "J"), ("check-next", "K")]
                .into_iter()
                .map(|(action, key)| KeyboardShortcut {
                    action: action.into(),
                    key: key.into(),
                })
                .collect(),
        ),
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window.draw(Some(Key::K));
    window.draw(Some(Key::K));
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [3]);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3]);
    window.draw(Some(Key::J));
    settle_checks(&mut window);
    assert_eq!(focused_thumbnail_pages(&window), [2]);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
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
    assert_eq!(checked_sample_pages(&window), [1, 2, 3, 4]);
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2, 3, 4]);
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);

    window.draw(Some(Key::ArrowRight));
    window.draw(Some(Key::Space));
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
    assert_eq!(focused_thumbnail_pages(&window), [3, 4]);
    shift_navigation(&mut window, Key::ArrowLeft);
    settle_checks(&mut window);
    assert!(checked_sample_pages(&window).is_empty());
    assert_eq!(focused_thumbnail_pages(&window), [1, 2]);
    shift_navigation(&mut window, Key::ArrowRight);
    settle_checks(&mut window);
    assert_eq!(checked_sample_pages(&window), [1, 2]);
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
