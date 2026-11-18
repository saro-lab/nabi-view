use super::*;

struct MobilePreview;
impl MobilePreview {
    fn enable() -> Self {
        crate::ui::mobile::set_preview(true);
        Self
    }
}
impl Drop for MobilePreview {
    fn drop(&mut self) {
        crate::ui::mobile::set_preview(false);
    }
}

fn viewer() -> (egui::Context, Viewer) {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Err("prewarm fixture".into()),
    );
    app.memory_monitor = crate::core::cache::Monitor::disabled();
    app.settings.spread = false;
    app.settings.cache_image_limit = 1000;
    app.files = (0..1001)
        .map(|i| PathBuf::from(format!("warm-{i}.png")))
        .collect();
    app.apply_memory_sample(Some(crate::platform::system_memory::Snapshot {
        total: 4 * 1024 * 1024 * 1024,
        available: 2 * 1024 * 1024 * 1024,
    }));
    ready_picture(&ctx, &mut app, 0);
    (ctx, app)
}

fn ready_picture(ctx: &egui::Context, app: &mut Viewer, index: usize) {
    app.pictures.insert(
        app.files[index].clone(),
        Picture {
            hdr: None,
            animation: None,
            handle: ctx.load_texture(
                format!("ready-{index}"),
                egui::ColorImage::filled([8, 4], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
            original: [8, 4],
            requested: 4096,
            last_used: 0,
        },
    );
}

fn draw(ctx: &egui::Context, app: &mut Viewer) {
    let _ = ctx.run_pass(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(390., 844.),
            )),
            ..Default::default()
        },
        |ctx| app.thumbnail_bar(ctx),
    );
}

