#[test]
fn shortcut_editing_and_search_block_repeated_quit_and_native_close() {
    use eframe::App;
    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    for editing in [false, true] {
        let directory = tempfile::tempdir_in(&root).unwrap();
        let ctx = egui::Context::default();
        let store = Store::in_directories(
            directory.path().join("config"),
            directory.path().join("state"),
        );
        let signal = store.close_signal_path();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Ok(store),
        );
        app.library.open = false;
        app.settings_open = true;
        if editing {
            app.shortcut_editor = Some(settings::ShortcutEditor {
                index: None,
                action: "open".into(),
                key: String::new(),
                listening: true,
                error: String::new(),
            });
        } else {
            app.shortcut_search = Some(String::new());
        }
        let modifiers = egui::Modifiers {
            command: true,
            mac_cmd: cfg!(target_os = "macos"),
            ctrl: !cfg!(target_os = "macos"),
            ..Default::default()
        };
        // egui 0.37 reports modifier keys too: they must not finish key capture.
        let _ = ctx.run_pass(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: if cfg!(target_os = "macos") {
                        Key::SuperLeft
                    } else {
                        Key::ControlLeft
                    },
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| app.keyboard(ctx),
        );
        if editing {
            assert!(app.shortcut_editor.as_ref().unwrap().listening);
            assert!(app.shortcut_editor.as_ref().unwrap().key.is_empty());
        } else {
            assert!(app.shortcut_search.as_ref().unwrap().is_empty());
        }
        for _ in 0..3 {
            let input = egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::Key {
                        key: Key::Q,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                ],
                ..Default::default()
            };
            let output = ctx.run_pass(input, |ctx| app.keyboard(ctx));
            assert!(
                !output
                    .viewport_output
                    .values()
                    .any(|v| v.commands.contains(&egui::ViewportCommand::Close))
            );
            assert!(!signal.exists(), "capture broadcast a quit request");
        }
        assert_eq!(
            if editing {
                &app.shortcut_editor.as_ref().unwrap().key
            } else {
                app.shortcut_search.as_ref().unwrap()
            },
            "Primary+Q"
        );
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let output = ctx.run_ui(input.clone(), |ui| {
            let mut frame = eframe::Frame::_new_kittest();
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        assert!(
            output
                .viewport_output
                .values()
                .any(|v| v.commands.contains(&egui::ViewportCommand::CancelClose))
        );
        // A minimized window only receives logic, but capture must still guard it.
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .minimized = Some(true);
        let output = ctx.run_logic(&input, |ctx| {
            app.logic(ctx, &mut eframe::Frame::_new_kittest());
        });
        assert!(
            output
                .viewport_commands
                .values()
                .any(|commands| { commands.contains(&egui::ViewportCommand::CancelClose) })
        );
        // An intentional all-window request from another viewer still wins.
        app.closing_all = true;
        let output = ctx.run_ui(input, |ui| {
            let mut frame = eframe::Frame::_new_kittest();
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        assert!(
            !output
                .viewport_output
                .values()
                .any(|v| v.commands.contains(&egui::ViewportCommand::CancelClose))
        );
    }
}

#[test]
fn hidden_window_processes_close_all_without_ui() {
    use eframe::App;
    use std::time::{Duration, Instant};

    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    let directory = tempfile::tempdir_in(root).unwrap();
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    let path = directory.path().join("close-all");
    app.close_signal = Some(crate::platform::close_signal::CloseSignal::new(
        path.clone(),
        ctx.clone(),
    ));
    let mut sender = crate::platform::close_signal::CloseSignal::new(path, ctx.clone());
    let mut input = egui::RawInput {
        focused: false,
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .minimized = Some(true);
    sender.broadcast().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = ctx.run_logic(&input, |ctx| {
            app.logic(ctx, &mut eframe::Frame::_new_kittest());
        });
        if output
            .viewport_commands
            .values()
            .any(|commands| commands.contains(&egui::ViewportCommand::Close))
        {
            break;
        }
        assert!(Instant::now() < deadline, "hidden window ignored close-all");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.closing_all);
}

#[test]
fn mobile_safe_area_is_applied_once_to_viewport_content_and_popups() {
    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(390., 844.));
    let safe = Rect::from_min_max(egui::pos2(0., 59.), egui::pos2(390., 810.));
    let native = egui::SafeAreaInsets(egui::epaint::MarginF32 {
        top: 59.,
        bottom: 34.,
        ..Default::default()
    });
    let pointer = egui::pos2(120., 200.);
    // Native integrations only send the inset again on a window-state change.
    for native_insets in [Some(native), None] {
        let mut input = egui::RawInput {
            screen_rect: Some(screen),
            safe_area_insets: native_insets,
            events: vec![egui::Event::PointerMoved(pointer)],
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(3.);
        prepare_mobile_safe_area(&ctx, &mut input, [177., 0., 102., 0.]);
        assert_eq!(input.screen_rect, Some(safe));
        assert_eq!(input.safe_area_insets, Some(Default::default()));
        let _ = ctx.run_ui(input, |ui| {
            assert_eq!(ui.ctx().viewport_rect(), safe);
            assert_eq!(ui.ctx().content_rect(), safe);
            assert_eq!(ui.max_rect(), safe);
            assert_eq!(ui.ctx().pointer_latest_pos(), Some(pointer));
            let clip = egui::Area::new(egui::Id::unique("safe-area-popup"))
                .fixed_pos(safe.min)
                .show(ui.ctx(), |popup| popup.clip_rect())
                .inner;
            assert_eq!(clip, safe);
        });
    }
    // None must also be cleared when the integration omits the screen rectangle.
    let mut input = egui::RawInput::default();
    prepare_mobile_safe_area(&ctx, &mut input, [177., 0., 102., 0.]);
    assert_eq!(input.safe_area_insets, Some(Default::default()));
}

#[test]
fn open_path_keeps_missing_paths_and_distinct_files() {
    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    let folder = tempfile::tempdir_in(root).unwrap();
    let missing = folder.path().join("missing.png");
    assert_eq!(normalize_open_path(missing.clone()), missing);
    let lower = folder.path().join("image.png");
    let upper = folder.path().join("IMAGE.PNG");
    std::fs::write(&lower, b"lower").unwrap();
    if !upper.exists() {
        std::fs::write(&upper, b"upper").unwrap();
        assert_ne!(normalize_open_path(lower), normalize_open_path(upper));
    }
}

#[cfg(unix)]
#[test]
fn open_path_preserves_symlink_entry_and_its_folder() {
    let root = crate::test_support::root();
    std::fs::create_dir_all(&root).unwrap();
    let folder = tempfile::tempdir_in(root).unwrap();
    let target_dir = folder.path().join("target");
    std::fs::create_dir(&target_dir).unwrap();
    let target = target_dir.join("image.png");
    std::fs::write(&target, b"original").unwrap();
    let link = folder.path().join("selected.png");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        normalize_open_path(link),
        folder.path().canonicalize().unwrap().join("selected.png")
    );
    assert_eq!(std::fs::read(target).unwrap(), b"original");
}

#[test]
fn adaptive_spreads_follow_monitor_aspect_and_never_skip_pages() {
    assert!(width_limited([2400, 1000], egui::vec2(1920., 1080.)));
    assert!(width_limited([1600, 1200], egui::vec2(1920., 1080.)));
    assert!(!width_limited([800, 1200], egui::vec2(1920., 1080.)));
    assert!(width_limited([1600, 1200], egui::vec2(1080., 1920.)));
    assert!(!width_limited([0, 100], egui::vec2(1920., 1080.)));
    // All possible standalone combinations in a seven-page folder.
    for mask in 0u32..128 {
        for cover in [false, true] {
            let span = |i| adaptive_focus_span(i, 7, true, cover, |j| mask & (1 << j) != 0);
            let mut i = 0;
            let mut visited = Vec::new();
            let mut groups = Vec::new();
            loop {
                let (a, b) = span(i);
                groups.push((a, b));
                visited.extend(a..=b);
                for member in a..=b {
                    assert_eq!(span(member), (a, b));
                }
                if b == 6 {
                    break;
                }
                i = b + 1;
            }
            assert_eq!(visited, (0..7).collect::<Vec<_>>());
            for pair in groups.windows(2) {
                assert_eq!(span(pair[1].0 - 1), pair[0]);
            }
        }
    }
}

#[test]
fn random_cycles_visit_every_file_and_reconcile_changes() {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    app.files = (0..9).map(|i| PathBuf::from(format!("{i}.png"))).collect();
    let expected: HashSet<_> = app.files.iter().cloned().collect();
    app.run("random", &ctx);
    for _ in 0..3 {
        let mut seen = HashSet::new();
        for _ in 0..9 {
            assert!(seen.insert(app.files[app.focus_index].clone()));
            app.move_page(&ctx, true);
        }
        assert_eq!(seen, expected);
    }
    let current = app.files[app.focus_index].clone();
    let mut live = app.files.clone();
    live.remove(4);
    live.push("added.png".into());
    app.refresh_folder(live.clone(), &[]);
    assert_eq!(app.files[app.focus_index], current);
    assert_eq!(
        app.files.iter().collect::<HashSet<_>>(),
        live.iter().collect()
    );
    assert_eq!(app.files.last().unwrap(), &PathBuf::from("added.png"));
    app.run("random", &ctx);
    assert!(
        app.files
            .windows(2)
            .all(|p| media::compare_file_names(&p[0], &p[1]).is_le())
    );
    assert_eq!(app.files[app.focus_index], current);
}

#[test]
fn slideshow_interval_shortcuts_repeat_with_configured_timing_and_stop() {
    for (key, action, step) in [
        (Key::Comma, "slideshow-decrease", -100),
        (Key::Period, "slideshow-increase", 100),
        (Key::K, "slideshow-increase", 100),
    ] {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Err(String::new()),
        );
        app.settings.key_delay_ms = 400;
        app.library.open = false;
        app.settings.key_repeat_ms = 200;
        if key == Key::K {
            app.save_shortcut_rows(vec![KeyboardShortcut {
                action: action.into(),
                key: "K".into(),
            }])
            .unwrap();
        }
        let event = |pressed, repeat| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers: egui::Modifiers::NONE,
        };
        let tick = |app: &mut Viewer, time, events, focused| {
            let _ = ctx.run_pass(
                egui::RawInput {
                    time: Some(time),
                    events,
                    focused,
                    ..Default::default()
                },
                |ctx| app.keyboard(ctx),
            );
        };
        let expected = |count| (1000 + step * count) as u32;
        tick(&mut app, 0., vec![event(true, false)], true);
        assert_eq!(app.settings.slide_ms, expected(1));
        tick(&mut app, 0.2, vec![event(true, true)], true);
        tick(&mut app, 0.39, vec![], true);
        assert_eq!(
            app.settings.slide_ms,
            expected(1),
            "OS repeat bypassed hold delay"
        );
        tick(&mut app, 0.41, vec![], true);
        assert_eq!(app.settings.slide_ms, expected(2));
        tick(&mut app, 0.60, vec![], true);
        assert_eq!(
            app.settings.slide_ms,
            expected(2),
            "Repeat interval was ignored"
        );
        tick(&mut app, 0.62, vec![], true);
        assert_eq!(app.settings.slide_ms, expected(3));
        tick(&mut app, 0.63, vec![event(false, false)], true);
        tick(&mut app, 1., vec![], true);
        assert_eq!(app.settings.slide_ms, expected(3));

        tick(&mut app, 1.1, vec![event(true, false)], true);
        tick(&mut app, 1.6, vec![], false);
        tick(&mut app, 2., vec![], true);
        assert_eq!(
            app.settings.slide_ms,
            expected(4),
            "Focus loss must cancel the hold"
        );
        tick(&mut app, 2.01, vec![event(false, false)], true);

        // Opening a setting/menu/text input must cancel a held shortcut.
        for block in 0..3 {
            let time = 3. + block as f64 * 2.;
            tick(&mut app, time, vec![event(true, false)], true);
            let before = app.settings.slide_ms;
            app.settings_open = block == 0;
            app.menu_open = block == 1;
            app.zoom_input = (block == 2).then(|| "100".into());
            tick(&mut app, time + 0.5, vec![], true);
            assert!(app.held_key.is_none());
            app.settings_open = false;
            app.menu_open = false;
            app.zoom_input = None;
            tick(&mut app, time + 1., vec![], true);
            assert_eq!(app.settings.slide_ms, before);
            tick(&mut app, time + 1.1, vec![event(false, false)], true);
        }
    }
}

