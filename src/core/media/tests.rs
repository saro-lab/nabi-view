use super::*;

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
