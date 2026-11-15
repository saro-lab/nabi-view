use super::*;
use crate::{
    core::{
        commands::default_shortcut_rows,
        settings::{KeyboardShortcut, Settings},
    },
    i18n::Language,
};

fn sample() -> TestWindow {
    sample_in(Language::English)
}
fn sample_in(language: Language) -> TestWindow {
    let mut shortcuts = default_shortcut_rows();
    shortcuts.push(KeyboardShortcut {
        action: "normalize-names".into(),
        key: "N".into(),
    });
    let mut window = TestWindow::sample_with_settings(&Settings {
        language,
        keyboard_shortcuts: Some(shortcuts),
        toast_ms: 5000,
        ..Default::default()
    });
    window.reveal_controls();
    window.wait_for("1 / 5");
    window
}
fn ready(window: &mut TestWindow) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = window.draw(None).0;
        if !text.contains(crate::i18n::tr("rename-checking")) {
            return;
        }
        assert!(Instant::now() < deadline, "validation timed out: {text}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn type_in(window: &mut TestWindow, id: &str, text: &str) {
    let response = window.ctx.read_response(egui::Id::unique(id)).unwrap();
    window.click(response.rect.center());
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    window.draw_events(
        [true, false]
            .map(|pressed| key_event(Key::A, pressed, false, modifiers))
            .into(),
        modifiers,
    );
    window.draw_events(vec![Event::Text(text.into())], egui::Modifiers::NONE);
}
fn preview(window: &mut TestWindow) {
    window.click(window.label_position("Apply"));
    ready(window);
}

#[test]
fn rename_files_preview_is_read_only_cumulative_and_commits_all_in_list_order() {
    let mut window = sample();
    window.draw(Some(Key::Space)); // A check must not limit normalization to this file.
    window.draw(Some(Key::ArrowRight));
    window.wait_for("2 / 5");
    let source = window._images.as_ref().unwrap().path().to_owned();
    let folder_name = source.file_name().unwrap().to_string_lossy().into_owned();
    let before: Vec<_> = (1..=5)
        .map(|i| {
            let names = [
                "01 Alpine Light.png",
                "02 Pelagic.png",
                "03 Dune Forms.png",
                "04 After Dark.png",
                "05 Evergreen.png",
            ];
            let name = names[i - 1];
            (name, std::fs::read(source.join(name)).unwrap())
        })
        .collect();
    let store = Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    );
    store
        .add_bookmark(&source.join(before[3].0), "Album")
        .unwrap();
    window.draw(Some(Key::N));
    window.wait_for("Use folder name");
    ready(&mut window);
    preview(&mut window);
    window.wait_for(&format!("{folder_name}_0001.png"));
    assert!(before.iter().all(|(name, _)| source.join(name).exists()));
    window.click(window.label_position("Replace"));
    type_in(&mut window, "rename-from", &folder_name);
    type_in(&mut window, "rename-to", "Album");
    preview(&mut window);
    window.wait_for("Album_0005.png");
    type_in(&mut window, "rename-from", "Album");
    type_in(&mut window, "rename-to", "Final");
    preview(&mut window);
    window.wait_for("Final_0005.png");
    assert!(before.iter().all(|(name, _)| source.join(name).exists()));
    // Viewer shortcuts must not operate while editing names.
    window.draw(Some(Key::Delete));
    window.draw(Some(Key::ArrowRight));
    assert!(before.iter().all(|(name, _)| source.join(name).exists()));
    window.click(window.label_position("Apply changes"));
    window.wait_for("File names changed.");
    window.reveal_controls();
    window.wait_for("Final_0002.png");
    for (i, (name, bytes)) in before.iter().enumerate() {
        assert!(!source.join(name).exists());
        assert_eq!(
            &std::fs::read(source.join(format!("Final_{:04}.png", i + 1))).unwrap(),
            bytes
        );
    }
    let bookmark = store
        .library()
        .unwrap()
        .into_iter()
        .find(|e| e.kind == crate::core::library::Kind::Bookmark)
        .unwrap();
    assert_eq!(
        bookmark.image.unwrap().file_name().unwrap(),
        "Final_0004.png"
    );
    // The old checked file remains the checked target after renaming.
    window.draw(Some(Key::M));
    window.wait_for("Images to move: 1");
    window.draw(Some(Key::Escape));
}