#[test]
fn setting_toasts_survive_automatic_slides_without_extending_their_lifetime() {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    app.files = vec!["a.png".into(), "b.png".into(), "c.png".into()];
    app.library.open = false;
    app.settings.slide_ms = 100;
    app.run("slideshow", &ctx);
    app.show_slide_interval_toast(&ctx);
    let notice = app.toast.as_ref().unwrap().message.clone();
    let expires = app.toast.as_ref().unwrap().deadline;
    let tick = |app: &mut Viewer, time| {
        let _ = ctx.run_pass(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                app.advance_slideshow(ctx);
                app.toast_overlay(ctx);
            },
        );
    };
    tick(&mut app, 0.);
    for (time, page) in [(0.11, 1), (0.22, 2), (0.33, 0)] {
        tick(&mut app, time);
        assert_eq!(app.focus_index, page);
        let toast = app.toast.as_ref().unwrap();
        assert_eq!(
            toast.message, notice,
            "Automatic boundary notices must not replace setting feedback"
        );
        assert_eq!(toast.deadline, expires);
    }
    app.toast.as_mut().unwrap().deadline =
        std::time::Instant::now() - std::time::Duration::from_secs(1);
    tick(&mut app, 0.34);
    assert!(app.toast.is_none());
    app.show_slide_interval_toast(&ctx);
    app.run("next", &ctx);
    assert!(
        app.toast.is_none(),
        "Manual page navigation still dismisses the notice"
    );
    app.run("in", &ctx);
    assert!(!app.toast.as_ref().unwrap().keep_on_slideshow);
    tick(&mut app, 0.35);
    tick(&mut app, 0.46);
    assert!(
        !app.toast
            .as_ref()
            .is_some_and(|t| t.message.starts_with(tr("zoom")))
    );

    app.toast = None;
    app.checked.insert(app.files[app.focus_index].clone());
    app.run("random", &ctx);
    assert!(app.random_prompt && !app.random);
    assert!(
        app.toast.is_none(),
        "An unconfirmed setting must not claim success"
    );
}

