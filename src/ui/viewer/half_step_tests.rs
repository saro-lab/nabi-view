use super::*;

fn viewer() -> (Viewer, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    app.files = (0..6).map(|i| PathBuf::from(format!("{i}.png"))).collect();
    app.settings.spread = true;
    app.landscape = true;
    app.library.open = false;
    (app, ctx)
}

#[test]
fn half_steps_overlap_one_page_and_keep_normal_navigation_in_pairs() {
    for rtl in [false, true] {
        let (mut app, ctx) = viewer();
        app.settings.rtl = rtl;
        app.checked.insert(app.files[0].clone());
        assert_eq!(app.focus_span(), (0, 1));
        app.run("page-next", &ctx);
        assert_eq!(app.focus_span(), (1, 2));
        app.run("next", &ctx);
        assert_eq!(app.focus_span(), (3, 4));
        app.run("prev", &ctx);
        assert_eq!(app.focus_span(), (1, 2));
        app.run("page-next", &ctx);
        assert_eq!(app.focus_span(), (2, 3));
        app.run("page-prev", &ctx);
        assert_eq!(app.focus_span(), (1, 2));
        app.run("page-prev", &ctx);
        assert_eq!(app.focus_span(), (0, 1));
        assert_eq!(
            app.toast.as_ref().unwrap().message,
            tr("this-is-the-first-image")
        );
        app.run("page-prev", &ctx);
        assert_eq!(app.focus_span(), (5, 5));
        app.run("page-next", &ctx);
        assert_eq!(app.focus_span(), (0, 1));
        assert_eq!(app.checked, HashSet::from([app.files[0].clone()]));
    }
}

#[test]
fn half_steps_in_random_order_visit_every_anchor_before_reshuffling() {
    let (mut app, ctx) = viewer();
    app.random = true;
    let order = app.files.clone();
    for (index, path) in order.iter().enumerate() {
        assert_eq!(app.focus_span().0, index);
        assert_eq!(&app.files[index], path);
        app.run("page-next", &ctx);
    }
    assert_eq!(app.focus_index, 0);
    assert_ne!(&app.files[0], order.last().unwrap());
    assert_eq!(
        app.files.iter().collect::<HashSet<_>>(),
        order.iter().collect()
    );
}

#[test]
fn half_steps_start_from_the_visible_spread_even_when_its_second_page_is_focused() {
    let (mut app, ctx) = viewer();
    app.focus(1);
    assert_eq!(app.focus_span(), (0, 1));
    app.run("page-next", &ctx);
    assert_eq!(app.focus_span(), (1, 2));
    app.run("spread-left", &ctx);
    assert_eq!(app.focus_span(), (1, 2));
    app.run("single", &ctx);
    assert_eq!(app.focus_span(), (1, 1));
    app.run("page-next", &ctx);
    assert_eq!(app.focus_span(), (2, 2));
    app.run("page-prev", &ctx);
    assert_eq!(app.focus_span(), (1, 1));
    app.landscape = false;
    app.settings.spread = true;
    app.run("page-next", &ctx);
    assert_eq!(app.focus_span(), (2, 2));
}

#[test]
fn half_steps_preserve_standalone_pages_and_normal_navigation_never_skips_files() {
    // Exercise every wide-image/cover arrangement and both pairing alignments.
    for mask in 0..64 {
        for cover in [false, true] {
            let (mut app, ctx) = viewer();
            app.layout_monitor = egui::vec2(1920., 1080.);
            app.settings.cover = cover;
            for index in 0..6 {
                if mask & (1 << index) != 0 {
                    app.page_sizes
                        .insert(app.files[index].clone(), [2400, 1000]);
                }
            }
            for index in 0..6 {
                let (first, last) = app.focus_span();
                assert_eq!(first, index);
                assert_eq!(app.focus_index, index);
                if mask & (1 << index) != 0 || (cover && index == 0) {
                    assert_eq!(last, first);
                }
                app.run("page-next", &ctx);
            }
            for index in (0..6).rev() {
                app.run("page-prev", &ctx);
                assert_eq!(app.focus_span().0, index);
            }
            for shifted in [false, true] {
                app.spread_shifted = shifted;
                app.focus(0);
                let mut visited = Vec::new();
                loop {
                    let (first, last) = app.focus_span();
                    visited.extend(first..=last);
                    if last == 5 {
                        break;
                    }
                    app.run("next", &ctx);
                }
                assert_eq!(visited, (0..6).collect::<Vec<_>>());
            }
        }
    }
}

#[test]
fn half_steps_queue_with_other_navigation_and_handle_empty_or_single_file() {
    let (mut app, ctx) = viewer();
    app.listing = Some(1);
    app.run("page-next", &ctx);
    app.run("next", &ctx);
    app.run("page-prev", &ctx);
    assert_eq!(app.focus_span(), (0, 1));
    app.listing = None;
    app.replay_pending_navigation(&ctx);
    assert_eq!(app.focus_span(), (2, 3));
    for count in [1, 0] {
        app.files.truncate(count);
        app.focus(0);
        for action in ["page-prev", "page-next"] {
            app.run(action, &ctx);
            assert_eq!(app.focus_index, 0);
        }
    }
}

#[test]
fn half_steps_repeat_rebind_and_stop_when_alt_is_released() {
    let (mut app, ctx) = viewer();
    let frame = |app: &mut Viewer, key, pressed, modifiers, time| {
        let _ = ctx.run_pass(
            egui::RawInput {
                time: Some(time),
                focused: true,
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    },
                ],
                ..Default::default()
            },
            |ctx| app.keyboard(ctx),
        );
    };
    for (index, key) in [Key::D, Key::ArrowRight].into_iter().enumerate() {
        let time = index as f64;
        app.focus(0);
        app.spread_shifted = false;
        frame(&mut app, key, true, egui::Modifiers::ALT, time);
        assert_eq!(app.focus_span(), (1, 2));
        let _ = ctx.run_pass(
            egui::RawInput {
                time: Some(time + 0.31),
                ..Default::default()
            },
            |ctx| app.keyboard(ctx),
        );
        assert_eq!(app.focus_span(), (2, 3));
        frame(&mut app, key, false, egui::Modifiers::NONE, time + 0.32);
        assert!(app.held_key.is_none());
    }
    for (index, key) in [Key::A, Key::ArrowLeft].into_iter().enumerate() {
        let time = index as f64 + 2.;
        app.focus(2);
        app.spread_shifted = false;
        frame(&mut app, key, true, egui::Modifiers::ALT, time);
        assert_eq!(app.focus_span(), (1, 2));
        // Modifier release alone must cancel the timer while the key stays down.
        let _ = ctx.run_pass(
            egui::RawInput {
                time: Some(time + 0.5),
                events: vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| app.keyboard(ctx),
        );
        assert_eq!(app.focus_span(), (1, 2));
        assert!(app.held_key.is_none());
        frame(&mut app, key, false, egui::Modifiers::NONE, time + 0.6);
    }
    app.save_shortcut_rows(vec![KeyboardShortcut {
        action: "page-next".into(),
        key: "J".into(),
    }])
    .unwrap();
    frame(&mut app, Key::D, true, egui::Modifiers::ALT, 4.);
    assert_eq!(app.focus_span(), (1, 2));
    frame(&mut app, Key::J, true, egui::Modifiers::NONE, 4.1);
    assert_eq!(app.focus_span(), (2, 3));
}
