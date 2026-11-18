use super::*;

struct Mobile;
impl Mobile {
    fn enter() -> Self {
        crate::ui::mobile::set_preview(true);
        Self
    }
}
impl Drop for Mobile {
    fn drop(&mut self) {
        crate::ui::mobile::set_preview(false);
    }
}

fn viewer() -> (egui::Context, Viewer) {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err(String::new()),
    );
    app.library.open = false;
    app.settings.spread = false;
    app.memory_monitor = crate::core::cache::Monitor::disabled();
    app.files = (0..6)
        .map(|i| PathBuf::from(format!("swipe-{i}.png")))
        .collect();
    for (i, path) in app.files.iter().enumerate() {
        app.pictures.insert(
            path.clone(),
            Picture {
                hdr: None,
                animation: None,
                handle: ctx.load_texture(
                    format!("swipe-{i}"),
                    egui::ColorImage::filled([100, 150], Color32::WHITE),
                    egui::TextureOptions::LINEAR,
                ),
                original: [100, 150],
                requested: MAX_DECODE_SIDE,
                last_used: 0,
            },
        );
    }
    (ctx, app)
}

fn frame(
    app: &mut Viewer,
    ctx: &egui::Context,
    time: f64,
    events: Vec<egui::Event>,
    size: Vec2,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            time: Some(time),
            events,
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| {
            app.advance_slideshow(ui.ctx());
            app.canvas(ui);
        },
    )
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    }
}

fn bounds(output: &egui::FullOutput, id: egui::TextureId) -> Option<(Rect, Rect)> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Mesh(mesh) if mesh.texture_id == id => {
            Some((mesh.calc_bounds(), shape.clip_rect))
        }
        _ => None,
    })
}

#[test]
fn enlarged_drags_pan_without_previewing_or_changing_pages_even_when_pan_is_disabled() {
    let _mobile = Mobile::enter();
    for size in [egui::vec2(390., 844.), egui::vec2(1194., 834.)] {
        for rtl in [false, true] {
            for pan_enabled in [false, true] {
                let (ctx, mut app) = viewer();
                app.settings.rtl = rtl;
                app.settings.touch.one_finger_pan = pan_enabled;
                app.settings.spread = size.x > size.y;
                app.settings.cover = false;
                app.landscape = size.x > size.y;
                for picture in app.pictures.values_mut() {
                    picture.original = [1200, 1800];
                }
                app.zoom = 2.;
                let first = frame(&mut app, &ctx, 0., vec![], size);
                let canvas = app.presentation.canvas.unwrap();
                let current = app.pictures[&app.files[0]].handle.id();
                let original = bounds(&first, current).unwrap().0;
                let adjacent = app.pictures[&app.files[if app.settings.spread { 2 } else { 1 }]]
                    .handle
                    .id();
                let start = canvas.center();
                frame(
                    &mut app,
                    &ctx,
                    0.1,
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                    size,
                );
                let end = start - egui::vec2(100., 0.);
                let dragged = frame(
                    &mut app,
                    &ctx,
                    0.2,
                    vec![egui::Event::PointerMoved(end)],
                    size,
                );
                assert!(app.touch_gesture.pan_only());
                assert!(app.mobile_swipe.is_none());
                assert!(bounds(&dragged, adjacent).is_none());
                let changed = bounds(&dragged, current).unwrap().0.center().x - original.center().x;
                assert!((changed - if pan_enabled { -100. } else { 0. }).abs() < 0.01);
                frame(
                    &mut app,
                    &ctx,
                    0.3,
                    vec![button(end, false), egui::Event::PointerGone],
                    size,
                );
                assert_eq!(app.focus_index, 0);
                assert!(app.mobile_swipe.is_none());
            }
        }
    }
}