#[test]
fn slideshow_interval_shortcuts_persist_clamp_and_reschedule_playback() {
    let directory = tempfile::tempdir().unwrap();
    let store = || {
        Store::in_directories(
            directory.path().join("config"),
            directory.path().join("state"),
        )
    };
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Ok(store()),
    );
    app.files = vec!["a.png".into(), "b.png".into(), "c.png".into()];
    app.library.open = false;
    app.run("slideshow", &ctx);
    let tick = |app: &mut Viewer, time, action| {
        let _ = ctx.run_pass(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                if let Some(action) = action {
                    app.run(action, ctx);
                }
                app.advance_slideshow(ctx);
            },
        );
    };
    tick(&mut app, 0., None);
    tick(&mut app, 0.2, Some("slideshow-increase"));
    assert_eq!(store().load().unwrap().slide_ms, 1100);
    assert!(app.toast.as_ref().unwrap().message.contains("1.1"));
    tick(&mut app, 1.01, None);
    assert_eq!(app.focus_index, 0, "Old playback deadline must be replaced");
    tick(&mut app, 1.31, None);
    assert_eq!(app.focus_index, 1);
    app.settings.slide_ms = 100;
    let deadline = app.slide_deadline;
    app.run("slideshow-decrease", &ctx);
    assert_eq!(app.settings.slide_ms, 100);
    assert_eq!(
        app.slide_deadline, deadline,
        "Clamped changes must not postpone playback"
    );
    app.run("slideshow-increase", &ctx);
    app.run("slideshow-decrease", &ctx);
    assert_eq!(store().load().unwrap().slide_ms, 100);
    app.settings.slide_ms = 99_900;
    app.run("slideshow-increase", &ctx);
    app.run("slideshow-increase", &ctx);
    assert_eq!(app.settings.slide_ms, 100_000);
    assert_eq!(store().load().unwrap().slide_ms, 100_000);
    assert!(app.slideshow);
    app.run("slideshow", &ctx);
    app.run("slideshow-decrease", &ctx);
    assert!(
        !app.slideshow,
        "Changing the interval must not start playback"
    );
    assert_eq!(store().load().unwrap().slide_ms, 99_900);
    assert_eq!(shortcut_label("Comma"), ",");
    assert_eq!(shortcut_label("Period"), ".");
}

