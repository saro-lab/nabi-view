mod archive;
mod chrome;
mod dpi;
mod input;
mod library;
mod localization;
mod media;
mod metadata;
mod mobile;
mod mouse;
mod move_files;
mod navigation;
mod overlays;
mod rename_files;
mod selection;
mod settings;
mod touch;

use crate::{app::sample_path, storage::Store, ui::viewer::Viewer};
use eframe::{App, CreationContext, Frame, egui};
use egui::{Color32, Event, Key, Shape, epaint::ClippedShape};
use std::time::{Duration, Instant};

struct TestWindow {
    ctx: egui::Context,
    app: Viewer,
    frame: Frame,
    paint: Vec<ClippedShape>,
    texture_updates: Vec<(egui::TextureId, egui::epaint::ImageDelta)>,
    repaint_delay: Duration,
    viewport_commands: Vec<egui::ViewportCommand>,
    cursor: egui::CursorIcon,
    clipboard_writes: Vec<String>,
    time: f64,
    focused: bool,
    screen_size: egui::Vec2,
    monitor_size: Option<egui::Vec2>,
    /// Native display scale (e.g. 1.25 for 125% on Windows).
    scale: f32,
    _storage: tempfile::TempDir,
    _images: Option<tempfile::TempDir>,
}

struct PhonePreview;
impl PhonePreview {
    fn enter() -> Self {
        crate::ui::mobile::set_preview(true);
        Self
    }
}
impl Drop for PhonePreview {
    fn drop(&mut self) {
        crate::ui::mobile::set_preview(false);
    }
}

impl TestWindow {
    fn sample() -> Self {
        Self::sample_with_settings(&crate::core::settings::Settings {
            language: crate::i18n::Language::Korean,
            ..Default::default()
        })
    }

