use super::*;

#[test]
fn tab_inspects_embedded_generation_data_of_the_displayed_page() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    assert!(window.ctx.memory(|m| m.focused()).is_none());
    window.draw(Some(Key::Tab));
    window.wait_for("314159");
    assert!(
        window.ctx.memory(|m| m.focused()).is_none(),
        "Tab took focus from the canvas"
    );
    window.assert_canvas_overlay("314159");
    window.draw(Some(Key::Tab));
    assert!(!window.draw(None).0.contains("314159"));
    assert!(
        window.ctx.memory(|m| m.focused()).is_none(),
        "Closing metadata advanced widget focus"
    );
    window.draw(Some(Key::ArrowRight));
    window.wait_for("02 Pelagic.png");
    window.draw(Some(Key::Tab));
    window.wait_for("CLIPTextEncode");
    window.assert_canvas_overlay("CLIPTextEncode");
    window.draw(Some(Key::ArrowLeft));
    window.wait_for("314159");
}

#[test]
fn metadata_click_copies_one_value_flashes_blue_and_shows_korean_toast() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        language: crate::i18n::Language::Korean,
        toast_ms: 5_000,
        ..Default::default()
    });
    window.draw(Some(Key::Tab));
    window.wait_for("314159");
    let report = crate::core::metadata::read(
        &window
            ._images
            .as_ref()
            .unwrap()
            .path()
            .join("01 Alpine Light.png"),
    )
    .unwrap();
    let (key, value) = report
        .fields
        .iter()
        .find(|(_, value)| value.contains("314159"))
        .expect("The fixture must contain an embedded generation prompt");
    assert!(value.len() > 80 && value.len() < 4096);
    let prompt_text = format!("{key}\n{value}");
    let prompt_rect = window
        .paint
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text)
                if text.galley.job.text == prompt_text
                    && text.override_text_color == Some(Color32::WHITE) =>
            {
                Some(text.visual_bounding_rect().intersect(shape.clip_rect))
            }
            _ => None,
        })
        .expect("The generation field must be visible");
    let summary_left = window
        .paint
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text)
                if text.galley.job.text == "PNG"
                    && text.override_text_color == Some(Color32::WHITE) =>
            {
                Some(text.visual_bounding_rect().left())
            }
            _ => None,
        })
        .unwrap();
    assert!(
        (prompt_rect.left() - summary_left).abs() < 2.,
        "Plain metadata must align with the summary without a tree-marker gutter: {prompt_rect:?}, {summary_left}"
    );
    // Clicking the field's heading also copies its full value, without the key.
    click_metadata_without_window_drag(&mut window, prompt_rect.min + egui::vec2(8., 8.));
    assert_eq!(window.clipboard_writes, std::slice::from_ref(value));
    let text = window.draw(None).0;
    assert!(text.contains("복사되었습니다."));

    let prompt_color = |window: &TestWindow| {
        window
            .paint
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text) if text.galley.job.text == prompt_text => text
                    .override_text_color
                    .filter(|color| *color != Color32::BLACK),
                _ => None,
            })
            .expect("The generation field must stay visible after copying")
    };
    assert_eq!(prompt_color(&window), Color32::from_rgb(150, 205, 255));
    assert!(window.repaint_delay <= Duration::from_millis(16));
    window.time += 0.25;
    window.draw(None);
    let fading = prompt_color(&window);
    assert!(fading.r() > 150 && fading.r() < 255);
    assert!(fading.g() > 205 && fading.g() < 255);
    assert_eq!(fading.b(), 255);
    window.time += 0.6;
    window.draw(None);
    assert_eq!(prompt_color(&window), Color32::WHITE);

    // Summary parts have independent hit targets and copy only that part.
    let extension_position = window.label_position("PNG");
    click_metadata_without_window_drag(&mut window, extension_position);
    assert_eq!(window.clipboard_writes, [value.clone(), "PNG".into()]);
    assert_eq!(prompt_color(&window), Color32::WHITE);
    let text = window.draw(None).0;
    assert!(!text.contains("Copy all") && !text.contains("모두 복사"));
    assert!(!text.contains("Path  ·"));
    assert!(
        !text.contains(
            &window
                ._images
                .as_ref()
                .unwrap()
                .path()
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(
        text.matches("01 Alpine Light.png").count(),
        1,
        "The filename should appear only in the top chrome"
    );
}

#[test]
fn avif_metadata_summary_includes_resolution_frames_and_both_size_forms() {
    let source = crate::core::media::animation_fixtures::avif_sample();
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("animated.avif");
    std::fs::copy(source, &path).unwrap();
    let report = crate::core::metadata::read(&path).unwrap();
    let size = &report
        .fields
        .iter()
        .find(|(key, _)| key == "File size")
        .unwrap()
        .1;
    assert!(size.contains(" (") && size.contains(',') && size.ends_with(')'));
    assert!(size.contains("KB") || size.contains("MB"));

    let mut window = TestWindow::open(Some(path));
    window._images = Some(images);
    window.draw(Some(Key::Tab));
    window.wait_for("498 × 286");
    let text = window.draw(None).0;
    assert!(text.contains("AVIF"));
    assert!(text.contains(&format!("(22{})", crate::i18n::test_tr(" frames"))));
    assert!(text.contains(size));
    assert_eq!(text.matches("animated.avif").count(), 1);
    assert!(!text.contains("Path  ·"));
    window.assert_canvas_overlay("498 × 286");

    window.screen_size = egui::vec2(560., 420.);
    window.draw(None);
    let canvas = window.photo_rects().0;
    // At the smallest supported window size, each summary value must remain
    // wholly visible and independently clickable rather than being clipped.
    for needle in ["AVIF", "498 × 286", size.as_str()] {
        let shape = window
            .paint
            .iter()
            .find(|shape| {
                matches!(&shape.shape, Shape::Text(text)
                if text.galley.job.text.contains(needle)
                    && text.override_text_color == Some(Color32::WHITE))
            })
            .unwrap_or_else(|| panic!("Missing narrow-window summary part: {needle}"));
        let Shape::Text(text) = &shape.shape else {
            unreachable!()
        };
        let bounds = text.visual_bounding_rect();
        assert!(
            canvas.expand(1.).contains_rect(bounds),
            "Summary left the canvas: {needle}"
        );
        assert!(
            shape.clip_rect.expand(1.).contains_rect(bounds),
            "Summary was clipped: {needle}"
        );
    }
    let size_position = window.label_position(size);
    click_metadata_without_window_drag(&mut window, size_position);
    assert_eq!(window.clipboard_writes, std::slice::from_ref(size));
}

#[test]
fn tab_preserves_existing_widget_focus_through_repeats_and_shift() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    window.wait_for("01 Alpine Light.png");
    let center = window.titlebar_icon("settings").rect.center();
    window.draw_events(vec![Event::PointerMoved(center)], egui::Modifiers::NONE);
    let hovered = window.ctx.interaction_snapshot(|s| s.hovered.clone());
    let button = hovered
        .into_iter()
        .find(|&id| {
            window
                .ctx
                .read_response(id)
                .is_some_and(|r| r.sense.is_focusable() && r.rect.contains(center))
        })
        .expect("hover target should be the real Settings button");
    window.ctx.memory_mut(|m| m.request_focus(button));
    window.draw(None);
    assert_eq!(window.ctx.memory(|m| m.focused()), Some(button));

    let send_tab = |window: &mut TestWindow, pressed, repeat, modifiers| {
        let output = window.draw_events(
            vec![key_event(Key::Tab, pressed, repeat, modifiers)],
            modifiers,
        );
        assert_eq!(
            window.ctx.memory(|m| m.focused()),
            Some(button),
            "Tab event moved focus (pressed={pressed}, repeat={repeat}, modifiers={modifiers:?})"
        );
        output.0
    };
    send_tab(&mut window, true, false, egui::Modifiers::NONE);
    window.wait_for("314159");
    assert!(send_tab(&mut window, true, true, egui::Modifiers::NONE).contains("314159"));
    assert!(send_tab(&mut window, false, false, egui::Modifiers::NONE).contains("314159"));
    // A fresh Tab press (separate physical cycle) still closes the overlay.
    assert!(!send_tab(&mut window, true, false, egui::Modifiers::NONE).contains("314159"));
    send_tab(&mut window, false, false, egui::Modifiers::NONE);
    send_tab(&mut window, true, false, egui::Modifiers::SHIFT);
    send_tab(&mut window, false, false, egui::Modifiers::SHIFT);
}

