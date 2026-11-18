use super::*;
use crate::core::documents::ProviderCache;
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

struct Source {
    batch: ImportBatch,
    copies: Arc<AtomicUsize>,
    allowed: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}

fn source(root: &Path) -> Source {
    let mut batch = ImportBatch::new_folder(&root.join("imports")).unwrap();
    batch
        .set_source(&root.join("document-sources"), "nabi-ios-library", "Photos")
        .unwrap();
    batch.request_page(Some(batch.source().unwrap().root.join("selected.png")));
    batch.set_ios_access_stamp(7, crate::core::ios_access::PhotoStatus::Authorized);
    let source = Source {
        batch,
        copies: Arc::new(AtomicUsize::new(0)),
        allowed: Arc::new(AtomicBool::new(true)),
        release: Arc::new(AtomicBool::new(false)),
    };
    add(source, "selected.png")
}

fn add(mut source: Source, name: &str) -> Source {
    let pixels = image::RgbaImage::from_pixel(32, 24, image::Rgba([20, 70, 180, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let bytes = png.into_inner();
    let modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100));
    let valid = source.allowed.clone();
    let provider = ProviderCache::new(
        name.as_bytes().to_vec(),
        modified,
        Arc::new(move || valid.load(Ordering::Acquire)),
    )
    .unwrap();
    let copies = source.copies.clone();
    let release = source.release.clone();
    source
        .batch
        .add_deferred_with_cache(
            name,
            bytes.len() as u64,
            modified,
            Some(provider),
            Arc::new(move |output, _, live| {
                copies.fetch_add(1, Ordering::AcqRel);
                let deadline = Instant::now() + Duration::from_secs(5);
                while !release.load(Ordering::Acquire) {
                    if !live() {
                        return Err("Cancelled".into());
                    }
                    assert!(Instant::now() < deadline, "test source was never released");
                    std::thread::sleep(Duration::from_millis(1));
                }
                if !live() {
                    return Err("Cancelled".into());
                }
                output.write_all(&bytes).map_err(|error| error.to_string())
            }),
        )
        .unwrap();
    source
}

fn viewer(root: &Path) -> (Viewer, egui::Context) {
    let ctx = egui::Context::default();
    let app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        None,
        Ok(Store::in_directories(
            root.join("config"),
            root.join("state"),
        )),
    );
    (app, ctx)
}

fn wait(app: &mut Viewer, ctx: &egui::Context, condition: impl Fn(&Viewer) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition(app) {
        assert!(
            Instant::now() < deadline,
            "seed display did not complete: {:?}",
            app.failures
        );
        app.collect(ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn painted(app: &mut Viewer, ctx: &egui::Context, texture: egui::TextureId) -> bool {
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(390., 844.),
            )),
            ..Default::default()
        },
        |ui| app.canvas(ui),
    );
    ctx.tessellate(output.shapes, output.pixels_per_point).iter().any(|shape| {
        matches!(&shape.primitive, egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture)
    })
}

fn assert_partial_commands(app: &mut Viewer, ctx: &egui::Context) {
    assert!(app.source_list_pending());
    for command in ["next", "last", "random", "bookmark"] {
        assert!(!app.command_available(command), "{command}");
    }
    let focus = app.focus_index;
    let ticket = app.tickets[0].clone();
    app.run("next", ctx);
    app.move_page(ctx, true);
    assert_eq!(app.focus_index, focus);
    assert_eq!(app.tickets[0], ticket);
    assert!(app.listing.is_none() && app.folder_watch.is_none());
    assert!(app.library.remember.is_none());
    assert!(app.warm_thumbnail_candidate().is_none());
    app.queue_prefetch();
    assert!(app.prefetch_ticket.is_none());
}

#[test]
fn seed_uses_exact_display_cache_and_paints_before_full_list_without_original_copy() {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let source = source(root.path());
    let selected = source.batch.first().unwrap().to_owned();
    let (mut app, ctx) = viewer(root.path());
    let cache = crate::core::disk_cache::Cache::new(root.path().join("display-cache"), 1024 * 1024);
    let key = cache
        .key(&selected, media::display_tier(app.fit_limit))
        .unwrap();
    let pixels = egui::ColorImage::filled([32, 24], Color32::from_rgb(20, 70, 180));
    cache.store(&key, &pixels, [32, 24]).unwrap();
    app.media.configure_cache(Some(cache));
    app.accept_mobile_seed(source.batch.snapshot_seed().unwrap());
    assert_partial_commands(&mut app, &ctx);
    wait(&mut app, &ctx, |app| app.pictures.contains_key(&selected));
    let texture = app.pictures[&selected].handle.id();
    assert!(painted(&mut app, &ctx, texture));
    assert!(app.source_list_pending());
    assert_eq!(source.copies.load(Ordering::Acquire), 0);
    assert_eq!(std::fs::metadata(&selected).unwrap().len(), 0);

    let mut full = add(source, "neighbor.png");
    full.batch.commit_source().unwrap();
    app.accept_mobile_selection(full.batch);
    assert!(!app.source_list_pending());
    assert_eq!(app.files.len(), 2);
    assert_eq!(app.files[app.focus_index], selected);
    assert_eq!(app.pictures[&selected].handle.id(), texture);
    assert!(painted(&mut app, &ctx, texture));
    for command in ["next", "last", "random", "bookmark"] {
        assert!(app.command_available(command), "{command}");
    }
    assert!(app.listing.is_some() && app.library.remember.is_some());
    assert_eq!(full.copies.load(Ordering::Acquire), 0);
}

