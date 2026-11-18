use super::*;
use crate::core::{
    documents::{ImportBatch, ProviderCache},
    ios_access::PhotoStatus,
};
use eframe::App;
use image::ImageEncoder;
use std::{
    io::Write,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

struct Phone;
impl Drop for Phone {
    fn drop(&mut self) {
        crate::ui::mobile::set_preview(false);
    }
}

fn imported(root: &Path, pages: &[(&str, u64)], requested: Option<&Path>) -> ImportBatch {
    let mut bytes = Vec::new();
    let pixels = image::RgbImage::from_pixel(512, 256, image::Rgb([30, 100, 220]));
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pixels, 512, 256, image::ExtendedColorType::Rgb8)
        .unwrap();
    let mut batch = ImportBatch::new_folder(root).unwrap();
    for &(name, revision) in pages {
        let bytes = bytes.clone();
        let modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(revision));
        let identity =
            ProviderCache::new(name.as_bytes().to_vec(), modified, Arc::new(|| true)).unwrap();
        batch
            .add_deferred_with_cache(
                name,
                bytes.len() as u64,
                modified,
                Some(identity),
                Arc::new(move |output, _, _| {
                    output.write_all(&bytes).map_err(|error| error.to_string())
                }),
            )
            .unwrap();
    }
    batch
        .set_source(&root.join("document-sources"), "nabi-ios-library", "Photos")
        .unwrap();
    batch.commit_source().unwrap();
    batch.request_page(requested.map(Path::to_owned));
    batch
}

fn fixture() -> (
    tempfile::TempDir,
    Viewer,
    egui::Context,
    ImportBatch,
    PathBuf,
    egui::TextureId,
) {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let batch = imported(root.path(), &[("first.png", 7)], None);
    let target = batch.source().unwrap().root.join("first.png");
    let store = Store::in_directories(root.path().join("config"), root.path().join("state"));
    store.add_bookmark(&target, "Photos").unwrap();
    let ctx = egui::Context::default();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Ok(store),
    );
    app.memory_monitor = crate::core::cache::Monitor::disabled();
    app.deferred_mobile_open_requests = Some(Vec::new());
    app.library.preview_test_status = Some(PhotoStatus::Authorized);
    let key = app.library.items[0].shown().unwrap().to_owned();
    let texture = ctx.load_texture(
        "old-representative-thumbnail",
        egui::ColorImage::filled([256, 128], Color32::RED),
        egui::TextureOptions::LINEAR,
    );
    let id = texture.id();
    app.library.thumbnails.textures.insert(key, (texture, 0));
    (root, app, ctx, batch, target, id)
}

fn load(app: &mut Viewer, ctx: &egui::Context, batch: ImportBatch) {
    app.accept_mobile_selection(batch);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.listing.is_some() || app.pictures.is_empty() {
        assert!(
            Instant::now() < deadline,
            "display image did not load: {}",
            app.last_error
        );
        app.collect(ctx);
        app.queue_images();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn draw(
    app: &mut Viewer,
    ctx: &egui::Context,
    time: &mut f64,
    size: Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    *time += 1. / 60.;
    let mut input = egui::RawInput {
        time: Some(*time),
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        focused: true,
        ..Default::default()
    };
    app.raw_input_hook(ctx, &mut input);
    prepare_mobile_safe_area(ctx, &mut input, [0., 59., 0., 34.]);
    let mut frame = eframe::Frame::_new_kittest();
    ctx.run_ui(input, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    })
}

fn paints_large(output: &egui::FullOutput, id: egui::TextureId) -> bool {
    output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == id && mesh.calc_bounds().width() > 250.))
}