#[test]
fn slideshow_obeys_interval_pause_and_stop() {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    app.files = vec!["a.png".into(), "b.png".into(), "c.png".into()];
    app.library.open = false;
    app.run("slideshow", &ctx);
    let tick = |app: &mut Viewer, time| {
        let _ = ctx.run_pass(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ctx| app.advance_slideshow(ctx),
        );
    };
    tick(&mut app, 0.);
    tick(&mut app, 0.9);
    assert_eq!(app.focus_index, 0);
    tick(&mut app, 1.01);
    assert_eq!(app.focus_index, 1);
    app.settings_open = true;
    tick(&mut app, 3.);
    assert_eq!(app.focus_index, 2);
    app.settings_open = false;
    tick(&mut app, 3.1);
    tick(&mut app, 3.2);
    assert_eq!(app.focus_index, 2);
    app.run("slideshow", &ctx);
    tick(&mut app, 8.);
    assert_eq!(app.focus_index, 2);
}
use super::*;

#[test]
fn pending_navigation_keeps_the_last_painted_texture_and_geometry() {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    app.files = vec!["1.png".into(), "2.png".into(), "3.png".into()];
    let texture = |name: &str| {
        ctx.load_texture(
            name,
            egui::ColorImage::filled([16, 24], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        )
    };
    let first = texture("first");
    app.pictures.insert(
        app.files[0].clone(),
        Picture {
            hdr: None,
            animation: None,
            handle: first.clone(),
            original: [16, 24],
            requested: 24,
            last_used: 0,
        },
    );
    app.zoom = 2.;
    app.pan = egui::vec2(20., 30.);
    let draw = |app: &mut Viewer| {
        let output = ctx.run_pass(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1080., 760.),
                )),
                ..Default::default()
            },
            |ctx| ctx.root_ui(|ui| app.canvas(ui)),
        );
        output
            .shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Mesh(m)
                    if m.texture_id != egui::TextureId::default()
                        && !crate::ui::transparency::is_backdrop(&ctx, m.texture_id) =>
                {
                    Some((m.texture_id, m.calc_bounds()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let original = draw(&mut app);
    assert_eq!(original.len(), 1);
    app.focus(1);
    // Simulate eviction and a slow decoder: only the presented handle owns the image.
    app.pictures.clear();
    for _ in 0..6 {
        assert_eq!(draw(&mut app), original);
    }
    app.hide_controls = false;
    let pinned = draw(&mut app);
    assert_eq!(pinned[0].0, original[0].0);
    assert!(
        (pinned[0].1.aspect_ratio() - original[0].1.aspect_ratio()).abs() < 0.001,
        "Pinning chrome distorted the retained image"
    );
    assert!(pinned[0].1.height() < original[0].1.height());
    app.hide_controls = true;
    assert_eq!(draw(&mut app), original);
    app.focus(2);
    let stale = texture("superseded");
    app.pictures.insert(
        app.files[1].clone(),
        Picture {
            hdr: None,
            animation: None,
            handle: stale,
            original: [16, 24],
            requested: 24,
            last_used: 0,
        },
    );
    assert_eq!(
        draw(&mut app),
        original,
        "A superseded target must not flash on screen"
    );
    let current = texture("current");
    app.pictures.insert(
        app.files[2].clone(),
        Picture {
            hdr: None,
            animation: None,
            handle: current.clone(),
            original: [16, 24],
            requested: 24,
            last_used: 0,
        },
    );
    let replacement = draw(&mut app);
    assert_eq!(replacement.len(), 1);
    assert_eq!(replacement[0].0, current.id());
    assert!(!app.transition_waiting);
    app.settings.spread = true;
    app.focus(0);
    assert_eq!(
        draw(&mut app),
        replacement,
        "Keep the complete previous view while one spread page is missing"
    );
    app.pictures.insert(
        app.files[0].clone(),
        Picture {
            hdr: None,
            animation: None,
            handle: first,
            original: [16, 24],
            requested: 24,
            last_used: 0,
        },
    );
    assert_eq!(draw(&mut app).len(), 2, "Swap a prepared spread together");
    app.focus(2);
    app.pictures.remove(&app.files[2]);
    app.failures
        .insert(app.files[2].clone(), "Unreadable image".into());
    assert!(
        draw(&mut app).is_empty(),
        "A terminal error must not pretend the previous picture is the target"
    );
    assert!(!app.transition_waiting);
    app.refresh_folder(Vec::new(), &[]);
    assert!(draw(&mut app).is_empty());
    assert!(app.presentation.images.is_empty());
}

#[test]
fn navigation_wraps_each_layout_in_both_directions() {
    for (spread, cover, expected) in [
        (false, false, vec![0, 1, 2, 3, 4, 0]),
        (true, false, vec![0, 2, 4, 0]),
        (true, true, vec![0, 1, 3, 0]),
    ] {
        let mut index = 0;
        for next in expected.into_iter().skip(1) {
            index = next_page(index, 5, spread, cover, true);
            assert_eq!(index, next);
        }
        assert_eq!(next_page(0, 5, spread, cover, false), 4);
        assert_eq!(next_page(0, 1, spread, cover, true), 0);
        assert_eq!(next_page(0, 0, spread, cover, false), 0);
    }
}

#[test]
fn refreshed_focus_preserves_anchor_or_uses_natural_successor() {
    let paths = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
    let old = paths(&["1.png", "2.png", "10.png"]);
    assert_eq!(
        refreshed_focus_index(
            &old,
            1,
            &paths(&["0.png", "1.png", "2.png", "3.png", "10.png"])
        ),
        2
    );
    assert_eq!(
        refreshed_focus_index(&old, 1, &paths(&["1.png", "3.png", "10.png"])),
        1
    );
    assert_eq!(
        refreshed_focus_index(&old, 2, &paths(&["0.png", "1.png"])),
        0
    );
    assert_eq!(refreshed_focus_index(&old, 1, &[]), 0);
}

#[test]
fn imported_control_chords_execute_and_preserve_the_saved_spelling() {
    for saved in ["Ctrl+R", "Primary+Ctrl+R"] {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Err(String::new()),
        );
        app.settings.keyboard_shortcuts = Some(vec![KeyboardShortcut {
            action: "flip-horizontal".into(),
            key: saved.into(),
        }]);
        app.library.open = false;
        assert!(validate_keys(&app.settings).is_ok());
        let command = !cfg!(target_os = "macos") || saved.starts_with("Primary+");
        let modifiers = egui::Modifiers {
            ctrl: true,
            command,
            mac_cmd: cfg!(target_os = "macos") && command,
            ..Default::default()
        };
        let _ = ctx.run_pass(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::Key {
                        key: Key::R,
                        physical_key: Some(Key::R),
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                ],
                ..Default::default()
            },
            |ctx| app.keyboard(ctx),
        );
        assert!(
            app.settings.flip_horizontal,
            "Saved {saved} must match its physical modifier"
        );
        assert_eq!(
            app.settings.keyboard_shortcuts.as_ref().unwrap()[0].key,
            saved
        );
        let json = serde_json::to_value(&app.settings).unwrap();
        assert_eq!(json["keyboard_shortcuts"][0]["key"], saved);
    }
}

