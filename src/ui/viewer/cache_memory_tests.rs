use super::*;
use crate::platform::system_memory::Snapshot;

const MIB: u64 = 1024 * 1024;

fn memory(available_mib: u64) -> Option<Snapshot> {
    Some(Snapshot {
        total: 16 * 1024 * MIB,
        available: available_mib * MIB,
    })
}

fn viewer() -> (egui::Context, Viewer) {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err("test".into()),
    );
    app.memory_monitor = crate::core::cache::Monitor::disabled();
    app.prefetch_settings = (
        app.settings.cache_image_limit,
        app.settings.cache_memory_mb,
        app.fit_limit,
    );
    app.files = (0..4)
        .map(|index| PathBuf::from(format!("memory-test-{index}.png")))
        .collect();
    (ctx, app)
}

fn picture(ctx: &egui::Context, index: usize) -> Picture {
    Picture {
        hdr: None,
        animation: Some(media::animation::Animation::new(
            &PathBuf::from(format!("memory-test-{index}.gif")),
            image::ImageFormat::Gif,
            2048,
            [8, 8],
            &egui::ColorImage::filled([8, 8], Color32::WHITE),
        )),
        handle: ctx.load_texture(
            format!("cache-test-{index}"),
            egui::ColorImage::filled([8, 8], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        ),
        original: [8, 8],
        requested: 2048,
        last_used: index as u64,
    }
}

#[test]
fn pressure_releases_neighbors_and_jobs_but_preserves_current_and_presented_images() {
    for spread in [false, true] {
        let (ctx, mut app) = viewer();
        app.settings.spread = spread;
        let settings = app.settings.clone();
        for index in 0..app.files.len() {
            app.pictures
                .insert(app.files[index].clone(), picture(&ctx, index));
        }
        let previous = app.pictures[&app.files[2]].handle.clone();
        let previous_id = previous.id();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        app.presentation = Presentation {
            images: vec![PresentedImage {
                hdr: None,
                handle: previous,
                rect,
                clip: rect,
                backdrop: rect,
            }],
            canvas: Some(rect),
            pan: Vec2::ZERO,
        };
        app.thumbs.insert(
            app.files[3].clone(),
            ctx.load_texture(
                "thumbnail-test",
                egui::ColorImage::filled([4, 4], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
        );
        app.apply_memory_sample(memory(12 * 1024));
        let before = app.cache_bytes();
        let path = app.files[3].clone();
        let id = app.media.request_prefetch(path.clone(), app.fit_limit);
        app.prefetch_ticket = Some((id, path.clone(), app.fit_limit));
        let id = app.media.request_thumbnail(path.clone(), 192);
        app.thumb_ticket = Some((id, path));

        app.apply_memory_sample(memory(128));

        assert_eq!(
            app.settings, settings,
            "Pressure must not rewrite portable settings"
        );
        assert_eq!(app.cache_budget(), 0);
        assert_eq!(app.pictures.len(), if spread { 2 } else { 1 });
        for index in 0..=usize::from(spread) {
            assert!(app.pictures[&app.files[index]].animation.is_some());
        }
        assert!(!app.pictures.contains_key(&app.files[2]));
        assert!(!app.pictures.contains_key(&app.files[3]));
        assert!(app.thumbs.is_empty());
        assert!(app.prefetch_ticket.is_none());
        assert!(app.thumb_ticket.is_none());
        assert!(
            app.media.try_recv().is_none(),
            "Canceled ready results must be discarded"
        );
        assert_eq!(app.presentation.images.len(), 1);
        assert_eq!(app.presentation.images[0].handle.id(), previous_id);
        assert_eq!(app.presentation.images[0].handle.byte_size(), 8 * 8 * 4);
        assert_eq!(app.presentation.images[0].rect, rect);
        assert!(app.cache_bytes() > app.cache_budget());
        assert!(app.cache_bytes() < before);

        app.queue_prefetch();
        assert!(
            app.prefetch_ticket.is_none(),
            "Pressure must prevent new background jobs"
        );
    }
}

#[test]
fn recovered_budget_retries_neighbors_rejected_under_the_smaller_budget() {
    let (ctx, mut app) = viewer();
    app.pictures.insert(app.files[0].clone(), picture(&ctx, 0));
    app.apply_memory_sample(memory(128));
    let blocked = app.files[1].clone();
    app.prefetch_blocked.insert(blocked.clone());
    for _ in 0..4 {
        app.apply_memory_sample(memory(8 * 1024));
        assert_eq!(app.cache_budget(), 0);
        assert!(app.prefetch_blocked.contains(&blocked));
        app.queue_prefetch();
        assert!(app.prefetch_ticket.is_none());
    }

    app.apply_memory_sample(memory(8 * 1024));
    assert!(app.cache_budget() > 0);
    assert!(app.prefetch_blocked.is_empty());
    // Recovery is gradual. Reserve the first display frame before prefetch;
    // animation workers are started only when their page becomes visible.
    while app.cache_budget() < app.cache_bytes() + media::prefetch_output_budget(app.fit_limit) {
        app.queue_prefetch();
        assert!(app.prefetch_ticket.is_none());
        app.apply_memory_sample(memory(8 * 1024));
    }
    app.queue_prefetch();
    assert_eq!(
        app.prefetch_ticket.as_ref().map(|ticket| &ticket.1),
        Some(&blocked)
    );
    app.cancel_prefetch();

    let recovered_budget = app.cache_budget();
    app.prefetch_blocked.insert(blocked);
    app.apply_memory_sample(memory(8 * 1024));
    assert!(app.cache_budget() > recovered_budget);
    assert!(
        app.prefetch_blocked.is_empty(),
        "Further budget growth must retry rejected neighbors"
    );
}

#[test]
fn native_detail_keeps_the_preview_and_releases_pixels_under_pressure() {
    let (ctx, mut app) = viewer();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.png");
    image::RgbaImage::from_fn(9001, 33, |x, y| image::Rgba([x as u8, y as u8, 80, 128]))
        .save(&path)
        .unwrap();
    let source = std::fs::read(&path).unwrap();
    let frame = media::decode_still(&path, 512).unwrap();
    let handle = ctx.load_texture("wide-preview", frame.image, egui::TextureOptions::LINEAR);
    app.files = vec![path.clone()];
    app.pictures.insert(
        path.clone(),
        Picture {
            hdr: None,
            animation: None,
            handle,
            original: frame.original_size,
            requested: 512,
            last_used: 0,
        },
    );
    app.settings.view_mode = "actual".into();
    app.apply_memory_sample(memory(8 * 1024));
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(5120., 2880.),
        )),
        ..Default::default()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let _ = ctx.run_pass(input.clone(), |ctx| ctx.root_ui(|ui| app.canvas(ui)));
        if app.presentation.images.len() == 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Native detail never arrived"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let base = &app.presentation.images[0];
    let detail = &app.presentation.images[1];
    assert_eq!(detail.handle.size()[1], 33);
    assert!(detail.handle.size()[0] > 4096);
    assert_eq!(base.rect.min.x.fract(), 0.);
    assert_eq!(base.rect.min.y.fract(), 0.);
    assert!((detail.rect.width() - detail.handle.size_vec2().x).abs() < 0.01);
    assert!(detail.rect.contains_rect(base.rect.intersect(base.clip)));
    assert!(
        base.handle.size()[0] < 9000,
        "The full image must remain a bounded preview"
    );
    // A reduced nonzero budget must release the previous native crop promptly.
    app.details.begin(&ctx, None, 1);
    assert_eq!(app.details.retained_bytes(), 0);
    app.apply_memory_sample(memory(128));
    let _ = ctx.run_pass(input, |ctx| ctx.root_ui(|ui| app.canvas(ui)));
    assert_eq!(app.presentation.images.len(), 1);
    assert_eq!(app.details.retained_bytes(), 0);
    assert_eq!(std::fs::read(path).unwrap(), source);
}

#[test]
fn automatic_fit_can_be_below_manual_twenty_percent_floor() {
    let (ctx, mut app) = viewer();
    let mut image = picture(&ctx, 0);
    image.original = [10000, 8000];
    app.pictures.insert(app.files[0].clone(), image);
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1080., 760.),
        )),
        ..Default::default()
    };
    let _ = ctx.run_pass(input, |ctx| {
        assert!(app.zoom_percent(ctx) < 20.);
        app.change_zoom(ctx, 0.5);
        assert_eq!(app.zoom_percent(ctx), 20.);
        app.zoom_override = None;
        assert!(app.zoom_percent(ctx) < 20.);
    });
}

#[test]
fn unavailable_memory_defers_prefetch_and_visible_thumbnail_requests() {
    let (ctx, mut app) = viewer();
    app.pictures.insert(app.files[0].clone(), picture(&ctx, 0));
    app.apply_memory_sample(None);
    assert!(app.cache_budget() > app.cache_bytes());
    app.queue_prefetch();
    assert!(app.prefetch_ticket.is_none());

    let draw_thumbnails = |app: &mut Viewer| {
        let _ = ctx.run_pass(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1080., 760.),
                )),
                ..Default::default()
            },
            |ctx| app.thumbnail_bar(ctx),
        );
    };
    draw_thumbnails(&mut app);
    draw_thumbnails(&mut app);
    assert!(
        app.thumb_ticket.is_none(),
        "Available cache space alone must not authorize background work"
    );
    assert!(app.pictures.contains_key(&app.files[0]));

    app.apply_memory_sample(memory(8 * 1024));
    draw_thumbnails(&mut app);
    assert!(
        app.thumb_ticket.is_some(),
        "A healthy sample should allow visible thumbnails again"
    );
}