#[test]
fn touch_navigation_uses_rendered_size_instead_of_manual_zoom_flags() {
    let _mobile = Mobile::enter();
    for (original, zoom, manual, mode, navigates) in [
        ([100, 150], 1., Some(1.), "fit", true),
        ([100, 150], 8., Some(1.), "fit", true),
        ([100, 150], 1., None, "actual", true),
        ([100, 150], 2., None, "fit", false),
        ([1200, 1800], 1., Some(1.), "fit", false),
        ([1200, 1800], 1., None, "actual", false),
        ([1200, 1800], 1., None, "fill", false),
        ([1200, 1800], 1., None, "fit", true),
    ] {
        let (ctx, mut app) = viewer();
        app.settings.view_mode = mode.into();
        app.zoom = zoom;
        app.zoom_override = manual;
        for picture in app.pictures.values_mut() {
            picture.original = original;
        }
        let size = egui::vec2(390., 844.);
        frame(&mut app, &ctx, 0., vec![], size);
        let start = app.presentation.canvas.unwrap().center();
        frame(
            &mut app,
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            size,
        );
        let end = start - egui::vec2(100., 0.);
        frame(
            &mut app,
            &ctx,
            0.2,
            vec![egui::Event::PointerMoved(end)],
            size,
        );
        frame(
            &mut app,
            &ctx,
            0.3,
            vec![button(end, false), egui::Event::PointerGone],
            size,
        );
        assert_eq!(
            app.focus_index,
            usize::from(navigates),
            "{original:?} zoom={zoom} manual={manual:?} {mode}"
        );
    }
}

#[test]
fn a_pan_stays_locked_when_zoom_returns_to_fit_or_a_second_finger_lifts() {
    let _mobile = Mobile::enter();
    for pinch in [false, true] {
        let (ctx, mut app) = viewer();
        for picture in app.pictures.values_mut() {
            picture.original = [1200, 1800];
        }
        app.zoom = 2.;
        let size = egui::vec2(390., 844.);
        frame(&mut app, &ctx, 0., vec![], size);
        let start = app.presentation.canvas.unwrap().center();
        let touch = |id, phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        };
        frame(
            &mut app,
            &ctx,
            0.1,
            vec![
                egui::Event::PointerMoved(start),
                button(start, true),
                touch(1, egui::TouchPhase::Start, start),
            ],
            size,
        );
        let end = start - egui::vec2(100., 0.);
        frame(
            &mut app,
            &ctx,
            0.2,
            vec![
                egui::Event::PointerMoved(end),
                touch(1, egui::TouchPhase::Move, end),
            ],
            size,
        );
        if pinch {
            let second = start + egui::vec2(60., 0.);
            frame(
                &mut app,
                &ctx,
                0.3,
                vec![touch(2, egui::TouchPhase::Start, second)],
                size,
            );
            frame(
                &mut app,
                &ctx,
                0.4,
                vec![touch(
                    2,
                    egui::TouchPhase::Move,
                    second + egui::vec2(80., 0.),
                )],
                size,
            );
            frame(
                &mut app,
                &ctx,
                0.5,
                vec![touch(
                    2,
                    egui::TouchPhase::End,
                    second + egui::vec2(80., 0.),
                )],
                size,
            );
        } else {
            app.zoom = 1.;
        }
        let released = end - egui::vec2(60., 0.);
        frame(
            &mut app,
            &ctx,
            0.6,
            vec![
                egui::Event::PointerMoved(released),
                touch(1, egui::TouchPhase::Move, released),
            ],
            size,
        );
        frame(
            &mut app,
            &ctx,
            0.7,
            vec![
                button(released, false),
                touch(1, egui::TouchPhase::End, released),
                egui::Event::PointerGone,
            ],
            size,
        );
        assert_eq!(app.focus_index, 0);
        assert!(app.mobile_swipe.is_none());
    }
}