#[test]
fn control_alias_collisions_follow_the_target_platform() {
    let mut settings = Settings {
        keyboard_shortcuts: Some(vec![
            KeyboardShortcut {
                action: "open".into(),
                key: "Ctrl+O".into(),
            },
            KeyboardShortcut {
                action: "next".into(),
                key: "Primary+O".into(),
            },
        ]),
        ..Default::default()
    };
    assert!(validate_keys_for_platform(&settings, false).is_err());
    assert!(validate_keys_for_platform(&settings, true).is_ok());
    settings.keyboard_shortcuts.as_mut().unwrap()[1].key = "Primary+Ctrl+O".into();
    assert!(validate_keys_for_platform(&settings, false).is_err());
    assert!(validate_keys_for_platform(&settings, true).is_ok());
    assert_eq!(
        canonical_chord_for_platform("Ctrl+Alt+Shift+O", false),
        "Primary+Alt+Shift+O"
    );
    assert_eq!(
        canonical_chord_for_platform("Ctrl+Alt+Shift+O", true),
        "Ctrl+Alt+Shift+O"
    );
}

#[test]
fn modifier_labels_use_native_symbols_and_order() {
    for (saved, windows, macos) in [
        ("Primary+O", "Ctrl+O", "⌘O"),
        ("Ctrl+O", "Ctrl+O", "⌃O"),
        ("Primary+Ctrl+Alt+Shift+O", "Ctrl+Alt+Shift+O", "⌃⌥⇧⌘O"),
        ("Primary+Alt+Shift+OpenBracket", "Ctrl+Alt+Shift+[", "⌥⇧⌘["),
        ("Shift+CloseBracket", "Shift+]", "⇧]"),
        ("Alt+Enter", "Alt+Enter", "⌥Enter"),
        ("Escape", "Esc", "Esc"),
        ("Primary+Comma", "Ctrl+,", "⌘,"),
        ("Shift+Left", "Shift+←", "⇧←"),
        ("Right", "→", "→"),
        ("Equals", "=", "="),
        ("Minus", "−", "−"),
    ] {
        assert_eq!(shortcut_label_for_platform(saved, false), windows);
        assert_eq!(shortcut_label_for_platform(saved, true), macos);
    }
}