#[test]
fn metadata_summary_stays_at_viewport_corner_when_zooming_and_switching_to_spread() {
    let mut window = TestWindow::sample();
    window.reveal_controls();
    let filename = "01 Alpine Light.png";
    window.wait_for(filename);
    window.draw(Some(Key::Tab));
    window.wait_for("314159");

    let text_origin = |window: &TestWindow, needle: &str| {
        window
            .paint
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t)
                    if t.override_text_color == Some(Color32::WHITE)
                        && t.galley.job.text.contains(needle)
                        && s.clip_rect.intersects(t.visual_bounding_rect()) =>
                {
                    Some(t.pos)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing visible overlay text: {needle}"))
    };
    let (canvas, before_photo) = window.photo_rects();
    let summary = text_origin(&window, "PNG");
    assert!(
        before_photo.width() < before_photo.height(),
        "Test needs a portrait image"
    );
    assert!(
        summary.x < before_photo.left(),
        "Metadata should start in the left margin outside the portrait"
    );
    assert!(
        canvas.contains(summary)
            && (summary.x - canvas.left()) < canvas.width() * 0.05
            && (summary.y - canvas.top()) <= 60.,
        "The summary should start near the upper-left corner of the viewer: {summary:?} / {canvas:?}"
    );

    window.draw(Some(Key::Equals));
    let (_, zoomed_photo) = window.photo_rects();
    assert!(
        zoomed_photo.width() > before_photo.width(),
        "The image must actually zoom"
    );
    let zoomed_summary = text_origin(&window, "PNG");
    assert!(
        summary.distance(zoomed_summary) <= 1.0,
        "Zoom moved the metadata from {summary:?} to {zoomed_summary:?}"
    );

    window.draw(Some(Key::CloseBracket));
    window.wait_for("314159");
    window.wait_for("CLIPTextEncode");
    let spread_summary = text_origin(&window, "PNG");
    assert!(
        summary.distance(spread_summary) <= 1.0,
        "The left summary should stay at the same position in spread mode"
    );
    let right_summary = window
        .paint
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text)
                if text.galley.job.text == "PNG"
                    && text.override_text_color == Some(Color32::WHITE)
                    && text.pos.x >= canvas.center().x =>
            {
                Some(text.pos)
            }
            _ => None,
        })
        .expect("Both pages must show their summaries simultaneously");
    assert!(
        (right_summary.x - canvas.center().x).abs() <= 3.
            && (right_summary.y - spread_summary.y).abs() <= 1.,
        "The right summary must start at the midpoint, aligned with the left: {right_summary:?}"
    );
    let text = window.draw(None).0;
    assert!(!text.contains("Page 1") && !text.contains("Page 2"));
}