fn fill_visible(ctx: &egui::Context, app: &mut Viewer) {
    let visible = thumbnail_range(
        app.files.len(),
        app.focus_index,
        390. - THUMBNAIL_MARGIN * 2.,
        ctx.global_style().spacing.item_spacing.x,
    );
    for index in visible {
        app.thumbs.insert(
            app.files[index].clone(),
            ctx.load_texture(
                format!("thumb-{index}"),
                egui::ColorImage::filled([8, 4], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
        );
    }
}

#[test]
fn mobile_prewarm_clips_display_neighbors_without_changing_desktop_settings() {
    let (_, mut app) = viewer();
    let desktop = app.nearby_cache_paths();
    assert_eq!(desktop.len(), 1000);
    assert!(
        desktop.contains(app.files.last().unwrap()),
        "desktop retains wrapping behavior"
    );
    let _mobile = MobilePreview::enable();
    // Memory keeps the current page, five ahead and two behind; the rest of
    // ±10 is prepared on disk only.
    assert_eq!(app.nearby_cache_paths(), app.files[..6]);
    assert_eq!(app.settings.cache_image_limit, 1000);
    app.focus_index = 500;
    let nearby = app.nearby_cache_paths();
    assert_eq!(nearby.len(), 8);
    assert!(nearby.iter().all(|path| app.files[498..506].contains(path)));
    let disk: Vec<_> = app
        .prefetch_targets()
        .into_iter()
        .filter(|target| target.disk_only)
        .map(|target| target.index)
        .collect();
    assert_eq!(disk.len(), 13);
    assert!(disk.iter().all(|&index| (490..=510).contains(&index)));
    app.backward = true;
    assert_eq!(app.nearby_cache_paths()[1], app.files[499]);
    app.backward = false;
    app.focus_index = 1000;
    assert_eq!(app.nearby_cache_paths().len(), 3);
    assert!(!app.nearby_cache_paths().contains(&app.files[0]));
    app.settings.cache_image_limit = 3;
    assert_eq!(app.nearby_cache_paths().len(), 3);
}

#[test]
fn mobile_prewarm_prioritizes_visible_thumbnails_and_reuses_one_ticket() {
    let _mobile = MobilePreview::enable();
    let (ctx, mut app) = viewer();
    draw(&ctx, &mut app);
    assert_eq!(app.thumb_ticket.as_ref().map(|t| &t.1), Some(&app.files[1]));
    fill_visible(&ctx, &mut app);
    draw(&ctx, &mut app);
    let warm = app.thumb_ticket.clone().unwrap();
    assert!(!app.thumbs.contains_key(&warm.1));
    assert!(app.files[..20].contains(&warm.1));
    assert_ne!(warm.1, app.files[1]);
    draw(&ctx, &mut app);
    assert_eq!(
        app.thumb_ticket.as_ref(),
        Some(&warm),
        "a warm request must not restart each frame"
    );
    app.queue_prefetch();
    assert_eq!(
        app.prefetch_ticket.as_ref().map(|t| &t.1),
        Some(&app.files[1]),
        "the next page runs beside range thumbnails"
    );
    assert_eq!(app.prefetch_urgency, media::Urgency::Lead);
    app.cancel_prefetch();
    ready_picture(&ctx, &mut app, 1);
    app.queue_prefetch();
    assert!(
        app.prefetch_ticket.is_none(),
        "range thumbnails precede further display prefetch"
    );
    app.pictures.remove(&app.files[1]);

    // A newly missing visible thumbnail displaces background range work.
    app.thumbs.remove(&app.files[1]);
    draw(&ctx, &mut app);
    assert_eq!(app.thumb_ticket.as_ref().map(|t| &t.1), Some(&app.files[1]));
    assert_ne!(app.thumb_ticket.as_ref().unwrap().0, warm.0);
    fill_visible(&ctx, &mut app);
    app.thumb_errors.insert(warm.1.clone());
    draw(&ctx, &mut app);
    assert_ne!(
        app.thumb_ticket.as_ref().unwrap().1,
        warm.1,
        "failed warm thumbnails do not retry in a loop"
    );

    // Navigation cancels the old work and queues the user's page first.
    app.focus(500);
    assert!(app.thumb_ticket.is_none());
    draw(&ctx, &mut app);
    assert!(app.thumb_ticket.is_none());
    assert!(app.current_images_pending());
    app.media.clear_slot(0);
    app.tickets[0] = None;
    ready_picture(&ctx, &mut app, 500);
    fill_visible(&ctx, &mut app);
    draw(&ctx, &mut app);
    assert!(app.files[490..511].contains(&app.thumb_ticket.as_ref().unwrap().1));
}

#[test]
fn mobile_prewarm_obeys_pressure_gestures_source_reset_and_cache_clear() {
    let _mobile = MobilePreview::enable();
    let (ctx, mut app) = viewer();
    fill_visible(&ctx, &mut app);
    draw(&ctx, &mut app);
    assert!(app.thumb_ticket.is_some());
    app.touch_gesture.start(egui::pos2(200., 100.));
    draw(&ctx, &mut app);
    assert!(app.thumb_ticket.is_none());
    app.touch_gesture.cancel();
    app.apply_memory_sample(Some(crate::platform::system_memory::Snapshot {
        total: 4 * 1024 * 1024 * 1024,
        available: 0,
    }));
    draw(&ctx, &mut app);
    assert!(app.thumb_ticket.is_none());
    assert_eq!(app.cache_budget(), 0);

    app.reset_view();
    app.files = (0..40)
        .map(|i| PathBuf::from(format!("replacement-{i}.png")))
        .collect();
    ready_picture(&ctx, &mut app, 0);
    app.memory_policy = crate::core::cache::Policy::default();
    app.apply_memory_sample(Some(crate::platform::system_memory::Snapshot {
        total: 4 * 1024 * 1024 * 1024,
        available: 2 * 1024 * 1024 * 1024,
    }));
    fill_visible(&ctx, &mut app);
    draw(&ctx, &mut app);
    assert!(app.files[..20].contains(&app.thumb_ticket.as_ref().unwrap().1));
    assert!(
        app.thumb_ticket
            .as_ref()
            .unwrap()
            .1
            .to_string_lossy()
            .starts_with("replacement-")
    );
    app.thumbs.clear();
    app.media.clear_thumbnail();
    app.thumb_ticket = None;
    draw(&ctx, &mut app);
    assert_eq!(
        app.thumb_ticket.as_ref().map(|t| &t.1),
        Some(&app.files[1]),
        "cleared cache starts with visible thumbnails again"
    );
}

#[test]
fn mobile_prewarm_respects_byte_budget_and_skips_uncopied_provider_originals() {
    use std::{
        io::Write,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    let _mobile = MobilePreview::enable();
    let (ctx, mut app) = viewer();
    app.settings.cache_memory_mb = 1;
    app.pictures.get_mut(&app.files[0]).unwrap().handle = ctx.load_texture(
        "pinned-budget",
        egui::ColorImage::filled([512, 512], Color32::WHITE),
        egui::TextureOptions::LINEAR,
    );
    assert!(app.memory_policy.background_allowed());
    draw(&ctx, &mut app);
    assert!(
        app.thumb_ticket.is_none(),
        "a healthy OS sample cannot bypass the configured byte budget"
    );
    assert_eq!(app.pictures[&app.files[0]].handle.size(), [512, 512]);

    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let copies = Arc::new(AtomicUsize::new(0));
    let counted = copies.clone();
    let mut batch = crate::core::documents::ImportBatch::new_folder(root.path()).unwrap();
    batch
        .add_deferred(
            "provider.png",
            0,
            None,
            Arc::new(move |file, _, _| {
                counted.fetch_add(1, Ordering::Relaxed);
                file.write_all(b"original")
                    .map_err(|error| error.to_string())
            }),
        )
        .unwrap();
    let provider = batch.first().unwrap().to_owned();
    app.files = vec![
        app.files[0].clone(),
        provider.clone(),
        "local-neighbor.png".into(),
    ];
    assert_eq!(app.warm_thumbnail_candidate(), Some(app.files[2].clone()));
    assert_eq!(
        copies.load(Ordering::Relaxed),
        0,
        "range warming must not open an unknown provider original"
    );
    crate::core::documents::materialize(&provider, &|| true).unwrap();
    assert_eq!(
        app.warm_thumbnail_candidate(),
        Some(provider),
        "an already-owned original can warm normally"
    );
    assert_eq!(copies.load(Ordering::Relaxed), 1);
}

#[test]
fn short_steps_set_the_reading_direction_and_jumps_reset_it() {
    let _mobile = MobilePreview::enable();
    let (_, mut app) = viewer();
    app.focus_index = 500;
    app.focus(499);
    assert!(app.backward);
    assert_eq!(
        app.prefetch_targets()[0].index,
        498,
        "the lead follows the step"
    );
    assert_eq!(app.prefetch_targets()[1].index, 500);
    app.focus(500);
    assert!(!app.backward);
    assert_eq!(app.prefetch_targets()[0].index, 501);
    app.focus(498);
    assert!(app.backward, "a two-page step still counts as a step");
    let id = app.media.request_thumbnail(app.files[497].clone(), 192);
    app.thumb_ticket = Some((id, app.files[497].clone()));
    app.focus(700);
    assert!(!app.backward, "a jump resets to forward");
    assert!(
        app.thumb_ticket.is_none(),
        "a jump drops far background work"
    );
}
