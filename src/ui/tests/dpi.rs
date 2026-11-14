use super::*;
use crate::core::settings::Settings;

fn picture(path: &std::path::Path, native: f32, ui_zoom: f32, mode: &str) -> TestWindow {
    let mut window = TestWindow::open_with_settings(
        Some(path.to_owned()),
        &Settings {
            language: crate::i18n::Language::English,
            view_mode: mode.into(),
            ..Default::default()
        },
    );
    window.scale = native;
    window.ctx.set_zoom_factor(ui_zoom);
    window.screen_size = egui::vec2(1080., 760.) / ui_zoom;
    window.wait_for("");
    window.reveal_controls();
    window
}

fn edit_percent(window: &mut TestWindow, value: &str) {
    window.reveal_controls();
    window.click(zoom_widget(window, "zoom-value").rect.center());
    replace_zoom_text(window, value);
    window.draw(Some(Key::Enter));
    window.draw(None);
}

fn assert_os_size(window: &TestWindow, expected: egui::Vec2) {
    let (_, photo) = window.photo_rects();
    let actual = photo.size() * window.ctx.pixels_per_point() / window.scale;
    assert!(
        (actual - expected).length() < 0.001,
        "native={}, UI zoom={}: expected {expected:?} OS points, got {actual:?}",
        window.scale,
        window.ctx.zoom_factor(),
    );
}

#[test]
fn desktop_percent_follows_os_scale_independently_of_ui_zoom() {
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("400x369.png");
    image::RgbImage::new(400, 369).save(&path).unwrap();
    for native in [1., 1.25, 2.] {
        for ui_zoom in [1., 1.25, 2.] {
            let mut window = picture(&path, native, ui_zoom, "actual");
            assert_os_size(&window, egui::vec2(400., 369.));
            assert!(window.draw(None).0.contains("100%"));
            edit_percent(&mut window, "50");
            assert_os_size(&window, egui::vec2(200., 184.5));
            assert!(window.draw(None).0.contains("50%"));
            let (canvas, photo) = window.photo_rects();
            assert!(
                canvas.center().distance(photo.center()) <= 0.71 / window.ctx.pixels_per_point()
            );
            // 50% on a 2x desktop is still backing-pixel 1:1, including odd heights.
            if native == 2. {
                for coordinate in [photo.min.x, photo.min.y] {
                    let pixel = coordinate * window.ctx.pixels_per_point();
                    assert!((pixel - pixel.round()).abs() < 0.001);
                }
            }
            window.draw(Some(Key::Num1));
            window.draw(None);
            assert_os_size(&window, egui::vec2(400., 369.));
        }
    }
}

#[test]
fn explicit_percent_survives_native_monitor_and_ui_scale_changes() {
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("400x369.png");
    image::RgbImage::new(400, 369).save(&path).unwrap();
    let mut window = picture(&path, 2., 1., "actual");
    edit_percent(&mut window, "50");
    for (native, ui_zoom) in [(1.25, 1.), (1., 1.), (2., 1.25), (2., 2.)] {
        window.scale = native;
        window.ctx.set_zoom_factor(ui_zoom);
        window.screen_size = egui::vec2(1080., 760.) / ui_zoom;
        window.draw(None);
        window.draw(None);
        assert_os_size(&window, egui::vec2(200., 184.5));
        window.reveal_controls();
        assert!(window.draw(None).0.contains("50%"));
    }
}

#[test]
fn desktop_fit_geometry_and_small_image_cap_use_os_points() {
    let images = tempfile::tempdir().unwrap();
    let large = images.path().join("large.png");
    let small = images.path().join("small.png");
    image::RgbImage::new(1600, 1200).save(&large).unwrap();
    image::RgbImage::new(40, 30).save(&small).unwrap();
    for native in [1., 1.25, 2.] {
        for ui_zoom in [1., 1.25] {
            let mut window = picture(&large, native, ui_zoom, "fit");
            let (canvas, photo) = window.photo_rects();
            let expected = (canvas.width() / 1600.).min(canvas.height() / 1200.);
            assert!((photo.size() - egui::vec2(1600., 1200.) * expected).length() < 0.001);
            assert!(
                canvas.center().distance(photo.center()) <= 0.71 / window.ctx.pixels_per_point()
            );
            assert!(window.drag_pointer(photo.center(), egui::vec2(30., 20.)));
            assert_eq!(window.photo_rects(), (canvas, photo));

            let window = picture(&small, native, ui_zoom, "fit");
            // The default 400% small-image cap follows the same unit as manual zoom.
            assert_os_size(&window, egui::vec2(160., 120.));
        }
    }
}

#[test]
fn desktop_wheel_stops_at_os_scaled_hundred_percent() {
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("400x369.png");
    image::RgbImage::new(400, 369).save(&path).unwrap();
    for native in [1.25, 2.] {
        for ui_zoom in [1., 1.25] {
            let mut window = picture(&path, native, ui_zoom, "actual");
            let wheel = |direction| Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0., direction),
                modifiers: egui::Modifiers::NONE,
            };
            for (percent, direction, next) in [("90", 1., 1.25), ("110", -1., 0.8)] {
                edit_percent(&mut window, percent);
                let center = window.photo_rects().1.center();
                window.pointer(center.x, center.y);
                window.draw_events(
                    vec![wheel(direction), wheel(direction)],
                    egui::Modifiers::NONE,
                );
                window.draw(None);
                assert_os_size(&window, egui::vec2(400., 369.));
                window.draw_events(vec![wheel(direction)], egui::Modifiers::NONE);
                window.draw(None);
                assert_os_size(&window, egui::vec2(400., 369.) * next);
            }
        }
    }
}

#[test]
fn mobile_actual_size_keeps_backing_pixel_semantics() {
    let _phone = PhonePreview::enter();
    let images = tempfile::tempdir().unwrap();
    let path = images.path().join("400x369.png");
    image::RgbImage::new(400, 369).save(&path).unwrap();
    let window = picture(&path, 2., 1.25, "actual");
    let (_, photo) = window.photo_rects();
    assert!(
        (photo.size() * window.ctx.pixels_per_point() - egui::vec2(400., 369.)).length() < 0.001
    );
}
