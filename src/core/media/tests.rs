use super::*;

#[test]
fn deferred_original_copy_does_not_invalidate_the_folder_snapshot() {
    use std::io::Write;
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let mut batch = crate::core::documents::ImportBatch::new_folder(root.path()).unwrap();
    batch
        .add_deferred(
            "large.png",
            0,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100)),
            Arc::new(|file, _, _| {
                file.write_all(b"original bytes")
                    .map_err(|error| error.to_string())
            }),
        )
        .unwrap();
    let before = scan_snapshot(batch.directory(), &|| true).unwrap();
    crate::core::documents::materialize(batch.first().unwrap(), &|| true).unwrap();
    assert_eq!(before, scan_snapshot(batch.directory(), &|| true).unwrap());
}

fn scheduler_frame() -> Frame {
    Frame {
        hdr: None,
        animation: None,
        image: egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
        original_size: [1, 1],
    }
}

fn provider_cache_batch(
    root: &Path,
    asset: &[u8],
    resource: &[u8],
    modified: Option<SystemTime>,
    permitted: Arc<std::sync::atomic::AtomicBool>,
    revoke_during_copy: bool,
) -> (
    crate::core::documents::ImportBatch,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    use crate::core::documents::{ImportBatch, ProviderCache};
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();
    let copies = Arc::new(AtomicUsize::new(0));
    let copied = copies.clone();
    let allowed = permitted.clone();
    let mut opaque = b"test-provider-v1".to_vec();
    for part in [asset, resource] {
        opaque.extend((part.len() as u64).to_le_bytes());
        opaque.extend(part);
    }
    let identity = ProviderCache::new(
        opaque,
        modified,
        Arc::new(move || allowed.load(Ordering::Acquire)),
    );
    let mut batch = ImportBatch::new_folder(root).unwrap();
    batch
        .add_deferred_with_cache(
            "photo.png",
            0,
            modified,
            identity,
            Arc::new(move |file, _, live| {
                copied.fetch_add(1, Ordering::Relaxed);
                assert!(live());
                file.write_all(&png).map_err(|error| error.to_string())?;
                if revoke_during_copy {
                    permitted.store(false, Ordering::Release);
                    assert!(!live());
                }
                Ok(())
            }),
        )
        .unwrap();
    (batch, copies)
}

fn request_provider_frame(media: &Media, path: &Path) -> Result<Frame, String> {
    let expected = media.request_image_in_slot(0, path.to_owned(), 512);
    match next_event(media) {
        MediaEvent::Image { id, result, .. } => {
            assert_eq!(id, expected);
            result
        }
        _ => panic!("expected a visible image result"),
    }
}

fn thumbnail_provider_batch(
    root: &Path,
    thumb_calls: Arc<std::sync::atomic::AtomicUsize>,
    copies: Arc<std::sync::atomic::AtomicUsize>,
    allowed: Arc<std::sync::atomic::AtomicBool>,
    fail_or_revoke: Option<bool>,
) -> crate::core::documents::ImportBatch {
    use crate::core::documents::{ImportBatch, ProviderCache};
    use std::{io::Write, sync::atomic::Ordering};
    let modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100));
    let permitted = allowed.clone();
    let cache = ProviderCache::new(
        b"thumbnail-provider-test".to_vec(),
        modified,
        Arc::new(move || permitted.load(Ordering::Acquire)),
    )
    .unwrap()
    .with_thumbnail(Arc::new(move |edge, live| {
        assert!(live());
        assert_eq!(edge, 192);
        thumb_calls.fetch_add(1, Ordering::Relaxed);
        match fail_or_revoke {
            Some(false) => return Err("No local thumbnail".into()),
            Some(true) => allowed.store(false, Ordering::Release),
            None => {}
        }
        Ok(egui::ColorImage::filled([2, 1], egui::Color32::RED))
    }));
    let mut batch = ImportBatch::new_folder(root).unwrap();
    batch
        .add_deferred_with_cache(
            "photo.png",
            0,
            modified,
            Some(cache),
            Arc::new(move |file, _, _| {
                copies.fetch_add(1, Ordering::Relaxed);
                let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 255, 255]));
                let mut png = std::io::Cursor::new(Vec::new());
                image.write_to(&mut png, image::ImageFormat::Png).unwrap();
                file.write_all(png.get_ref())
                    .map_err(|error| error.to_string())
            }),
        )
        .unwrap();
    batch
}

fn request_provider_thumbnail(media: &Media, path: &Path) -> Result<Frame, String> {
    let expected = media.request_thumbnail(path.to_owned(), 192);
    match next_event(media) {
        MediaEvent::Thumbnail { id, result, .. } => {
            assert_eq!(id, expected);
            result
        }
        _ => panic!("expected a thumbnail result"),
    }
}