    /// The sample with the non-default gestures: the wheel turns pages, a plain
    /// drag moves the window, and Ctrl/⌘ zooms or pans instead.
    fn sample_with_modifier_gestures() -> Self {
        Self::sample_with_settings(&crate::core::settings::Settings {
            language: crate::i18n::Language::Korean,
            mouse: crate::core::settings::MouseSettings {
                wheel: "page".into(),
                primary_wheel: "zoom".into(),
                drag: "window".into(),
                primary_drag: "pan".into(),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    fn sample_with_settings(settings: &crate::core::settings::Settings) -> Self {
        // The user may add photos to the demo collection while running the app.
        // Each test watches its own exact copy of the five original fixtures.
        let originals = [
            "01 Alpine Light.png",
            "02 Pelagic.png",
            "03 Dune Forms.png",
            "04 After Dark.png",
            "05 Evergreen.png",
        ];
        let sample = sample_path().unwrap();
        let source = sample.parent().unwrap();
        let images = tempfile::tempdir().unwrap();
        for name in originals {
            std::fs::copy(source.join(name), images.path().join(name)).unwrap();
        }
        let mut window = Self::open_with_settings(Some(images.path().join(originals[0])), settings);
        window._images = Some(images);
        window
    }

    fn open(path: Option<std::path::PathBuf>) -> Self {
        Self::open_with_settings(
            path,
            &crate::core::settings::Settings {
                language: crate::i18n::Language::Korean,
                ..Default::default()
            },
        )
    }

    fn open_with_settings(
        path: Option<std::path::PathBuf>,
        settings: &crate::core::settings::Settings,
    ) -> Self {
        let storage = tempfile::tempdir().unwrap();
        let store =
            Store::in_directories(storage.path().join("config"), storage.path().join("state"));
        store.save(settings).unwrap();
        let ctx = egui::Context::default();
        let app = Viewer::new(&CreationContext::_new_kittest(ctx.clone()), path, Ok(store));
        Self {
            ctx,
            app,
            frame: Frame::_new_kittest(),
            paint: Vec::new(),
            texture_updates: Vec::new(),
            repaint_delay: Duration::MAX,
            viewport_commands: Vec::new(),
            cursor: egui::CursorIcon::Default,
            clipboard_writes: Vec::new(),
            time: 0.,
            focused: true,
            screen_size: egui::vec2(1080., 760.),
            monitor_size: Some(egui::vec2(1920., 1080.)),
            scale: 1.,
            _storage: storage,
            _images: None,
        }
    }

    fn draw(&mut self, key: Option<Key>) -> (String, bool) {
        let events = key
            .into_iter()
            .flat_map(|key| {
                [true, false].map(|pressed| key_event(key, pressed, false, egui::Modifiers::NONE))
            })
            .collect();
        self.draw_events(events, egui::Modifiers::NONE)
    }

    fn pointer(&mut self, x: f32, y: f32) -> (String, bool) {
        self.draw_events(
            vec![Event::PointerMoved(egui::pos2(x, y))],
            egui::Modifiers::NONE,
        )
    }

    fn reveal_controls(&mut self) {
        self.pointer(500., 2.);
        self.draw(None);
        // Area fade-in now follows real frame time; inspect settled paint colors.
        self.time += f64::from(self.ctx.global_style().animation_time);
        self.draw(None);
    }

    fn titlebar_visible(&self) -> bool {
        self.paint.iter().any(|s| {
            matches!(&s.shape, Shape::Rect(r)
                // Area fade-in changes the premultiplied fill color for its first frames.
                if r.fill.a() > 0
                    && r.rect.top() <= 1.
                    && r.rect.width() >= self.screen_size.x - 2.
                    && (46. ..=60.).contains(&r.rect.height()))
        })
    }

    fn titlebar_icon(&self, name: &str) -> egui::Response {
        assert!(self.titlebar_visible(), "Titlebar is not painted");
        self.ctx
            .read_response(egui::Id::unique(("titlebar", name)))
            .unwrap_or_else(|| panic!("Missing titlebar icon: {name}"))
    }

    fn click(&mut self, position: egui::Pos2) {
        self.pointer(position.x, position.y);
        for pressed in [true, false] {
            self.draw_events(
                vec![Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
        self.draw(None);
    }

    fn label_position(&self, label: &str) -> egui::Pos2 {
        self.paint
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.job.text == label => {
                    Some(t.visual_bounding_rect().center())
                }
                _ => None,
            })
            .min_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap_or_else(|| panic!("Missing label: {label}"))
    }

    fn icon_has_stroke(&self, name: &str, color: Color32) -> bool {
        let icon = self.titlebar_icon(name).rect;
        self.paint.iter().any(|s| match &s.shape {
            Shape::LineSegment { points, stroke } => {
                stroke.color == color && points.iter().all(|p| icon.contains(*p))
            }
            Shape::Rect(r) => r.stroke.color == color && icon.contains_rect(r.rect),
            Shape::Circle(c) => c.stroke.color == color && icon.contains(c.center),
            Shape::Path(p) => {
                matches!(&p.stroke.color, egui::epaint::ColorMode::Solid(c) if *c == color)
                    && p.points.iter().all(|point| icon.contains(*point))
            }
            _ => false,
        })
    }

    fn started_window_drag(&self) -> bool {
        self.viewport_commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::StartDrag))
    }

    fn drag_pointer(&mut self, start: egui::Pos2, delta: egui::Vec2) -> bool {
        self.drag_pointer_with_modifiers(start, delta, egui::Modifiers::NONE)
    }

    fn drag_pointer_with_modifiers(
        &mut self,
        start: egui::Pos2,
        delta: egui::Vec2,
        modifiers: egui::Modifiers,
    ) -> bool {
        self.draw_events(vec![Event::PointerMoved(start)], modifiers);
        let mut window_drag = self.started_window_drag();
        self.draw_events(
            vec![Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            }],
            modifiers,
        );
        window_drag |= self.started_window_drag();
        let end = start + delta;
        self.draw_events(vec![Event::PointerMoved(end)], modifiers);
        window_drag |= self.started_window_drag();
        self.draw_events(vec![], modifiers);
        window_drag |= self.started_window_drag();
        self.draw_events(
            vec![Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
        window_drag |= self.started_window_drag();
        self.draw(None);
        window_drag | self.started_window_drag()
    }

    fn is_photo_texture(&self, id: egui::TextureId) -> bool {
        id != egui::TextureId::default()
            && !crate::ui::transparency::is_backdrop(&self.ctx, id)
            && self.ctx.tex_manager().read().meta(id).is_some_and(|meta| {
                meta.name != "nabi-app-icon" && !meta.name.starts_with("library-thumbnail-")
            })
    }

    fn photo_rects(&self) -> (egui::Rect, egui::Rect) {
        self.paint
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Mesh(m) if self.is_photo_texture(m.texture_id) => {
                    Some((s.clip_rect, m.calc_bounds()))
                }
                _ => None,
            })
            .expect("Missing sample photo")
    }

    fn photo_texture(&self) -> egui::TextureId {
        self.paint
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Mesh(m) if self.is_photo_texture(m.texture_id) => Some(m.texture_id),
                _ => None,
            })
            .expect("Missing sample photo")
    }