#[test]
fn metadata_width_is_limited_to_each_half_and_follows_spread_reading_direction() {
    let mut window = TestWindow::sample();
    window.draw(Some(Key::Tab));
    window.wait_for("314159");

    let assert_column = |window: &TestWindow, needle: &str, right: bool| {
        let middle = window.screen_size.x * 0.5;
        let (left, end) = if right {
            (middle, window.screen_size.x)
        } else {
            (0., middle)
        };
        let mut found = false;
        for shape in &window.paint {
            let Shape::Text(text) = &shape.shape else {
                continue;
            };
            if !text.galley.job.text.contains(needle)
                || text.override_text_color != Some(Color32::WHITE)
                || !shape.clip_rect.intersects(text.visual_bounding_rect())
            {
                continue;
            }
            found = true;
            assert!(
                shape.clip_rect.left() >= left - 1.
                    && shape.clip_rect.right() <= end + 1.
                    && shape.clip_rect.width() <= middle + 1.,
                "Metadata escaped its half: {needle}, clip={:?}",
                shape.clip_rect
            );
            let bounds = text.visual_bounding_rect();
            assert!(
                bounds.left() >= left - 1. && bounds.right() <= end + 1.,
                "Metadata should wrap within its half: {needle}, bounds={bounds:?}"
            );
        }
        assert!(found, "Missing visible metadata: {needle}");
    };

    for size in [egui::vec2(1080., 760.), egui::vec2(560., 420.)] {
        window.screen_size = size;
        window.draw(None);
        assert_column(&window, "314159", false);
    }

    window.draw(Some(Key::CloseBracket));
    window.wait_for("314159");
    window.wait_for("CLIPTextEncode");
    for size in [egui::vec2(1080., 760.), egui::vec2(560., 420.)] {
        window.screen_size = size;
        window.draw(None);
        assert_column(&window, "314159", false);
        assert_column(&window, "CLIPTextEncode", true);
    }

    let field_origin = |window: &TestWindow, needle: &str| {
        window
            .paint
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text)
                    if text.galley.job.text.contains(needle)
                        && text.override_text_color == Some(Color32::WHITE) =>
                {
                    Some(text.pos)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing metadata field: {needle}"))
    };
    let left_before = field_origin(&window, "314159");
    let right_before = field_origin(&window, "CLIPTextEncode");
    window.pointer(window.screen_size.x * 0.75, right_before.y + 30.);
    window.draw_events(
        vec![Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -32.),
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    for _ in 0..12 {
        let text = window.draw(None).0;
        assert!(
            text.contains("01 Alpine Light.png") && !text.contains("03 Dune Forms.png"),
            "Scrolling the right metadata column navigated to another spread"
        );
        assert!(
            field_origin(&window, "314159").distance(left_before) <= 1.,
            "Scrolling the right metadata column moved the left column"
        );
    }
    assert!(
        field_origin(&window, "CLIPTextEncode").y < right_before.y - 1.,
        "The wheel should scroll the right metadata column"
    );

    window.draw(Some(Key::OpenBracket));
    window.wait_for("314159");
    window.wait_for("CLIPTextEncode");
    assert_column(&window, "CLIPTextEncode", false);
    assert_column(&window, "314159", true);

    // Returning to single mode keeps the focused first file and removes its mate.
    window.draw(Some(Key::P));
    window.wait_for("314159");
    assert_column(&window, "314159", false);
    assert!(!window.draw(None).0.contains("CLIPTextEncode"));
}

#[test]
fn spread_metadata_copies_each_pages_value_without_dragging_the_window() {
    let mut window = TestWindow::sample();
    window.draw(Some(Key::CloseBracket));
    window.draw(Some(Key::Tab));
    window.wait_for("314159");
    window.wait_for("CLIPTextEncode");

    let mut expected = Vec::new();
    for (filename, needle, right) in [
        ("01 Alpine Light.png", "314159", false),
        ("02 Pelagic.png", "CLIPTextEncode", true),
    ] {
        let report =
            crate::core::metadata::read(&window._images.as_ref().unwrap().path().join(filename))
                .unwrap();
        let value = &report
            .fields
            .iter()
            .find(|(_, value)| value.contains(needle))
            .unwrap()
            .1;
        let bounds = window
            .paint
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text)
                    if text.galley.job.text.contains(needle)
                        && text.override_text_color == Some(Color32::WHITE) =>
                {
                    let bounds = text.visual_bounding_rect().intersect(shape.clip_rect);
                    bounds.is_positive().then_some(bounds)
                }
                _ => None,
            })
            .expect("Each page's prompt must be visible before copying");
        assert_eq!(bounds.left() >= window.screen_size.x * 0.5, right);
        click_metadata_without_window_drag(&mut window, bounds.min + egui::vec2(8., 8.));
        // Structured metadata now copies the clicked leaf, not its whole document.
        expected.push(if right {
            needle.to_owned()
        } else {
            value.clone()
        });
        assert_eq!(window.clipboard_writes, expected);
    }
}