#[test]
fn provider_thumbnail_avoids_original_copy_and_never_satisfies_display_cache() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 1024 * 1024);
    let media = Media::new(egui::Context::default());
    media.configure_cache(Some(cache.clone()));
    let thumbs = Arc::new(AtomicUsize::new(0));
    let copies = Arc::new(AtomicUsize::new(0));
    let batch = thumbnail_provider_batch(
        root.path(),
        thumbs.clone(),
        copies.clone(),
        Arc::new(AtomicBool::new(true)),
        None,
    );
    let path = batch.first().unwrap();
    assert!(crate::core::documents::can_warm_thumbnail(path));
    assert_eq!(
        request_provider_thumbnail(&media, path)
            .unwrap()
            .image
            .pixels[0],
        egui::Color32::RED
    );
    assert_eq!(copies.load(Ordering::Relaxed), 0);
    assert_eq!(fs::metadata(path).unwrap().len(), 0);
    let key = cache.provider_thumbnail_key(path, 192).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while cache.load(&key).is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        cache
            .load(&cache.key(path, display_tier(192)).unwrap())
            .is_none()
    );
    drop(batch);
    // A fresh authorized capability can recover the thumbnail after restart,
    // still without opening the original or invoking the native image API.
    let batch = thumbnail_provider_batch(
        root.path(),
        thumbs.clone(),
        copies.clone(),
        Arc::new(AtomicBool::new(true)),
        None,
    );
    let path = batch.first().unwrap();
    assert_eq!(
        request_provider_thumbnail(&media, path)
            .unwrap()
            .image
            .pixels[0],
        egui::Color32::RED
    );
    assert_eq!(thumbs.load(Ordering::Relaxed), 1);
    assert_eq!(copies.load(Ordering::Relaxed), 0);
    media.request_image_in_slot(0, path.to_owned(), 192);
    match next_event(&media) {
        MediaEvent::Image { result, .. } => {
            assert_eq!(result.unwrap().image.pixels[0], egui::Color32::BLUE)
        }
        _ => panic!("expected display image"),
    }
    assert_eq!(copies.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_or_revoked_provider_thumbnail_never_falls_back_to_original() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    for revoke in [false, true] {
        let media = Media::new(egui::Context::default());
        let copies = Arc::new(AtomicUsize::new(0));
        let batch = thumbnail_provider_batch(
            root.path(),
            Arc::new(AtomicUsize::new(0)),
            copies.clone(),
            Arc::new(AtomicBool::new(true)),
            Some(revoke),
        );
        assert!(request_provider_thumbnail(&media, batch.first().unwrap()).is_err());
        assert_eq!(copies.load(Ordering::Relaxed), 0);
        assert_eq!(fs::metadata(batch.first().unwrap()).unwrap().len(), 0);
    }
}

#[test]
fn unknown_provider_stamp_blocks_thumbnail_copy_but_allows_current_original() {
    use crate::core::documents::{ImportBatch, ProviderCache};
    use std::{
        io::Write,
        sync::atomic::{AtomicUsize, Ordering},
    };
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let copies = Arc::new(AtomicUsize::new(0));
    let copied = copies.clone();
    let mut batch = ImportBatch::new_folder(root.path()).unwrap();
    let capability = ProviderCache::without_identity(Arc::new(|| true))
        .with_thumbnail(Arc::new(|_, _| Err("Unknown change stamp".into())));
    batch
        .add_deferred_with_cache(
            "photo.png",
            0,
            None,
            Some(capability),
            Arc::new(move |file, _, _| {
                copied.fetch_add(1, Ordering::Relaxed);
                let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 255, 255]));
                let mut png = std::io::Cursor::new(Vec::new());
                image.write_to(&mut png, image::ImageFormat::Png).unwrap();
                file.write_all(png.get_ref())
                    .map_err(|error| error.to_string())
            }),
        )
        .unwrap();
    let path = batch.first().unwrap();
    let media = Media::new(egui::Context::default());
    assert!(!crate::core::documents::can_warm_thumbnail(path));
    assert!(request_provider_thumbnail(&media, path).is_err());
    assert_eq!(copies.load(Ordering::Relaxed), 0);
    assert_eq!(
        request_provider_frame(&media, path).unwrap().image.pixels[0],
        egui::Color32::BLUE
    );
    assert_eq!(copies.load(Ordering::Relaxed), 1);
}

fn seed_provider_display_cache(
    root: &Path,
    cache: &crate::core::disk_cache::Cache,
    media: &Media,
) -> (crate::core::documents::ImportBatch, Frame) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (batch, copies) = provider_cache_batch(
        root,
        b"asset-1",
        b"photo:original.png",
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100)),
        Arc::new(AtomicBool::new(true)),
        false,
    );
    let frame = request_provider_frame(media, batch.first().unwrap()).unwrap();
    assert_eq!(copies.load(Ordering::Relaxed), 1);
    let key = cache
        .key(batch.first().unwrap(), display_tier(512))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while cache.load(&key).is_none() {
        assert!(
            Instant::now() < deadline,
            "display cache writer did not finish"
        );
        thread::sleep(Duration::from_millis(5));
    }
    (batch, frame)
}

#[test]
fn provider_display_cache_survives_new_batch_without_copying_original() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 1024 * 1024);
    let media = Media::new(egui::Context::default());
    media.configure_cache(Some(cache.clone()));
    let (first, original) = seed_provider_display_cache(root.path(), &cache, &media);
    let first_directory = first.directory().to_owned();
    drop(first); // Simulate a new import/session with no surviving old capability.
    let (second, copies) = provider_cache_batch(
        root.path(),
        b"asset-1",
        b"photo:original.png",
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100)),
        Arc::new(AtomicBool::new(true)),
        false,
    );
    assert_ne!(first_directory, second.directory());
    let frame = request_provider_frame(&media, second.first().unwrap()).unwrap();
    assert_eq!(frame.image.pixels, original.image.pixels);
    assert_eq!(frame.original_size, original.original_size);
    assert_eq!(copies.load(Ordering::Relaxed), 0);
    assert_eq!(fs::metadata(second.first().unwrap()).unwrap().len(), 0);
}