#[test]
fn pinch_primary_finger_handoff_cannot_restart_as_a_page_swipe() {
    let _mobile = Mobile::enter();
    let (ctx, mut app) = viewer();
    let size = egui::vec2(390., 844.);
    app.zoom = 2.;
    frame(&mut app, &ctx, 0., vec![], size);
    let first = egui::pos2(220., 400.);
    let second = egui::pos2(280., 400.);
    let touch = |id, phase, pos| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos,
        force: None,
    };
    frame(
        &mut app,
        &ctx,
        0.1,
        vec![
            egui::Event::PointerMoved(first),
            button(first, true),
            touch(1, egui::TouchPhase::Start, first),
        ],
        size,
    );
    frame(
        &mut app,
        &ctx,
        0.2,
        vec![touch(2, egui::TouchPhase::Start, second)],
        size,
    );
    // Pinch back to fit, then lift the primary finger while the second stays.
    app.zoom = 1.;
    app.zoom_override = None;
    frame(
        &mut app,
        &ctx,
        0.3,
        vec![button(first, false), touch(1, egui::TouchPhase::End, first)],
        size,
    );
    assert!(app.touch_gesture.pan_only());
    frame(
        &mut app,
        &ctx,
        0.4,
        vec![egui::Event::PointerMoved(second), button(second, true)],
        size,
    );
    let end = second - egui::vec2(120., 0.);
    frame(
        &mut app,
        &ctx,
        0.5,
        vec![
            egui::Event::PointerMoved(end),
            touch(2, egui::TouchPhase::Move, end),
        ],
        size,
    );
    frame(
        &mut app,
        &ctx,
        0.6,
        vec![
            button(end, false),
            touch(2, egui::TouchPhase::End, end),
            egui::Event::PointerGone,
        ],
        size,
    );
    assert_eq!(app.focus_index, 0);
    assert!(app.mobile_swipe.is_none());
    assert!(!app.touch_gesture.active());
    // A genuinely new touch at fit regains ordinary page navigation.
    frame(
        &mut app,
        &ctx,
        0.8,
        vec![
            egui::Event::PointerMoved(first),
            button(first, true),
            touch(3, egui::TouchPhase::Start, first),
        ],
        size,
    );
    let next = first - egui::vec2(100., 0.);
    frame(
        &mut app,
        &ctx,
        0.9,
        vec![
            egui::Event::PointerMoved(next),
            touch(3, egui::TouchPhase::Move, next),
        ],
        size,
    );
    frame(
        &mut app,
        &ctx,
        1.,
        vec![button(next, false), touch(3, egui::TouchPhase::End, next)],
        size,
    );
    assert_eq!(app.focus_index, 1);
}

#[test]
fn swipe_tracks_finger_and_settles_in_both_directions_and_spreads() {
    let _mobile = Mobile::enter();
    for spread in [false, true] {
        for rtl in [false, true] {
            for delta in [-100., 100.] {
                let (ctx, mut app) = viewer();
                app.settings.rtl = rtl;
                app.settings.spread = spread;
                app.settings.cover = false;
                app.landscape = spread;
                app.layout_monitor = egui::vec2(1200., 800.);
                let size = if spread {
                    egui::vec2(900., 650.)
                } else {
                    egui::vec2(390., 844.)
                };
                let first = frame(&mut app, &ctx, 0., vec![], size);
                let canvas = app.presentation.canvas.unwrap();
                let outgoing = app.pictures[&app.files[0]].handle.id();
                let original = bounds(&first, outgoing).unwrap().0;
                let target = app.swipe_target((delta < 0.) != rtl).unwrap();
                let target_id = app.pictures[&app.files[target]].handle.id();
                let start = canvas.center();
                frame(
                    &mut app,
                    &ctx,
                    0.1,
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                    size,
                );
                let end = start + egui::vec2(delta, 0.);
                let dragging = frame(
                    &mut app,
                    &ctx,
                    0.2,
                    vec![egui::Event::PointerMoved(end)],
                    size,
                );
                assert_eq!(app.focus_index, 0);
                let (moved, clip) = bounds(&dragging, outgoing).unwrap();
                assert!((moved.center().x - original.center().x - delta).abs() < 0.01);
                assert!(canvas.contains_rect(clip));
                assert!(
                    bounds(&dragging, target_id).is_some(),
                    "Adjacent page must also slide"
                );
                frame(
                    &mut app,
                    &ctx,
                    0.3,
                    vec![button(end, false), egui::Event::PointerGone],
                    size,
                );
                assert_eq!(app.focus_index, target);
                let moving = frame(&mut app, &ctx, 0.4, vec![], size);
                let travelled =
                    (bounds(&moving, outgoing).unwrap().0.center().x - original.center().x).abs();
                assert!(travelled > delta.abs() && travelled < canvas.width());
                let settled = frame(&mut app, &ctx, 0.6, vec![], size);
                assert!(app.mobile_swipe.is_none());
                assert!(bounds(&settled, outgoing).is_none());
                assert!(bounds(&settled, target_id).is_some());
            }
        }
    }
}