#[test]
fn legacy_mapping_is_listed_without_adding_default_keys() {
    let mut s = Settings {
        keyboard_shortcuts: None,
        ..Default::default()
    };
    s.shortcuts.insert("new-window".into(), "D".into());
    s.normalize();
    assert!(validate_keys(&s).is_ok());
    assert_eq!(effective_keys(&s, "new-window"), vec!["D"]);
    assert!(effective_keys(&s, "next").is_empty());
    assert!(effective_keys(&s, "metadata").is_empty());
    assert!(s.shortcuts.is_empty());
    s.keyboard_shortcuts = Some(Vec::new());
    s.shortcuts.insert("new-window".into(), "D".into());
    s.normalize();
    assert!(effective_keys(&s, "new-window").is_empty());
}
#[test]
fn direct_open_keeps_middle_page_in_each_layout() {
    let focus_index = 3; // Page 4 of 5 opened through the file manager.
    assert_eq!(page_focus_span(focus_index, 5, false, false), (3, 3));
    assert_eq!(page_focus_span(focus_index, 5, true, false), (2, 3));
    assert_eq!(page_focus_span(focus_index, 5, true, true), (3, 4));
    assert_eq!(page_focus_span(0, 5, true, true), (0, 0));
    assert_eq!(page_focus_span(4, 5, true, false), (4, 4));
}