#[test]
fn completing_seed_preserves_live_ticket_focus_and_single_original_copy() {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let source = source(root.path());
    let selected = source.batch.first().unwrap().to_owned();
    let (mut app, ctx) = viewer(root.path());
    app.settings.sort_descending = true;
    app.accept_mobile_seed(source.batch.snapshot_seed().unwrap());
    assert_partial_commands(&mut app, &ctx);
    let ticket = app.tickets[0].clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    while source.copies.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut full = add(source, "zz-neighbor.png");
    full.batch.commit_source().unwrap();
    app.accept_mobile_selection(full.batch);
    assert_eq!(app.tickets[0], ticket);
    assert_eq!(app.files[app.focus_index], selected);
    assert!(app.settings.sort_descending);
    full.release.store(true, Ordering::Release);
    wait(&mut app, &ctx, |app| {
        app.pictures.contains_key(&selected) && app.listing.is_none()
    });
    assert_eq!(app.files[app.focus_index], selected);
    assert_eq!(full.copies.load(Ordering::Acquire), 1);
    let texture = app.pictures[&selected].handle.id();
    assert!(painted(&mut app, &ctx, texture));
}

#[test]
fn cancelling_or_revoking_a_seed_discards_pixels_tickets_and_stale_results() {
    for ready in [false, true] {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let source = source(root.path());
        let selected = source.batch.first().unwrap().to_owned();
        let (mut app, ctx) = viewer(root.path());
        source.release.store(ready, Ordering::Release);
        app.accept_mobile_seed(source.batch.snapshot_seed().unwrap());
        if ready {
            wait(&mut app, &ctx, |app| app.pictures.contains_key(&selected));
        }
        source.allowed.store(false, Ordering::Release);
        if ready {
            assert!(app.handle_back(&ctx));
        } else {
            app.discard_mobile_seed();
        }
        source.release.store(true, Ordering::Release);
        for _ in 0..20 {
            app.collect(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.files.is_empty() && app.pictures.is_empty());
        assert!(app.tickets.iter().all(Option::is_none));
        assert!(app.mobile_selection.is_none());
        assert!(app.library.open);
        assert!(app.presentation.images.is_empty());
    }
}

#[test]
fn seed_completion_rechecks_authorization_revision_and_source_before_preserving_pixels() {
    use crate::core::ios_access::{AccessIssue, PhotoStatus};
    for (status, revision, other_source, issue, expected) in [
        (PhotoStatus::Authorized, 7, false, None, true),
        (PhotoStatus::Denied, 7, false, None, false),
        (PhotoStatus::Limited, 7, false, None, false),
        (PhotoStatus::Authorized, 8, false, None, false),
        (PhotoStatus::Authorized, 7, true, None, false),
        (
            PhotoStatus::Authorized,
            7,
            false,
            Some(AccessIssue::PhotoRead),
            false,
        ),
    ] {
        let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
        let mut source = source(root.path());
        let selected = source.batch.first().unwrap().to_owned();
        let (mut app, ctx) = viewer(root.path());
        source.release.store(true, Ordering::Release);
        assert!(app.accept_ios_seed(
            source.batch.snapshot_seed().unwrap(),
            PhotoStatus::Authorized,
            7
        ));
        wait(&mut app, &ctx, |app| app.pictures.contains_key(&selected));
        let texture = app.pictures[&selected].handle.id();
        source.batch.commit_source().unwrap();
        let request = if other_source {
            root.path().join("document-sources/nabi-ios-screenshots")
        } else {
            source.batch.source().unwrap().root.clone()
        };
        assert_eq!(
            app.accept_ios_selection(
                source.batch,
                Some((Some(request), issue, 7)),
                status,
                revision
            ),
            expected
        );
        if expected {
            assert!(!app.source_list_pending());
            assert_eq!(app.pictures[&selected].handle.id(), texture);
        } else {
            assert!(app.files.is_empty() && app.pictures.is_empty());
            assert!(app.mobile_selection.is_none());
            assert!(app.library.open);
        }
    }
    // Even a published seed must be rechecked when its UI event is consumed.
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let source = source(root.path());
    let (mut app, _) = viewer(root.path());
    assert!(!app.accept_ios_seed(
        source.batch.snapshot_seed().unwrap(),
        PhotoStatus::Limited,
        7
    ));
    assert!(app.files.is_empty() && app.tickets.iter().all(Option::is_none));
}