#[test]
fn short_swipe_returns_and_native_cancellation_does_not_turn_page() {
    let _mobile = Mobile::enter();
    for cancel in [false, true] {
        let (ctx, mut app) = viewer();
        let size = egui::vec2(390., 844.);
        frame(&mut app, &ctx, 0., vec![], size);
        let start = egui::pos2(200., 400.);
        frame(
            &mut app,
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            size,
        );
        let end = start - egui::vec2(if cancel { 100. } else { 30. }, 0.);
        frame(
            &mut app,
            &ctx,
            0.2,
            vec![egui::Event::PointerMoved(end)],
            size,
        );
        assert!(app.mobile_swipe.is_some());
        let mut events = vec![button(end, false), egui::Event::PointerGone];
        if cancel {
            events.push(egui::Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(1),
                phase: egui::TouchPhase::Cancel,
                pos: end,
                force: None,
            });
        }
        frame(&mut app, &ctx, 0.3, events, size);
        assert_eq!(app.focus_index, 0);
        assert_eq!(app.mobile_swipe.is_some(), !cancel);
        frame(&mut app, &ctx, 0.6, vec![], size);
        assert!(app.mobile_swipe.is_none());
        assert_eq!(app.focus_index, 0);
    }
}

#[test]
fn dragging_back_to_the_start_restores_the_image_before_release() {
    let _mobile = Mobile::enter();
    let (ctx, mut app) = viewer();
    let size = egui::vec2(390., 844.);
    let first = frame(&mut app, &ctx, 0., vec![], size);
    let id = app.pictures[&app.files[0]].handle.id();
    let original = bounds(&first, id).unwrap().0;
    let start = egui::pos2(200., 400.);
    frame(
        &mut app,
        &ctx,
        0.1,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        size,
    );
    for (time, delta) in [(0.2, -100.), (0.3, 0.), (0.4, 80.), (0.5, 3.)] {
        let end = start + egui::vec2(delta, 0.);
        let shown = frame(
            &mut app,
            &ctx,
            time,
            vec![egui::Event::PointerMoved(end)],
            size,
        );
        assert!(
            (bounds(&shown, id).unwrap().0.center().x - original.center().x - delta).abs() < 0.01
        );
    }
    frame(&mut app, &ctx, 0.6, vec![button(start, false)], size);
    frame(&mut app, &ctx, 0.9, vec![], size);
    assert_eq!(app.focus_index, 0);
    assert!(app.mobile_swipe.is_none());
}

#[test]
fn swipe_waits_for_decode_and_can_be_interrupted_by_the_next_swipe() {
    let _mobile = Mobile::enter();
    let (ctx, mut app) = viewer();
    let size = egui::vec2(390., 844.);
    frame(&mut app, &ctx, 0., vec![], size);
    let next = app.pictures.remove(&app.files[1]).unwrap();
    let start = egui::pos2(200., 400.);
    let end = start - egui::vec2(100., 0.);
    frame(
        &mut app,
        &ctx,
        0.1,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        size,
    );
    frame(
        &mut app,
        &ctx,
        0.2,
        vec![egui::Event::PointerMoved(end)],
        size,
    );
    frame(&mut app, &ctx, 0.3, vec![button(end, false)], size);
    frame(&mut app, &ctx, 1., vec![], size);
    assert_eq!(app.focus_index, 1);
    assert_eq!(app.mobile_swipe.as_ref().unwrap().offset, -100.);
    assert!(
        app.mobile_swipe
            .as_ref()
            .unwrap()
            .settle
            .as_ref()
            .unwrap()
            .started
            .is_none()
    );
    // An evicted outgoing image is still owned by Presentation and counted.
    app.pictures.remove(&app.files[0]);
    assert_eq!(app.cache_bytes(), 5 * 100 * 150 * 4);
    app.pictures.insert(app.files[1].clone(), next);
    frame(&mut app, &ctx, 1.1, vec![], size);
    frame(
        &mut app,
        &ctx,
        1.15,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        size,
    );
    assert!(app.touch_gesture.active());
    frame(
        &mut app,
        &ctx,
        1.2,
        vec![egui::Event::PointerMoved(end)],
        size,
    );
    frame(&mut app, &ctx, 1.25, vec![button(end, false)], size);
    assert_eq!(app.focus_index, 2);
    frame(&mut app, &ctx, 1.6, vec![], size);
    assert!(app.mobile_swipe.is_none());
}

