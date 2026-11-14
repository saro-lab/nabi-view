use super::*;
use crate::{core::settings::Settings, i18n::Language};

fn sample() -> TestWindow {
    let mut window = TestWindow::sample_with_settings(&Settings {
        language: Language::English,
        toast_ms: 5_000,
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window
}

fn store(window: &TestWindow) -> Store {
    Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    )
}

#[test]
fn move_dialog_moves_checked_images_with_arrow_selection_and_enter() {
    let mut window = sample();
    let destination = tempfile::tempdir().unwrap();
    let older = tempfile::tempdir().unwrap();
    let destination_path = destination
        .path()
        .canonicalize()
        .map(crate::core::paths::simplified)
        .unwrap();
    store(&window).record_move(older.path(), 10).unwrap();
    store(&window).record_move(&destination_path, 10).unwrap();
    let source = window._images.as_ref().unwrap().path().to_owned();
    shift_navigation(&mut window, Key::ArrowRight);
    window.draw(Some(Key::ArrowRight));
    window.wait_for("3 / 5");
    window.draw(Some(Key::M));
    window.wait_for(&crate::i18n::format("move-image-count", &[("count", "2")]));
    window.draw(Some(Key::ArrowDown));
    window.draw(Some(Key::ArrowDown));
    window.draw(Some(Key::ArrowUp));
    // Viewer navigation and destructive shortcuts are blocked while entering a path.
    window.draw(Some(Key::ArrowRight));
    let modifiers = egui::Modifiers {
        command: true,
        ctrl: true,
        ..Default::default()
    };
    window.draw_events(
        [true, false]
            .map(|pressed| key_event(Key::E, pressed, false, modifiers))
            .into(),
        modifiers,
    );
    window.draw(Some(Key::Enter));
    window.wait_for("Images moved.");
    window.reveal_controls();
    window.wait_for("1 / 3");
    for name in ["01 Alpine Light.png", "02 Pelagic.png"] {
        assert!(!source.join(name).exists());
        assert!(destination.path().join(name).exists());
    }
    assert!(source.join("03 Dune Forms.png").exists());
    assert!(std::fs::read_dir(older.path()).unwrap().next().is_none());
    assert_eq!(
        store(&window).recent_moves(10).unwrap()[0],
        destination_path
    );
}

#[test]
fn move_conflict_keeps_original_and_dialog_open_and_escape_does_not_close_viewer() {
    let mut window = sample();
    let destination = tempfile::tempdir().unwrap();
    let name = "01 Alpine Light.png";
    std::fs::write(destination.path().join(name), b"existing").unwrap();
    let original = window._images.as_ref().unwrap().path().join(name);
    let bytes = std::fs::read(&original).unwrap();
    window.draw(Some(Key::M));
    window.wait_for("Recent move destinations");
    window.draw_events(
        vec![Event::Text(destination.path().display().to_string())],
        egui::Modifiers::NONE,
    );
    window.draw(Some(Key::Enter));
    window.wait_for("A file already exists at the destination");
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    assert_eq!(
        std::fs::read(destination.path().join(name)).unwrap(),
        b"existing"
    );
    assert!(store(&window).recent_moves(10).unwrap().is_empty());
    window.draw(Some(Key::Escape));
    assert!(
        !window
            .viewport_commands
            .contains(&egui::ViewportCommand::Close)
    );
    assert!(!window.draw(None).0.contains("Recent move destinations"));
}

#[test]
fn move_history_rows_delete_independently_and_clear_from_cache_settings() {
    let mut window = sample();
    let paths = (0..3)
        .map(|_| tempfile::tempdir().unwrap())
        .collect::<Vec<_>>();
    for path in &paths {
        store(&window).record_move(path.path(), 10).unwrap();
    }
    window.draw(Some(Key::M));
    window.wait_for("Recent move destinations");
    let response = window
        .ctx
        .read_response(egui::Id::unique(("move-history-remove", paths[1].path())))
        .unwrap();
    window.click(response.rect.center());
    assert_eq!(
        store(&window).recent_moves(10).unwrap(),
        [paths[2].path(), paths[0].path()]
    );
    window.draw(Some(Key::Escape));
    window.reveal_controls();
    window.click(window.titlebar_icon("settings").rect.center());
    window.wait_for("Settings");
    window.click(window.label_position("Cache"));
    window.wait_for("Maximum number of recent move destinations");
    let response = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "move-history-clear")))
        .unwrap();
    window.click(response.rect.center());
    assert!(store(&window).recent_moves(10).unwrap().is_empty());
    assert!(paths.iter().all(|path| path.path().is_dir()));
}