#[test]
fn listed_keys_are_fixed_and_duplicates_are_rejected() {
    let mut s = Settings::default();
    assert!(validate_keys(&s).is_ok());
    assert_eq!(effective_keys(&s, "next"), vec!["Right", "D"]);
    s.rtl = true;
    s.spread = true;
    assert_eq!(effective_keys(&s, "next"), vec!["Right", "D"]);
    s.keyboard_shortcuts = Some(vec![KeyboardShortcut {
        action: "next".into(),
        key: "J".into(),
    }]);
    assert!(validate_keys(&s).is_ok());
    assert_eq!(effective_keys(&s, "next"), vec!["J"]);
    assert!(effective_keys(&s, "prev").is_empty());
    s.keyboard_shortcuts
        .as_mut()
        .unwrap()
        .push(KeyboardShortcut {
            action: "prev".into(),
            key: "J".into(),
        });
    assert!(validate_keys(&s).is_err());
    s.keyboard_shortcuts = Some(vec![KeyboardShortcut {
        action: "fit".into(),
        key: "Escape".into(),
    }]);
    assert!(validate_keys(&s).is_err());
    s.keyboard_shortcuts.as_mut().unwrap()[0].action = "close-window".into();
    assert!(validate_keys(&s).is_ok());
}
#[test]
fn shortcut_rows_are_independent_and_reset_only_changes_shortcuts() {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx),
        None,
        Err(String::new()),
    );
    app.settings.flip_horizontal = true;
    app.settings.rtl = true;
    app.settings.spread = true;
    app.settings.shortcuts.insert("next".into(), "J".into());
    let rows = vec![
        KeyboardShortcut {
            action: "next".into(),
            key: "J".into(),
        },
        KeyboardShortcut {
            action: "next".into(),
            key: "K".into(),
        },
        KeyboardShortcut {
            action: "close-window".into(),
            key: "Escape".into(),
        },
    ];
    app.save_shortcut_rows(rows.clone()).unwrap();
    assert!(app.settings.shortcuts.is_empty());
    assert_eq!(app.bindings("next"), vec!["J", "K"]);
    assert!(app.bindings("prev").is_empty());
    app.settings.rtl = false;
    assert_eq!(app.bindings("next"), vec!["J", "K"]);
    let mut duplicate = rows.clone();
    duplicate.push(KeyboardShortcut {
        action: "prev".into(),
        key: "K".into(),
    });
    assert!(app.save_shortcut_rows(duplicate).is_err());
    assert_eq!(
        app.shortcut_rows(),
        rows,
        "Rejected edits must keep the saved list"
    );
    let mut duplicate = rows.clone();
    duplicate.push(rows[0].clone());
    assert!(app.save_shortcut_rows(duplicate).is_err());
    app.save_shortcut_rows(Vec::new()).unwrap();
    assert!(app.shortcut_rows().is_empty());
    assert!(app.bindings("close-window").is_empty());
    app.reset_shortcuts().unwrap();
    assert_eq!(
        app.settings.keyboard_shortcuts,
        Some(default_shortcut_rows())
    );
    assert!(app.settings.flip_horizontal && app.settings.spread);
    assert_eq!(app.bindings("next"), vec!["Right", "D"]);
    assert!(app.bindings("close-window").contains(&"Escape".to_owned()));
    app.settings.rtl = true;
    assert_eq!(app.bindings("next"), vec!["Right", "D"]);
}