#[test]
fn metadata_json_tree_keeps_all_children_visible_while_copying_leaves_and_branches() {
    let mut window = TestWindow::sample();
    window.draw(Some(Key::ArrowRight));
    window.draw(Some(Key::Tab));
    window.wait_for("CLIPTextEncode");

    let bounds = |window: &TestWindow, label: &str| {
        window
            .paint
            .iter()
            .find_map(|shape| {
                let Shape::Text(text) = &shape.shape else {
                    return None;
                };
                (text.galley.job.text == label && text.override_text_color != Some(Color32::BLACK))
                    .then(|| text.visual_bounding_rect().intersect(shape.clip_rect))
                    .filter(|rect| rect.is_positive())
            })
            .unwrap_or_else(|| panic!("Missing visible tree row: {label}"))
    };
    let root = "PNG zTXt · prompt  ·  {…} (1)";
    let leaf = "- class_type: CLIPTextEncode";
    let inputs = "- inputs: {…} (1)";
    let click_value = |window: &mut TestWindow, label: &str| {
        let rect = bounds(window, label);
        click_metadata_without_window_drag(window, rect.min + egui::vec2(8., 8.));
    };

    assert!(bounds(&window, leaf).left() > bounds(&window, root).left() + 20.);
    click_value(&mut window, leaf);
    assert_eq!(window.clipboard_writes, ["CLIPTextEncode"]);
    let prompt = "Illustrated ocean waves under a warm sun";
    let text_row = format!("- text: {prompt}");
    window.wait_for(&text_row);
    assert!(bounds(&window, &text_row).left() > bounds(&window, inputs).left() + 15.);
    click_value(&mut window, &text_row);
    assert_eq!(window.clipboard_writes.last().unwrap(), prompt);

    click_value(&mut window, inputs);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(window.clipboard_writes.last().unwrap()).unwrap(),
        serde_json::json!({"text": prompt})
    );
    assert!(
        window.draw(None).0.contains(&text_row),
        "Copying a branch must keep its children visible"
    );
    click_value(&mut window, root);
    let full: serde_json::Value =
        serde_json::from_str(window.clipboard_writes.last().unwrap()).unwrap();
    assert_eq!(full["1"]["class_type"], "CLIPTextEncode");
    assert_eq!(full["1"]["inputs"]["text"], prompt);
    window.wait_for(&text_row);
    assert_eq!(window.clipboard_writes.len(), 4);

    window.draw(Some(Key::Tab));
    window.draw(Some(Key::Tab));
    window.wait_for(leaf);
    let visible = window.draw(None).0;
    assert!(
        !visible.contains("+/−")
            && !visible.contains("Click a value")
            && !visible.contains("항목 클릭")
    );
    assert!(
        window.draw(None).0.contains(&text_row),
        "JSON values always remain visible"
    );
}