#[test]
fn rename_files_conflicting_preview_is_cancelled_before_commit_and_escape_discards() {
    let mut window = sample();
    let source = window._images.as_ref().unwrap().path().to_owned();
    window.draw(Some(Key::N));
    ready(&mut window);
    window.click(window.label_position("Replace"));
    type_in(&mut window, "rename-from", "01 Alpine Light");
    type_in(&mut window, "rename-to", "02 Pelagic");
    preview(&mut window);
    window.wait_for("A name conflicts with another file");
    window.wait_for("01 Alpine Light.png");
    assert_eq!(std::fs::read_dir(&source).unwrap().count(), 5);
    window.draw(Some(Key::Escape));
    assert!(!window.draw(None).0.contains("Preview only."));
    assert!(
        !window
            .viewport_commands
            .contains(&egui::ViewportCommand::Close)
    );
    window.draw(Some(Key::N));
    ready(&mut window);
    preview(&mut window);
    window.draw(Some(Key::Escape));
    assert!(source.join("01 Alpine Light.png").exists());
}

#[test]
fn rename_files_controls_and_bottom_apply_fit_small_windows() {
    let mut window = sample();
    window.draw(Some(Key::N));
    ready(&mut window);
    for size in [
        egui::vec2(320., 568.),
        egui::vec2(560., 420.),
        egui::vec2(1080., 760.),
    ] {
        window.screen_size = size;
        for _ in 0..4 {
            window.draw(None);
        }
        for label in ["Use folder name", "Replace", "Apply changes"] {
            let p = window.label_position(label);
            assert!(
                p.x >= 0. && p.x <= size.x && p.y >= 0. && p.y <= size.y,
                "{label} outside {size:?}: {p:?}"
            );
        }
        let buttons =
            ["Reset", "Cancel", "Apply changes"].map(|label| window.label_position(label));
        assert!(
            buttons
                .windows(2)
                .all(|pair| (pair[0].y - pair[1].y).abs() < 3. && pair[0].x < pair[1].x),
            "footer buttons are not on one row at {size:?}: {buttons:?}"
        );
        let lefts: Vec<_> = window
            .paint
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text)
                    if matches!(
                        text.galley.job.text.as_str(),
                        "02 Pelagic.png" | "03 Dune Forms.png" | "04 After Dark.png"
                    ) =>
                {
                    Some(text.visual_bounding_rect().left())
                }
                _ => None,
            })
            .collect();
        assert_eq!(lefts.len(), 3);
        assert!(
            lefts.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 1.),
            "names are not left-aligned: {lefts:?}"
        );
        window.click(window.label_position("Replace"));
        for _ in 0..4 {
            window.draw(None);
        }
        for id in ["rename-from", "rename-to"] {
            let rect = window.ctx.read_response(egui::Id::unique(id)).unwrap().rect;
            assert!(
                rect.left() >= 0. && rect.right() <= size.x && rect.bottom() <= size.y,
                "{rect:?} outside {size:?}"
            );
        }
        assert!(window.label_position("Apply changes").y <= size.y);
        window.click(window.label_position("Use folder name"));
    }
}

#[test]
fn rename_files_translated_buttons_fit_compact_windows() {
    for language in Language::ALL[1..].iter().cloned() {
        let mut window = sample_in(language.clone());
        window.screen_size = egui::vec2(320., 568.);
        window.draw(Some(Key::N));
        ready(&mut window);
        for replace in [false, true] {
            if replace {
                window.click(window.label_position(crate::i18n::tr("rename-replace-tab")));
            }
            for _ in 0..4 {
                window.draw(None);
            }
            let p = window.label_position(crate::i18n::tr("rename-commit"));
            assert!(
                p.x >= 0. && p.x <= 320. && p.y >= 0. && p.y <= 568.,
                "{language:?}: {p:?}"
            );
        }
    }
}

#[test]
fn rename_files_edge_hover_does_not_cover_modal_buttons() {
    let mut window = sample();
    window.draw(Some(Key::N));
    ready(&mut window);
    for size in [
        egui::vec2(520., 360.),
        egui::vec2(320., 568.),
        egui::vec2(780., 360.),
    ] {
        window.screen_size = size;
        for _ in 0..4 {
            window.draw(None);
        }
        for y in [2., size.y - 2.] {
            window.pointer(size.x / 2., y);
            for _ in 0..4 {
                window.draw(None);
            }
            for label in ["Use folder name", "Reset", "Cancel", "Apply changes"] {
                let position = window.label_position(label);
                assert_eq!(
                    window.ctx.layer_id_at(position).unwrap().id,
                    egui::Id::unique("normalize-names-window"),
                    "{label} covered at {size:?} with hover y={y}"
                );
            }
        }
        preview(&mut window);
        window.click(window.label_position("Reset"));
        ready(&mut window);
        window.wait_for("01 Alpine Light.png");
    }
    window.screen_size = egui::vec2(320., 568.);
    for _ in 0..4 {
        window.draw(None);
    }
    let back = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "normalize-names-close")))
        .unwrap();
    window.click(back.rect.center());
    assert!(!window.draw(None).0.contains("Preview only."));
    window.pointer(160., 2.);
    window.draw(None);
    assert!(window.titlebar_visible());
}