fn bare_viewer() -> (Viewer, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    app.last_error.clear();
    (app, ctx)
}

#[test]
fn phone_back_closes_the_topmost_layer_before_leaving_the_app() {
    let (mut app, ctx) = bare_viewer();
    // Without images the library is the start screen: back leaves the app.
    assert!(app.library.open);
    assert!(!app.handle_back(&ctx));
    assert!(app.library.open);

    app.files = vec!["1.png".into(), "2.png".into()];
    app.fullscreen = true;
    app.inspector.open = true;
    app.settings_open = true;
    app.menu_open = true;
    app.random_prompt = true;
    app.library.clear_prompt = true;
    app.shortcut_reset_prompt = true;
    app.held_key = Some((Key::ArrowRight, "next", 0.));

    assert!(app.handle_back(&ctx));
    assert!(!app.shortcut_reset_prompt && app.random_prompt);
    assert!(app.held_key.is_none());
    assert!(app.handle_back(&ctx));
    assert!(!app.random_prompt && app.library.clear_prompt);
    assert!(app.handle_back(&ctx));
    assert!(!app.library.clear_prompt && app.menu_open);
    assert!(app.handle_back(&ctx));
    assert!(!app.menu_open && app.settings_open);
    assert!(app.handle_back(&ctx));
    assert!(!app.settings_open && app.library.open);
    assert!(app.handle_back(&ctx));
    assert!(!app.library.open && app.inspector.open);
    assert!(app.handle_back(&ctx));
    assert!(!app.inspector.open && app.fullscreen);
    assert!(app.handle_back(&ctx));
    assert!(!app.fullscreen);
    assert!(!app.handle_back(&ctx));
}

#[test]
fn replacing_settings_ends_shortcut_editing_and_held_input() {
    let (mut app, _ctx) = bare_viewer();
    app.settings_open = true;
    app.shortcut_filter = "zoom".into();
    app.shortcut_search = Some(String::new());
    app.shortcut_editor = Some(settings::ShortcutEditor {
        index: Some(0),
        action: "open".into(),
        key: "O".into(),
        listening: false,
        error: String::new(),
    });
    app.held_key = Some((Key::ArrowRight, "next", 0.));
    app.held_button = Some((true, 0.));
    let imported = Settings {
        slide_ms: 2500,
        ..Default::default()
    };
    app.replace_settings(imported.clone());
    assert_eq!(app.settings, imported);
    assert!(app.shortcut_editor.is_none() && app.shortcut_search.is_none());
    assert!(app.shortcut_filter.is_empty());
    assert!(app.held_key.is_none() && app.held_button.is_none());
}

#[test]
fn spread_tooltips_and_setting_ranges_use_existing_catalog_keys() {
    for key in ["two-page-spread", "spread-left", "spread-right"] {
        assert_ne!(tr(key), key, "{key} is not in the catalog");
    }
    let defaults = Settings::default();
    for ms in [
        defaults.slide_ms,
        defaults.key_delay_ms,
        defaults.key_repeat_ms,
        defaults.toast_ms,
        defaults.cursor_hide_ms,
    ] {
        assert!(limits::TIME_MS.contains(&ms) && ms % limits::TIME_STEP_MS == 0);
    }
    assert!(limits::SMALL_IMAGE_PERCENT.contains(&defaults.small_image_percent));
    assert!(limits::CACHE_IMAGE_LIMIT.contains(&defaults.cache_image_limit));
    assert!(limits::CACHE_MEMORY_MB.contains(&defaults.cache_memory_mb));
    assert!(limits::RECENT_LIMIT.contains(&defaults.recent_limit));
    assert!(limits::RECENT_LIMIT.contains(&defaults.recent_move_limit));
    assert!(limits::VIEW_MODES.contains(&defaults.view_mode.as_str()));
}