#[test]
fn provider_display_cache_misses_changed_unknown_and_disabled_sources() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 1024 * 1024);
    let media = Media::new(egui::Context::default());
    media.configure_cache(Some(cache.clone()));
    let (first, _) = seed_provider_display_cache(root.path(), &cache, &media);
    let original_key = cache
        .key(first.first().unwrap(), display_tier(512))
        .unwrap();
    drop(first);
    let modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100));
    for (asset, resource, date, enabled) in [
        (
            &b"asset-1"[..],
            &b"photo:original.png"[..],
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(101)),
            true,
        ),
        (b"asset-1", b"full-size:edited.png", modified, true),
        (b"asset-2", b"photo:original.png", modified, true),
        (b"asset-1", b"photo:original.png", None, true),
        (b"asset-1", b"photo:original.png", modified, false),
    ] {
        media.configure_cache(enabled.then(|| cache.clone()));
        let (batch, copies) = provider_cache_batch(
            root.path(),
            asset,
            resource,
            date,
            Arc::new(AtomicBool::new(true)),
            false,
        );
        request_provider_frame(&media, batch.first().unwrap()).unwrap();
        assert_eq!(
            copies.load(Ordering::Relaxed),
            1,
            "changed/unknown/disabled sources must copy normally"
        );
    }
    cache.clear().unwrap();
    cache
        .store(&original_key, &scheduler_frame().image, [1, 1])
        .unwrap();
    media.configure_cache(Some(cache.clone()));
    let (batch, copies) = provider_cache_batch(
        root.path(),
        b"asset-1",
        b"photo:original.png",
        modified,
        Arc::new(AtomicBool::new(true)),
        false,
    );
    request_provider_frame(&media, batch.first().unwrap()).unwrap();
    assert_eq!(
        copies.load(Ordering::Relaxed),
        1,
        "clear-all must invalidate stable display entries"
    );
}

#[test]
fn provider_display_cache_and_original_copy_reject_revoked_capabilities() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 1024 * 1024);
    let media = Media::new(egui::Context::default());
    media.configure_cache(Some(cache.clone()));
    let (first, _) = seed_provider_display_cache(root.path(), &cache, &media);
    drop(first);
    let modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100));
    let (batch, copies) = provider_cache_batch(
        root.path(),
        b"asset-1",
        b"photo:original.png",
        modified,
        Arc::new(AtomicBool::new(false)),
        false,
    );
    assert!(
        cache
            .key(batch.first().unwrap(), display_tier(512))
            .is_none()
    );
    assert!(request_provider_frame(&media, batch.first().unwrap()).is_err());
    assert_eq!(
        copies.load(Ordering::Relaxed),
        0,
        "revoked identity cannot fall back to cached or copied bytes"
    );
    let (batch, copies) = provider_cache_batch(
        root.path(),
        b"uncached-asset",
        b"photo:original.png",
        modified,
        Arc::new(AtomicBool::new(true)),
        true,
    );
    let key = cache
        .key(batch.first().unwrap(), display_tier(512))
        .unwrap();
    assert!(request_provider_frame(&media, batch.first().unwrap()).is_err());
    assert_eq!(copies.load(Ordering::Relaxed), 1);
    assert_eq!(
        fs::metadata(batch.first().unwrap()).unwrap().len(),
        0,
        "revoked in-flight copy must not publish an original"
    );
    assert!(cache.load(&key).is_none());
}

#[test]
fn visible_decode_reads_the_disk_cache_while_its_lock_is_held() {
    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let source = root.path().join("source.png");
    let pixel = image::Rgba([20, 40, 60, 255]);
    image::RgbaImage::from_pixel(2, 1, pixel)
        .save(&source)
        .unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 1024 * 1024);
    let key = cache.key(&source, display_tier(512)).unwrap();
    // A distinct cached pixel proves the visible worker read the cache entry
    // while a writer or pruner held the cache lock.
    cache
        .store(
            &key,
            &egui::ColorImage::filled([2, 1], egui::Color32::WHITE),
            [2, 1],
        )
        .unwrap();
    let held = fs::File::options()
        .read(true)
        .write(true)
        .open(cache.root().join("cache.lock"))
        .unwrap();
    held.lock().unwrap();
    let media = Media::new(egui::Context::default());
    media.configure_cache(Some(cache));
    let expected = media.request_image_in_slot(0, source, 512);
    let event = next_event(&media);
    drop(held);
    match event {
        MediaEvent::Image { id, result, .. } => {
            assert_eq!(id, expected);
            let frame = result.unwrap();
            assert_eq!(frame.original_size, [2, 1]);
            assert_eq!(frame.image.pixels, vec![egui::Color32::WHITE; 2]);
        }
        _ => panic!("the visible image must finish while the cache lock is held"),
    }
}