#[test]
fn metadata_filter_shows_matching_keys_or_values_and_types_over_shortcuts() {
    let mut window = TestWindow::sample();
    window.draw(Some(Key::ArrowRight));
    window.draw(Some(Key::Tab));
    window.wait_for("CLIPTextEncode");
    let class_row = "- class_type: CLIPTextEncode";
    let text_row = "- text: Illustrated ocean waves under a warm sun";
    let filter_id = egui::Id::unique(("metadata-filter", 0usize));
    assert!(
        window
            .draw(None)
            .0
            .contains(crate::i18n::tr("shortcut-filter"))
    );
    assert!(
        window.ctx.memory(|m| m.focused()).is_none(),
        "Opening metadata must not focus the filter"
    );
    let field = window.ctx.read_response(filter_id).unwrap();
    click_metadata_without_window_drag(&mut window, field.rect.center());
    assert!(window.ctx.memory(|m| m.has_focus(filter_id)));

    // `I` and arrows are text editing keys here, not viewer shortcuts.
    let none = egui::Modifiers::NONE;
    window.draw_events(
        vec![
            key_event(Key::I, true, false, none),
            Event::Text("I".into()),
            key_event(Key::I, false, false, none),
            Event::Text("LLUSTRATED".into()),
            key_event(Key::ArrowLeft, true, false, none),
            key_event(Key::ArrowLeft, false, false, none),
        ],
        none,
    );
    let visible = window.draw(None).0;
    assert!(visible.contains(text_row) && visible.contains("- inputs: {…} (1)"));
    assert!(
        !visible.contains(class_row),
        "Unmatched siblings are hidden"
    );
    assert!(visible.contains("PNG"), "The summary is not filtered");
    let highlights: Vec<_> = window
        .paint
        .iter()
        .filter_map(|shape| match &shape.shape {
            Shape::Rect(rect)
                if rect.fill == Color32::from_rgba_premultiplied(150, 112, 0, 150) =>
            {
                Some(rect.rect)
            }
            _ => None,
        })
        .collect();
    assert!(
        !highlights.is_empty() && highlights.iter().all(|rect| rect.width() < 120.),
        "Only the matched words are highlighted: {highlights:?}"
    );
    // The field spans the panel width and has no border, even while focused.
    let field = window.ctx.read_response(filter_id).unwrap();
    assert!(field.rect.width() > 300., "{:?}", field.rect);
    assert!(window.paint.iter().all(|shape| match &shape.shape {
        Shape::Rect(rect) if rect.rect.intersects(field.rect) => rect.stroke.is_empty(),
        _ => true,
    }));

    // Enter releases focus; the filter then follows navigation.
    window.draw(Some(Key::Enter));
    assert!(!window.ctx.memory(|m| m.has_focus(filter_id)));
    window.draw(Some(Key::ArrowLeft));
    window.wait_for("01 Alpine Light.png");
    window.draw(Some(Key::ArrowRight));
    window.wait_for(text_row);
    assert!(!window.draw(None).0.contains(class_row));

    // Escape clears a filter first, keeping focus, then releases focus.
    let field = window.ctx.read_response(filter_id).unwrap();
    click_metadata_without_window_drag(&mut window, field.rect.center());
    let escape = |window: &mut TestWindow| {
        window.draw(Some(Key::Escape));
        assert!(
            !window
                .viewport_commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close)),
            "Escape in the filter must not close the window"
        );
    };
    escape(&mut window);
    window.wait_for(class_row);
    assert!(window.ctx.memory(|m| m.has_focus(filter_id)));
    escape(&mut window);
    assert!(!window.ctx.memory(|m| m.has_focus(filter_id)));
    assert!(
        window.draw(None).0.contains(class_row),
        "Metadata stays open"
    );

    // Closing the inspector starts the next one with an empty filter.
    window.draw_events(
        vec![
            key_event(Key::I, true, false, none),
            key_event(Key::I, false, false, none),
        ],
        none,
    );
    assert!(!window.draw(None).0.contains(class_row));
    window.draw(Some(Key::Tab));
    window.wait_for(class_row);
    assert!(window.draw(None).0.contains(text_row));
}

#[test]
fn metadata_font_setting_reaches_the_rendered_inspector() {
    let mut window = TestWindow::sample_with_settings(&crate::core::settings::Settings {
        metadata_font_size: 24,
        reduce_motion: true,
        ..Default::default()
    });
    window.draw(Some(Key::Tab));
    window.wait_for("314159");
    assert!(window.paint.iter().any(|shape| {
        match &shape.shape {
            Shape::Text(text) if text.galley.job.text.contains("314159") => text
                .galley
                .job
                .sections
                .iter()
                .all(|section| section.format.font_id.size == 24.),
            _ => false,
        }
    }));
}