    fn wait_for_texture_replacement(&mut self, previous: egui::TextureId) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let (_, image) = self.draw(None);
            if image && self.photo_texture() != previous {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "The displayed texture was never replaced"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn draw_events(
        &mut self,
        mut events: Vec<Event>,
        modifiers: egui::Modifiers,
    ) -> (String, bool) {
        self.time += 1. / 60.;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let mut input = egui::RawInput {
            time: Some(self.time),
            focused: self.focused,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                self.screen_size,
            )),
            events,
            ..Default::default()
        };
        let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
        viewport.monitor_size = self.monitor_size;
        viewport.native_pixels_per_point = Some(self.scale);
        // eframe gives the application the native events before egui starts its
        // focus traversal, then calls logic before ui for a visible frame.
        self.app.raw_input_hook(&self.ctx, &mut input);
        let mut output = self.ctx.run_ui(input, |ui| {
            self.app.logic(ui.ctx(), &mut self.frame);
            self.app.ui(ui, &mut self.frame);
        });
        let mut text = String::new();
        let mut image = false;
        self.texture_updates = std::mem::take(&mut output.textures_delta.set)
            .into_iter()
            .flat_map(|(id, updates)| updates.into_iter().map(move |delta| (id, delta)))
            .collect();
        self.repaint_delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        self.cursor = output.platform_output.cursor_icon;
        self.clipboard_writes
            .extend(
                output
                    .platform_output
                    .commands
                    .into_iter()
                    .filter_map(|command| {
                        if let egui::OutputCommand::CopyText(text) = command {
                            Some(text)
                        } else {
                            None
                        }
                    }),
            );
        self.viewport_commands = output
            .viewport_output
            .into_values()
            .flat_map(|viewport| viewport.commands)
            .collect();
        self.paint = output.shapes;
        for shape in &self.paint {
            match &shape.shape {
                Shape::Text(t) => {
                    text.push_str(&t.galley.job.text);
                    text.push('\n');
                }
                Shape::Mesh(m) if self.is_photo_texture(m.texture_id) => image = true,
                _ => {}
            }
        }
        (text, image)
    }

    fn assert_canvas_overlay(&self, sample_text: &str) {
        assert!(
            !self.ctx.memory(|m| m
                .areas()
                .visible_layer_ids()
                .iter()
                .any(|l| l.order == egui::Order::Middle)),
            "Metadata opened a floating window instead of drawing in the image canvas"
        );
        let (photo_index, canvas) = self
            .paint
            .iter()
            .enumerate()
            .find_map(|(i, s)| {
                matches!(&s.shape, Shape::Mesh(m) if self.is_photo_texture(m.texture_id))
                    .then_some((i, s.clip_rect))
            })
            .expect("The image must be drawn before the metadata");

        // Inspect normal-weight text and its outline before
        // tessellation, independently of the GPU and font rasterizer.
        let painted = &self.paint[photo_index + 1..];
        let (white_index, white, visible_text) = painted
            .iter()
            .enumerate()
            .find_map(|(i, s)| {
                let Shape::Text(t) = &s.shape else {
                    return None;
                };
                let bounds = t
                    .visual_bounding_rect()
                    .intersect(s.clip_rect)
                    .intersect(canvas);
                (t.galley.job.text.contains(sample_text)
                    && t.override_text_color == Some(Color32::WHITE)
                    && bounds.is_positive())
                .then_some((i, t, bounds))
            })
            .expect("Expected visible white metadata drawn over the image canvas");
        assert_eq!(
            painted
                .iter()
                .filter(|shape| matches!(&shape.shape, Shape::Text(t)
                if std::sync::Arc::ptr_eq(&t.galley, &white.galley)
                    && t.override_text_color != Some(Color32::BLACK)))
                .count(),
            1,
            "Metadata must keep one regular-weight foreground pass"
        );
        let outline: Vec<_> = painted[..white_index]
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(t)
                    if std::sync::Arc::ptr_eq(&t.galley, &white.galley)
                        && t.override_text_color == Some(Color32::BLACK) =>
                {
                    Some(t.pos - white.pos)
                }
                _ => None,
            })
            .collect();
        for direction in [
            egui::vec2(-1., 0.),
            egui::vec2(1., 0.),
            egui::vec2(0., -1.),
            egui::vec2(0., 1.),
        ] {
            assert!(
                outline
                    .iter()
                    .any(|offset| (*offset - direction).length_sq() < 0.01),
                "Black outline must surround the text, including {direction:?}"
            );
        }
        assert!(
            !painted[..white_index].iter().any(|shape| {
                matches!(&shape.shape, Shape::Rect(background)
                    if background.fill != Color32::TRANSPARENT
                        && background.rect.intersect(shape.clip_rect).contains_rect(visible_text)
                        && canvas.contains_rect(background.rect))
            }),
            "Metadata must leave the photo visible without a background panel"
        );
    }

    fn wait_for(&mut self, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let (text, image) = self.draw(None);
            if image && text.contains(label) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Timed out waiting for {label:?}; image drawn: {image}. Visible labels: {text}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn click_metadata_without_window_drag(window: &mut TestWindow, position: egui::Pos2) {
    window.pointer(position.x, position.y);
    assert!(!window.started_window_drag());
    for pressed in [true, false] {
        window.draw_events(
            vec![Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        assert!(
            !window.started_window_drag(),
            "Metadata click started a native window drag (pressed={pressed})"
        );
    }
    window.draw(None);
    assert!(!window.started_window_drag());
}

fn key_event(key: Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers,
    }
}

fn focused_thumbnail_pages(window: &TestWindow) -> Vec<usize> {
    let accent = Color32::from_rgb(204, 232, 165);
    let mut pages: Vec<_> = window
        .paint
        .iter()
        .filter_map(|shape| match &shape.shape {
            Shape::Rect(r)
                if r.fill == accent
                    && r.rect.size() == egui::vec2(69., 61.)
                    && r.rect.top() >= window.screen_size.y - 123. =>
            {
                let page = window.paint.iter().find_map(|shape| match &shape.shape {
                    Shape::Text(text) if r.rect.contains(text.visual_bounding_rect().center()) => {
                        text.galley.job.text.parse::<usize>().ok()
                    }
                    _ => None,
                });
                Some(page.expect("Focused thumbnail must show its page number"))
            }
            _ => None,
        })
        .collect();
    pages.sort_unstable();
    pages
}

fn checked_sample_pages(window: &TestWindow) -> Vec<usize> {
    let folder = window._images.as_ref().unwrap().path();
    [
        "01 Alpine Light.png",
        "02 Pelagic.png",
        "03 Dune Forms.png",
        "04 After Dark.png",
        "05 Evergreen.png",
    ]
    .iter()
    .enumerate()
    .filter_map(|(index, filename)| {
        window
            .ctx
            .read_response(egui::Id::unique(("thumbnail-check", folder.join(filename))))
            .map(|badge| {
                assert!(
                    window.paint.iter().any(|shape| matches!(&shape.shape,
                        Shape::Path(path)
                            if path.points.len() == 3
                                && !path.closed
                                && path.points.iter().all(|point| badge.rect.contains(*point))
                                && path.points[1].y > path.points[0].y
                                && path.points[1].y > path.points[2].y
                    )),
                    "A checked thumbnail must paint a checkmark, not just a border"
                );
                index + 1
            })
    })
    .collect()
}

fn settle_checks(window: &mut TestWindow) {
    // read_response may still return the previous frame's removed badge.
    window.draw(None);
    window.draw(None);
}

fn shift_navigation(window: &mut TestWindow, key: Key) -> String {
    let modifiers = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    window
        .draw_events(
            [true, false]
                .map(|pressed| key_event(key, pressed, false, modifiers))
                .into(),
            modifiers,
        )
        .0
}

fn saved_viewer_settings(window: &TestWindow) -> crate::core::settings::Settings {
    Store::in_directories(
        window._storage.path().join("config"),
        window._storage.path().join("state"),
    )
    .load()
    .unwrap()
}

fn assert_image_uv(mesh: &egui::epaint::Mesh, horizontal: bool, vertical: bool) {
    let bounds = mesh.calc_bounds();
    let top_left = mesh.vertices.iter().find(|v| v.pos == bounds.min).unwrap();
    let bottom_right = mesh.vertices.iter().find(|v| v.pos == bounds.max).unwrap();
    assert_eq!(
        top_left.uv,
        egui::pos2(
            if horizontal { 1. } else { 0. },
            if vertical { 1. } else { 0. }
        )
    );
    assert_eq!(
        bottom_right.uv,
        egui::pos2(
            if horizontal { 0. } else { 1. },
            if vertical { 0. } else { 1. }
        )
    );
}

fn assert_canvas_flips(window: &TestWindow, horizontal: bool, vertical: bool) -> usize {
    let mut count = 0;
    for shape in &window.paint {
        if let Shape::Mesh(mesh) = &shape.shape {
            if crate::ui::transparency::is_backdrop(&window.ctx, mesh.texture_id) {
                continue;
            }
            if window.is_photo_texture(mesh.texture_id) && shape.clip_rect.height() > 200. {
                assert_image_uv(mesh, horizontal, vertical);
                count += 1;
            }
        }
    }
    assert!(count > 0, "No canvas image was painted");
    count
}

fn zoom_widget(window: &TestWindow, name: &str) -> egui::Response {
    window
        .ctx
        .read_response(egui::Id::unique(name))
        .unwrap_or_else(|| panic!("Missing zoom widget: {name}"))
}

fn replace_zoom_text(window: &mut TestWindow, text: &str) {
    let modifiers = egui::Modifiers {
        command: true,
        ctrl: true,
        ..Default::default()
    };
    window.draw_events(
        vec![
            key_event(Key::A, true, false, modifiers),
            key_event(Key::A, false, false, modifiers),
        ],
        modifiers,
    );
    window.draw_events(vec![Event::Text(text.into())], egui::Modifiers::NONE);
}

fn assert_photo_scale(window: &TestWindow, expected: f32) {
    let original = image::image_dimensions(sample_path().unwrap()).unwrap();
    let (_, photo) = window.photo_rects();
    let ratio = photo.width() * window.ctx.pixels_per_point() / window.scale / original.0 as f32;
    assert!(
        (ratio - expected).abs() < 0.0001,
        "Expected scale {expected}, got {ratio}"
    );
}

fn choose_shortcut_action(window: &mut TestWindow, current_title: &str, action_title: &str) {
    let current = window
        .paint
        .iter()
        .rev()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.job.text == current_title => {
                Some(text.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("Missing action selector in shortcut editor");
    window.click(current);
    for _ in 0..24 {
        // Let smooth wheel scrolling finish before the click helper adds its
        // pointer-move frame; otherwise the row can move under that pointer.
        for _ in 0..24 {
            window.draw(None);
        }
        if let Some(position) = window
            .paint
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text)
                    if text.galley.job.text == action_title
                        && shape.clip_rect.height() > 40.
                        && shape.clip_rect.height() < 240. =>
                {
                    let bounds = text.visual_bounding_rect();
                    shape
                        .clip_rect
                        .contains_rect(bounds)
                        .then_some(bounds.center())
                }
                _ => None,
            })
        {
            window.click(position);
            return;
        }
        // The function list is deliberately scrollable. Reach later commands
        // with real wheel input rather than clicking text outside its clip.
        let popup = window
            .paint
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text)
                    if shape.clip_rect.height() > 40.
                        && shape.clip_rect.height() < 240.
                        && shape.clip_rect.intersects(text.visual_bounding_rect()) =>
                {
                    Some(shape.clip_rect)
                }
                _ => None,
            })
            .expect("Missing action popup scroll region");
        window.pointer(popup.center().x, popup.center().y);
        window.draw_events(
            vec![Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., -60.),
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        );
        window.time += 0.2;
    }
    panic!("Could not scroll to shortcut action {action_title}");
}

fn add_shortcut_row(window: &mut TestWindow, key: Key, action_title: &str) {
    record_new_shortcut_row(window, key, action_title);
    window.click(window.label_position("Save shortcut"));
}

fn record_new_shortcut_row(window: &mut TestWindow, key: Key, action_title: &str) {
    window.click(window.label_position("+ Add"));
    window.wait_for("Add shortcut");
    choose_shortcut_action(window, "Open image", action_title);
    window.click(window.label_position("Record key combination"));
    window.draw(Some(key));
}

fn shortcut_row(window: &TestWindow, index: usize) -> egui::Response {
    window
        .ctx
        .read_response(egui::Id::unique(("shortcut-row", index)))
        .unwrap_or_else(|| panic!("Missing shortcut row {index}"))
}

fn close_settings_screen(window: &mut TestWindow) {
    let close = window
        .ctx
        .read_response(egui::Id::unique(("titlebar", "settings-close")))
        .expect("Missing settings close icon");
    window.click(close.rect.center());
    window.time += 0.25;
    window.draw(None);
}