#[test]
fn visible_decode_passes_a_blocked_thumbnail_and_discards_its_stale_result() {
    use std::sync::mpsc;

    let media = Media(Arc::new(Shared::default()));
    let (started, receive_started) = mpsc::channel();
    let (release, receive_release) = mpsc::channel();
    let receive_release = Mutex::new(receive_release);
    spawn_decode_workers(&media.0, &egui::Context::default(), move |path, _, _| {
        if path == Path::new("slow.png") {
            started.send(()).unwrap();
            // Model a native codec that cannot cooperate with cancellation.
            receive_release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(scheduler_frame())
    });
    media.request_thumbnail("slow.png".into(), 192);
    receive_started
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let thumbnail = media.request_thumbnail("new.png".into(), 192);
    let visible = media.request_image_in_slot(0, "current.png".into(), 1024);
    match next_event(&media) {
        MediaEvent::Image { id, result, .. } => {
            assert_eq!(id, visible);
            assert!(result.is_ok());
        }
        _ => panic!("current page must finish while the old thumbnail is blocked"),
    }
    release.send(()).unwrap();
    match next_event(&media) {
        MediaEvent::Thumbnail { id, path, result } => {
            assert_eq!(id, thumbnail);
            assert_eq!(path, Path::new("new.png"));
            assert!(result.is_ok());
        }
        _ => panic!("only the latest thumbnail may be published"),
    }
}

#[test]
fn background_decode_waits_for_both_visible_pages_and_prioritizes_thumbnails() {
    use std::sync::mpsc;

    let media = Media(Arc::new(Shared::default()));
    // Submit in reverse priority order before starting either worker.
    media.request_prefetch("prefetch.png".into(), 1024);
    media.request_thumbnail("thumbnail.png".into(), 192);
    media.request_image_in_slot(1, "current.png".into(), 1024);
    media.request_image_in_slot(0, "mate.png".into(), 1024);
    let (started, receive_started) = mpsc::channel();
    let (release, receive_release) = mpsc::channel();
    let receive_release = Mutex::new(receive_release);
    spawn_decode_workers(&media.0, &egui::Context::default(), move |path, _, _| {
        started.send(path.to_owned()).unwrap();
        if path == Path::new("current.png") {
            receive_release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(scheduler_frame())
    });
    assert_eq!(
        receive_started
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        Path::new("current.png")
    );
    assert_eq!(
        receive_started.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "no new background decode may overlap a pending current page"
    );
    release.send(()).unwrap();
    for expected in ["mate.png", "thumbnail.png", "prefetch.png"] {
        assert_eq!(
            receive_started
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            Path::new(expected)
        );
    }
}

#[test]
fn finished_preempted_range_thumbnail_is_published_once() {
    use std::sync::{atomic::AtomicUsize, mpsc};

    let media = Media(Arc::new(Shared::default()));
    let (started, receive_started) = mpsc::channel();
    let (release, receive_release) = mpsc::channel();
    let receive_release = Mutex::new(receive_release);
    let decodes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&decodes);
    spawn_decode_workers(&media.0, &egui::Context::default(), move |path, _, _| {
        if path == Path::new("range.png") {
            counted.fetch_add(1, AtomicOrdering::Relaxed);
            started.send(()).unwrap();
            // A native codec that finishes despite the preemption request.
            receive_release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(scheduler_frame())
    });
    let thumbnail = media.request_thumbnail_with("range.png".into(), 192, Urgency::RangeThumbnail);
    receive_started
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let visible = media.request_image_in_slot(0, "current.png".into(), 1024);
    match next_event(&media) {
        MediaEvent::Image { id, .. } => assert_eq!(id, visible),
        _ => panic!("the current page finishes first"),
    }
    release.send(()).unwrap();
    match next_event(&media) {
        MediaEvent::Thumbnail { id, result, .. } => {
            assert_eq!(id, thumbnail);
            assert!(result.is_ok(), "a finished decode is not thrown away");
        }
        _ => panic!("expected the preempted thumbnail"),
    }
    thread::sleep(Duration::from_millis(100));
    assert_eq!(decodes.load(AtomicOrdering::Relaxed), 1, "no second decode");
}

#[test]
fn reduced_jpeg_thumbnails_run_beside_a_busy_background_decoder() {
    use std::sync::mpsc;

    let media = Media(Arc::new(Shared::default()));
    let (started, receive_started) = mpsc::channel();
    let (release, receive_release) = mpsc::channel();
    let receive_release = Mutex::new(receive_release);
    spawn_decode_workers(&media.0, &egui::Context::default(), move |path, _, _| {
        started.send(path.to_owned()).unwrap();
        if path == Path::new("prefetch.png") {
            receive_release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(scheduler_frame())
    });
    media.request_prefetch_with("prefetch.png".into(), 1024, Urgency::Lead);
    let next = || {
        receive_started
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
    };
    assert_eq!(next(), Path::new("prefetch.png"));
    // A full-size thumbnail shares the busy background decoder...
    media.request_thumbnail_with("full.png".into(), 192, Urgency::RangeThumbnail);
    assert_eq!(
        receive_started.recv_timeout(Duration::from_millis(150)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "at most one full-size background decode at a time"
    );
    // ...a reduced JPEG thumbnail does not wait for it.
    media.request_thumbnail_with("reduced.jpg".into(), 192, Urgency::RangeThumbnail);
    assert_eq!(next(), Path::new("reduced.jpg"));
    release.send(()).unwrap();
}

#[test]
fn background_decoder_runs_the_lead_page_before_range_thumbnails() {
    use std::sync::mpsc;

    let media = Media(Arc::new(Shared::default()));
    media.request_thumbnail_with("range.png".into(), 192, Urgency::RangeThumbnail);
    media.request_prefetch_with("lead.png".into(), 1024, Urgency::Lead);
    let (started, receive_started) = mpsc::channel();
    spawn_decode_workers(&media.0, &egui::Context::default(), move |path, _, _| {
        started.send(path.to_owned()).unwrap();
        Ok(scheduler_frame())
    });
    for expected in ["lead.png", "range.png"] {
        assert_eq!(
            receive_started
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            Path::new(expected)
        );
    }
}

#[test]
fn disk_warm_fills_the_cache_once_without_returning_pixels() {
    use std::sync::atomic::AtomicUsize;

    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let source = root.path().join("neighbor.png");
    image::RgbaImage::from_pixel(4, 2, image::Rgba([20, 40, 60, 255]))
        .save(&source)
        .unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 1024 * 1024);
    let media = Media(Arc::new(Shared::default()));
    media.configure_cache(Some(cache.clone()));
    let decodes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&decodes);
    spawn_decode_workers(
        &media.0,
        &egui::Context::default(),
        move |path, side, live| {
            counted.fetch_add(1, AtomicOrdering::Relaxed);
            decode(path, side, live)
        },
    );
    let warm = |media: &Media| {
        let id = media.request_disk_warm(source.clone(), 512, Urgency::Ahead);
        match next_event(media) {
            MediaEvent::Warmed {
                id: got, result, ..
            } => {
                assert_eq!(got, id);
                assert!(result.is_ok());
            }
            _ => panic!("a disk-only prefetch answers with Warmed"),
        }
    };
    warm(&media);
    let key = cache.key(&source, display_tier(512)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cache.contains(&key) {
        assert!(Instant::now() < deadline, "the entry reaches the disk");
        thread::sleep(Duration::from_millis(10));
    }
    warm(&media);
    assert_eq!(
        decodes.load(AtomicOrdering::Relaxed),
        1,
        "an existing entry is not decoded"
    );
    assert_eq!(cache.load(&key).unwrap().original_size, [4, 2]);
}

#[test]
fn resized_window_reuses_a_larger_display_tier_without_decoding() {
    use std::sync::atomic::AtomicUsize;

    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let source = root.path().join("page.png");
    image::RgbaImage::from_pixel(4, 2, image::Rgba([20, 40, 60, 255]))
        .save(&source)
        .unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 1024 * 1024);
    // An entry written by a larger window; white marks a cache hit.
    cache
        .store(
            &cache.key(&source, 2048).unwrap(),
            &egui::ColorImage::filled([4, 2], egui::Color32::WHITE),
            [4, 2],
        )
        .unwrap();
    let media = Media(Arc::new(Shared::default()));
    media.configure_cache(Some(cache));
    let decodes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&decodes);
    spawn_decode_workers(
        &media.0,
        &egui::Context::default(),
        move |path, side, live| {
            counted.fetch_add(1, AtomicOrdering::Relaxed);
            decode(path, side, live)
        },
    );
    // 1536 shares the 2048 tier; 900 falls back to the larger 2048 entry.
    for request in [1536, 900] {
        let id = media.request_image_in_slot(0, source.clone(), request);
        match next_event(&media) {
            MediaEvent::Image {
                id: got, result, ..
            } => {
                assert_eq!(got, id);
                assert_eq!(result.unwrap().image.pixels[0], egui::Color32::WHITE);
            }
            _ => panic!("expected the visible image"),
        }
    }
    assert_eq!(decodes.load(AtomicOrdering::Relaxed), 0);
    assert_eq!(display_tier(1536), 2048);
    assert_eq!(display_tier(900), 1024);
    assert_eq!(display_tier(4096), 4096);
}

#[test]
fn strip_thumbnail_lands_in_the_library_store_and_is_decoded_once() {
    use std::sync::atomic::AtomicUsize;

    let root = tempfile::tempdir_in(crate::test_support::root()).unwrap();
    let source = root.path().join("cover.png");
    image::RgbaImage::from_pixel(600, 300, image::Rgba([20, 40, 60, 255]))
        .save(&source)
        .unwrap();
    let cache = crate::core::disk_cache::Cache::new(root.path().join("cache"), 8 * 1024 * 1024);
    let media = Media(Arc::new(Shared::default()));
    media.configure_cache(Some(cache.clone()));
    let decodes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&decodes);
    spawn_decode_workers(
        &media.0,
        &egui::Context::default(),
        move |path, side, live| {
            counted.fetch_add(1, AtomicOrdering::Relaxed);
            decode(path, side, live)
        },
    );
    let thumbnail = |media: &Media| {
        let id = media.request_thumbnail(source.clone(), crate::core::library::THUMBNAIL_EDGE);
        match next_event(media) {
            MediaEvent::Thumbnail {
                id: got, result, ..
            } => {
                assert_eq!(got, id);
                result.unwrap().image.size
            }
            _ => panic!("expected a thumbnail"),
        }
    };
    assert_eq!(thumbnail(&media), [256, 128]);
    // The library list reads the same file under its own key.
    let key = crate::core::library::thumbnail_key(&source).unwrap();
    let listed = crate::core::library::load_thumbnail(&cache.thumbnail_dir(), &key).unwrap();
    assert_eq!(listed.size, [256, 128]);
    assert!(
        !root.path().join("cache").join("display").exists(),
        "no second copy in the display cache"
    );
    assert_eq!(thumbnail(&media), [256, 128]);
    assert_eq!(
        decodes.load(AtomicOrdering::Relaxed),
        1,
        "one decode serves both"
    );
}
#[test]
fn os_decoder_original_size_follows_its_upright_output() {
    // Header size before EXIF rotation, output after it: swap.
    assert_eq!(upright_original([4000, 3000], [1536, 2048]), [3000, 4000]);
    // Already in the same orientation, square output or square source: keep.
    assert_eq!(upright_original([3000, 4000], [1536, 2048]), [3000, 4000]);
    assert_eq!(upright_original([4000, 3000], [256, 256]), [4000, 3000]);
    assert_eq!(upright_original([3000, 3000], [256, 192]), [3000, 3000]);
}
#[test]
fn expanded_formats_decode_and_corrupt_files_stay_in_folder() {
    let folder = tempfile::tempdir().unwrap();
    let image = image::RgbaImage::from_pixel(4, 3, image::Rgba([60, 120, 180, 255]));
    for ext in ["tiff", "ico", "tga", "qoi", "pam", "ff"] {
        let path = folder.path().join(format!("sample.{ext}"));
        if ext == "ff" {
            DynamicImage::ImageRgba8(image.clone())
                .to_rgba16()
                .save(&path)
                .unwrap();
        } else {
            image.save(&path).unwrap();
        }
        assert_eq!(decode(&path, 16, &|| true).unwrap().original_size, [4, 3]);
    }
    let svg = folder.path().join("vector.svg");
    fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3"><rect width="4" height="3" fill="red"/></svg>"#).unwrap();
    assert_eq!(decode(&svg, 16, &|| true).unwrap().original_size, [4, 3]);
    for ext in EXTENSIONS {
        let path = folder
            .path()
            .join(format!("broken.{}", ext.to_ascii_uppercase()));
        fs::write(&path, b"broken image").unwrap();
        assert!(is_supported(&path));
        assert!(decode(&path, 16, &|| true).is_err());
    }
    let media = Media::new(egui::Context::default());
    media.request_folder(folder.path().to_owned());
    match next_event(&media) {
        MediaEvent::Folder { result, .. } => {
            assert_eq!(result.unwrap().len(), EXTENSIONS.len() + 7)
        }
        _ => panic!("expected folder listing"),
    }
}