#[test]
fn move_and_delete_icons_precede_zoom_without_overlapping_controls() {
    let mut window = sample();
    for width in [1080., 560.] {
        window.screen_size = egui::vec2(width, 760.);
        window.reveal_controls();
        let moving = window.titlebar_icon("move").rect;
        let delete = window.titlebar_icon("trash").rect;
        let metadata = window.titlebar_icon("metadata").rect;
        let zoom = zoom_widget(&window, "zoom-value").rect;
        assert!(moving.right() <= delete.left());
        assert!(delete.right() < zoom.left());
        assert!(
            metadata.right() <= moving.left(),
            "overlap at {width}: {metadata:?} / {moving:?}"
        );
        assert!(zoom.right() < width);
        window.click(moving.center());
        window.wait_for("Recent move destinations");
        let input = window
            .ctx
            .read_response(egui::Id::unique("move-path"))
            .unwrap();
        assert!(input.rect.right() <= width);
        window.draw(Some(Key::Escape));
    }
}

#[test]
fn move_dialog_controls_align_and_fit_at_each_window_width() {
    for language in [
        Language::English,
        Language::Korean,
        Language::German,
        Language::Arabic,
    ] {
        let mut window = TestWindow::sample_with_settings(&Settings {
            language: language.clone(),
            ..Default::default()
        });
        window.reveal_controls();
        window.wait_for("1 / 5");
        let destination = tempfile::tempdir().unwrap();
        store(&window).record_move(destination.path(), 10).unwrap();
        window.draw(Some(Key::M));
        for width in [320., 390., 560., 1080.] {
            window.screen_size = egui::vec2(width, 568.);
            for _ in 0..4 {
                window.draw(None);
            }
            let input = window
                .ctx
                .read_response(egui::Id::unique("move-path"))
                .unwrap()
                .rect;
            let browse = window
                .ctx
                .read_response(egui::Id::unique(("titlebar", "move-browse")))
                .unwrap()
                .rect;
            let clear = window
                .ctx
                .read_response(egui::Id::unique(("titlebar", "move-history-clear")))
                .unwrap()
                .rect;
            let remove = window
                .ctx
                .read_response(egui::Id::unique((
                    "move-history-remove",
                    destination.path(),
                )))
                .unwrap()
                .rect;
            // The text button's actual painted background, including its padding.
            let submit = window
                .paint
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Rect(r)
                        if r.rect.left() >= input.right()
                            && (r.rect.center().y - input.center().y).abs() < 1.
                            && r.rect.height() < 60. =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .next()
                .expect("Missing move button background");
            for rect in [input, browse, submit, clear, remove] {
                assert!(
                    (rect.height() - 36.).abs() < 1.,
                    "{} / {width}: {rect:?}",
                    language.code()
                );
                assert!(
                    window.ctx.viewport_rect().contains_rect(rect),
                    "{} / {width}: {rect:?}",
                    language.code()
                );
            }
            assert!((input.top() - browse.top()).abs() < 1.);
            assert!((input.top() - submit.top()).abs() < 1.);
            assert!((input.left() - browse.right() - 8.).abs() < 1.);
            assert!((submit.left() - input.right() - 8.).abs() < 1.);
            assert!(remove.top() >= clear.bottom() + 7.);
            assert!(
                (clear.center().x - remove.center().x).abs() < 1.,
                "History actions are misaligned: {clear:?}, {remove:?}"
            );
        }
    }
}