#[test]
fn cold_library_tap_never_enlarges_a_stale_representative_thumbnail() {
    crate::ui::mobile::set_preview(true);
    let _phone = Phone;
    for size in [egui::vec2(390., 844.), egui::vec2(1180., 820.)] {
        let (_root, mut app, ctx, batch, target, old_thumbnail) = fixture();
        // remember_page updates image without changing the previous thumbnail.
        // A cold source has no current image key, so the row legitimately falls
        // back to that representative image; the viewer must never enlarge it.
        let key = app.library.items[0].shown().unwrap().to_owned();
        let texture = app.library.thumbnails.textures.remove(&key).unwrap();
        app.library.items[0].key = None;
        app.library.items[0].entry.thumbnail = Some("old-page-thumbnail".into());
        app.library
            .thumbnails
            .textures
            .insert("old-page-thumbnail".into(), texture);
        assert_eq!(
            app.library.items[0].entry.image.as_deref(),
            Some(target.as_path())
        );
        let mut time = 0.;
        for _ in 0..3 {
            draw(&mut app, &ctx, &mut time, size, Vec::new());
        }
        let row = ctx
            .read_response(egui::Id::unique((
                "bookmark-folder".to_owned(),
                target.parent().unwrap(),
            )))
            .unwrap();
        let pos = row.rect.center();
        draw(
            &mut app,
            &ctx,
            &mut time,
            size,
            vec![egui::Event::PointerMoved(pos)],
        );
        for pressed in [true, false] {
            draw(
                &mut app,
                &ctx,
                &mut time,
                size,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(
            app.deferred_mobile_open_requests.as_ref().unwrap(),
            &vec![target]
        );
        for _ in 0..20 {
            let output = draw(&mut app, &ctx, &mut time, size, Vec::new());
            assert!(!app.library.opening_preview_active());
            assert!(app.library.open);
            assert!(!paints_large(&output, old_thumbnail));
            assert!(!app.default_menu_visible());
        }
        load(&mut app, &ctx, batch);
        let output = draw(&mut app, &ctx, &mut time, size, Vec::new());
        assert!(!paints_large(&output, old_thumbnail));
        assert!(!app.library.open);
    }
}

#[test]
fn display_preview_requires_an_explicit_page_full_access_and_exact_display_pixels() {
    let (_root, mut app, ctx, batch, target, thumbnail) = fixture();
    assert!(
        app.library_preview_for_open(0, &target, PhotoStatus::Authorized)
            .is_none()
    );
    load(&mut app, &ctx, batch);
    app.library.open = true;
    for status in [
        PhotoStatus::NotDetermined,
        PhotoStatus::Denied,
        PhotoStatus::Restricted,
        PhotoStatus::Limited,
    ] {
        assert!(app.library_preview_for_open(0, &target, status).is_none());
    }
    for rejected in [
        target.parent().unwrap().to_owned(),
        target.with_file_name("other.png"),
    ] {
        assert!(
            app.library_preview_for_open(0, &rejected, PhotoStatus::Authorized)
                .is_none()
        );
    }
    let display = app.pictures[&app.files[app.focus_index]].handle.id();
    let before = app.cache_bytes();
    let preview = app
        .library_preview_for_open(0, &target, PhotoStatus::Authorized)
        .unwrap();
    assert_eq!(preview.texture.id(), display);
    assert_ne!(preview.texture.id(), thumbnail);
    app.start_library_preview(Some(preview), &ctx);
    assert_eq!(app.cache_bytes(), before);
    app.clear_library_preview_for_source(&target.with_file_name("other.png"));
    assert!(!app.library.opening_preview_active());
    assert!(app.library.open);
}

#[test]
fn exact_display_preview_preserves_transform_safe_area_and_read_only_controls() {
    crate::ui::mobile::set_preview(true);
    let _phone = Phone;
    for size in [egui::vec2(390., 844.), egui::vec2(1180., 820.)] {
        let (_root, mut app, ctx, batch, target, _) = fixture();
        load(&mut app, &ctx, batch);
        app.library.open = true;
        let mut time = 0.;
        for _ in 0..3 {
            draw(&mut app, &ctx, &mut time, size, Vec::new());
        }
        app.settings.rotation_quarter_turns = 1;
        app.settings.flip_horizontal = true;
        let preview = app
            .library_preview_for_open(0, &target, PhotoStatus::Authorized)
            .unwrap();
        let display = preview.texture.id();
        app.start_library_preview(Some(preview), &ctx);
        let output = draw(&mut app, &ctx, &mut time, size, Vec::new());
        let mesh = output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh)
                    if mesh.texture_id == display && mesh.calc_bounds().height() > 300. =>
                {
                    Some(mesh)
                }
                _ => None,
            })
            .unwrap();
        let bounds = mesh.calc_bounds();
        assert!((bounds.width() / bounds.height() - 0.5).abs() < 0.001);
        let corner = |pos| {
            mesh.vertices
                .iter()
                .find(|vertex| vertex.pos == pos)
                .unwrap()
                .uv
        };
        assert_eq!(corner(bounds.left_top()), egui::pos2(0., 0.));
        assert_eq!(corner(bounds.right_top()), egui::pos2(0., 1.));
        assert!(
            !output
                .textures_delta
                .set
                .iter()
                .any(|(id, _)| *id == display)
        );
        for _ in 0..3 {
            draw(&mut app, &ctx, &mut time, size, Vec::new());
        }
        for id in [
            egui::Id::unique(("titlebar", "library-close")),
            egui::Id::unique("library-preview-loading"),
        ] {
            assert!(
                ctx.content_rect()
                    .contains_rect(ctx.read_response(id).unwrap().rect)
            );
        }
        for action in [
            "next", "zoom-in", "actual", "trash", "share", "metadata", "bookmark",
        ] {
            app.run(action, &ctx);
        }
        assert_eq!(app.zoom, 1.);
        assert!(!app.inspector.open);
        assert!(app.library.dialog.is_none());
        assert!(app.library.opening_preview_active());
    }
}

#[test]
fn imported_page_and_content_must_match_before_retaining_a_display_preview() {
    for scenario in [
        "same",
        "deleted",
        "modified",
        "reordered",
        "explicit-reordered",
    ] {
        let (root, mut app, ctx, batch, target, _) = fixture();
        load(&mut app, &ctx, batch);
        app.preserve_current_photo_preview(&target);
        assert!(app.library.opening_preview_active());
        let pages: &[(&str, u64)] = match scenario {
            "deleted" => &[("second.png", 7)],
            "modified" => &[("first.png", 8)],
            "reordered" | "explicit-reordered" => &[("second.png", 7), ("first.png", 7)],
            _ => &[("first.png", 7)],
        };
        let requested = (scenario != "reordered").then_some(target.as_path());
        let next = imported(root.path(), pages, requested);
        app.accept_mobile_selection(next);
        let matches = matches!(scenario, "same" | "explicit-reordered");
        assert_eq!(app.library.opening_preview_active(), matches, "{scenario}");
        if matches {
            assert_eq!(app.page_location(&app.files[app.focus_index]), target);
            assert!(
                !app.library
                    .opening_preview
                    .as_ref()
                    .unwrap()
                    .waiting_for_import
            );
            // Folder sorting can still move the final focus after acceptance.
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.listing.is_some() {
                assert!(Instant::now() < deadline);
                app.collect(&ctx);
                std::thread::sleep(Duration::from_millis(5));
            }
            if app.files.len() > 1 {
                app.focus_index = app
                    .files
                    .iter()
                    .position(|path| app.page_location(path) != target)
                    .unwrap();
                assert!(!app.preview_matches_selection(true));
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    app.library_opening_preview_window(ui.ctx());
                });
                assert!(!app.library.opening_preview_active());
            }
        }
    }
}