fn next_event(media: &Media) -> MediaEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = media.try_recv() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "folder watcher did not deliver an event"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn next_update(media: &Media, expected_id: u64) -> FolderUpdate {
    match next_event(media) {
        MediaEvent::FolderChanged { id, result, .. } => {
            assert_eq!(id, expected_id);
            result.unwrap()
        }
        _ => panic!("expected a folder update"),
    }
}

#[test]
fn folder_watch_tracks_add_rewrite_remove_and_recreation() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("watched");
    fs::create_dir(&folder).unwrap();
    let first = folder.join("1.png");
    let second = folder.join("2.png");
    let tenth = folder.join("10.PNG");
    fs::write(&first, b"first").unwrap();
    let media = Media::new(egui::Context::default());
    let generation = media.request_folder(folder.clone());
    match next_event(&media) {
        MediaEvent::Folder { id, result, .. } => {
            assert_eq!(id, generation);
            assert_eq!(result.unwrap(), vec![first.clone()]);
        }
        _ => panic!("expected initial listing"),
    }
    fs::write(&tenth, b"ten").unwrap();
    let update = next_update(&media, generation);
    assert_eq!(update.files, vec![first.clone(), tenth.clone()]);
    assert_eq!(update.changed, vec![tenth.clone()]);
    fs::write(&second, b"two").unwrap();
    let update = next_update(&media, generation);
    assert_eq!(
        update.files,
        vec![first.clone(), second.clone(), tenth.clone()]
    );
    assert!(update.changed.contains(&second));
    assert!(!update.changed.contains(&tenth));
    assert!(!update.changed.contains(&first));

    fs::write(&second, b"new contents, deliberately different length").unwrap();
    let update = next_update(&media, generation);
    assert_eq!(update.changed, vec![second.clone()]);

    fs::remove_file(&first).unwrap();
    fs::remove_file(&second).unwrap();
    fs::remove_file(&tenth).unwrap();
    fs::remove_dir(&folder).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !next_update(&media, generation).files.is_empty() {
        assert!(Instant::now() < deadline, "folder deletion did not settle");
    }
    fs::create_dir(&folder).unwrap();
    fs::write(&second, b"recreated").unwrap();
    let update = next_update(&media, generation);
    assert_eq!(update.files, vec![second.clone()]);
    assert_eq!(update.changed, vec![second]);
}

