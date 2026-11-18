use super::*;
use eframe::App;
use std::time::{Duration, Instant};

struct Preview;

impl Drop for Preview {
    fn drop(&mut self) {
        crate::ui::mobile::set_preview(false);
    }
}

fn window() -> (tempfile::TempDir, Viewer, egui::Context, PathBuf) {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let directory = root.path().join("images");
    std::fs::create_dir(&directory).unwrap();
    let image = directory.join("page.png");
    image::RgbImage::from_pixel(8, 4, image::Rgb([220, 50, 50]))
        .save(&image)
        .unwrap();
    let image = crate::core::paths::canonicalize(&image).unwrap();
    let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
    store.add_bookmark(&image, "Cached bookmark").unwrap();
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Ok(store),
    );
    app.memory_monitor = crate::core::cache::Monitor::disabled();
    (root, app, ctx, image)
}

fn draw(app: &mut Viewer, ctx: &egui::Context, time: &mut f64) -> egui::FullOutput {
    *time += 1. / 60.;
    let mut input = egui::RawInput {
        time: Some(*time),
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            if crate::app::is_mobile() {
                egui::vec2(390., 844.)
            } else {
                egui::vec2(1080., 760.)
            },
        )),
        focused: true,
        ..Default::default()
    };
    app.raw_input_hook(ctx, &mut input);
    let mut frame = eframe::Frame::_new_kittest();
    ctx.run_ui(input, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    })
}

fn painted_thumbnail(output: &egui::FullOutput, ctx: &egui::Context) -> Option<egui::TextureId> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Mesh(mesh)
            if ctx
                .tex_manager()
                .read()
                .meta(mesh.texture_id)
                .is_some_and(|meta| meta.name.starts_with("library-thumbnail-")) =>
        {
            Some(mesh.texture_id)
        }
        _ => None,
    })
}