#[test]
fn slideshow_and_reduced_motion_finish_without_slide_animation() {
    let _mobile = Mobile::enter();
    for slideshow in [false, true] {
        let (ctx, mut app) = viewer();
        let size = egui::vec2(390., 844.);
        frame(&mut app, &ctx, 0., vec![], size);
        let start = egui::pos2(200., 400.);
        let end = start - egui::vec2(100., 0.);
        frame(
            &mut app,
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            size,
        );
        frame(
            &mut app,
            &ctx,
            0.2,
            vec![egui::Event::PointerMoved(end)],
            size,
        );
        assert!(app.mobile_swipe.is_some());
        let events = if slideshow {
            app.slideshow = true;
            app.slide_deadline = Some(0.3);
            vec![]
        } else {
            app.settings.reduce_motion = true;
            vec![button(end, false)]
        };
        let shown = frame(&mut app, &ctx, 0.3, events, size);
        assert_eq!(app.focus_index, 1);
        assert!(app.mobile_swipe.is_none());
        assert!(bounds(&shown, app.pictures[&app.files[0]].handle.id()).is_none());
        let next = bounds(&shown, app.pictures[&app.files[1]].handle.id())
            .unwrap()
            .0;
        assert!((next.center().x - size.x / 2.).abs() < 0.01);
    }
}

#[test]
fn another_swipe_can_reverse_a_page_that_is_still_decoding() {
    let _mobile = Mobile::enter();
    let (ctx, mut app) = viewer();
    let size = egui::vec2(390., 844.);
    frame(&mut app, &ctx, 0., vec![], size);
    app.pictures.remove(&app.files[1]);
    let start = egui::pos2(200., 400.);
    for (time, delta) in [(0.1, -100.), (0.5, 100.)] {
        frame(
            &mut app,
            &ctx,
            time,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            size,
        );
        assert!(app.touch_gesture.active());
        let end = start + egui::vec2(delta, 0.);
        frame(
            &mut app,
            &ctx,
            time + 0.1,
            vec![egui::Event::PointerMoved(end)],
            size,
        );
        frame(
            &mut app,
            &ctx,
            time + 0.2,
            vec![button(end, false), egui::Event::PointerGone],
            size,
        );
    }
    assert_eq!(app.focus_index, 0);
    assert!(app.mobile_swipe.is_none());
}

#[test]
fn pinch_resize_and_covering_ui_cancel_a_drag_without_navigation() {
    let _mobile = Mobile::enter();
    for interruption in ["pinch", "resize", "settings"] {
        let (ctx, mut app) = viewer();
        let mut size = egui::vec2(390., 844.);
        frame(&mut app, &ctx, 0., vec![], size);
        let start = egui::pos2(200., 400.);
        let end = start - egui::vec2(100., 0.);
        frame(
            &mut app,
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            size,
        );
        frame(
            &mut app,
            &ctx,
            0.2,
            vec![egui::Event::PointerMoved(end)],
            size,
        );
        assert!(app.mobile_swipe.is_some());
        let mut events = vec![];
        match interruption {
            "pinch" => {
                for id in 1..=2 {
                    events.push(egui::Event::Touch {
                        device_id: egui::TouchDeviceId(1),
                        id: egui::TouchId(id),
                        phase: egui::TouchPhase::Start,
                        pos: end + egui::vec2(id as f32 * 40., 0.),
                        force: None,
                    });
                }
            }
            "resize" => size = egui::vec2(844., 390.),
            _ => app.settings_open = true,
        }
        frame(&mut app, &ctx, 0.3, events, size);
        assert!(app.mobile_swipe.is_none());
        frame(&mut app, &ctx, 0.4, vec![button(end, false)], size);
        assert_eq!(app.focus_index, 0);
        assert!(app.mobile_swipe.is_none());
    }
}