#[test]
fn switching_folder_discards_queued_old_generation() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(first.path().join("old.png"), b"old").unwrap();
    let new_file = second.path().join("new.png");
    fs::write(&new_file, b"new").unwrap();
    let media = Media::new(egui::Context::default());
    media.request_folder(first.path().to_owned());
    let generation = media.request_folder(second.path().to_owned());
    match next_event(&media) {
        MediaEvent::Folder { id, path, result } => {
            assert_eq!(id, generation);
            assert_eq!(path, second.path());
            assert_eq!(result.unwrap(), vec![new_file]);
        }
        _ => panic!("expected latest folder listing"),
    }
}

#[test]
fn coalescing_preserves_invalidations_but_drops_removed_paths() {
    let mut q = Queues::default();
    let job = Job {
        id: 1,
        path: PathBuf::from("folder"),
        max_side: 0,
        urgency: Urgency::default(),
        disk_only: false,
    };
    let a = PathBuf::from("a.png");
    let b = PathBuf::from("b.png");
    publish_folder_update(
        &mut q,
        &job,
        FolderUpdate {
            files: vec![a.clone(), b.clone()],
            changed: vec![a.clone(), b],
        },
    );
    publish_folder_update(
        &mut q,
        &job,
        FolderUpdate {
            files: vec![a.clone()],
            changed: vec![],
        },
    );
    match q.ready[WATCH_SLOT].take().unwrap() {
        MediaEvent::FolderChanged {
            result: Ok(update), ..
        } => {
            assert_eq!(update.files, vec![a.clone()]);
            assert_eq!(update.changed, vec![a]);
        }
        _ => panic!("expected coalesced snapshot"),
    }
}