fn wait_thumbnail(app: &mut Viewer, ctx: &egui::Context, time: &mut f64) -> egui::TextureId {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(id) = painted_thumbnail(&draw(app, ctx, time), ctx) {
            return id;
        }
        assert!(Instant::now() < deadline, "thumbnail was not drawn");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn reopening_library_reuses_texture_in_first_frame_and_rechecks_image_key() {
    for mobile in [false, true] {
        crate::ui::mobile::set_preview(mobile);
        let _preview = Preview;
        let (_root, mut app, ctx, image) = window();
        let mut time = 0.;
        let first = wait_thumbnail(&mut app, &ctx, &mut time);
        let original_key = library::thumbnail_key(&image).unwrap();
        assert!(app.library.thumbnails.textures.contains_key(&original_key));
        assert_eq!(app.cache_bytes(), 8 * 4 * 4);

        if mobile {
            // Mobile recent is now explicit home navigation. The library's
            // close control still returns to the retained viewer.
            app.library.open = false;
        } else {
            app.run("recent", &ctx);
        }
        for _ in 0..3 {
            draw(&mut app, &ctx, &mut time);
        }
        assert!(!app.library.open);
        assert!(app.library.thumbnails.worker.is_none());
        assert_eq!(app.library.thumbnails.textures[&original_key].0.id(), first);

        app.run("recent", &ctx);
        let reopened = draw(&mut app, &ctx, &mut time);
        assert_eq!(painted_thumbnail(&reopened, &ctx), Some(first));
        assert!(
            !reopened
                .textures_delta
                .set
                .iter()
                .any(|(id, _)| *id == first),
            "a warm visit must not upload the same thumbnail again"
        );
        assert!(app.library.thumbnails.worker.is_none());

        if mobile {
            app.library.open = false;
        } else {
            app.run("recent", &ctx);
        }
        draw(&mut app, &ctx, &mut time);
        // Different dimensions guarantee a changed file size and thumbnail key,
        // even on filesystems with coarse modification timestamp precision.
        image::RgbImage::from_pixel(31, 17, image::Rgb([30, 180, 80]))
            .save(&image)
            .unwrap();
        let changed_key = library::thumbnail_key(&image).unwrap();
        assert_ne!(original_key, changed_key);
        app.run("recent", &ctx);
        // The old representative may remain in the row while the changed image
        // is read. It must neither occupy the new key nor prevent its request.
        let deadline = Instant::now() + Duration::from_secs(8);
        let changed = loop {
            let output = draw(&mut app, &ctx, &mut time);
            if let Some((texture, _)) = app.library.thumbnails.textures.get(&changed_key) {
                assert_ne!(
                    texture.id(),
                    first,
                    "old pixels were aliased under the new key"
                );
                if painted_thumbnail(&output, &ctx) == Some(texture.id()) {
                    break texture.id();
                }
            }
            assert!(
                Instant::now() < deadline,
                "existing fallback prevented the changed image from appearing"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_ne!(changed, first);
        assert_eq!(
            app.library.thumbnails.textures[&changed_key].0.id(),
            changed
        );
        assert_eq!(
            app.library.items[0].entry.thumbnail.as_deref(),
            Some(changed_key.as_str())
        );
        app.reload_library();
        assert!(!app.library.thumbnails.textures.contains_key(&original_key));
    }
}

fn ready_worker(count: usize, edge: usize) -> Worker {
    ready_worker_for((0..count).map(|index| format!("ready-{index:03}")), edge)
}

fn ready_worker_for(keys: impl IntoIterator<Item = String>, edge: usize) -> Worker {
    let (jobs, _inbox) = mpsc::channel();
    let (outbox, done) = mpsc::channel();
    let mut pending = 0;
    for shown in keys {
        outbox
            .send(Done {
                shown,
                owner: (Kind::Bookmark, PathBuf::from("test")),
                result: Ok(Thumbnail {
                    pixels: egui::ColorImage::filled([edge, edge], Color32::WHITE),
                    written: None,
                    fallback: None,
                }),
            })
            .unwrap();
        pending += 1;
    }
    Worker {
        jobs,
        done,
        pending,
        limit: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1024)),
    }
}

#[test]
fn library_completed_bursts_obey_count_and_memory_limits() {
    let (_root, mut app, ctx, _image) = window();
    app.library.open = false;
    app.library.thumbnails.worker = Some(ready_worker(TEXTURE_LIMIT + 9, 4));
    app.poll_library_thumbnails(&ctx);
    assert_eq!(app.library.thumbnails.textures.len(), TEXTURE_LIMIT);
    assert!(app.library.thumbnails.worker.is_none());
    assert_eq!(app.cache_bytes(), TEXTURE_LIMIT * 4 * 4 * 4);

    app.library.clear_thumbnails();
    app.settings.cache_memory_mb = 32;
    // Accepted legacy cache entries can be larger than freshly made 256px
    // thumbnails, so a texture count alone is not a sufficient memory bound.
    app.library.thumbnails.worker = Some(ready_worker(12, 1024));
    app.poll_library_thumbnails(&ctx);
    assert!(app.cache_bytes() <= 32 * 1024 * 1024);
    assert_eq!(app.library.thumbnails.textures.len(), 8);

    // LRU retention uses the latest frame that actually displayed the row.
    let newest = "ready-007";
    app.library.thumbnails.textures.get_mut(newest).unwrap().1 = 100;
    app.library.trim_thumbnails(4 * 1024 * 1024);
    assert!(app.library.thumbnails.textures.contains_key(newest));
}

#[test]
fn library_pressure_and_explicit_clear_drop_retained_and_queued_pixels() {
    let (_root, mut app, ctx, _image) = window();
    let mut time = 0.;
    let _ = wait_thumbnail(&mut app, &ctx, &mut time);
    app.run("recent", &ctx);
    draw(&mut app, &ctx, &mut time);
    assert!(app.cache_bytes() > 0);

    app.library.thumbnails.worker = Some(ready_worker(1, 4));
    app.apply_memory_sample(Some(crate::platform::system_memory::Snapshot {
        total: 16 * 1024 * 1024 * 1024,
        available: 128 * 1024 * 1024,
    }));
    assert_eq!(app.cache_budget(), 0);
    assert_eq!(app.cache_bytes(), 0);
    assert!(app.library.thumbnails.worker.is_none());
    app.poll_library_thumbnails(&ctx);
    app.run("recent", &ctx);
    draw(&mut app, &ctx, &mut time);
    assert!(app.library.thumbnails.worker.is_none());
    assert!(app.library.thumbnails.textures.is_empty());

    for _ in 0..5 {
        app.apply_memory_sample(Some(crate::platform::system_memory::Snapshot {
            total: 16 * 1024 * 1024 * 1024,
            available: 12 * 1024 * 1024 * 1024,
        }));
    }
    let _ = wait_thumbnail(&mut app, &ctx, &mut time);
    app.library.thumbnails.worker = Some(ready_worker(1, 4));
    let limit = app
        .library
        .thumbnails
        .worker
        .as_ref()
        .unwrap()
        .limit
        .clone();
    app.library.clear_thumbnails();
    assert_eq!(limit.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(app.library.thumbnails.worker.is_none());
    app.poll_library_thumbnails(&ctx);
    assert_eq!(app.cache_bytes(), 0);
    assert!(app.library.thumbnails.textures.is_empty());
}

#[test]
fn library_lru_does_not_count_dialog_preview_as_freed() {
    let (_root, mut app, ctx, image) = window();
    let preview = ctx.load_texture(
        "dialog-preview",
        egui::ColorImage::filled([4, 4], Color32::WHITE),
        egui::TextureOptions::LINEAR,
    );
    app.library
        .thumbnails
        .textures
        .insert("old-preview".into(), (preview.clone(), 0));
    app.library.thumbnails.textures.insert(
        "newer-preview".into(),
        (
            ctx.load_texture(
                "newer-preview",
                egui::ColorImage::filled([4, 4], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
            1,
        ),
    );
    app.library.dialog = Some(BookmarkDialog {
        folder: image.parent().unwrap().to_owned(),
        image,
        name: "Rename".into(),
        preview: Some(preview),
        rename: true,
        replaces: false,
        focus: false,
        error: String::new(),
    });
    assert_eq!(app.cache_bytes(), 2 * 4 * 4 * 4);
    assert_eq!(app.library.trim_thumbnails(4 * 4 * 4), 4 * 4 * 4);
    assert!(app.library.thumbnails.textures.is_empty());
    assert_eq!(app.cache_bytes(), 4 * 4 * 4);
    assert!(app.library.dialog.as_ref().unwrap().preview.is_some());
}

#[test]
fn library_waits_for_memory_without_redecoding_rejected_thumbnails() {
    let (_root, mut app, ctx, image) = window();
    app.settings.cache_memory_mb = 32;
    app.files = vec![image.clone()];
    let key = library::thumbnail_key(&image).unwrap();
    let pin = |app: &mut Viewer, height| {
        app.pictures.insert(
            image.clone(),
            Picture {
                handle: ctx.load_texture(
                    "pinned-image",
                    egui::ColorImage::filled([2048, height], Color32::WHITE),
                    egui::TextureOptions::LINEAR,
                ),
                original: [2048, height as u32],
                requested: 2048,
                last_used: 0,
                hdr: None,
                animation: None,
            },
        );
    };
    pin(&mut app, 4096);
    app.request_thumbnails(&ctx, &[0]);
    assert!(app.library.thumbnails.worker.is_none());

    // A few spare bytes are not enough to reserve a normal thumbnail either.
    pin(&mut app, 4095);
    assert!(app.cache_bytes() < app.cache_budget());
    app.request_thumbnails(&ctx, &[0]);
    assert!(app.library.thumbnails.worker.is_none());

    // The page may grow after a request was queued. Legacy cached thumbnails
    // can also exceed the usual reservation; retain only their required size.
    pin(&mut app, 3968);
    assert_eq!(app.cache_budget() - app.cache_bytes(), 1024 * 1024);
    let (requests, requested) = mpsc::channel();
    let mut completed = ready_worker_for([key.clone()], 1024);
    completed.jobs = requests;
    app.library.thumbnails.worker = Some(completed);
    app.poll_library_thumbnails(&ctx);
    assert!(app.library.thumbnails.textures.is_empty());
    assert_eq!(app.library.thumbnails.required_bytes[&key], 4 * 1024 * 1024);
    for _ in 0..20 {
        app.request_thumbnails(&ctx, &[0]);
        app.poll_library_thumbnails(&ctx);
        assert!(app.library.thumbnails.requested.is_empty());
        assert_eq!(app.library.thumbnails.worker.as_ref().unwrap().pending, 0);
        assert!(requested.try_recv().is_err());
    }

    app.library.open = false;
    app.poll_library_thumbnails(&ctx);
    assert!(app.library.thumbnails.worker.is_none());
    app.library.open = true;
    app.pictures.clear();
    app.request_thumbnails(&ctx, &[0]);
    assert!(app.library.thumbnails.requested.contains(&key));
    let deadline = Instant::now() + Duration::from_secs(8);
    while !app.library.thumbnails.textures.contains_key(&key) {
        assert!(
            Instant::now() < deadline,
            "thumbnail did not recover after memory was released"
        );
        app.poll_library_thumbnails(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.library.thumbnails.required_bytes.contains_key(&key));
}

#[test]
fn new_visible_rows_reclaim_offscreen_thumbnails_then_neighbor_images() {
    for neighbors in [false, true] {
        let (_root, mut app, ctx, image) = window();
        app.settings.cache_memory_mb = 32;
        app.files = vec![image.clone()];
        let picture = |name: &str, width: usize, height: usize| Picture {
            handle: ctx.load_texture(
                name,
                egui::ColorImage::filled([width, height], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
            original: [width as u32, height as u32],
            requested: 2048,
            last_used: 0,
            hdr: None,
            animation: None,
        };
        app.pictures
            .insert(image.clone(), picture("current", 2048, 2048));
        let current = app.pictures[&image].handle.id();
        let neighbor = image.with_file_name("neighbor.png");
        if neighbors {
            app.files.push(neighbor.clone());
            app.pictures
                .insert(neighbor.clone(), picture("neighbor", 2048, 2016));
        }
        let count = if neighbors { 1 } else { TEXTURE_LIMIT };
        for index in 0..count {
            app.library.thumbnails.textures.insert(
                format!("retained-{index:02}"),
                (
                    ctx.load_texture(
                        format!("retained-{index:02}"),
                        egui::ColorImage::filled([256, 256], Color32::WHITE),
                        egui::TextureOptions::LINEAR,
                    ),
                    index as u64,
                ),
            );
        }
        let shown_key = "retained-00";
        let shown_id = app.library.thumbnails.textures[shown_key].0.id();
        let mut visible_entry = app.library.items[0].entry.clone();
        visible_entry.name = Some("Already visible".into());
        app.library.items.push(Item {
            entry: visible_entry,
            key: Some(shown_key.into()),
        });
        if !neighbors {
            app.library.dialog = Some(BookmarkDialog {
                folder: image.parent().unwrap().to_owned(),
                image: image.clone(),
                name: "Rename".into(),
                preview: Some(app.library.thumbnails.textures["retained-01"].0.clone()),
                rename: true,
                replaces: false,
                focus: false,
                error: String::new(),
            });
        }
        assert_eq!(app.cache_bytes(), app.cache_budget());

        // The new row precedes a cached visible row. Collect every visible key
        // before eviction, rather than accidentally evicting a later row.
        app.request_thumbnails(&ctx, &[0, 1]);
        let key = library::thumbnail_key(&image).unwrap();
        assert!(app.library.thumbnails.requested.contains(&key));
        assert_eq!(app.pictures[&image].handle.id(), current);
        assert_eq!(app.library.thumbnails.textures[shown_key].0.id(), shown_id);
        if neighbors {
            assert!(!app.pictures.contains_key(&neighbor));
        } else {
            assert!(app.library.thumbnails.textures.contains_key("retained-01"));
            assert!(!app.library.thumbnails.textures.contains_key("retained-02"));
            assert_eq!(app.library.thumbnails.textures.len(), TEXTURE_LIMIT - 1);
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while !app.library.thumbnails.textures.contains_key(&key) {
            assert!(
                Instant::now() < deadline,
                "new visible row was starved by old cache"
            );
            app.poll_library_thumbnails(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.cache_bytes() <= app.cache_budget());
        assert_eq!(app.library.thumbnails.textures[shown_key].0.id(), shown_id);
    }
}

#[test]
fn unavailable_page_keeps_the_old_row_thumbnail_under_its_own_key() {
    let (root, mut app, ctx, image) = window();
    let old_image = root.path().join("old-page.png");
    image::RgbImage::from_pixel(16, 8, image::Rgb([230, 30, 40]))
        .save(&old_image)
        .unwrap();
    let old_key = library::thumbnail_key(&old_image).unwrap();
    let store = app.store.as_ref().unwrap();
    library::create_thumbnail(&old_image, &store.thumbnail_dir(), &old_key, || {
        64 * 1024 * 1024
    })
    .unwrap();
    let folder = app.library.items[0].entry.folder.clone();
    store
        .set_thumbnail(Kind::Bookmark, &folder, &old_key)
        .unwrap();
    app.library.items[0].entry.thumbnail = Some(old_key.clone());
    let requested_key = app.library.items[0].key.clone().unwrap();
    std::fs::write(&image, b"unreadable PNG").unwrap();
    app.request_thumbnails(&ctx, &[0]);
    let deadline = Instant::now() + Duration::from_secs(8);
    while !app.library.thumbnails.textures.contains_key(&old_key) {
        assert!(
            Instant::now() < deadline,
            "offline representative thumbnail was not loaded"
        );
        app.poll_library_thumbnails(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    let old_id = app.library.thumbnails.textures[&old_key].0.id();
    assert!(!app.library.thumbnails.textures.contains_key(&requested_key));
    assert_eq!(
        app.library.thumbnail(&app.library.items[0]).unwrap().id(),
        old_id
    );
    assert!(app.library.thumbnails.failed.contains(&requested_key));
    for _ in 0..20 {
        app.request_thumbnails(&ctx, &[0]);
        assert_eq!(app.library.thumbnails.worker.as_ref().unwrap().pending, 0);
    }

    app.library.open = false;
    app.poll_library_thumbnails(&ctx);
    image::RgbImage::from_pixel(20, 12, image::Rgb([20, 80, 210]))
        .save(&image)
        .unwrap();
    app.reload_library();
    app.library.open = true;
    let repaired_key = app.library.items[0].key.clone().unwrap();
    assert_eq!(
        app.library.thumbnail(&app.library.items[0]).unwrap().id(),
        old_id
    );
    app.request_thumbnails(&ctx, &[0]);
    let deadline = Instant::now() + Duration::from_secs(8);
    while !app.library.thumbnails.textures.contains_key(&repaired_key) {
        assert!(
            Instant::now() < deadline,
            "repaired source was not retried on reopen"
        );
        app.poll_library_thumbnails(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_ne!(
        app.library.thumbnail(&app.library.items[0]).unwrap().id(),
        old_id
    );
    assert_eq!(
        app.library.items[0].entry.thumbnail.as_deref(),
        Some(repaired_key.as_str())
    );
}
