use super::*;
use crate::core::{
    disk_cache::Cache,
    documents::{ImportBatch, ProviderCache},
};
use eframe::App;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

struct Mode;
impl Mode {
    fn new(mobile: bool) -> Self {
        crate::ui::mobile::set_preview(mobile);
        Self
    }
}
impl Drop for Mode {
    fn drop(&mut self) {
        crate::ui::mobile::set_preview(false);
    }
}

#[derive(Debug)]
struct Dropped(PathBuf);
impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        panic!("the viewer must read through its media worker")
    }
}

fn input(size: Vec2, ppp: f32, insets: [f32; 4], ctx: &egui::Context) -> egui::RawInput {
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
        focused: true,
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(ppp);
    prepare_mobile_safe_area(ctx, &mut input, insets);
    input
}

fn draw(app: &mut Viewer, ctx: &egui::Context, input: egui::RawInput) {
    let mut frame = eframe::Frame::_new_kittest();
    let _ = ctx.run_ui(input, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    });
}

fn batch(root: &Path, copies: Arc<AtomicUsize>) -> ImportBatch {
    let modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(17));
    let capability = ProviderCache::new(
        b"viewport-fit-provider".to_vec(),
        modified,
        Arc::new(|| true),
    );
    let mut batch = ImportBatch::new_folder(root).unwrap();
    batch
        .add_deferred_with_cache(
            "photo.png",
            0,
            modified,
            capability,
            Arc::new(move |_, _, _| {
                copies.fetch_add(1, Ordering::Relaxed);
                Err("the matching display cache must avoid the original loader".into())
            }),
        )
        .unwrap();
    batch
}

#[test]
fn cold_first_ui_open_uses_viewport_cache_without_copying_small_original() {
    // These physical-pixel buckets differ from both the 2048 fallback and
    // each other. Mobile includes its safe area and the reserved viewer bars.
    for (mobile, size, ppp, insets, expected) in [
        (false, egui::vec2(1440., 900.), 2., [0.; 4], 3072),
        (true, egui::vec2(1024., 1366.), 2., [48., 0., 40., 0.], 2560),
    ] {
        let _mode = Mode::new(mobile);
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
        let cache = Cache::new(store.cache_dir().to_owned(), 32 * 1024 * 1024);
        let copies = Arc::new(AtomicUsize::new(0));
        let prior = batch(root.path(), copies.clone());
        let prior_directory = prior.directory().to_owned();
        cache
            .store(
                &cache
                    .key(prior.first().unwrap(), media::display_tier(expected))
                    .unwrap(),
                &egui::ColorImage::filled([2, 1], Color32::from_rgb(30, 60, 90)),
                [2, 1],
            )
            .unwrap();
        drop(prior);
        let current = batch(root.path(), copies.clone());
        assert_ne!(current.directory(), prior_directory);
        let path = current.first().unwrap().to_owned();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Ok(store),
        );
        app.memory_monitor = crate::core::cache::Monitor::disabled();
        app.settings.remember_recent = false;
        assert_eq!(
            app.fit_limit, 2048,
            "headless creation has no known viewport yet"
        );
        let mut first = input(size, ppp, insets, &ctx);
        first.dropped_files.push(Arc::new(Dropped(path.clone())));
        // This opens and submits work inside the first real ui(), before
        // canvas(). A late-only fit update misses the preseeded cache.
        draw(&mut app, &ctx, first);
        assert_eq!(app.fit_limit, expected);
        let requested = app.tickets[0]
            .as_ref()
            .map(|ticket| ticket.2)
            .or_else(|| app.pictures.get(&path).map(|picture| picture.requested));
        assert_eq!(requested, Some(expected));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.listing.is_some() || !app.pictures.contains_key(&path) {
            assert!(
                Instant::now() < deadline,
                "first cached frame failed: {:?}",
                app.failures
            );
            assert!(app.failures.is_empty());
            app.collect(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        let picture = &app.pictures[&path];
        assert_eq!(picture.requested, expected);
        assert_eq!(picture.original, [2, 1]);
        assert_eq!(picture.handle.size(), [2, 1]);
        let texture = picture.handle.id();
        assert_eq!(copies.load(Ordering::Relaxed), 0);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);

        // A larger viewport updates the bucket but never re-decodes an image
        // whose original pixels already fit in the retained display texture.
        for _ in 0..2 {
            draw(
                &mut app,
                &ctx,
                input(egui::vec2(2200., 1600.), 2., insets, &ctx),
            );
            assert_eq!(app.fit_limit, MAX_DECODE_SIDE);
            assert!(app.tickets.iter().all(Option::is_none));
            assert_eq!(app.pictures[&path].handle.id(), texture);
            assert_eq!(copies.load(Ordering::Relaxed), 0);
        }
    }
}

#[test]
fn empty_viewport_and_canvas_share_safe_area_chrome_and_pixel_scale() {
    for (mobile, size, ppp, insets, fullscreen, pinned, expected) in [
        (
            true,
            egui::vec2(390., 844.),
            3.,
            [177., 0., 102., 0.],
            false,
            false,
            2048,
        ),
        (
            true,
            egui::vec2(390., 844.),
            3.,
            [177., 0., 102., 0.],
            true,
            false,
            2560,
        ),
        (
            false,
            egui::vec2(600., 1100.),
            2.,
            [0.; 4],
            false,
            true,
            2048,
        ),
        (
            false,
            egui::vec2(600., 1100.),
            2.,
            [0.; 4],
            false,
            false,
            2560,
        ),
    ] {
        let _mode = Mode::new(mobile);
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            None,
            Err("fit fixture".into()),
        );
        app.memory_monitor = crate::core::cache::Monitor::disabled();
        app.fullscreen = fullscreen;
        app.hide_controls = !pinned;
        draw(&mut app, &ctx, input(size, ppp, insets, &ctx));
        assert!(app.files.is_empty());
        assert_eq!(app.fit_limit, expected);
        // Canvas itself must calculate the identical bucket before its empty
        // files early return, independently of the early app UI update.
        let _ = ctx.run_ui(input(size, ppp, insets, &ctx), |ui| {
            app.fit_limit = 1024;
            app.canvas(ui);
            assert_eq!(app.fit_limit, expected);
        });
        let valid = app.fit_limit;
        app.update_fit_limit(Rect::NOTHING, ppp);
        app.update_fit_limit(Rect::from_min_size(egui::Pos2::ZERO, size), f32::NAN);
        assert_eq!(app.fit_limit, valid);
    }
}

#[test]
fn initial_desktop_file_uses_known_native_size_before_first_ui() {
    let _mode = Mode::new(false);
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let path = root.path().join("small.png");
    image::RgbaImage::from_pixel(2, 1, image::Rgba([30, 60, 90, 255]))
        .save(&path)
        .unwrap();
    let ctx = egui::Context::default();
    // This is the viewport information eframe injects before AppCreator.
    ctx.input_mut(|input| {
        input.pixels_per_point = 2.;
        input
            .raw
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .inner_rect = Some(Rect::from_min_size(
            egui::pos2(100., 50.),
            egui::vec2(1440., 900.),
        ));
    });
    let app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx),
        Some(path),
        Err("fit fixture".into()),
    );
    assert_eq!(app.fit_limit, 3072);
    assert_eq!(app.tickets[0].as_ref().unwrap().2, 3072);
}