#[test]
fn snapshot_ignores_unrelated_files_and_supported_directory_names() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("1.png");
    fs::write(&path, b"image").unwrap();
    let before = scan_snapshot(folder.path(), &|| true).unwrap();
    fs::write(folder.path().join("notes.txt"), b"unrelated").unwrap();
    fs::create_dir(folder.path().join("directory.png")).unwrap();
    assert_eq!(scan_snapshot(folder.path(), &|| true).unwrap(), before);
}

#[test]
fn rotated_jpeg_reports_upright_dimensions_after_scaling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("camera.jpg");
    DynamicImage::new_rgb8(20, 10).save(&path).unwrap();
    let jpeg = fs::read(&path).unwrap();
    // APP1 with a little-endian TIFF IFD holding orientation 6 (90°).
    let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut oriented = jpeg[..2].to_vec();
    oriented.extend_from_slice(&[0xff, 0xe1]);
    oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    oriented.extend_from_slice(exif);
    oriented.extend_from_slice(&jpeg[2..]);
    fs::write(&path, &oriented).unwrap();
    let frame = decode(&path, 8, &|| true).unwrap();
    assert_eq!(frame.original_size, [10, 20]);
    assert_eq!(frame.image.size, [4, 8]);
}

#[test]
fn natural_page_order_handles_leading_zeroes_and_long_numbers() {
    let mut names = [
        "p10.PNG",
        "p2.png",
        "p0002.png",
        "p100000000000000000000.png",
        "p1.jpg",
    ];
    names.sort_by(|a, b| natural_cmp(a.as_bytes(), b.as_bytes()).then_with(|| a.cmp(b)));
    assert_eq!(
        names,
        [
            "p1.jpg",
            "p0002.png",
            "p2.png",
            "p10.PNG",
            "p100000000000000000000.png"
        ]
    );
}

#[test]
fn output_fits_both_texture_and_pixel_limits() {
    assert_eq!(output_size(4000, 2000, 1000), [1000, 500]);
    assert_eq!(output_size(1, 32000, 8192), [1, 8192]);
    let [w, h] = output_size(8000, 8000, 8192);
    assert!(u64::from(w) * u64::from(h) <= MAX_OUTPUT_PIXELS);
}

fn write_jpeg(path: &Path, image: &DynamicImage) {
    let file = std::io::BufWriter::new(fs::File::create(path).unwrap());
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, 95)
        .encode_image(image)
        .unwrap();
}

fn textured(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let wave = ((fx * 23.).sin() * (fy * 17.).cos() * 35.) as i32;
        image::Rgb([
            (60 + (150. * fx) as i32 + wave).clamp(0, 255) as u8,
            (40 + (170. * fy) as i32 - wave).clamp(0, 255) as u8,
            (200 - (120. * fx * fy) as i32 + wave / 2).clamp(0, 255) as u8,
        ])
    }))
}

#[test]
fn reduced_jpeg_decoding_matches_full_decoding_within_tolerance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.jpg");
    let source = textured(1600, 1200);
    write_jpeg(&path, &source);
    for side in [192, 256, 600] {
        // The reduced path must actually handle these requests.
        let frame = jpeg::decode_scaled(&path, side, &|| true).unwrap().unwrap();
        let [out_w, out_h] = output_size(1600, 1200, side);
        assert_eq!(frame.image.size, [out_w as usize, out_h as usize]);
        assert_eq!(frame.original_size, [1600, 1200]);
        let reference = image::open(&path)
            .unwrap()
            .thumbnail_exact(out_w, out_h)
            .into_rgb8();
        let total: u64 = frame
            .image
            .pixels
            .iter()
            .zip(reference.pixels())
            .map(|(a, b)| {
                let a = a.to_srgba_unmultiplied();
                (0..3).map(|i| u64::from(a[i].abs_diff(b[i]))).sum::<u64>()
            })
            .sum();
        let mean = total as f64 / (f64::from(out_w * out_h) * 3.);
        assert!(mean < 3.0, "side {side}: mean channel error {mean}");
        assert_eq!(
            decode(&path, side, &|| true).unwrap().image.size,
            frame.image.size
        );
    }
}

#[test]
fn full_size_jpeg_requests_stay_on_the_general_decoder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("small.jpg");
    write_jpeg(&path, &textured(300, 200));
    assert!(jpeg::decode_scaled(&path, 300, &|| true).unwrap().is_none());
    assert!(jpeg::decode_scaled(&path, 200, &|| true).unwrap().is_none());
    assert_eq!(decode(&path, 300, &|| true).unwrap().image.size, [300, 200]);
    // Not actually a JPEG: the general decoder reports the format by content.
    let disguised = dir.path().join("disguised.jpg");
    DynamicImage::new_rgb8(40, 20)
        .save_with_format(&disguised, image::ImageFormat::Png)
        .unwrap();
    assert!(
        jpeg::decode_scaled(&disguised, 8, &|| true)
            .unwrap()
            .is_none()
    );
    assert_eq!(decode(&disguised, 8, &|| true).unwrap().image.size, [8, 4]);
}

#[test]
fn reduced_jpeg_decoding_applies_exif_orientation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("camera.jpg");
    write_jpeg(&path, &textured(200, 100));
    let jpeg = fs::read(&path).unwrap();
    let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut oriented = jpeg[..2].to_vec();
    oriented.extend_from_slice(&[0xff, 0xe1]);
    oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    oriented.extend_from_slice(exif);
    oriented.extend_from_slice(&jpeg[2..]);
    fs::write(&path, &oriented).unwrap();
    let frame = jpeg::decode_scaled(&path, 40, &|| true).unwrap().unwrap();
    assert_eq!(frame.original_size, [100, 200]);
    assert_eq!(frame.image.size, [20, 40]);
}

#[test]
fn jpeg_above_the_full_decode_pixel_limit_opens_reduced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.jpg");
    // 65.5 million pixels: above MAX_PIXELS, below MAX_SCALED_PIXELS.
    let (w, h) = (16_384, 4_000);
    assert!(u64::from(w) * u64::from(h) > MAX_PIXELS);
    write_jpeg(
        &path,
        &DynamicImage::ImageLuma8(image::GrayImage::from_fn(w, h, |x, _| {
            image::Luma([(x % 256) as u8])
        })),
    );
    let thumbnail = decode(&path, 256, &|| true).unwrap();
    assert_eq!(thumbnail.original_size, [w, h]);
    assert_eq!(thumbnail.image.size, [256, 62]);
    let screen = decode(&path, 4096, &|| true).unwrap();
    assert_eq!(screen.image.size, [4096, 1000]);
}

#[test]
fn cancelled_reduced_jpeg_decoding_reports_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.jpg");
    write_jpeg(&path, &textured(800, 600));
    assert_eq!(
        jpeg::decode_scaled(&path, 100, &|| false).err().as_deref(),
        Some("Cancelled")
    );
}

#[cfg(any(windows, target_os = "macos", target_os = "ios"))]
fn oriented_jpeg(path: &Path, w: u32, h: u32) {
    write_jpeg(path, &textured(w, h));
    let jpeg = fs::read(path).unwrap();
    let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut oriented = jpeg[..2].to_vec();
    oriented.extend_from_slice(&[0xff, 0xe1]);
    oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    oriented.extend_from_slice(exif);
    oriented.extend_from_slice(&jpeg[2..]);
    fs::write(path, &oriented).unwrap();
}

#[cfg(any(windows, target_os = "macos", target_os = "ios"))]
#[test]
fn native_scaled_decoding_applies_orientation_and_matches_rust_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("camera.jpg");
    oriented_jpeg(&path, 400, 200);
    let scaled = crate::platform::platform_image::decode_scaled(&path, 100, 6 * 1024 * 1024)
        .unwrap()
        .expect("The platform ships a JPEG codec");
    assert_eq!(scaled.original, [200, 400]);
    assert_eq!(scaled.image.dimensions(), (50, 100));
    // Same picture as the Rust decoder within resampling noise.
    let rust = jpeg::decode_scaled(&path, 100, &|| true).unwrap().unwrap();
    assert_eq!(rust.image.size, [50, 100]);
    let error: f64 = scaled
        .image
        .pixels()
        .zip(&rust.image.pixels)
        .map(|(wic, rust)| {
            let rust = rust.to_srgba_unmultiplied();
            (0..3)
                .map(|c| (f64::from(wic[c]) - f64::from(rust[c])).abs())
                .sum::<f64>()
                / 3.0
        })
        .sum::<f64>()
        / (50.0 * 100.0);
    assert!(error < 8.0, "mean channel error {error}");
    // Over the pixel limit is refused, not decoded at full size.
    assert_eq!(
        crate::platform::platform_image::decode_scaled(&path, 4096, 100)
            .unwrap()
            .unwrap()
            .image
            .dimensions(),
        (7, 14)
    );
}
